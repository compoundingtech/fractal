// This module uses only APIs present before rebuild coalescing. Copy it unchanged and add
// `mod coalesce_run_part_rebuild;` to store/tests.rs to measure the base revision identically.
use super::*;

pub(super) fn coalesced_rebuild_fixture() -> (Store, Store, Store, tempfile::TempDir, String) {
    let controller = Store::open_memory("controller").unwrap();
    let mut kdl = String::from("version 2\nmission \"large-live\" state=\"ready\" {\n goal \"Keep working.\"\n");
    for index in 0..256 {
        kdl.push_str(&format!(" step \"work-{index}\" {{ agentless }}\n"));
    }
    kdl.push_str("}\n");
    publish_mission(&controller, &kdl, "large-live-mission");
    let run = controller.create_mission_run(&MissionRunRequest {
        mission: "large-live".into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: "large-live-run".into(),
    }).unwrap();
    assert_eq!(run.steps.len(), 256);
    let directory = tempfile::tempdir().unwrap();
    let uninterrupted = Store::open_memory("worker").unwrap();
    let interrupted = Store::open(&directory.path().join("worker.sqlite"), "worker").unwrap();
    for worker in [&uninterrupted, &interrupted] {
        replicate_fully(&controller, worker);
        worker.set_write_clock_at(now_ms() + 1_000_000).unwrap();
        worker.append_claim(&ClaimInput {
            subject: "custom/coalesce/order".into(),
            kind: "custom.coalesce.order".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        worker.project_replication_backlog().unwrap();
    }
    (controller, uninterrupted, interrupted, directory, run.steps[0].subject.clone())
}

pub(super) fn write_coalesced_rebuild_backlog(controller: &Store, step: &str, count: usize) {
    for index in 0..count {
        controller.set_step_state(
            step,
            if index % 2 == 0 { "blocked" } else { "ready" },
            Some("replicated backlog"),
        ).unwrap();
    }
}

#[test]
#[ignore = "same-host replication backlog measurement"]
fn measure_receive_run_part_rebuild_backlog() {
    let count = std::env::var("FRACTAL_REBUILD_BENCH_ENVELOPES")
        .map_or(8_192, |value| value.parse::<usize>().unwrap());
    let bound_ms = std::env::var("FRACTAL_REBUILD_BENCH_RECEIVE_BOUND_MS")
        .map_or(30_000, |value| value.parse::<u128>().unwrap());
    let (controller, worker, _unused, _directory, step) = coalesced_rebuild_fixture();
    write_coalesced_rebuild_backlog(&controller, &step, count);
    let pending_windows = std::sync::Arc::new(std::sync::Mutex::new((
        BTreeMap::<String, std::time::Instant>::new(),
        std::time::Duration::ZERO,
        0_usize,
    )));
    let observed = pending_windows.clone();
    // The existing managed-commit observer runs when the committed writer loan returns.
    // Marker appearance/disappearance measures postcommit-to-postcommit pending time,
    // including drains; it does not timestamp SQLite's COMMIT instruction itself.
    // On the base revision no marker exists; its printed max receive time is the explicitly
    // labelled upper bound for the admission-to-projection window instead.
    let _pending_observer = worker.connection.observe_commits(move |connection| {
        let now = std::time::Instant::now();
        let mut statement = connection.prepare(
            "SELECT key FROM meta WHERE key GLOB 'projection-run-rebuild:*'",
        ).unwrap();
        let current = statement.query_map([], |row| row.get::<_, String>(0))
            .unwrap().collect::<rusqlite::Result<BTreeSet<_>>>().unwrap();
        let mut windows = observed.lock().unwrap();
        let mut completed = Vec::new();
        for (part, started) in &windows.0 {
            if !current.contains(part) {
                completed.push((part.clone(), now.duration_since(*started)));
            }
        }
        for (part, elapsed) in completed {
            windows.0.remove(&part);
            windows.1 = windows.1.max(elapsed);
            windows.2 += 1;
        }
        for part in current {
            windows.0.entry(part).or_insert(now);
        }
    });
    let mut received = 0;
    let mut wall_times = Vec::new();
    let mut longest_hold = std::time::Duration::ZERO;
    let mut total_rebuilds = 0;
    loop {
        let exchange = exchange_from(&controller, &worker.replication_inventory().unwrap());
        if exchange.envelopes.is_empty() {
            break;
        }
        RUN_TREE_REBUILDS.with(|count| count.set(0));
        let started = std::time::Instant::now();
        let (_, hold) = smallclaims::windows::holds::longest_during(|| {
            worker.receive_replication_exchange("controller", TEST_FLEET, &exchange).unwrap();
            let admission = worker.validate_replication_backlog().unwrap();
            assert_eq!(admission.invalid, 0);
            assert_eq!(admission.unknown, 0);
            assert!(worker.project_replication_backlog().unwrap());
        });
        let elapsed = started.elapsed().as_millis();
        let rebuilds = RUN_TREE_REBUILDS.with(std::cell::Cell::get);
        received += exchange.envelopes.len();
        wall_times.push(elapsed);
        longest_hold = longest_hold.max(hold);
        total_rebuilds += rebuilds;
        eprintln!("receive={} envelopes={} chunks>={} rebuilds={rebuilds} wall_ms={elapsed} longest_writer_hold_us={} reported_bound_ms={bound_ms} exceeds_reported_bound={}",
            wall_times.len(), exchange.envelopes.len(), exchange.envelopes.len() / 128,
            hold.as_micros(), elapsed > bound_ms);
    }
    assert!(received >= count, "catch-up must consume the complete backlog");
    assert!(!wall_times.is_empty());
    wall_times.sort_unstable();
    eprintln!("backlog_envelopes={received} receives={} p50_wall_ms={} max_wall_ms={} longest_writer_hold_us={} rebuilds={total_rebuilds}",
        wall_times.len(), wall_times[(wall_times.len() - 1) / 2], wall_times[wall_times.len() - 1], longest_hold.as_micros());
    {
        let windows = pending_windows.lock().unwrap();
        assert!(windows.0.is_empty(), "receive must finish all pending parts");
        if windows.2 == 0 {
            eprintln!("pending_commit_window=unavailable_on_base window1_admission_to_projection_upper_bound_ms={} source=max_per_receive_wall_ms longest_writer_hold_us={}",
                wall_times[wall_times.len() - 1], longest_hold.as_micros());
        } else {
            eprintln!("longest_part_pending_commit_to_commit_us={} completed_pending_intervals={} longest_writer_hold_us={} clock=postcommit_writer_loan_observer",
                windows.1.as_micros(), windows.2, longest_hold.as_micros());
        }
    }
    let projected = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(projected, graph_digest_of(&worker));

    write_coalesced_rebuild_backlog(&controller, &step, 10);
    let exchange = exchange_from(&controller, &worker.replication_inventory().unwrap());
    assert_eq!(exchange.envelopes.len(), 10);
    RUN_TREE_REBUILDS.with(|count| count.set(0));
    let started = std::time::Instant::now();
    let (_, hold) = smallclaims::windows::holds::longest_during(|| {
        worker.receive_replication_exchange("controller", TEST_FLEET, &exchange).unwrap();
        let admission = worker.validate_replication_backlog().unwrap();
        assert_eq!(admission.invalid, 0);
        assert_eq!(admission.unknown, 0);
        assert!(worker.project_replication_backlog().unwrap());
    });
    let elapsed = started.elapsed().as_millis();
    eprintln!("steady_envelopes=10 wall_ms={elapsed} longest_writer_hold_us={} rebuilds={} reported_bound_ms={bound_ms} exceeds_reported_bound={}",
        hold.as_micros(), RUN_TREE_REBUILDS.with(std::cell::Cell::get), elapsed > bound_ms);
    let projected = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(projected, graph_digest_of(&worker));
}
