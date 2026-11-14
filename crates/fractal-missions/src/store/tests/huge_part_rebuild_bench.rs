#[path = "huge_part_digest.rs"] mod huge_part_digest;
#[path = "huge_part_snapshot.rs"] mod huge_part_snapshot;
// Apply this benchmark patch to both comparison revisions; contended mode needs bounded fences.
use super::*;
use super::coalesce_run_part_rebuild::write_coalesced_rebuild_backlog;
use fractal_store::store::rebuild_observer::{Collector, Event};

fn target_rebuild(event: &Event, part: &str, root: &str) -> bool {
    (event.kind == "run-part" && event.id == part)
        || (event.kind == "run-tree" && (event.id == root || event.id == part))
}

fn rebuild_breakdown(events: &[Event]) -> BTreeMap<String, (usize, f64)> {
    let mut counts = BTreeMap::new();
    for event in events {
        let row = counts.entry(format!("{}/{}/{}", event.kind, event.role, event.cause))
            .or_insert((0, 0.0));
        row.0 += 1;
        row.1 += event.elapsed_ms;
    }
    counts
}

fn target_page_max(events: &[Event], ends: &[(usize, std::time::Duration)], part: &str, root: &str) -> usize {
    let mut previous = 0;
    let mut maximum = 0;
    for (index, (end, _)) in ends.iter().copied().enumerate() {
        // Work completing after the final receive is part of that last page's budget.
        let end = if index + 1 == ends.len() { events.len() } else { end };
        maximum = maximum.max(events[previous..end].iter()
            .filter(|event| target_rebuild(event, part, root)).count());
        previous = end;
    }
    maximum
}

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

