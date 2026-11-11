//! Durable receive admission, terminal recovery, and queued projection fencing.
use super::*;
use std::sync::mpsc;

const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
const RECOVERY: &str = "projection-recovery-required";
const INTERNAL_RECOVERY: &str = "projection-recovery-required/internal";

fn arm(runtime: &FaultRuntime, code: i32) {
    *runtime.stage.lock().unwrap() = Some("incremental");
    runtime.fault.store(code, Ordering::Relaxed);
}

fn recovery_health(store: &Store) -> (String, u64, Option<String>, Option<String>) {
    store.readers.get().query_row(
        "SELECT status,last_good_store_index,error_code,error_message FROM projection_health WHERE aggregate='graph'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap()
}

fn receive_pending(source: &Store, target: &Store) -> (ReplicationExchange, String) {
    source.bind_fleet(FLEET).unwrap();
    target.bind_fleet(FLEET).unwrap();
    pending_claim(source);
    let claim = source.claims_for("note/pending", Some("example.note")).unwrap().pop().unwrap();
    let exchange = source.export_replication_exchange(FLEET, &target.replication_inventory().unwrap()).unwrap();
    let receipt = target.receive_replication_exchange("source", FLEET, &exchange).unwrap();
    assert!(receipt.received > 0, "delivery must acknowledge durable input before projection");
    assert!(target.claim_by_id(&claim.id).unwrap().is_none(), "receipt is not admission");
    let admission = target.validate_replication_backlog().unwrap();
    assert!(admission.valid > 0);
    assert!(target.claim_by_id(&claim.id).unwrap().is_some());
    (exchange, claim.id)
}

#[test]
fn t3_non_contention_failure_parks_with_typed_health_and_never_replays() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let runtime = Arc::new(FaultRuntime::new());
    let store = Store::open(&path, "node", runtime.clone()).unwrap();
    store.project_replication_backlog().unwrap();
    let before = health(&store);
    let before_effect = effect(&store, "busy-test-effect");
    let source = Store::open_memory("source", Arc::new(Plain)).unwrap();
    let (_, claim) = receive_pending(&source, &store);
    let target = store.index().unwrap();
    assert!(target > before.1);
    arm(&runtime, rusqlite::ffi::SQLITE_ERROR);
    let mut lines = Vec::new();
    assert!(!store.project_replication_backlog_with_log("receive", |line| lines.push(line.to_owned())).unwrap());
    assert_eq!(runtime.replays.load(Ordering::Relaxed), 0);
    let failed = recovery_health(&store);
    assert_eq!((&failed.0, failed.1, failed.2.as_deref()), (&"stale".to_owned(), before.1, Some(INTERNAL_RECOVERY)));
    assert!(failed.3.as_deref().is_some_and(|message| message.contains("internal")),
        "recovery must retain the original non-contention error code: {failed:?}");
    assert_eq!(effect(&store, "busy-test-effect"), before_effect, "speculative writes must roll back");
    assert!(store.replication_projection_deferred());
    assert!(store.replication_projection_recovery_required().unwrap());
    assert!(store.claim_by_id(&claim).unwrap().is_some());
    assert!(!lines.iter().any(|line| line.starts_with("fractal: projection full replay ")));
    let diagnostic = lines.iter().find_map(|line| {
        let (_, payload) = line.split_once(": projection recovery required ")?;
        Some(serde_json::from_str::<Value>(payload).expect("recovery diagnostic is structured JSON"))
    }).expect("the real projection log sink must emit the recovery-required event");
    assert_eq!(diagnostic["event"], "replication.projection.recovery_required");
    assert_eq!(diagnostic["code"], RECOVERY);
    assert_eq!(diagnostic["reason_code"], "internal");
    assert_eq!(diagnostic["frontier"], json!(before.1));
    assert_eq!(diagnostic["target"], json!(target));
    assert_eq!(diagnostic["phase"], "receive");
    let public = store.replication_status(true, Some(FLEET), &[]).unwrap();
    let graph = public.unhealthy.iter().find(|projection| projection.aggregate == "graph").expect("public health must expose recovery");
    assert_eq!(graph.status, "stale");
    assert_eq!(graph.error_code.as_deref(), Some(INTERNAL_RECOVERY));
    assert!(graph.error_message.as_deref().is_some_and(|message| message.contains("internal")));
    // A restart must retain the terminal reason; opening or an automatic pass must
    // not reinterpret a parked target as retryable, healthy work.
    drop(store);
    let reopened = Store::open(&path, "node", runtime.clone()).unwrap();
    assert!(reopened.replication_projection_recovery_required().unwrap());
    assert!(reopened.replication_projection_deferred());
    assert!(!reopened.project_replication_backlog().unwrap());
    assert_eq!(recovery_health(&reopened), failed);
    assert_eq!(runtime.replays.load(Ordering::Relaxed), 0);
    reopened.replication_status_sealed(true, Some(FLEET), &[]).unwrap();
    let export = reopened.export_replication_summary(FLEET).unwrap();
    assert!(export.graph_digest.is_empty());
    assert!(export.projection_digests.is_empty());
    assert!(reopened.replication_projection_deferred());
    assert_eq!(recovery_health(&reopened), failed, "sealed export must not rewrite parked health");
    assert_eq!(effect(&reopened, "busy-test-effect"), before_effect, "sealed export must not project");
    reopened.replay_replication_graph().unwrap();
    assert!(!reopened.replication_projection_recovery_required().unwrap());
    assert!(reopened.project_replication_backlog().unwrap());
    assert!(!reopened.replication_projection_deferred());
    reopened.replication_status_sealed(true, Some(FLEET), &[]).unwrap();
    let export = reopened.export_replication_summary(FLEET).unwrap();
    assert!(!export.graph_digest.is_empty());
    assert!(!export.projection_digests.is_empty());
    assert_eq!(runtime.replays.load(Ordering::Relaxed), 1);
}

