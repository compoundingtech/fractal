//! Filesystem watch filtering for native delivery and shared driver state.
//!
//! Linux `notify` subscribes to inotify `OPEN` events and reports them as [`EventKind::Access`].
//! Forwarding those events makes a reader self-wake forever: each pass reads its watched tree, that
//! read emits another event, and the next timed wait returns immediately. Only mutations are useful
//! wakeups; timer polling remains the fallback for anything a backend cannot classify.

use std::path::Path;
use std::sync::mpsc::Sender;

use notify::{Event, EventKind, RecursiveMode, Watcher};

/// Watch only the inputs a native delivery pump consumes: the agent's `resources/inbox` subtree
/// and its `status` file. Runtime records written beside them by the pump's own process group —
/// the presence refresh's temp siblings, `harness-state`, `harness-context`, stream state — must
/// never wake delivery, or a writer that observes on every turn boundary pumps itself
/// continuously. The allowlist is what makes that free: a new sibling record is ignored with no
/// production change (HC-R08).
#[cfg(test)]
pub(crate) fn watch_delivery_inputs(
    agent_dir: &Path,
    tx: Sender<()>,
) -> Option<notify::RecommendedWatcher> {
    watch_delivery_inputs_with_status(agent_dir, tx, true)
}

/// Native graph-owned pumps watch the inbox only; catalog pumps also watch status/DND.
pub(crate) fn watch_delivery_inputs_with_status(
    agent_dir: &Path,
    tx: Sender<()>,
    legacy_status: bool,
) -> Option<notify::RecommendedWatcher> {
    let inbox = agent_dir.join("resources").join("inbox");
    let inbox_for_callback = inbox.clone();
    let status = agent_dir.join("status");
    let mut watcher = recommended_watcher(move |result: notify::Result<Event>| {
        if result.is_ok_and(|event| {
            is_mutation(&event)
                && event.paths.iter().any(|path| {
                    path.starts_with(&inbox_for_callback) || (legacy_status && *path == status)
                })
        }) {
            let _ = tx.send(());
        }
    })
    .ok()?;
    // Two shallow subscriptions instead of one recursive walk over the agent dir: Resource
    // payload trees live beside the inbox under `resources/`, and a recursive subscription pays
    // one inotify watch per payload directory before any filtering ever runs. The agent-dir watch
    // stays shallow because everything beside `status` is runtime state; the inbox itself is
    // st2-owned and message-flat, so its recursion is bounded by delivery traffic, not payloads.
    let inbox_for_watch = inbox;
    if legacy_status {
        watcher.watch(agent_dir, RecursiveMode::NonRecursive).ok()?;
    }
    watcher
        .watch(&inbox_for_watch, RecursiveMode::Recursive)
        .ok()?;
    Some(watcher)
}

/// A [`notify::RecommendedWatcher`] that never follows symlinks while walking.
fn recommended_watcher(
    handler: impl FnMut(notify::Result<Event>) + Send + 'static,
) -> notify::Result<notify::RecommendedWatcher> {
    notify::RecommendedWatcher::new(
        handler,
        notify::Config::default().with_follow_symlinks(false),
    )
}

