//! Initial terminal attachment lifecycle; callbacks are fenced before any selected-view effects.
use super::{Command, Direct, Fetched, Following, Ui, connecting_text, screen_lines};
use crate::feed::TerminalUpdate;
use std::{collections::BTreeMap, future::Future, net::Shutdown, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use tokio::{sync::mpsc, task::JoinHandle};

// Local discovery is one daemon read; direct open includes the existing 4s Fabric dial.
pub(super) const DISCOVERY: Duration = Duration::from_secs(6);
pub(super) const DIRECT: Duration = Duration::from_secs(6);
// The gateway crosses a paired-device connection and may forward to a remote owner.
pub(super) const GATEWAY: Duration = Duration::from_secs(8);
// One budget for all phases, not a fresh budget for every shell retry.
pub(super) const DEADLINE: Duration = Duration::from_secs(DISCOVERY.as_secs() + DIRECT.as_secs() + GATEWAY.as_secs() * 2 + 3);
pub(super) const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(2)];

pub(super) async fn phase<T>(name: &str, budget: Duration, future: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::time::timeout(budget, future).await.map_err(|_| format!("{name} timed out"))?
}

#[derive(Default)]
pub(super) struct State {
    generation: u64,
    selected: Option<String>,
    pending: BTreeMap<String, u64>,
    serial: BTreeMap<String, Arc<tokio::sync::Semaphore>>,
    started: Option<Instant>,
    note: Option<String>,
    wanted: BTreeMap<String, Arc<AtomicBool>>,
    pub(super) tasks: BTreeMap<String, JoinHandle<()>>,
    pub(super) opens: BTreeMap<String, Instant>,
}

impl Drop for State {
    fn drop(&mut self) {
        for task in self.tasks.values() { task.abort(); }
    }
}

impl State {
    pub(super) fn cancel(&mut self, agent: &str) {
        self.pending.remove(agent);
        if let Some(task) = self.tasks.get(agent) { task.abort(); }
        if let Some(wanted) = self.wanted.remove(agent) { wanted.store(false, Ordering::Relaxed); }
    }

    pub(super) fn select(&mut self, ui: &mut Ui, selected: Option<&str>) {
        if self.selected.as_deref() == selected { return; }
        self.generation += 1;
        if let Some(previous) = self.selected.take() {
            self.cancel(&previous);
            if let Some(view) = ui.terminal_view_mut(&previous)
                && view.native.is_none()
                && view.stale.as_deref().is_some_and(|status| status.starts_with("connecting"))
            {
                view.stale = Some(failed("selection changed"));
            }
        }
        self.selected = selected.map(str::to_owned);
        self.started = None;
        self.note = None;
    }

