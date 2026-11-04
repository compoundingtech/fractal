use super::*;
use serde_json::json;

fn setup() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    enable(root.path(), "runtime").unwrap();
    crate::harness_state::claim(root.path(), "agent/test", "omp", "provider").unwrap();
    // claim's ownership/coalescing reads can retain a reader in this thread's LRU.
    READ_CONNECTIONS.with(|connections| {
        connections.borrow_mut().retain(|entry| entry.path != database_path(root.path()));
    });
    assert_no_open_spool_connections(root.path());
    root
}

fn assert_no_open_spool_connections(root: &Path) {
    let paths = [database_path(root), root.join(format!("{DATABASE}-wal")),
        root.join(format!("{DATABASE}-shm"))];
    READ_CONNECTIONS.with(|connections| {
        assert!(connections.borrow().iter().all(|entry| !paths.contains(&entry.path)));
    });
    #[cfg(target_os = "linux")]
    for fd in std::fs::read_dir("/proc/self/fd").unwrap().flatten() {
        if let Ok(path) = std::fs::read_link(fd.path()) {
            assert!(!paths.contains(&path), "spool still open via {}: {}", fd.path().display(), path.display());
        }
    }
}

#[test]
fn unchanged_reports_do_not_write_and_changed_reports_survive_reopen() {
    let root = setup();
    let body = serde_json::to_vec(&json!({"incarnation":"provider", "tasks":[]})).unwrap();
    write_snapshot(root.path(), "harness-todo", &body).unwrap();
    let observer = open(root.path()).unwrap();
    let version = || observer.pragma_query_value(None, "data_version", |row| row.get::<_, u64>(0)).unwrap();
    let before = version();
    let events = pending(root.path(), 100).unwrap().len();
    for _ in 0..1000 {
        write_snapshot(root.path(), "harness-todo", &body).unwrap();
    }
    assert_eq!(version(), before, "no committed page changes for identical reports");
    assert_eq!(pending(root.path(), 100).unwrap().len(), events);
    let changed = serde_json::to_vec(&json!({"incarnation":"provider", "tasks":["new"]})).unwrap();
    write_snapshot(root.path(), "harness-todo", &changed).unwrap();
    assert_ne!(version(), before);
    drop(observer);
    assert_eq!(read_snapshot(root.path(), "harness-todo").unwrap(), Some(changed));
}

#[test]
fn timeline_prunes_in_batches_and_readers_see_the_original_window() {
    let root = setup();
    let operation: Operation = serde_json::from_value(json!({
        "operation":"append", "entryId":"entry", "sequence":1, "revision":1,
        "role":"assistant", "entryType":"message", "finalEntry":true,
        "body":{"text":"hello"}, "driver":"omp", "incarnationId":"provider",
        "observedAtUnixMs":1
    })).unwrap();
    let mut record = Record {
        schema: "st.harness-timeline.v1".into(), driver: "omp".into(),
        incarnation_id: "provider".into(), next_sequence: 4353, operations: vec![],
    };
    let operations: Vec<_> = (1..=4352).map(|sequence| {
        let mut op = operation.clone();
        op.sequence = sequence;
        op
    }).collect();
    write_timeline(root.path(), &record, &operations).unwrap();
    let count = || open(root.path()).unwrap().query_row("SELECT COUNT(*) FROM timeline", [], |row| row.get::<_, u64>(0)).unwrap();
    assert_eq!(count(), 4352, "physical slack is retained without pruning");
    let visible = read_timeline(root.path()).unwrap().unwrap();
    assert_eq!(visible.operations.len(), 4096);
    assert_eq!(visible.operations[0].sequence, 257);
    record.next_sequence += 1;
    let mut last = operation;
    last.sequence = 4353;
    write_timeline(root.path(), &record, &[last]).unwrap();
    assert_eq!(count(), 4096, "crossing the slack threshold prunes a batch");
    let visible = read_timeline(root.path()).unwrap().unwrap();
    assert_eq!(visible.operations.len(), 4096);
    assert_eq!(visible.operations[0].sequence, 258);
    assert_eq!(visible.next_sequence, 4354);
    // The byte bound applies even when the row count is far below its limit.
    let mut large = visible.operations.last().unwrap().clone();
    large.body = json!({"text":"x".repeat(8192)});
    let size = serde_json::to_vec(&large).unwrap().len();
    let batch = vec![large.clone(); 260];
    write_timeline(root.path(), &record, &batch).unwrap();
    let visible = read_timeline(root.path()).unwrap().unwrap();
    assert!(visible.operations.len() < 260);
    assert!(visible.operations.iter().map(|op| serde_json::to_vec(op).unwrap().len()).sum::<usize>() <= 2097152);
    write_timeline(root.path(), &record, &vec![large; 20]).unwrap();
    let bytes: usize = open(root.path()).unwrap().query_row(
        "SELECT SUM(length(CAST(body AS BLOB))) FROM timeline", [], |row| row.get(0),
    ).unwrap();
    assert!(bytes <= 2097152, "crossing byte slack prunes a batch of {size}-byte rows");
}

#[test]
fn unchanged_subagent_ledger_keeps_its_inode() {
    let root = tempfile::tempdir().unwrap();
    crate::subagents::update(root.path(), |ledger| ledger.incarnation = Some("runtime".into())).unwrap();
    let path = crate::subagents::ledger_path(root.path());
    let before = path.metadata().unwrap();
    crate::subagents::update(root.path(), |_| ()).unwrap();
    let after = path.metadata().unwrap();
    assert_eq!((before.ino(), before.mtime(), before.mtime_nsec()),
        (after.ino(), after.mtime(), after.mtime_nsec()));
}

