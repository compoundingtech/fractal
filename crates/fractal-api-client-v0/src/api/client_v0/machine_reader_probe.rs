//! Observational probe: no injected delays and no timing-based pass/fail threshold.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn snapshot_live() -> bool {
    smallclaims::sqlite::live_read_locations().iter().any(|(site, snapshot)| {
        *snapshot && site.contains("client_v0.rs:")
    })
}

fn thread_cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/thread-self/stat").unwrap();
    let fields = stat.rsplit_once(')').unwrap().1.split_whitespace().collect::<Vec<_>>();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "synthetic machine inventory snapshot/WAL/CPU observation; run explicitly"]
async fn machine_inventory_reader_observation() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::test_state_named(root.path(), "alder");
    for runtime in 0..32 {
        for observation in 0..100 {
            state.store.append_claim(&crate::model::ClaimInput {
                subject: format!("agent/probe-{runtime}"),
                kind: "runtime.observed".into(), actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("incarnation_id".into(), json!(format!("inc-{observation}"))),
                    ("driver".into(), json!("codex")),
                    ("host".into(), json!("probe")),
                ]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
    }
    state.store.connection.write().execute_batch(
        "CREATE TABLE machine_reader_probe(id INTEGER PRIMARY KEY, payload BLOB);
         PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE);"
    ).unwrap();
    state.store.forget_current_views();
    let request: CollectionSubscribe = serde_json::from_value(json!({
        "kind":"subscribe", "id":"probe", "collection":"machines", "limit":200
    })).unwrap();
    let session = ClientSession::local(None).unwrap();

    for pass in ["cold", "warm"] {
        let done = Arc::new(AtomicBool::new(false));
        let (observed, observed_rx) = tokio::sync::oneshot::channel();
        let monitor_store = state.store.clone();
        let monitor_done = done.clone();
        let wal_path = root.path().join("graph.db-wal");
        let monitor = std::thread::spawn(move || {
            let checkpoint = rusqlite::Connection::open(root_path(&wal_path)).unwrap();
            checkpoint.busy_timeout(Duration::ZERO).unwrap();
            let mut observed = Some(observed);
            let mut first = None;
            let mut last = None;
            let mut peak_active = 0;
            let mut commits_while_pinned = 0;
            let mut checkpoint_busy = 0;
            let mut peak_wal_bytes = 0;
            let mut peak_unbackfilled = 0;
            while !monitor_done.load(Ordering::Acquire) {
                if snapshot_live() {
                    let now = std::time::Instant::now();
                    first.get_or_insert(now);
                    last = Some(now);
                    if let Some(observed) = observed.take() { let _ = observed.send(()); }
                    let usage = monitor_store.readers.usage();
                    peak_active = peak_active.max(usage.open.saturating_sub(usage.idle));
                    monitor_store.connection.write().execute(
                        "INSERT INTO machine_reader_probe(payload) VALUES(zeroblob(16384))", []
                    ).unwrap();
                    if snapshot_live() { commits_while_pinned += 1; }
                    let (busy, frames, backfilled): (i64, i64, i64) = checkpoint.query_row(
                        "PRAGMA wal_checkpoint(TRUNCATE)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    ).unwrap();
                    checkpoint_busy += usize::from(busy != 0);
                    peak_unbackfilled = peak_unbackfilled.max(frames - backfilled);
                    peak_wal_bytes = peak_wal_bytes.max(std::fs::metadata(&wal_path).map_or(0, |m| m.len()));
                } else {
                    std::thread::yield_now();
                }
            }
            let after: i64 = checkpoint.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0)).unwrap();
            (first.zip(last).map_or(Duration::ZERO, |(a,b)| b.duration_since(a)), peak_active,
                commits_while_pinned, checkpoint_busy, peak_wal_bytes, peak_unbackfilled, after)
        });
        // A single-slot controlled admission probe measures queueing behind this worker.
        // It does not claim production admission has only one slot.
        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = admission.clone().acquire_owned().await.unwrap();
        let (read_state, read_session, read_request) = (state.clone(), session.clone(), request.clone());
        let started = std::time::Instant::now();
        let read_done = done.clone();
        let read = tokio::spawn(async move {
            let result = collection_items(&read_state, &read_session, &read_request, permit).await;
            read_done.store(true, Ordering::Release);
            (result, started.elapsed())
        });
        let saw_snapshot = matches!(
            tokio::time::timeout(Duration::from_secs(20), observed_rx).await, Ok(Ok(()))
        );
        let queue_started = std::time::Instant::now();
        let queued = tokio::spawn(async move {
            let permit = admission.acquire_owned().await.unwrap();
            (permit, queue_started.elapsed())
        });
        let probe_store = state.store.clone();
        let independent = tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let pinned_at_start = snapshot_live();
            probe_store.readers.request_read(|| probe_store.index()).unwrap().unwrap();
            (started.elapsed(), pinned_at_start)
        });
        let (result, elapsed) = read.await.unwrap();
        let result = result.unwrap();
        let (_second_permit, queue_wait) = queued.await.unwrap();
        let (independent_wait, pinned_at_probe_start) = independent.await.unwrap();
        println!("reader_capacity pass={pass} independent_read_us={} snapshot_live_at_probe_start={pinned_at_probe_start}",
            independent_wait.as_micros());
        let metrics = monitor.join().unwrap();
        println!("machine_reader pass={pass} wall_us={} snapshot_observed={saw_snapshot} sampled_hold_us={} peak_active_connections={} commits_while_pinned={} truncate_busy={} peak_wal_bytes={} peak_unbackfilled_frames={} truncate_busy_after={} single_slot_queue_wait_us={} rows={} cut={}",
            elapsed.as_micros(), metrics.0.as_micros(), metrics.1, metrics.2, metrics.3,
            metrics.4, metrics.5, metrics.6, queue_wait.as_micros(), result.1.len(), result.0.store_index);
        assert_eq!(metrics.6, 0, "checkpoint must complete after the inventory reader releases");
    }

    // Isolate runtime-reduction thread CPU from the concurrent writer/monitor above.
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        for pass in ["cold", "warm"] {
            if pass == "cold" { store.forget_current_views(); }
            let before = thread_cpu_ticks();
            let started = std::time::Instant::now();
            let rows = store.read_snapshot(|cut| store.status_for_claim_kind_at(
                "runtime.observed", Some(cut), false, crate::store::ClaimsReduction::Runtime,
            )).unwrap();
            println!("runtime_reduction pass={pass} thread_cpu_ticks={} wall_us={} rows={}",
                thread_cpu_ticks() - before, started.elapsed().as_micros(), rows.subjects.len());
            assert_eq!(rows.subjects.len(), 32);
        }
    }).await.unwrap();
}

fn root_path(wal: &std::path::Path) -> std::path::PathBuf {
    wal.with_file_name("graph.db")
}
