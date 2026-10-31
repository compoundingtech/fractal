use super::*;
use serde_json::json;

fn setup() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    enable(root.path(), "runtime").unwrap();
    crate::harness_state::claim(root.path(), "agent/test", "omp", "provider").unwrap();
    root
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
                tx.execute("INSERT INTO snapshots VALUES (?1,?2) ON CONFLICT(kind) DO UPDATE SET body=excluded.body",
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