// WAL frame bytes, not filesystem/ZFS write amplification. Isolate every commit
// with TRUNCATE so WAL reuse cannot hide page writes in a file-size comparison.
#[test]
fn report_wal_bytes_per_1000_unchanged_reports() {
    let root = setup();
    let body = serde_json::to_vec(&json!({"incarnation":"provider", "tasks":[]})).unwrap();
    write_snapshot(root.path(), "harness-todo", &body).unwrap();
    let mut connection = open(root.path()).unwrap();
    let wal = root.path().join(format!("{DATABASE}-wal"));
    let mut before = 0;
    let mut after = 0;
    for baseline in [true, false] {
        for _ in 0..1000 {
            connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
            if baseline {
                let tx = connection.transaction().unwrap();
                tx.execute(/* sqlite-large-value: allow-for-now lv-h00d8663c1682 owner=fractal_drivers::harness_events_churn_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO snapshots VALUES (?1,?2) ON CONFLICT(kind) DO UPDATE SET body=excluded.body",
                    params!["harness-todo", body]).unwrap();
                append_event(&tx, "harness-todo", &serde_json::from_slice(&body).unwrap()).unwrap();
                tx.commit().unwrap();
                before += wal.metadata().unwrap().len();
            } else {
                write_snapshot(root.path(), "harness-todo", &body).unwrap();
                after += wal.metadata().unwrap().len();
            }
        }
    }
    assert!(before > 0);
    assert_eq!(after, 0);
    eprintln!("1000 identical snapshot reports: before_wal_bytes={before} after_wal_bytes={after}; FULL durability; checkpoint-isolated WAL frames, not device bytes");
}

#[test]
fn checkpoint_budget_bounds_wal_across_5000_short_lived_producers() {
    let root = setup();
    let connection = open(root.path()).unwrap();
    let pages: u64 = connection.pragma_query_value(None, "wal_autocheckpoint", |row| row.get(0)).unwrap();
    let page_size: u64 = connection.pragma_query_value(None, "page_size", |row| row.get(0)).unwrap();
    assert_eq!(pages, 1000);
    assert_eq!(connection.pragma_query_value(None, "synchronous", |row| row.get::<_, u64>(0)).unwrap(), 2);
    drop(connection);
    assert_no_open_spool_connections(root.path());
    // A connection opened just below the 1000-frame close budget can append
    // one transaction before a successor enables last-close checkpointing.
    // Bound = 32-byte WAL header + (1000 + max_tx_frames) * (page_size + 24).
    // These small reports reserve 32 frames for that transaction, including splits.
    let bound = 32 + (pages + 32) * (page_size + 24);
    let wal = root.path().join(format!("{DATABASE}-wal"));
    let mut peak = 0;
    let mut checkpoints = 0;
    for report in 0..5000 {
        let body = serde_json::to_vec(&json!({"incarnation":"provider", "report":report})).unwrap();
        write_snapshot(root.path(), "harness-todo", &body).unwrap();
        assert_no_open_spool_connections(root.path());
        let bytes = wal.metadata().map_or(0, |metadata| metadata.len());
        if bytes == 0 {
            checkpoints += 1;
        }
        if report < 5 || report % 250 == 249 {
            eprintln!("short-lived producer report={report} wal_bytes={bytes}");
        }
        peak = peak.max(bytes);
    }
    assert_eq!(pending(root.path(), 6000).unwrap().len(), 5001);
    eprintln!("5000 short-lived producers: peak_wal_bytes={peak} bound={bound} checkpoints={checkpoints} autocheckpoint_pages={pages} page_size={page_size}");
    assert!(peak > 0 && checkpoints > 0 && checkpoints < 500, "checkpoints must be batched, not per-report");
    assert!(peak <= bound, "WAL high-water mark {peak} exceeded {bound}");
}

#[test]
fn crash_producer_helper() {
    let Some(path) = std::env::var_os("ST_TEST_CRASH_OBSERVATION_DIR") else { return };
    let mut connection = open(Path::new(&path)).unwrap();
    let tx = connection.transaction().unwrap();
    let value = json!({"incarnation":"provider", "tasks":["committed-before-exit"]});
    let body = serde_json::to_vec(&value).unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-hcbe1ad86b9e9 owner=fractal_drivers::harness_events_churn_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO snapshots VALUES ('harness-todo',?1) ON CONFLICT(kind) DO UPDATE SET body=excluded.body",
        [&body]).unwrap();
    append_event(&tx, "harness-todo", &value).unwrap();
    tx.commit().unwrap();
    // Exit without dropping the live SQLite connection or running its close hook.
    std::process::exit(0);
}

#[test]
fn committed_events_replay_after_process_exit_without_sqlite_close() {
    let root = setup();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "harness_events::churn_tests::crash_producer_helper", "--nocapture"])
        .env("ST_TEST_CRASH_OBSERVATION_DIR", root.path()).status().unwrap();
    assert!(status.success());
    assert_no_open_spool_connections(root.path());
    let events = pending(root.path(), 100).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].payload["tasks"][0], "committed-before-exit");
    let snapshot = read_snapshot(root.path(), "harness-todo").unwrap().unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&snapshot).unwrap()["tasks"][0], "committed-before-exit");
    acknowledge(root.path(), events[1].sequence).unwrap();
    assert!(pending(root.path(), 100).unwrap().is_empty());
}
