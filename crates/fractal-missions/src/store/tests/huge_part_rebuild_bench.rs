#[path = "huge_part_digest.rs"] mod huge_part_digest;
#[path = "huge_part_snapshot.rs"] mod huge_part_snapshot;
// Apply this benchmark patch to both comparison revisions; contended mode needs bounded fences.
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

fn measure(children: usize, steps_per_run: usize, extra_generations: usize, history: usize, contended: bool) {
    let controller = Store::open_memory("controller").unwrap();
    let mut revisions = Vec::new();
    for generation in 0..=extra_generations {
        let mut source = format!(
            "version 2\nmission \"huge-part\" state=\"ready\" {{\n goal \"Keep revision {generation} live.\"\n concurrent-runs max=1000000\n"
        );
        if contended {
            source.insert_str(0, "version 2\nagent \"bench\" { workspace \"/tmp\"; command \"true\" }\n");
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

    // A scoped thread borrows the very same Sync Store: no second connection/store
    // changes the contention being measured. The stop channel wakes the 50 ms pause.
    let ((mut probe_times, wall_times, received, total, longest_hold, rebuilds), mutation_hold, mutation_rebuilds) =
        with_contended_mutations(contended, std::sync::Arc::clone(&worker), claim_subject, || std::thread::scope(|scope| {
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
                    worker.receive_replication_exchange("controller", TEST_FLEET, &exchange).unwrap();
                    let admission = worker.validate_replication_backlog().unwrap();
                    assert_eq!(admission.invalid, 0);
                    assert_eq!(admission.unknown, 0);
                    let projected = worker.project_replication_backlog().unwrap();
                    assert!(projected || contended);
                    wall_times.push((exchange.envelopes.len(), page_started.elapsed()));
                    received += exchange.envelopes.len();
                }
                // Concurrent local probe writes can advance the index past a finishing pass,
                // which then keeps the deferral (smallclaims store.rs "Preserve newer projection
                // deferrals"). The daemon's next pass settles it; count those timed passes.
                let mut settle_passes = 0;
                while worker.replication_projection_deferred() && settle_passes < 8 {
                    worker.project_replication_backlog().unwrap();
                    settle_passes += 1;
                }
                eprintln!("huge_receive_settle passes={settle_passes}");
            });
            assert!(!worker.replication_projection_deferred());
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
    for (index, (envelopes, elapsed)) in wall_times.iter().enumerate() {
        eprintln!("huge_receive_page index={index} envelopes={envelopes} wall_ms={}", elapsed.as_millis());
    }
    let max_page = wall_times.iter().map(|(_, elapsed)| *elapsed).max().unwrap();
    let m1 = longest_hold < std::time::Duration::from_secs(15);
    let m2 = max_page < std::time::Duration::from_secs(15);
    let m3 = total < std::time::Duration::from_secs(60);
    let verdict = |pass| if pass { "pass" } else { "fail" };
    let rebuilds_per_page = rebuilds as f64 / wall_times.len() as f64;
    eprintln!(
        "huge_receive backlog={backlog} page={page} pages={} open_ms={open_ms} total_ms={} max_page_ms={} rebuilds={rebuilds} rebuilds_per_page={rebuilds_per_page:.3} longest_hold_ms={} probe_n={} probe_p50_ms={} probe_p99_ms={} probe_max_ms={} m1_max_hold_ms={} m1_limit_ms=15000 m1_gate={} m2_max_page_ms={} m2_limit_ms=15000 m2_gate={} m3_component_ms={} m3_limit_ms=60000 m3_gate={} gate_15s={} gate={}",
        wall_times.len(), total.as_millis(), max_page.as_millis(), longest_hold.as_millis(),
        probe_times.len(), percentile(50), percentile(99), probe_times.last().unwrap().as_millis(),
        longest_hold.as_millis(), verdict(m1), max_page.as_millis(), verdict(m2),
        total.as_millis(), verdict(m3), verdict(m1 && m2), verdict(m1 && m2 && m3),
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
    receive: impl FnOnce() -> T,
) -> (T, std::time::Duration, usize) {
    use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
    use fractal_secrets::client::{Client, Endpoint, set_mutation_observer};
    use crate::model::{PersonAskRequest, WorkRequest};
    use crate::store::person_work::PersonWorkStore;
    use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

    if !enabled {
        return (receive(), std::time::Duration::ZERO, 0);
    }
    #[derive(Default)]
    struct RouteSamples {
        calls: Vec<(u32, std::time::Duration)>,
        successes: usize,
        not_ready: usize,
        errors: usize,
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
                if route == "/bench/work-claim" {
                    let request: WorkRequest = serde_json::from_value(body).unwrap();
                    worker.work_action(&subject, "claim", &request)
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
                    if not_ready {
                        samples.lock().unwrap().entry(route.into()).or_default().not_ready += 1;
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
                    // Always finish any in-flight logical mutation, even after receive ends.
                    loop {
                        let key = format!("huge-contended-{route}-{index}");
                        let body = if route == "/bench/work-claim" {
                            json!({"actor": "agent/node.bench", "incarnation": "huge-bench", "idempotency_key": key})
                        } else {
                            json!({"person": "person/test", "actor": "agent/node.bench", "title": "Huge-part contention", "reason": "Measure bounded mutation progress", "idempotency_key": key})
                        };
                        let response: anyhow::Result<serde_json::Value> = client.request("POST", route, Some(&body)).await;
                        if response.is_err() {
                            samples.lock().unwrap().entry(route.into()).or_default().errors += 1;
                        } else {
                            samples.lock().unwrap().entry(route.into()).or_default().successes += 1;
                        }
                        index += 1;
                        if done.load(Ordering::Acquire) { break; }
                    }
                });
            }));
        }
        barrier.wait();
        let result = receive();
        done.store(true, Ordering::Release);
        for mutation in mutations { mutation.join().unwrap(); }
        result
    });
    shutdown.send(()).unwrap();
    server.join().unwrap();
    let samples = samples.lock().unwrap();
    for route in ["/bench/work-claim", "/bench/person-ask"] {
        let row = samples.get(route).expect("mutation observer must report each route");
        let mut attempts: Vec<_> = row.calls.iter().map(|(attempts, _)| *attempts).collect();
        attempts.sort_unstable();
        let n = attempts.len();
        assert!(n > 0, "each route must issue mutations during receive");
        let attempts_max = attempts[n - 1];
        let attempts_p50 = attempts[n.div_ceil(2) - 1];
        // End-to-end logical mutation time including gate, writer and retry waits.
        let mut took: Vec<_> = row.calls.iter().map(|(_, took)| *took).collect();
        took.sort_unstable();
        let took_max = took[n - 1];
        let took_p50 = took[n.div_ceil(2) - 1];
        let took_p99 = took[(n * 99).div_ceil(100) - 1];
        let m1_latency = took_max < std::time::Duration::from_secs(15);
        let pass = attempts_max <= 3 && took_max <= std::time::Duration::from_secs(60) && row.errors == 0 && row.successes > 0;
        eprintln!("contended_mutations route={route} n={n} successes={} refusals={} errors={} attempts_max={attempts_max} attempts_p50={attempts_p50} total_ms_p50={} total_ms_p99={} total_ms_max={} total_vs_15s={} gate={}",
            row.successes, row.not_ready, row.errors, took_p50.as_millis(), took_p99.as_millis(), took_max.as_millis(),
            if m1_latency { "pass" } else { "fail" }, if pass { "pass" } else { "fail" });
    }
    let metrics = *holds.lock().unwrap();
    (result, metrics.0, metrics.1)
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
