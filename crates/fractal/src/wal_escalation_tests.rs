use super::*;

/// The frames in a store's WAL, from the WAL-index header's `mxFrame`, so looking checkpoints
/// nothing.
fn wal_frames(path: &std::path::Path) -> i32 {
    let mut shm = path.as_os_str().to_owned();
    shm.push("-shm");
    let header = std::fs::read(shm).unwrap_or_default();
    header.get(16..20).map_or(0, |frames| i32::from_ne_bytes(frames.try_into().unwrap()))
}

/// A store an older build wrote: its envelopes carry the old payloads, which the daemon's startup
/// converts, one page a commit with a short rest, as soon as it starts.
fn older_build_store(path: &std::path::Path) {
    let store = Store::open(path, "alder").unwrap();
    store.bind_fleet("sample-fleet").unwrap();
    // Only a member with a key keeps signed envelopes to convert.
    store.set_member_key(Some(Arc::new(fractal_daemon::fleet::MemberKey::generate().unwrap().0))).unwrap();
    for n in 0..1_500 {
        store
            .append_as_older_build_for_test(&ClaimInput {
                subject: "daemon/alder".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("startup-wal")),
                    ("reason".into(), json!(format!("{n} {}", "x".repeat(4_000)))),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("startup-wal-{n}")),
            })
            .unwrap();
    }
    // Signed into envelopes as an exchange carries them.
    store.export_replication_exchange("sample-fleet", &fractal_daemon::model::ReplicationInventory::default()).unwrap();
    // As an older build stored them: payloads as base64 text, at schema 15, so opening the store
    // converts them again.
    {
        use base64::Engine as _;
        let connection = store.connection.write();
        let payloads: Vec<(i64, Vec<u8>)> = connection
            .prepare("SELECT rowid,payload FROM replica_envelopes")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (rowid, bytes) in payloads {
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h4cb8851484ee owner=fractal::wal_escalation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET payload=?1 WHERE rowid=?2",
                    rusqlite::params![base64::engine::general_purpose::STANDARD.encode(bytes), rowid],
                )
                .unwrap();
        }
        connection.execute_batch("PRAGMA user_version=15").unwrap();
    }
    let converting: i64 = store.readers.get().query_row("SELECT COUNT(*) FROM replica_envelopes WHERE typeof(payload)='text'", [], |row| row.get(0)).unwrap();
    assert_eq!(converting, 1_500, "every envelope waits to be converted");
    assert!(store.checkpoint_idle_wal(&rusqlite::Connection::open(path).unwrap()).unwrap(), "the seed leaves an empty WAL");
}

struct PinnedReader {
    release: std::sync::mpsc::SyncSender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl PinnedReader {
    fn finish(self) {
        self.release.send(()).unwrap();
        self.thread.join().unwrap();
    }
}

/// Return only after a real read snapshot is pinned. Releasing joins the reader, so the next
/// checkpoint cannot race its COMMIT. The SQLite transaction owns rollback/commit normally.
fn pin_reader(path: &std::path::Path) -> PinnedReader {
    let reader = rusqlite::Connection::open(path).unwrap();
    let (ready, pinned) = std::sync::mpsc::sync_channel(0);
    let (release, finish) = std::sync::mpsc::sync_channel(0);
    let thread = std::thread::spawn(move || {
        let read = reader.unchecked_transaction().unwrap();
        read.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get::<_, i64>(0)).unwrap();
        ready.send(()).unwrap();
        finish.recv().unwrap();
        read.commit().unwrap();
    });
    pinned.recv().unwrap();
    PinnedReader { release, thread }
}