    pub(super) fn begin(&mut self, ui: &mut Ui, agent: &str) -> (u64, Arc<AtomicBool>, Arc<tokio::sync::Semaphore>) {
        self.select(ui, Some(agent));
        self.cancel(agent);
        drop(self.tasks.remove(agent));
        // Abort is asynchronous. A per-agent permit prevents overlapping requests even when
        // several replacements themselves get aborted before their predecessor has dropped.
        let serial = self.serial.entry(agent.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1))).clone();
        self.generation += 1;
        self.pending.insert(agent.to_owned(), self.generation);
        self.started = Some(Instant::now());
        self.note = None;
        self.opens.insert(agent.to_owned(), Instant::now());
        let wanted = Arc::new(AtomicBool::new(true));
        self.wanted.insert(agent.to_owned(), wanted.clone());
        (self.generation, wanted, serial)
    }

    fn current(&self, agent: &str, generation: u64) -> bool {
        self.selected.as_deref() == Some(agent) && self.generation == generation
            && self.pending.get(agent) == Some(&generation)
    }

    pub(super) fn tick(&mut self, ui: &mut Ui) {
        for (agent, task) in &self.tasks {
            if ui.terminal_view(agent).is_none() {
                task.abort();
                self.pending.remove(agent);
                if let Some(wanted) = self.wanted.remove(agent) { wanted.store(false, Ordering::Relaxed); }
            }
        }
        self.tasks.retain(|_, task| !task.is_finished());
        let Some(started) = self.started else { return; };
        if started.elapsed() >= DEADLINE {
            if let Some(agent) = self.selected.take() {
                self.cancel(&agent);
                if let Some(view) = ui.terminal_view_mut(&agent) { view.stale = Some(failed("attach setup timed out")); }
                self.selected = Some(agent);
            }
            self.started = None;
            self.note = None;
        } else if let Some(agent) = self.selected.as_deref()
            && let Some(view) = ui.terminal_view_mut(agent)
            && view.native.is_none() && view.ended.is_none()
        {
            view.stale = Some(match &self.note {
                Some(reason) => format!("{} · {reason}, trying again", connecting_text(started.elapsed())),
                None => connecting_text(started.elapsed()),
            });
        }
    }

    // This is the actual receive dispatch called by the live loop and by regression tests.
    pub(super) fn apply_fetched(&mut self, ui: &mut Ui, msg: Fetched, commands: &mpsc::UnboundedSender<Command>) {
        let (_, selected) = ui.focus();
        self.select(ui, selected.as_deref());
        match msg {
            Fetched::Native { agent, generation, direct } => {
                if self.pending.get(&agent) != Some(&generation) {
                    close(direct);
                    return;
                }
                let current = self.current(&agent, generation);
                if current { self.note = None; self.started = None; }
                let (rows, columns) = ui.terminal_size.get();
                let route = direct.route.clone();
                if let Some(view) = ui.terminal_view_mut(&agent).filter(|view| view.native.is_none()) {
                    view.name = format!("{} · {route}", view.name);
                    view.title = view.name.clone();
                    view.native = Some(crate::ui::pty::NativeTerminal::spawn(direct.stream, &direct.name, direct.incarnation, rows, columns));
                    view.stale = None;
                } else {
                    close(direct);
                }
                if current { ui.flash(format!("Attached · {route}")); }
            }
            Fetched::AttachWaiting { agent, generation, reason } => {
                if self.current(&agent, generation) { self.note = Some(reason); }
                if ui.terminal_view(&agent).is_none() { self.cancel(&agent); }
            }
            Fetched::NativeFailed { agent, generation, runtime_ids, reason } => {
                if !self.current(&agent, generation) { return; }
                self.note = None;
                self.started = None;
                if self.opens.remove(&agent).is_some() { crate::ui::slo::METRICS.add("terminal_open_errors", 1); }
                // Native setup has failed even if the opportunistic screen feed never replies.
                // A real fallback screen can clear this state; errors must keep retry available.
                if let Some(view) = ui.terminal_view_mut(&agent) {
                    view.stale = Some(failed(&reason));
                    view.ended = None;
                }
                if reason.ends_with("timed out") {
                    self.cancel(&agent);
                    self.pending.remove(&agent);
                } else if !agent.starts_with("terminal/") && ui.terminal_view(&agent).is_some() {
                    if commands.send(Command::Follow { runtime_ids }).is_err() {
                        let reason = format!("{reason}; fallback Follow unavailable");
                        if let Some(view) = ui.terminal_view_mut(&agent) {
                            view.stale = Some(failed(&reason));
                        }
                        ui.flash(format!("Could not open the terminal: {reason}"));
                    } else {
                        ui.flash(format!("No direct terminal ({reason}); trying st's view"));
                    }
                }
            }
            _ => unreachable!("only initial attach results enter this dispatch"),
        }
    }
}

pub(super) fn close(direct: Direct) {
    // Dropping the byte connector cancels the gateway bridge too (client raw stream contract).
    let _ = direct.stream.shutdown(Shutdown::Both);
}

fn failed(reason: &str) -> String { format!("failed: {reason} · Ctrl+R to retry") }

pub(super) fn apply_terminal_update(ui: &mut Ui, update: TerminalUpdate, attached: &mut Option<Following>) {
    match update {
        TerminalUpdate::Attached { terminal_id, attachment_id, incarnation } => {
            *attached = Some(Following { terminal_id, attachment_id, incarnation });
        }
        TerminalUpdate::Screen(screen) => {
            if let Some(view) = ui.terminal.as_mut() {
                view.lines = screen_lines(&screen);
                view.cursor = screen.cursor.visible.then_some((screen.cursor.row, screen.cursor.column));
                view.stale = None;
                if !screen.title.is_empty() {
                    view.title = format!("{} · {}", view.name, screen.title);
                }
            }
        }
        TerminalUpdate::Reconnecting(reason) => {
            if let Some(view) = ui.terminal.as_mut() {
                view.stale = Some(if retryable(view) {
                    failed(&format!("fallback screen: {reason}"))
                } else {
                    reason
                });
            }
        }
        TerminalUpdate::Ended { restarted, reason } => {
            *attached = None;
            if let Some(view) = ui.terminal.as_mut().filter(|view| retryable(view)) {
                // Do not replace an initial-attach failure with an unretryable ended state.
                let reason = if restarted { format!("fallback terminal restarted: {reason}") }
                    else { format!("fallback screen: {reason}") };
                view.stale = Some(failed(&reason));
                return;
            }
            if ui.terminal.as_ref().is_some_and(|view| view.ended.is_none()) {
                crate::ui::slo::METRICS.add("dead_terminal_tabs", 1);
            }
            match ui.terminal.as_mut() {
                Some(view) => {
                    view.ended = Some(if restarted {
                        "Terminal restarted; open it again to follow the new one".into()
                    } else {
                        reason
                    });
                }
                None if !restarted => ui.flash(format!("Could not open the terminal: {reason}")),
                None => {}
            }
        }
    }
}