fn measure(children: usize, steps_per_run: usize, extra_generations: usize, history: usize, contended: bool) {
    let controller = Store::open_memory("controller").unwrap();
    let mut revisions = Vec::new();
    let mut claim_revisions = Vec::new();
    for generation in 0..=extra_generations {
        let mut source = format!(
            "version 2\nmission \"huge-part\" state=\"ready\" {{\n goal \"Keep revision {generation} live.\"\n concurrent-runs max=1000000\n"
        );
        if contended {
            source.insert_str(0, "version 2\nagent \"bench\" { workspace \"/tmp\"; command \"true\" }\nagent \"claim\" { workspace \"/tmp\"; command \"true\" }\n");
            // The publisher's fixture namespace is `node`, independently of store origin.
            source = source.replacen("version 2\nmission", "mission", 1);
        }
        for step in 0..steps_per_run {
            let assignee = if contended { "assigned-to \"agent/node.bench\"" } else { "agentless" };
            source.push_str(&format!(
                " step \"work-{step}\" {{ {assignee}; goal \"Revision {generation} work.\" }}\n"
            ));
        }
        source.push_str("}\n");
        revisions.push(publish_mission(&controller, &source, &format!("huge-definition-{generation}")));
        if contended {
            // Pin only the contended run's work-0 to a seat with no other ready work.
            let claim_source = source.replacen(
                " step \"work-0\" { assigned-to \"agent/node.bench\";",
                " step \"work-0\" { assigned-to \"agent/node.claim\";",
                1,
            );
            claim_revisions.push(publish_mission(
                &controller, &claim_source, &format!("huge-claim-definition-{generation}"),
            ));
        }
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
        let run_revisions = if contended && index == children { &claim_revisions } else { &revisions };
        let parent = if index == 0 { &root } else { &runs[0] };
        let mut run = controller.create_child_mission_run(
            &request(format!("huge-part-run-{index}"), run_revisions[0].revision.clone()),
            parent, &parent.steps[index % steps_per_run].subject, None,
        ).unwrap();
        // Finish this run's revisions before attaching descendants: revision adoption
        // intentionally cancels descendants of the superseded generation.
        subjects.insert(run.subject.clone());
        for (generation, revision) in run_revisions.iter().enumerate() {
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

    let claim_subject = runs.last().unwrap().steps[0].subject.clone();
    if contended {
        controller.set_step_state(&claim_subject, "ready", None).unwrap();
    }
    let open_started = std::time::Instant::now();
    let worker = std::sync::Arc::new(Store::open_memory("worker").unwrap());
    let open_ms = open_started.elapsed().as_millis();
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

    // Negotiate the same page cap as production; bare fixture inventories default to 512.
    let page = setting(
        "FRACTAL_HUGE_PART_PAGE",
        smallclaims::store::replication_page_limit(&worker.replication_inventory().unwrap()),
    );
    assert!((1..=smallclaims::store::REPLICATION_PAGE_LIMIT as usize).contains(&page));
    if contended {
        assert!(page >= 4096, "contended mutations require production-size pages");
        assert!(backlog >= page, "contended mutations require at least one full page");
    }
    let accepts = u32::try_from(page).unwrap();

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
    if contended {
        backlog_steps.retain(|(subject, _)| subject.as_str() != claim_subject);
        assert!(!backlog_steps.is_empty(), "contention needs another step in the live part");
    }
    let backlog_step_count = backlog_steps.len();
    for index in 0..backlog {
        let (subject, ready) = &mut backlog_steps[index % backlog_step_count];
        assert!(controller.set_step_state(
            subject,
            if *ready { "blocked" } else { "ready" },
            Some("huge-part full receive backlog"),
        ).unwrap());
        *ready = !*ready;
    }

    let collector = Collector::new();
    let _observer = collector.install();
    let page_ends = std::sync::Mutex::new(Vec::new());
    // A scoped thread borrows the very same Sync Store: no second connection/store
    // changes the contention being measured. The stop channel wakes the 50 ms pause.
    let ((mut probe_times, wall_times, received, total, longest_hold, rebuilds), mutation_hold, mutation_rebuilds, route_pass) =
        with_contended_mutations(contended, std::sync::Arc::clone(&worker), claim_subject, &collector, (&runs[0].id, &root.id), &page_ends, || std::thread::scope(|scope| {
            let (stop, stopped) = std::sync::mpsc::channel();
            let (ready, started) = std::sync::mpsc::sync_channel(0);
            let worker = &worker;
            let probe = scope.spawn(move || {
                let mut latencies = Vec::new();
                ready.send(()).unwrap();
                let (_, longest_hold) = smallclaims::windows::holds::longest_during(|| loop {
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
                });
                (latencies, longest_hold)
            });
            started.recv().unwrap();
            let mut wall_times = Vec::new();
            let mut received = 0;
            RUN_TREE_REBUILDS.with(|count| count.set(0));
            let started = std::time::Instant::now();
            let (_, longest_hold) = smallclaims::windows::holds::longest_during(|| {
                loop {
                    let mut inventory = worker.replication_inventory().unwrap();
                    inventory.accepts = Some(accepts);
                    let exchange = exchange_from(&controller, &inventory);
                    assert!(exchange.envelopes.len() <= page, "receive exceeded negotiated page cap");
                    if exchange.envelopes.is_empty() {
                        break;
                    }
                    let page_started = std::time::Instant::now();
                    let page_metrics = smallclaims::receive_metrics::RequestMetrics::default();
                    let _page_metrics = page_metrics.enter();
                    let receive_started = std::time::Instant::now();
                    worker.receive_replication_exchange("controller", TEST_FLEET, &exchange).unwrap();
                    let receive_time = receive_started.elapsed();
                    let admission_started = std::time::Instant::now();
                    let admission = worker.validate_replication_backlog().unwrap();
                    let admission_time = admission_started.elapsed();
                    assert_eq!(admission.invalid, 0);
                    assert_eq!(admission.unknown, 0);
                    let projection_started = std::time::Instant::now();
                    let projected = worker.project_replication_backlog().unwrap();
                    let projection_time = projection_started.elapsed();
                    assert!(projected || contended);
                    let end = collector.event_count();
                    let page_elapsed = page_started.elapsed();
                    page_ends.lock().unwrap().push((end, page_elapsed));
                    wall_times.push((exchange.envelopes.len(), page_elapsed,
                        page_metrics.snapshot(), end, (receive_time, admission_time, projection_time)));
                    received += exchange.envelopes.len();
                }
                // Keep both the probe and mutation writers running during timed settling.
                // Unwinding must stop the mutation writers before their scope joins them.
                let mut settle_passes = 0;
                while worker.replication_projection_deferred() && settle_passes < 8 {
                    worker.project_replication_backlog().unwrap();
                    settle_passes += 1;
                }
                let pending_runs = runs.iter()
                    .filter(|run| worker.run_projection_pending(&run.subject).unwrap())
                    .count();
                let deferred = worker.replication_projection_deferred();
                eprintln!("huge_receive_settle passes={settle_passes} deferred={deferred} pending_runs={pending_runs} writers=running contended={contended}");
                assert!(!deferred, "deferred projection still set after {settle_passes} settle passes; pending_runs={pending_runs}");
            });
            for run in &runs {
                assert!(!worker.run_projection_pending(&run.subject).unwrap());
            }
            let total = started.elapsed();
            let rebuilds = RUN_TREE_REBUILDS.with(std::cell::Cell::get);
            stop.send(()).unwrap();
            let (probe_times, probe_hold) = probe.join().unwrap();
            (probe_times, wall_times, received, total, longest_hold.max(probe_hold), rebuilds)
        }));
    let longest_hold = longest_hold.max(mutation_hold);
    let rebuilds = rebuilds + mutation_rebuilds;
    assert_eq!(received, backlog, "catch-up must consume the complete controller backlog");
    probe_times.sort_unstable();
    let percentile = |percent: usize| {
        probe_times[(probe_times.len() * percent).div_ceil(100) - 1].as_millis()
    };
    let events = collector.take();
    let redundant = events.iter().filter(|event| event.local_redundant).count();
    let target_total = events.iter().filter(|event| target_rebuild(event, &runs[0].id, &root.id)).count();
    let other_rebuilds = events.len() - target_total;
    let mut previous = 0;
    let mut target_page_max = 0;
    for (index, (envelopes, elapsed, metrics, end, (receive_time, admission_time, projection_time))) in wall_times.iter().enumerate() {
        // Assign any action/drain completing after the last receive to that final page,
        // rather than dropping in-flight mutation work from attribution.
        let end = if index + 1 == wall_times.len() { events.len() } else { *end };
        let page_events = &events[previous..end];
        previous = end;
        let target = page_events.iter().filter(|event| target_rebuild(event, &runs[0].id, &root.id)).count();
        target_page_max = target_page_max.max(target);
        let local = page_events.iter().filter(|event| event.local_redundant).count();
        eprintln!("huge_receive_page index={index} envelopes={envelopes} wall_ms={} receive_ms={} admission_validation_ms={} projection_ms={} gate_wait_ms={} writer_wait_ms={} final_drain_ms={} final_drain_count={} projection_chunks={} target_rebuilds={target} target_limit=2 target_gate={} redundant_local_rebuilds={local} redundant_local_gate={} rebuild_causes={:?}",
            elapsed.as_millis(), receive_time.as_millis(), admission_time.as_millis(), projection_time.as_millis(),
            metrics.gate_wait.as_millis(), metrics.writer_wait.as_millis(),
            metrics.final_drain_time.as_millis(), metrics.final_drain_count, metrics.projection_chunks,
            if target <= 2 { "pass" } else { "fail" }, if local == 0 { "pass" } else { "fail" },
            rebuild_breakdown(page_events));
    }
    let max_page = wall_times.iter().map(|(_, elapsed, _, _, _)| *elapsed).max().unwrap();
    let m1 = longest_hold < std::time::Duration::from_secs(15);
    let m2 = max_page < std::time::Duration::from_secs(15);
    let m3 = total < std::time::Duration::from_secs(60);
    let verdict = |pass| if pass { "pass" } else { "fail" };
    let rebuilds_per_page = rebuilds as f64 / wall_times.len() as f64;
    eprintln!(
        "huge_receive backlog={backlog} page={page} pages={} open_ms={open_ms} total_ms={} max_page_ms={} rebuilds={rebuilds} rebuilds_per_page={rebuilds_per_page:.3} longest_hold_ms={} probe_n={} probe_p50_ms={} probe_p99_ms={} probe_max_ms={} m1_max_hold_ms={} m1_limit_ms=15000 m1_gate={} m2_max_page_ms={} m2_limit_ms=15000 m2_gate={} m3_component_ms={} m3_limit_ms=60000 m3_gate={} gate_15s={} gate={} target_rebuilds={target_total} other_rebuilds={other_rebuilds} target_page_max={target_page_max} target_limit=2 target_gate={} redundant_local_rebuilds={redundant} redundant_local_gate={} route_gate={} rebuild_causes={:?}",
        wall_times.len(), total.as_millis(), max_page.as_millis(), longest_hold.as_millis(),
        probe_times.len(), percentile(50), percentile(99), probe_times.last().unwrap().as_millis(),
        longest_hold.as_millis(), verdict(m1), max_page.as_millis(), verdict(m2),
        total.as_millis(), verdict(m3), verdict(m1 && m2),
        verdict(m1 && m2 && m3 && target_page_max <= 2 && redundant == 0 && route_pass),
        verdict(target_page_max <= 2), verdict(redundant == 0), verdict(route_pass), rebuild_breakdown(&events),
    );
    assert!(target_page_max <= 2, "target live part rebuilt more than twice in one page");
    assert_eq!(redundant, 0, "origin-local already-applied work must not trigger rebuilds");
    assert!(m2, "M2 receive page exceeded 15 seconds, including under contention");
    assert!(route_pass, "at least one route made zero successes or failed bounded mutation progress");
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
        // Every sweep point enforces the per-page rebuild and receive-wall gates.
        measure(children, steps, generations, history, false);
    }
}

// This is the store-executor fallback, not the daemon router/authentication stack.
// The loopback adapter runs real work_action/ask_person, including their 8 s fences;
// Client::request owns the production 60 s refusal retry loop (no copied retry code).
fn with_contended_mutations<T>(
    enabled: bool,
    worker: std::sync::Arc<Store>,
    claim_subject: String,
    collector: &Collector,
    target: (&str, &str),
    page_ends: &std::sync::Mutex<Vec<(usize, std::time::Duration)>>,
    receive: impl FnOnce() -> T,
) -> (T, std::time::Duration, usize, bool) {
    use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
    use fractal_secrets::client::{Client, Endpoint, set_mutation_observer};
    use crate::model::{PersonAskRequest, WorkRequest};
    use crate::store::person_work::PersonWorkStore;
    use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

    if !enabled {
        return (receive(), std::time::Duration::ZERO, 0, true);
    }
    #[derive(Default)]
    struct RouteSamples {
        calls: Vec<(u32, std::time::Duration)>,
        successes: usize,
        not_ready: usize,
        errors: usize,
        handler_error_logged: bool,
    }
    type Samples = Arc<Mutex<BTreeMap<String, RouteSamples>>>;
    type StateValue = (Arc<Store>, String, Samples, Arc<Mutex<(std::time::Duration, usize)>>);
    let samples: Samples = Arc::default();
    let observed = Arc::clone(&samples);
    set_mutation_observer(move |observation| {
        observed.lock().unwrap().entry(observation.route.into()).or_default()
            .calls.push((observation.attempts, observation.took));
    });
    let holds = Arc::new(Mutex::new((std::time::Duration::ZERO, 0)));
    let state: StateValue = (worker, claim_subject, Arc::clone(&samples), Arc::clone(&holds));
    async fn mutate(
        State((worker, subject, samples, holds)): State<StateValue>,
        axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        tokio::task::spawn_blocking(move || {
            let route = uri.path();
            RUN_TREE_REBUILDS.with(|count| count.set(0));
            let (result, held) = smallclaims::windows::holds::longest_during(|| {
                if matches!(route, "/bench/work-claim" | "/bench/work-release") {
                    let request: WorkRequest = serde_json::from_value(body).unwrap();
                    let action = if route == "/bench/work-claim" { "claim" } else { "release" };
                    worker.work_action(&subject, action, &request)
                } else {
                    let request: PersonAskRequest = serde_json::from_value(body).unwrap();
                    worker.ask_person(&request)
                }
            });
            {
                let mut metrics = holds.lock().unwrap();
                metrics.0 = metrics.0.max(held);
                metrics.1 += RUN_TREE_REBUILDS.with(std::cell::Cell::get);
            }
            match result {
                Ok(value) => (StatusCode::OK, Json(json!({"api_version": "fractal.v1", "value": value}))),
                Err(error) => {
                    let not_ready = error.code == "database-busy"
                        && error.details.get("reason").and_then(serde_json::Value::as_str) == Some("run-projection-drain");
                    {
                        let mut samples = samples.lock().unwrap();
                        let row = samples.entry(route.into()).or_default();
                        if not_ready {
                            row.not_ready += 1;
                        } else if !row.handler_error_logged {
                            eprintln!("contended_mutation_first_422 route={route} code={} message={}", error.code, error.message);
                            row.handler_error_logged = true;
                        }
                    }
                    (if not_ready { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::UNPROCESSABLE_ENTITY },
                        Json(json!({"code": error.code, "message": error.message, "details": error.details})))
                }
            }
        }).await.unwrap()
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let (shutdown, stopped) = tokio::sync::oneshot::channel();
    let server = std::thread::spawn(move || {
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let router = Router::new()
                .route("/bench/work-claim", post(mutate))
                .route("/bench/work-release", post(mutate))
                .route("/bench/person-ask", post(mutate))
                .with_state(state);
            axum::serve(listener, router).with_graceful_shutdown(async { let _ = stopped.await; }).await.unwrap();
        });
    });
    let done = AtomicBool::new(false);
    let barrier = std::sync::Barrier::new(3);
    let result = std::thread::scope(|scope| {
        let mut mutations = Vec::new();
        for route in ["/bench/work-claim", "/bench/person-ask"] {
            let endpoint = &endpoint;
            let samples = &samples;
            let done = &done;
            let barrier = &barrier;
            mutations.push(scope.spawn(move || {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                runtime.block_on(async {
                    let client = Client::new(Endpoint::Http(endpoint.clone()));
                    barrier.wait();
                    let mut index = 0;
                    let mut error_logged = false;
                    let mut release_error_logged = false;
                    // Always finish any in-flight logical mutation, even after receive ends.
                    loop {
                        let key = format!("huge-contended-{route}-{index}");
                        let body = if route == "/bench/work-claim" {
                            json!({"actor": "agent/node.claim", "incarnation": "huge-claim", "idempotency_key": key})
                        } else {
                            json!({"person": "person/test", "actor": "agent/node.bench", "title": "Huge-part contention", "reason": "Measure bounded mutation progress", "idempotency_key": key})
                        };
                        let response: anyhow::Result<serde_json::Value> = client.request("POST", route, Some(&body)).await;
                        if let Err(error) = &response {
                            if !error_logged {
                                eprintln!("contended_mutation_first_error route={route} error={error:#}");
                                error_logged = true;
                            }
                            samples.lock().unwrap().entry(route.into()).or_default().errors += 1;
                        } else {
                            samples.lock().unwrap().entry(route.into()).or_default().successes += 1;
                            if route == "/bench/work-claim" {
                                // Release completes this logical pair even when receive has ended.
                                let release_route = "/bench/work-release";
                                let body = json!({
                                    "actor": "agent/node.claim",
                                    "incarnation": "huge-claim",
                                    "idempotency_key": format!("huge-contended-{release_route}-{index}"),
                                });
                                let release: anyhow::Result<serde_json::Value> =
                                    client.request("POST", release_route, Some(&body)).await;
                                if let Err(error) = release {
                                    if !release_error_logged {
                                        eprintln!("contended_mutation_first_error route={release_route} error={error:#}");
                                        release_error_logged = true;
                                    }
                                    samples.lock().unwrap().entry(release_route.into()).or_default().errors += 1;
                                } else {
                                    samples.lock().unwrap().entry(release_route.into()).or_default().successes += 1;
                                }
                            }
                        }
                        index += 1;
                        if done.load(Ordering::Acquire) { break; }
                    }
                });
            }));
        }
        struct StopMutations<'a>(&'a AtomicBool);
        impl Drop for StopMutations<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        // This guard lives inside the scope, so a receive panic signals stop before
        // the scope waits for its threads. Each thread still finishes its logical pair.
        let stop_mutations = StopMutations(&done);
        barrier.wait();
        let result = receive();
        drop(stop_mutations);
        for mutation in mutations { mutation.join().unwrap(); }
        result
    });
    shutdown.send(()).unwrap();
    server.join().unwrap();
    let samples = samples.lock().unwrap();
    let rebuild_events = collector.snapshot();
    let (part, root) = target;
    let target_rebuilds = rebuild_events.iter().filter(|event| target_rebuild(event, part, root)).count();
    let pages = page_ends.lock().unwrap();
    let target_page_max = target_page_max(&rebuild_events, &pages, part, root);
    let max_page = pages.iter().map(|(_, elapsed)| *elapsed).max().unwrap_or_default();
    let m2 = max_page < std::time::Duration::from_secs(15);
    let other_rebuilds = rebuild_events.len() - target_rebuilds;
    let mut route_pass = true;
    let redundant_local = rebuild_events.iter().filter(|event| event.local_redundant).count();
    for route in ["/bench/work-claim", "/bench/work-release", "/bench/person-ask"] {
        let empty = RouteSamples::default();
        let row = samples.get(route).unwrap_or(&empty);
        let mut attempts: Vec<_> = row.calls.iter().map(|(attempts, _)| *attempts).collect();
        attempts.sort_unstable();
        let n = attempts.len();
        let attempts_max = attempts.last().copied().unwrap_or_default();
        let attempts_p50 = attempts.get(n.saturating_sub(1) / 2).copied().unwrap_or_default();
        // End-to-end logical mutation time including gate, writer and retry waits.
        let mut took: Vec<_> = row.calls.iter().map(|(_, took)| *took).collect();
        took.sort_unstable();
        let took_max = took.last().copied().unwrap_or_default();
        let took_p50 = took.get(n.saturating_sub(1) / 2).copied().unwrap_or_default();
        let took_p99 = took.get((n * 99).div_ceil(100).saturating_sub(1)).copied().unwrap_or_default();
        let m1_latency = took_max < std::time::Duration::from_secs(15);
        let pass = attempts_max <= 3 && took_max <= std::time::Duration::from_secs(60) && row.errors == 0 && row.successes > 0;
        eprintln!("contended_mutations route={route} n={n} successes={} refusals={} errors={} attempts_max={attempts_max} attempts_p50={attempts_p50} total_ms_p50={} total_ms_p99={} total_ms_max={} total_vs_15s={} gate={} target_rebuilds={target_rebuilds} other_rebuilds={other_rebuilds} target_page_max={target_page_max} target_limit=2 target_gate={} redundant_local_rebuilds={redundant_local} redundant_local_gate={} m2_max_page_ms={} m2_limit_ms=15000 m2_gate={} rebuild_causes={:?}",
            row.successes, row.not_ready, row.errors, took_p50.as_millis(), took_p99.as_millis(), took_max.as_millis(),
            if m1_latency { "pass" } else { "fail" }, if pass && redundant_local == 0 && target_page_max <= 2 && m2 { "pass" } else { "fail" },
            if target_page_max <= 2 { "pass" } else { "fail" },
            if redundant_local == 0 { "pass" } else { "fail" }, max_page.as_millis(),
            if m2 { "pass" } else { "fail" }, rebuild_breakdown(&rebuild_events));
        route_pass &= pass;
    }
    let metrics = *holds.lock().unwrap();
    (result, metrics.0, metrics.1, route_pass)
}

#[test]
#[ignore = "same-host production-page receive with bounded work-claim/person-ask contention"]
fn measure_huge_part_contended_mutations() {
    let steps = setting("FRACTAL_HUGE_PART_STEPS", 32);
    let generations = setting("FRACTAL_HUGE_PART_GENERATIONS", 1);
    let history = setting("FRACTAL_HUGE_PART_HISTORY", 8);
    let children = setting("FRACTAL_HUGE_PART_CHILDREN", 64);
    assert!(steps > 0);
    assert!(setting("FRACTAL_HUGE_PART_BACKLOG", 0) >= 4096);
    assert!(setting("FRACTAL_HUGE_PART_PAGE", 512) >= 4096);
    assert!((children + 1) * steps > 1);
    measure(children, steps, generations, history, true);
}