/// Step the same conversion pages and recycle attempts as startup, with a reader pinned before
/// every page. Wall-clock offsets do not guarantee overlapping readers on a loaded host: a
/// checkpoint in a reader gap can truncate the control WAL or reset the escalation's growth.
fn start(path: &std::path::Path, limits: smallclaims::store::wal_escalation::WalEscalation) -> (i32, usize) {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let store = Store::open(path, "alder").unwrap();
    store.set_wal_escalation(limits);
    let reader = Arc::new(std::sync::Mutex::new(None::<PinnedReader>));
    let checkpoint = rusqlite::Connection::open(path).unwrap();
    // Let the pinned fold finish only once RESTART has been selected under the writer. This
    // exercises the real restart, without betting that a sleeping reader runs within `busy`.
    let finishing = Arc::clone(&reader);
    checkpoint.authorizer(Some(move |context: AuthContext<'_>| {
        if let AuthAction::Pragma { pragma_name: "wal_checkpoint", pragma_value: Some("RESTART") } = context.action {
            finishing.lock().unwrap().take().unwrap().finish();
        }
        Authorization::Allow
    }));
    let mut growth = smallclaims::store::wal_escalation::WalGrowth::default();
    let mut largest = 0;
    let mut escalated = 0;
    loop {
        // Establish the snapshot, not just BEGIN (which is deferred), before the writer runs.
        *reader.lock().unwrap() = Some(pin_reader(path));
        let page = store.convert_envelope_payloads().unwrap();
        // Sample every committed page before the checkpoint can reset mxFrame.
        largest = largest.max(wal_frames(path));
        let (_, restart) = store.recycle_wal(&checkpoint, &mut growth).unwrap();
        if let Some(restart) = restart {
            assert!(restart.recycled, "the selected restart must recycle the WAL");
            escalated += 1;
        }
        if let Some(reader) = reader.lock().unwrap().take() {
            reader.finish();
        }
        if page.done {
            return (largest, escalated);
        }
    }
}

/// A daemon's startup maintenance writes nearly all the time while the first folds keep readers on
/// older snapshots, so the recycle never finds the WAL copied and, without an escalation, the WAL
/// grows until the maintenance ends: 661 MB within two minutes of starting on a 57 GB store. Past
/// its bound and still growing, the recycle restarts it.
#[test]
fn a_starting_daemon_s_wal_recycle_restarts_a_wal_its_startup_work_keeps_growing() {
    let scratch = tempfile::tempdir().unwrap();
    let seeded = scratch.path().join("seeded.sqlite3");
    older_build_store(&seeded);
    let bound = 300;
    let limits = smallclaims::store::wal_escalation::WalEscalation {
        frames: bound,
        samples: 2,
        ..smallclaims::store::wal_escalation::WAL_ESCALATION
    };
    let run = |name: &str| {
        let path = scratch.path().join(name);
        std::fs::copy(&seeded, &path).unwrap();
        path
    };
    // Reproduce the old harness's losing interleaving without relying on thread scheduling:
    // one checkpoint sees growth, then both sleeping readers release before the next attempt.
    {
        let path = run("reader-gap.sqlite3");
        let store = Store::open(&path, "alder").unwrap();
        store.set_wal_escalation(limits);
        let reader = pin_reader(&path);
        let checkpoint = rusqlite::Connection::open(&path).unwrap();
        while wal_frames(&path) <= bound {
            assert!(!store.convert_envelope_payloads().unwrap().done);
        }
        let mut growth = smallclaims::store::wal_escalation::WalGrowth::default();
        let (pinned, restart) = store.recycle_wal(&checkpoint, &mut growth).unwrap();
        assert!(!pinned.recycled && restart.is_none());
        reader.finish();
        let (gap, restart) = store.recycle_wal(&checkpoint, &mut growth).unwrap();
        assert!(gap.recycled && restart.is_none(), "a reader gap recycles before escalation");
        assert_eq!(wal_frames(&path), 0, "the gap destroys the control's accumulated growth");
    }
    let (without, control_escalations) = start(&run("without.sqlite3"), smallclaims::store::wal_escalation::WalEscalation { frames: i32::MAX, ..limits });
    assert_eq!(control_escalations, 0);
    let (with, escalated) = start(&run("with.sqlite3"), limits);
    assert!(without > 4 * bound, "without escalating, the startup work grows the WAL to {without} frames");
    assert!(escalated > 0, "the recycle escalated");
    assert!(with < without / 2, "escalating, the WAL reached {with} frames, against {without} without");
}