pub(super) fn retryable(view: &crate::ui::TerminalView) -> bool {
    view.native.is_none() && view.stale.as_deref().is_some_and(|status| status.starts_with("failed: "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Pane, TerminalView, glass::Open};
    use std::os::unix::net::UnixStream;

    fn view(agent: &str) -> TerminalView {
        TerminalView { agent: agent.into(), title: agent.into(), name: agent.into(), lines: Vec::new(), cursor: None, stale: Some("connecting".into()), ended: None, native: None }
    }

    fn selected(agent: &str) -> (State, Ui, mpsc::UnboundedSender<Command>, mpsc::UnboundedReceiver<Command>) {
        let mut ui = Ui::new(crate::demo::world());
        ui.live = true;
        ui.open_in_glass(Pane::Terminal(agent.into()), Open::Here);
        ui.terminal = Some(view(agent));
        let (tx, rx) = mpsc::unbounded_channel();
        let mut state = State::default();
        let _ = state.begin(&mut ui, agent);
        (state, ui, tx, rx)
    }

    fn success(agent: &str, generation: u64) -> (Fetched, UnixStream) {
        let (stream, peer) = UnixStream::pair().unwrap();
        (Fetched::Native { agent: agent.into(), generation, direct: Direct { name: agent.into(), incarnation: "i1".into(), stream, route: "test route".into() } }, peer)
    }

    fn sentinel(state: &mut State, ui: &mut Ui) -> Instant {
        let at = Instant::now();
        state.started = Some(at);
        state.note = Some("B waiting".into());
        ui.flash = Some(("B flash".into(), at));
        at
    }

    fn unchanged(state: &State, ui: &Ui, rx: &mut mpsc::UnboundedReceiver<Command>, at: Instant) {
        assert_eq!(state.started, Some(at));
        assert_eq!(state.note.as_deref(), Some("B waiting"));
        assert_eq!(ui.flash, Some(("B flash".into(), at)));
        assert!(matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)), "no stale Follow command");
    }

    #[test]
    fn late_native_for_a_does_not_change_b_or_follow_a() {
        let (mut state, mut ui, tx, mut rx) = selected("agent/A");
        let old = state.generation;
        ui.open_in_glass(Pane::Terminal("agent/B".into()), Open::Here);
        let _ = state.begin(&mut ui, "agent/B");
        ui.parked.push(view("agent/A"));
        ui.terminal = Some(view("agent/B"));
        let at = sentinel(&mut state, &mut ui);
        let (msg, mut peer) = success("agent/A", old);
        state.apply_fetched(&mut ui, msg, &tx);
        unchanged(&state, &ui, &mut rx, at);
        assert!(ui.terminal_view("agent/A").unwrap().native.is_none());
        use std::io::Read;
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0, "cancelled late stream is closed");
    }

    #[test]
    fn late_failure_for_a_does_not_change_b_or_follow_a() {
        let (mut state, mut ui, tx, mut rx) = selected("agent/A");
        let old = state.generation;
        ui.open_in_glass(Pane::Terminal("agent/B".into()), Open::Here);
        let _ = state.begin(&mut ui, "agent/B");
        ui.parked.push(view("agent/A"));
        ui.terminal = Some(view("agent/B"));
        let at = sentinel(&mut state, &mut ui);
        state.apply_fetched(&mut ui, Fetched::NativeFailed { agent: "agent/A".into(), generation: old, runtime_ids: vec!["runtime/A".into()], reason: "refused".into() }, &tx);
        unchanged(&state, &ui, &mut rx, at);
    }

    #[test]
    fn same_agent_reopen_rejects_old_success_failure_and_waiting() {
        let (mut state, mut ui, tx, mut rx) = selected("agent/A");
        let old = state.generation;
        let _ = state.begin(&mut ui, "agent/A");
        assert_ne!(old, state.generation);
        let at = sentinel(&mut state, &mut ui);
        let (msg, mut peer) = success("agent/A", old);
        state.apply_fetched(&mut ui, msg, &tx);
        use std::io::Read;
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0, "discarded stream is explicitly closed");
        state.apply_fetched(&mut ui, Fetched::NativeFailed { agent: "agent/A".into(), generation: old, runtime_ids: vec!["runtime/A".into()], reason: "refused".into() }, &tx);
        state.apply_fetched(&mut ui, Fetched::AttachWaiting { agent: "agent/A".into(), generation: old, reason: "old wait".into() }, &tx);
        unchanged(&state, &ui, &mut rx, at);
        assert!(ui.terminal.as_ref().unwrap().native.is_none());
    }

    #[test]
    fn phase_timeout_dispatch_fails_with_retry_instead_of_following() {
        let (mut state, mut ui, tx, mut rx) = selected("agent/A");
        let generation = state.generation;
        state.apply_fetched(&mut ui, Fetched::NativeFailed {
            agent: "agent/A".into(), generation, runtime_ids: vec!["runtime/A".into()],
            reason: "raw terminal attachment timed out".into(),
        }, &tx);
        assert_eq!(ui.terminal.as_ref().unwrap().stale.as_deref(), Some("failed: raw terminal attachment timed out · Ctrl+R to retry"));
        assert!(state.started.is_none());
        assert!(state.note.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn matching_failure_still_follows_the_selected_agent() {
        let (mut state, mut ui, tx, mut rx) = selected("agent/A");
        let generation = state.generation;
        state.apply_fetched(&mut ui, Fetched::NativeFailed {
            agent: "agent/A".into(), generation, runtime_ids: vec!["runtime/A".into()],
            reason: "direct refused".into(),
        }, &tx);
        assert!(matches!(rx.try_recv(), Ok(Command::Follow { runtime_ids }) if runtime_ids == ["runtime/A"]));
        assert!(state.started.is_none());
    }

    #[test]
    fn native_failure_then_closed_follow_channel_is_failed_with_a_reason() {
        let (mut state, mut ui, tx, rx) = selected("agent/A");
        drop(rx);
        let generation = state.generation;
        state.apply_fetched(&mut ui, Fetched::NativeFailed {
            agent: "agent/A".into(), generation, runtime_ids: vec!["runtime/A".into()],
            reason: "remote unavailable".into(),
        }, &tx);
        state.tick(&mut ui);
        let view = ui.terminal.as_ref().unwrap();
        assert_eq!(view.stale.as_deref(), Some("failed: remote unavailable; fallback Follow unavailable · Ctrl+R to retry"));
        assert!(retryable(view));
        assert!(view.ended.is_none());
        assert!(state.started.is_none());
    }

    #[test]
    fn fallback_screen_failure_stays_failed_and_ctrl_r_starts_exactly_one_attempt() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let (mut state, mut ui, tx, mut rx) = selected("agent/A");
        let generation = state.generation;
        state.apply_fetched(&mut ui, Fetched::NativeFailed {
            agent: "agent/A".into(), generation, runtime_ids: vec!["runtime/A".into()],
            reason: "remote unavailable".into(),
        }, &tx);
        assert!(matches!(rx.try_recv(), Ok(Command::Follow { .. })));
        let mut attached = None;
        apply_terminal_update(&mut ui, TerminalUpdate::Reconnecting("gateway screen unavailable".into()), &mut attached);
        assert_eq!(ui.terminal.as_ref().unwrap().stale.as_deref(), Some("failed: fallback screen: gateway screen unavailable · Ctrl+R to retry"));
        apply_terminal_update(&mut ui, TerminalUpdate::Ended {
            restarted: false, reason: "screen read refused".into(),
        }, &mut attached);
        state.tick(&mut ui);
        let view = ui.terminal.as_ref().unwrap();
        assert_eq!(view.stale.as_deref(), Some("failed: fallback screen: screen read refused · Ctrl+R to retry"));
        assert!(view.ended.is_none());
        assert!(state.started.is_none());
        assert!(state.note.is_none());
        assert!(rx.try_recv().is_err());

        ui.effects.clear();
        ui.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let effects = std::mem::take(&mut ui.effects);
        let [crate::ui::Effect::OpenTerminal { agent }] = effects.as_slice() else {
            panic!("retry must emit exactly one OpenTerminal effect");
        };
        assert_eq!(agent, "agent/A");
        // Process the effect with the same lifecycle entry used by the live OpenTerminal arm.
        let (retry, _, _) = state.begin(&mut ui, agent);
        assert_eq!(retry, generation + 1);
        assert!(state.started.is_some());
        assert_eq!(state.pending.get(agent), Some(&retry));
        assert_eq!(state.wanted.len(), 1);
    }

    #[test]
    fn deadline_aborts_setup_and_stops_connecting_with_retry() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (mut state, mut ui, tx, mut rx) = selected("terminal/A");
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
            let (mut client, mut server) = tokio::net::UnixStream::pair().unwrap();
            struct Guard(Option<tokio::sync::oneshot::Sender<()>>);
            impl Drop for Guard { fn drop(&mut self) { let _ = self.0.take().unwrap().send(()); } }
            state.tasks.insert("terminal/A".into(), tokio::spawn(async move {
                let _guard = Guard(Some(dropped_tx));
                let _ = started_tx.send(());
                use tokio::io::AsyncReadExt;
                let _ = client.read(&mut [0]).await;
            }));
            started_rx.await.unwrap();
            state.started = Some(Instant::now() - DEADLINE);
            state.tick(&mut ui);
            tokio::time::timeout(Duration::from_secs(1), dropped_rx).await.unwrap().unwrap();
            use tokio::io::AsyncReadExt;
            let mut byte = [0];
            assert_eq!(tokio::time::timeout(Duration::from_secs(1), server.read(&mut byte)).await.unwrap().unwrap(), 0);
            assert!(state.started.is_none());
            let status = ui.terminal.as_ref().unwrap().stale.as_deref().unwrap();
            assert!(status.starts_with("failed: attach setup timed out"));
            assert!(status.contains("Ctrl+R"));
            assert!(!status.contains("connecting"));
            state.apply_fetched(&mut ui, Fetched::NativeFailed { agent: "terminal/A".into(), generation: state.generation, runtime_ids: vec![], reason: "late".into() }, &tx);
            assert!(rx.try_recv().is_err());
        });
    }

    #[test]
    fn phase_deadline_drops_an_unresponsive_setup_future() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let dropped = Arc::new(AtomicBool::new(false));
            struct Guard(Arc<AtomicBool>);
            impl Drop for Guard { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }
            let guard = Guard(dropped.clone());
            let (mut client, mut server) = tokio::net::UnixStream::pair().unwrap();
            let result = phase("raw terminal stream", Duration::from_millis(5), async move {
                let _guard = guard;
                use tokio::io::AsyncReadExt;
                let mut byte = [0];
                client.read_exact(&mut byte).await.map(|_| ()).map_err(|error| error.to_string())
            }).await;
            assert_eq!(result.unwrap_err(), "raw terminal stream timed out");
            assert!(dropped.load(Ordering::SeqCst));
            use tokio::io::AsyncReadExt;
            let mut byte = [0];
            assert_eq!(tokio::time::timeout(Duration::from_secs(1), server.read(&mut byte)).await.unwrap().unwrap(), 0, "timeout closes the stalled server connection");
        });
    }

    #[test]
    fn supersession_aborts_and_repeated_retries_never_overlap_setup() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (mut state, mut ui, _, _) = selected("terminal/A");
            let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            struct Guard(Arc<std::sync::atomic::AtomicUsize>);
            impl Drop for Guard { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
            for attempt in 0..8 {
                let (_, _, serial) = state.begin(&mut ui, "terminal/A");
                let active = active.clone();
                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(async move {
                    let _permit = serial.acquire_owned().await.unwrap();
                    assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                    let _guard = Guard(active);
                    let _ = ready_tx.send(());
                    std::future::pending::<()>().await;
                });
                state.tasks.insert("terminal/A".into(), task);
                if attempt == 0 || attempt == 7 {
                    ready_rx.await.unwrap();
                    assert_eq!(active.load(Ordering::SeqCst), 1);
                } else if attempt % 2 == 0 {
                    tokio::task::yield_now().await;
                }
            }
            state.select(&mut ui, Some("terminal/B"));
            let previous = state.tasks.remove("terminal/A").unwrap();
            assert!(previous.await.unwrap_err().is_cancelled());
            assert_eq!(active.load(Ordering::SeqCst), 0);
        });
    }
}
