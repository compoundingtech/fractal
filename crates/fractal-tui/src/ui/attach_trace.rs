//! Opt-in, bounded attach diagnostics. Never records capabilities, payloads or error text.
use std::{collections::{BTreeMap, BTreeSet}, fs::{File, OpenOptions}, io::Write as _, os::{fd::AsRawFd as _, unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _}}, sync::{Arc, Mutex}, time::Instant};

const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Trace(Arc<Inner>);

struct Inner {
    attempt: uuid::Uuid,
    agent: String,
    started: Instant,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    finished: bool,
    next: u64,
    last_phase: Option<&'static str>,
    active: BTreeMap<u64, (&'static str, Instant)>,
    milestones: BTreeSet<&'static str>,
    sink: Option<File>,
    #[cfg(test)]
    captured: Option<Arc<Mutex<Vec<serde_json::Value>>>>,
    #[cfg(test)]
    info_outcomes: usize,
}

impl Trace {
    pub(crate) fn new(agent: &str) -> Self {
        let sink = std::env::var_os("FRACTAL_TUI_ATTACH_TRACE_FILE").and_then(|path| {
            let path = std::path::Path::new(&path);
            if !path.is_absolute() { return None; }
            let file = OpenOptions::new().append(true).create(true).mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
            let metadata = file.metadata().ok()?;
            // Refuse non-files, other owners, hardlinks, and existing permissive files.
            if !metadata.is_file() || metadata.permissions().mode() & 0o777 != 0o600
                || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 { return None; }
            Some(file)
        });
        Self::with_state(agent, State { sink, ..State::default() })
    }

    fn with_state(agent: &str, state: State) -> Self {
        let trace = Self(Arc::new(Inner { attempt: uuid::Uuid::now_v7(), agent: agent.to_owned(), started: Instant::now(), state: Mutex::new(state) }));
        trace.0.emit(&mut trace.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner), "attempt", "start", None);
        trace
    }

    pub(crate) fn milestone(&self, phase: &'static str) {
        let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.finished && state.milestones.insert(phase) {
            state.last_phase = Some(phase);
            self.0.emit(&mut state, phase, "success", None);
        }
    }

    pub(crate) fn complete(&self) {
        self.milestone("first-screen-rendered");
        self.finish("success");
    }

    pub(crate) fn finish(&self, outcome: &'static str) {
        self.0.finish(outcome);
    }

    pub(crate) fn start_phase(&self, phase: &'static str) -> Phase {
        let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = state.next;
        state.next += 1;
        if !state.finished {
            state.last_phase = Some(phase);
            state.active.insert(id, (phase, Instant::now()));
            self.0.emit(&mut state, phase, "start", None);
        }
        Phase { trace: self.clone(), id }
    }

    #[cfg(test)]
    pub(crate) fn captured(agent: &str) -> (Self, Arc<Mutex<Vec<serde_json::Value>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (Self::with_state(agent, State { captured: Some(events.clone()), ..State::default() }), events)
    }
}

