#[path = "huge_part_digest.rs"] mod huge_part_digest;
#[path = "huge_part_snapshot.rs"] mod huge_part_snapshot;
// Keep this file identical on the base and head revisions: all fixture APIs predate coalescing.
use super::*;
use super::coalesce_run_part_rebuild::write_coalesced_rebuild_backlog;

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| {
        value.parse().unwrap_or_else(|_| panic!("{name} must be a nonnegative integer"))
    })
}

fn request(key: String, revision: String) -> MissionRunRequest {
    MissionRunRequest {
        mission: "huge-part".into(),
        revision: Some(revision),
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: key,
    }
}

fn measure(children: usize, steps_per_run: usize, extra_generations: usize, history: usize) {
    let controller = Store::open_memory("controller").unwrap();
    let mut revisions = Vec::new();
    for generation in 0..=extra_generations {
        let mut source = format!(
            "version 2\nmission \"huge-part\" state=\"ready\" {{\n goal \"Keep revision {generation} live.\"\n concurrent-runs max=1000000\n"
        );
        for step in 0..steps_per_run {
            source.push_str(&format!(
                " step \"work-{step}\" {{ agentless; goal \"Revision {generation} work.\" }}\n"
            ));
        }
        source.push_str("}\n");
        revisions.push(publish_mission(&controller, &source, &format!("huge-definition-{generation}")));
    }

    // Direct children of a never-ended root are DIFFERENT parts. Keep an outer root live,
    // then put every measured descendant below one child, the measured part's top. A star
    // immediately below the outer root would silently measure a one-run rebuild instead.
    let root = controller.create_mission_run(&request(
        "huge-outer-root".into(), revisions[0].revision.clone(),
    )).unwrap();
    let mut runs: Vec<MissionRunView> = Vec::new();
    let mut subjects = BTreeSet::new();
    let mut total_steps = 0;
    for index in 0..=children {
        let parent = if index == 0 { &root } else { &runs[0] };
        let mut run = controller.create_child_mission_run(
            &request(format!("huge-part-run-{index}"), revisions[0].revision.clone()),
            parent, &parent.steps[index % steps_per_run].subject, None,
        ).unwrap();
        // Finish this run's revisions before attaching descendants: revision adoption
        // intentionally cancels descendants of the superseded generation.
        subjects.insert(run.subject.clone());
        for (generation, revision) in revisions.iter().enumerate() {
            if generation != 0 {
                let previous = run.generation.clone();
                run = controller.adopt_mission_revision(
                    &run.id, revision, "person/test", "Grow the live part's generation history",
                    &format!("huge-revise-{index}-{generation}"),
                ).unwrap();
                assert_ne!(run.generation, previous);
            }
            subjects.insert(run.generation.clone());
            for step in &run.steps {
                subjects.insert(step.subject.clone());
                write_coalesced_rebuild_backlog(&controller, &step.subject, history);
                total_steps += 1;
            }
        }
        runs.push(run);
    }

    let worker = Store::open_memory("worker").unwrap();
    // Unlike replicate_fully's 64-page fixture limit, allow arbitrarily many pages for a
    // descendant-heavy sweep, while reusing the same exchange/admission/projection helpers.
    loop {
        let exchange = exchange_from(&controller, &worker.replication_inventory().unwrap());
        if exchange.envelopes.is_empty() {
            break;
        }
        receive_and_project(&worker, "controller", &exchange);
    }
    assert_eq!(worker.replication_inventory().unwrap().digest,
        controller.replication_inventory().unwrap().digest);

    // As in the coalescing benchmark, advance only the worker's canonical frontier. The
    // controller's next state claim then arrives out of order without mutating raw claims.
    worker.set_write_clock_at(now_ms() + 1_000_000).unwrap();
    worker.append_claim(&ClaimInput {
        subject: "custom/huge-part/order".into(),
        kind: "custom.huge-part.order".into(),
        actor: None,
        fields: BTreeMap::new(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    worker.project_replication_backlog().unwrap();
    let target = &runs[0].steps[0].subject;
    let state = if history % 2 == 1 { "ready" } else { "blocked" };
    assert!(controller.set_step_state(target, state, Some("one out-of-order rebuild")).unwrap());
    let exchange = exchange_from(&controller, &worker.replication_inventory().unwrap());
    assert_eq!(exchange.envelopes.len(), 1, "measure one dirty-part claim, not a backlog");

    // These are exactly the part's run/generation/step subjects; definitions, the outer root,
    // and the worker's custom ordering marker are not replayed by rebuild_runs_tx.
    let claims: usize = subjects.iter().map(|subject| {
        controller.claims_for(subject, None).unwrap().len()
    }).sum();
    RUN_TREE_REBUILDS.with(|count| count.set(0));
    RUN_PART_RUNS.with(|count| count.set(0));
    let started = std::time::Instant::now();
    let (_, hold) = smallclaims::windows::holds::longest_during(|| {
        worker.receive_replication_exchange("controller", TEST_FLEET, &exchange).unwrap();
        let admission = worker.validate_replication_backlog().unwrap();
        assert_eq!(admission.invalid, 0);
        assert_eq!(admission.unknown, 0);
        assert!(worker.project_replication_backlog().unwrap());
    });
    // rebuild_ms is the complete single-claim receive/admit/project wall time, not a span
    // timer inside SQLite. longest_hold uses the same managed-writer observer as the old bench.
    let elapsed = started.elapsed();
    huge_part_digest::print_part_digest(&worker.connection.write(), &runs.iter().map(|run| run.id.clone()).collect(), &subjects);
    let rebuilds = RUN_TREE_REBUILDS.with(std::cell::Cell::get);
    let part_runs = RUN_PART_RUNS.with(std::cell::Cell::get);
    let gate = elapsed <= std::time::Duration::from_secs(15)
        && hold <= std::time::Duration::from_secs(15);
    eprintln!(
        "huge_part claims={claims} runs={} steps={total_steps} rebuild_ms={} longest_hold_ms={} gate_15s={} rebuilds={rebuilds}",
        runs.len(), elapsed.as_millis(), hold.as_millis(), if gate { "pass" } else { "fail" },
    );
    assert_eq!(rebuilds, 1, "one receive must rebuild exactly one part");
    assert_eq!(part_runs, runs.len(), "the measured rebuild must include every descendant");
    assert!(controller.mission_run(&root.id).unwrap().unwrap().outcome.is_none());
    assert!(worker.mission_run(&root.id).unwrap().unwrap().outcome.is_none());

    let backlog = setting("FRACTAL_HUGE_PART_BACKLOG", 0);
    if backlog == 0 {
        return;
    }

    // Advance the same canonical frontier again before creating the controller backlog.
    worker.set_write_clock_at(now_ms() + 2_000_000).unwrap();
    worker.append_claim(&ClaimInput {
        subject: "custom/huge-part/backlog-order".into(),
        kind: "custom.huge-part.order".into(),
        actor: None,
        fields: BTreeMap::new(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    worker.project_replication_backlog().unwrap();

    // Interleave runs first, then their steps, all within the measured part. Track each
    // step's actual final fixture state (including the single-claim measurement above)
    // so every requested backlog entry is a real state change, even across full rounds.
    let mut backlog_steps: Vec<_> = (0..steps_per_run).flat_map(|step| {
        runs.iter().map(move |run| (&run.steps[step].subject, history.is_multiple_of(2)))
    }).collect();
    backlog_steps[0].1 = state == "ready";
    for index in 0..backlog {
        let (subject, ready) = &mut backlog_steps[index % (runs.len() * steps_per_run)];
        assert!(controller.set_step_state(
            subject,
            if *ready { "blocked" } else { "ready" },
            Some("huge-part full receive backlog"),
        ).unwrap());
        *ready = !*ready;
    }

    // A scoped thread borrows the very same Sync Store: no second connection/store
    // changes the contention being measured. The stop channel wakes the 50 ms pause.
    let (mut probe_times, wall_times, received, total, longest_hold, rebuilds) =
        std::thread::scope(|scope| {
            let (stop, stopped) = std::sync::mpsc::channel();
            let (ready, started) = std::sync::mpsc::sync_channel(0);
            let worker = &worker;
            let probe = scope.spawn(move || {
                let mut latencies = Vec::new();
                ready.send(()).unwrap();
                loop {
                    let index = latencies.len();
                    let started = std::time::Instant::now();
                    worker.append_claim(&ClaimInput {
                        subject: format!("custom/huge-part/probe/{index}"),
                        kind: "custom.huge-part.probe".into(),
                        actor: None,
                        fields: BTreeMap::new(),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: None,
                    }).unwrap();
                    latencies.push(started.elapsed());
                    match stopped.recv_timeout(std::time::Duration::from_millis(50)) {
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                latencies
            });
            started.recv().unwrap();
            let mut wall_times = Vec::new();
            let mut received = 0;
            RUN_TREE_REBUILDS.with(|count| count.set(0));
            let started = std::time::Instant::now();
            let (_, longest_hold) = smallclaims::windows::holds::longest_during(|| {
                loop {
                    let exchange = exchange_from(&controller, &worker.replication_inventory().unwrap());
                    if exchange.envelopes.is_empty() {
                        break;
                    }
                    let page_started = std::time::Instant::now();
                    worker.receive_replication_exchange("controller", TEST_FLEET, &exchange).unwrap();
                    let admission = worker.validate_replication_backlog().unwrap();
                    assert_eq!(admission.invalid, 0);
                    assert_eq!(admission.unknown, 0);
                    assert!(worker.project_replication_backlog().unwrap());
                    wall_times.push(page_started.elapsed());
                    received += exchange.envelopes.len();
                }
            });
            let total = started.elapsed();
            let rebuilds = RUN_TREE_REBUILDS.with(std::cell::Cell::get);
            stop.send(()).unwrap();
            (probe.join().unwrap(), wall_times, received, total, longest_hold, rebuilds)
        });
    assert_eq!(received, backlog, "catch-up must consume the complete controller backlog");
    probe_times.sort_unstable();
    let percentile = |percent: usize| {
        probe_times[(probe_times.len() * percent).div_ceil(100) - 1].as_millis()
    };
    let max_page = *wall_times.iter().max().unwrap();
    let gate = max_page <= std::time::Duration::from_secs(15)
        && longest_hold <= std::time::Duration::from_secs(15);
    eprintln!(
        "huge_receive backlog={backlog} pages={} total_ms={} max_page_ms={} rebuilds={rebuilds} longest_hold_ms={} probe_n={} probe_p50_ms={} probe_p99_ms={} probe_max_ms={} gate_15s={}",
        wall_times.len(), total.as_millis(), max_page.as_millis(), longest_hold.as_millis(),
        probe_times.len(), percentile(50), percentile(99), probe_times.last().unwrap().as_millis(),
        if gate { "pass" } else { "fail" },
    );
    huge_part_digest::print_part_digest(&worker.connection.write(), &runs.iter().map(|run| run.id.clone()).collect(), &subjects);
    // Keep the replay oracle outside both the receive measurement and the probe window.
    let projected = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(projected, graph_digest_of(&worker));
}

#[test]
#[ignore = "same-host single huge live part writer-budget measurement"]
fn measure_single_huge_part_rebuild() {
    let steps = setting("FRACTAL_HUGE_PART_STEPS", 32);
    let generations = setting("FRACTAL_HUGE_PART_GENERATIONS", 1);
    let history = setting("FRACTAL_HUGE_PART_HISTORY", 8);
    assert!(steps > 0, "FRACTAL_HUGE_PART_STEPS must be positive");
    let sweep = std::env::var("FRACTAL_HUGE_PART_SWEEP").is_ok_and(|value| value == "1");
    let sizes = if sweep {
        vec![16, 64, 256]
    } else {
        vec![setting("FRACTAL_HUGE_PART_CHILDREN", 64)]
    };
    for children in sizes {
        // Gates are diagnostic only: print every point on both base and head.
        measure(children, steps, generations, history);
    }
}