fn is_mutation(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{
        AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode,
    };

    #[test]
    fn only_mutations_wake_watch_loops() {
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
            EventKind::Remove(RemoveKind::File),
        ] {
            assert!(is_mutation(&Event::new(kind)), "{kind:?} must wake");
        }

        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Any,
            EventKind::Other,
        ] {
            assert!(!is_mutation(&Event::new(kind)), "{kind:?} must stay quiet");
        }
    }

    /// One real harness-context write, through the record's own writer: the guard, the lock, the
    /// staging outside the agent subtree, and the rename of the canonical name in.
    #[cfg(target_os = "linux")]
    fn st2_harness_context_write(agent_dir: &Path) {
        use crate::harness_context::{Harness, Reading, Writer};
        Writer::new(agent_dir, "h.worker", Harness::Codex)
            .unwrap()
            .observe(Reading {
                used_tokens: Some(92_283),
                window_tokens: Some(258_400),
                used_percent: Some(33.0),
                ..Reading::default()
            })
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn delivery_watcher_ignores_runtime_records_but_wakes_on_inbox_and_status() {
        use std::sync::mpsc::channel;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agents/h/worker");
        std::fs::create_dir_all(agent_dir.join("resources/inbox")).unwrap();

        let (tx, rx) = channel();
        let _watcher = watch_delivery_inputs(&agent_dir, tx).expect("start inotify watcher");

        // Runtime records the pump's own process group writes must stay silent: the observed
        // harness state and the harness context beside it, their locks and atomic temp siblings,
        // the presence temp sibling, stream state.
        std::fs::write(agent_dir.join("harness-state"), "{}").unwrap();
        std::fs::write(agent_dir.join(".harness-state.tmp-1-0"), "{}").unwrap();
        std::fs::write(agent_dir.join("harness-context"), "{}").unwrap();
        std::fs::write(agent_dir.join(".harness-context.lock"), "").unwrap();
        std::fs::write(agent_dir.join(".status.tmp-1-0"), "available\n").unwrap();
        std::fs::create_dir_all(agent_dir.join("resources/streams/s")).unwrap();
        std::fs::write(agent_dir.join("resources/streams/s/state.json"), "{}").unwrap();
        // …and a real harness-context write through its own writer, staging outside the agent
        // subtree and renaming the record in, which is the shape a producer actually produces.
        st2_harness_context_write(&agent_dir);
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "runtime-record writes must not wake the delivery pump"
        );

        // The two genuine delivery inputs wake it: an inbox arrival…
        std::fs::write(agent_dir.join("resources/inbox/0001-msg.md"), "hi").unwrap();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("inbox write must wake");
        while rx.try_recv().is_ok() {}

        // …and a presence change, including one landing via atomic tmp+rename.
        std::fs::rename(agent_dir.join(".status.tmp-1-0"), agent_dir.join("status")).unwrap();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("status rename must wake");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn graph_delivery_watcher_ignores_runtime_and_status_but_wakes_on_inbox() {
        use std::sync::mpsc::{RecvTimeoutError, channel};
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agents/h/worker");
        std::fs::create_dir_all(agent_dir.join("resources/inbox")).unwrap();
        let (tx, rx) = channel();
        let _watcher = watch_delivery_inputs_with_status(&agent_dir, tx, false)
            .expect("start graph delivery inotify watcher");

        std::fs::write(agent_dir.join("harness-state"), "{}").unwrap();
        std::fs::write(agent_dir.join(".harness-state.tmp-1-0"), "{}").unwrap();
        std::fs::write(agent_dir.join("harness-context"), "{}").unwrap();
        std::fs::write(agent_dir.join(".harness-context.lock"), "").unwrap();
        st2_harness_context_write(&agent_dir);
        std::fs::create_dir_all(agent_dir.join("resources/streams/s")).unwrap();
        std::fs::write(agent_dir.join("resources/streams/s/state.json"), "{}").unwrap();
        std::fs::write(agent_dir.join("status"), "available\n").unwrap();
        std::fs::write(agent_dir.join(".status.tmp-1-0"), "held\n").unwrap();
        std::fs::rename(agent_dir.join(".status.tmp-1-0"), agent_dir.join("status")).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(RecvTimeoutError::Timeout),
            "graph delivery must ignore runtime records and both status write shapes"
        );

        std::fs::write(agent_dir.join("resources/inbox/0001-msg.md"), "hi").unwrap();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("graph inbox write must wake delivery");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn delivery_watch_is_bounded_by_inputs_not_payload_depth() {
        use std::sync::mpsc::channel;

        const ISOLATED: &str = "ST2_TEST_ISOLATED_DELIVERY_WATCH_COUNT";

        // Other unit tests create inotify descriptors in this process. Run the descriptor count
        // in a child test process so their concurrent watches cannot change this measurement.
        if std::env::var_os(ISOLATED).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "watch::tests::delivery_watch_is_bounded_by_inputs_not_payload_depth",
                    "--test-threads=1",
                ])
                .env(ISOLATED, "1")
                .output()
                .expect("start isolated delivery watch test");
            assert!(
                output.status.success(),
                "isolated delivery watch test failed:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        fn inotify_watch_count() -> usize {
            std::fs::read_dir("/proc/self/fdinfo")
                .unwrap()
                .flatten()
                .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
                .filter(|content| content.contains("inotify wd:"))
                .map(|content| content.lines().count())
                .sum()
        }

        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path();
        let payload = agent_dir.join("resources/worktree/node_modules");
        for i in 0..300 {
            std::fs::create_dir_all(payload.join(format!("pkg-{i}/dist/sub"))).unwrap();
        }
        std::fs::create_dir_all(agent_dir.join("resources/inbox")).unwrap();

        let before = inotify_watch_count();
        let (tx, _rx) = channel();
        let _watcher = watch_delivery_inputs(agent_dir, tx).expect("start delivery watcher");
        // A watcher teardown can still make the count fall between reads. Saturation keeps that
        // unrelated teardown from producing an unsigned wrap.
        let delta = inotify_watch_count().saturating_sub(before);
        assert!(
            delta < 32,
            "payload depth must not drive watch allocation: {delta} watches for a \
             900-directory payload tree"
        );
    }

    #[test]
    fn delivery_watch_registration_failure_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let (tx, _rx) = std::sync::mpsc::channel();

        assert!(
            watch_delivery_inputs_with_status(&missing, tx, false).is_none(),
            "failed native inbox registration must not claim an active watcher"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reads_are_silent_but_real_mutations_wake() {
        use std::sync::mpsc::channel;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("resources/inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let file = inbox.join("watched");
        std::fs::write(&file, "before").unwrap();

        let (tx, rx) = channel();
        let _watcher = watch_delivery_inputs_with_status(dir.path(), tx, false)
            .expect("start native inbox watcher");

        for _ in 0..1_000 {
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "before");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "OPEN/read access must not wake a mutation watcher"
        );

        std::fs::write(&file, "after").unwrap();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("content mutation must wake");

        fn assert_wakes(setup: fn(&Path), mutate: fn(&Path)) {
            let dir = tempfile::tempdir().unwrap();
            let inbox = dir.path().join("resources/inbox");
            std::fs::create_dir_all(&inbox).unwrap();
            setup(&inbox);
            let (tx, rx) = channel();
            let _watcher = watch_delivery_inputs_with_status(dir.path(), tx, false)
                .expect("start native inbox watcher");
            mutate(&inbox);
            rx.recv_timeout(Duration::from_secs(1))
                .expect("filesystem mutation must wake");
        }

        assert_wakes(
            |_| {},
            |dir| std::fs::write(dir.join("created"), "value").unwrap(),
        );
        assert_wakes(
            |dir| std::fs::write(dir.join("renamed-from"), "value").unwrap(),
            |dir| {
                std::fs::rename(dir.join("renamed-from"), dir.join("renamed-to")).unwrap();
            },
        );
        assert_wakes(
            |dir| std::fs::write(dir.join("removed"), "value").unwrap(),
            |dir| std::fs::remove_file(dir.join("removed")).unwrap(),
        );
    }
}