#[test]
fn t4_receipt_and_admitted_input_are_durable_when_projection_is_deferred_or_failed() {
    for code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_ERROR] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let runtime = Arc::new(FaultRuntime::new());
        let source = Store::open_memory("source", Arc::new(Plain)).unwrap();
        let target = Store::open(&path, "node", runtime.clone()).unwrap();
        target.project_replication_backlog().unwrap();
        let (exchange, claim) = receive_pending(&source, &target);
        let inventory = target.replication_inventory().unwrap();
        arm(&runtime, code);
        // Projection failure is a separate outcome from the already committed receipt.
        assert!(!target.project_replication_backlog().unwrap());
        assert_eq!(runtime.replays.load(Ordering::Relaxed), 0);
        assert!(target.claim_by_id(&claim).unwrap().is_some());
        drop(target);
        let reopened = Store::open(&path, "node", runtime).unwrap();
        assert!(reopened.claim_by_id(&claim).unwrap().is_some());
        assert_eq!(reopened.replication_inventory().unwrap().digest, inventory.digest);
        let duplicate = reopened.receive_replication_exchange("source", FLEET, &exchange).unwrap();
        assert_eq!(duplicate.received, 0);
        assert!(duplicate.duplicate > 0, "the acknowledged envelope must survive restart");
        assert_eq!(reopened.claims_for("note/pending", Some("example.note")).unwrap().len(), 1);
    }
}

#[test]
fn t6_waiting_projection_rechecks_durable_recovery_under_the_gate() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Arc::new(FaultRuntime::new());
    let store = Arc::new(Store::open(&directory.path().join("claims.sqlite3"), "node", runtime.clone()).unwrap());
    store.project_replication_backlog().unwrap();
    pending_claim(&store);
    let before = health(&store);
    let attempts = Arc::new(AtomicUsize::new(0));
    // Count real runtime entry even when its SQL transaction rolls back. This is
    // connection-local, test-only fault instrumentation, not a production seam.
    let count = attempts.clone();
    store.connection.write().create_scalar_function("s6_incremental_attempt", 0,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8, move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(0_i64)
        }).unwrap();
    // Test-only fixed trigger: increments a scalar counter, without copying row values.
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-habfd091c5cba owner=smallclaims::store::projection_busy_tests::s6_recovery TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP TRIGGER s6_count_incremental BEFORE INSERT ON meta
         WHEN NEW.key='busy-test-effect' BEGIN SELECT s6_incremental_attempt(); END;"
    ).unwrap();
    arm(&runtime, rusqlite::ffi::SQLITE_ERROR);
    let (entered, entering) = mpsc::sync_channel(0);
    let (release, released) = mpsc::sync_channel(0);
    let first_store = store.clone();
    let first = std::thread::spawn(move || {
        let mut blocked = false;
        first_store.project_replication_backlog_with_progress("receive-first", |progress| {
            if progress.phase == "project-incremental" && !blocked {
                blocked = true;
                entered.send(()).unwrap();
                released.recv().unwrap();
            }
        })
    });
    entering.recv().unwrap(); // The first call holds projection gate and writer.
    let (queued, queuing) = mpsc::sync_channel(0);
    let second_store = store.clone();
    let second = std::thread::spawn(move || {
        second_store.project_replication_backlog_before_gate(|| queued.send(()).unwrap())
    });
    queuing.recv().unwrap(); // The second invocation has reached the locked gate.
    release.send(()).unwrap();
    assert!(!first.join().unwrap().unwrap());
    assert!(!second.join().unwrap().unwrap());
    assert_eq!(attempts.load(Ordering::SeqCst), 1,
        "the queued call must read parked state after acquiring the gate, before runtime work");
    assert_eq!(runtime.replays.load(Ordering::Relaxed), 0);
    assert!(store.replication_projection_recovery_required().unwrap());
    let failed = recovery_health(&store);
    assert_eq!(failed.0, "stale");
    assert_eq!(failed.1, before.1);
    assert_eq!(failed.2.as_deref(), Some(INTERNAL_RECOVERY));
    assert!(failed.3.as_deref().is_some_and(|message| message.contains("internal")));
}