impl Inner {
    fn emit(&self, state: &mut State, phase: &'static str, outcome: &'static str, duration: Option<u128>) {
        #[cfg(not(test))]
        if state.sink.is_none() { return; }
        #[cfg(test)]
        if state.sink.is_none() && state.captured.is_none() { return; }
        let level = if matches!(outcome, "start" | "success") { "DEBUG" } else { "INFO" };
        let at_unix_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        let event = serde_json::json!({"service":"fractal-tui", "level":level, "attempt_id":self.attempt.to_string(), "agent":self.agent, "phase":phase, "phase_reached":state.last_phase, "outcome":outcome, "at_unix_ms":at_unix_ms, "elapsed_ms":self.started.elapsed().as_millis(), "phase_elapsed_ms":duration});
        #[cfg(test)]
        if let Some(events) = &state.captured { events.lock().unwrap().push(event.clone()); }
        if let Some(file) = state.sink.as_mut()
            && let Ok(mut bytes) = serde_json::to_vec(&event)
        {
            bytes.push(b'\n');
            // Serialize append + size check without waiting behind another process:
            // diagnostics must not introduce another attach stall.
            // SAFETY: flock borrows the live file descriptor and retains no pointers.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                if file.metadata().is_ok_and(|metadata| metadata.len().saturating_add(bytes.len() as u64) <= MAX_BYTES) {
                    let _ = file.write_all(&bytes);
                }
                // SAFETY: the descriptor still belongs to this live File.
                let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
            }
        }
    }

    fn finish(&self, outcome: &'static str) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.finished { return; }
        state.finished = true;
        if outcome != "success" {
            tracing::info!(
                target: "fractal_tui::attach",
                attempt_id = %self.attempt,
                agent = %self.agent,
                phase_reached = state.last_phase.unwrap_or("attempt"),
                outcome,
                elapsed_ms = %self.started.elapsed().as_millis(),
                "terminal attach attempt ended"
            );
            // Unit tests capture this outcome in memory rather than touch the user's cache.
            #[cfg(not(test))]
            super::lastrun::note_attach_outcome(
                self.attempt, &self.agent, state.last_phase.unwrap_or("attempt"),
                outcome, self.started.elapsed().as_millis(),
            );
            #[cfg(test)]
            { state.info_outcomes += 1; }
        }
        for (_, (phase, started)) in std::mem::take(&mut state.active) {
            self.emit(&mut state, phase, outcome, Some(started.elapsed().as_millis()));
        }
        self.emit(&mut state, "attempt", outcome, None);
    }
}

impl Drop for Inner {
    fn drop(&mut self) { self.finish("dropped"); }
}

pub(crate) struct Phase { trace: Trace, id: u64 }

impl Phase {
    pub(crate) fn end(self, outcome: &'static str) { self.record(outcome); }
    fn record(&self, outcome: &'static str) {
        let mut state = self.trace.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((phase, started)) = state.active.remove(&self.id) {
            self.trace.0.emit(&mut state, phase, outcome, Some(started.elapsed().as_millis()));
        }
    }
}

impl Drop for Phase {
    fn drop(&mut self) { self.record("cancelled"); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaved_attempts_are_isolated_and_terminal_once() {
        let (a, first) = Trace::captured("agent/a");
        let (b, second) = Trace::captured("agent/b");
        let pa = a.start_phase("upgrade");
        let pb = b.start_phase("upgrade");
        a.finish("selection-changed");
        drop(pa);
        pb.end("success");
        b.milestone("first-frame-decoded");
        b.complete();
        b.finish("dropped");
        let first = first.lock().unwrap();
        let second = second.lock().unwrap();
        assert_ne!(first[0]["attempt_id"], second[0]["attempt_id"]);
        for (events, agent, outcome) in [(&*first, "agent/a", "selection-changed"), (&*second, "agent/b", "success")] {
            assert!(events.iter().all(|event| event["agent"] == agent && event["attempt_id"] == events[0]["attempt_id"]));
            let terminal: Vec<_> = events.iter().filter(|event| event["phase"] == "attempt" && event["outcome"] != "start").collect();
            assert_eq!(terminal.len(), 1);
            assert_eq!(terminal[0]["outcome"], outcome);
            assert_eq!(terminal[0]["phase_reached"], if agent == "agent/a" { "upgrade" } else { "first-screen-rendered" });
            let trace = if agent == "agent/a" { &a } else { &b };
            assert_eq!(trace.0.state.lock().unwrap().info_outcomes, usize::from(outcome != "success"));
            assert!(terminal[0]["at_unix_ms"].as_u64().is_some_and(|time| time > 0));
        }
    }

    #[test]
    fn final_owner_drop_records_terminal_outcome() {
        let (trace, events) = Trace::captured("agent/a");
        drop(trace);
        assert_eq!(events.lock().unwrap().last().unwrap()["outcome"], "dropped");
    }
    #[test]
    fn unsuccessful_terminal_outcomes_log_once_without_an_opt_in_sink() {
        for outcome in ["outer-deadline", "selection-changed", "dropped", "setup-failed"] {
            let trace = Trace::with_state("agent/a", State::default());
            trace.finish(outcome);
            trace.finish(outcome);
            assert_eq!(trace.0.state.lock().unwrap().info_outcomes, 1);
        }
        let trace = Trace::with_state("agent/a", State::default());
        trace.complete();
        assert_eq!(trace.0.state.lock().unwrap().info_outcomes, 0);
    }
}
