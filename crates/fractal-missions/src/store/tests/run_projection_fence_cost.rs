use super::*;
use super::coalesce_run_part_rebuild::write_coalesced_rebuild_backlog;

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| value.parse().unwrap())
}

fn pending_part(children: usize, steps: usize, generations: usize, history: usize) -> (Store, String, usize) {
    assert!(steps > 0);
    let controller = Store::open_memory("controller").unwrap();
    let mut revisions = Vec::new();
    for generation in 0..=generations {
        let mut source = format!("version 2\nmission \"fence-cost\" state=\"ready\" {{\n goal \"Revision {generation}.\"\n concurrent-runs max=1000000\n");
        for step in 0..steps {
            source.push_str(&format!(" step \"work-{step}\" {{ agentless; goal \"Revision {generation} work.\" }}\n"));
        }
        source.push_str("}\n");
        revisions.push(publish_mission(&controller, &source, &format!("fence-definition-{generation}")));
    }
    let request = |key: String| MissionRunRequest {
        mission: "fence-cost".into(), revision: Some(revisions[0].revision.clone()),
        workspace: "/tmp".into(), requester: Some("person/test".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: key,
    };
    let root = controller.create_mission_run(&request("fence-outer-root".into())).unwrap();
    let mut runs: Vec<MissionRunView> = Vec::new();
    for index in 0..=children {
        // The measured descendants all belong below ONE child of the still-live root.
        let parent = if index == 0 { &root } else { &runs[0] };
        let mut run = controller.create_child_mission_run(
            &request(format!("fence-run-{index}")), parent, &parent.steps[index % steps].subject, None,
        ).unwrap();
        for (generation, revision) in revisions.iter().enumerate() {
            if generation != 0 {
                run = controller.adopt_mission_revision(
                    &run.id, revision, "person/test", "Accumulate generation history",
                    &format!("fence-revise-{index}-{generation}"),
                ).unwrap();
            }
            for step in &run.steps {
                write_coalesced_rebuild_backlog(&controller, &step.subject, history);
            }
        }
        runs.push(run);
    }
    let worker = Store::open_memory("worker").unwrap();
    loop {
        let exchange = exchange_from(&controller, &worker.replication_inventory().unwrap());
        if exchange.envelopes.is_empty() { break; }
        receive_and_project(&worker, "controller", &exchange);
    }
    let step = runs[0].steps[0].subject.clone();
    controller.set_step_state(&step, if history % 2 == 1 { "ready" } else { "blocked" }, Some("pending external fence")).unwrap();
    receive_until_run_projection_is_pending(&controller, &worker);
    assert!(worker.run_projection_pending(&step).unwrap());
    (worker, step, runs.len())
}

fn action(transaction: &rusqlite::Transaction<'_>) -> Result<(), FractalError> {
    transaction.execute(/* sqlite-large-value: allow-for-now lv-ha6e753bedafa owner=fractal_missions::store::tests::run_projection_fence_cost TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET phase=phase WHERE id='absent-fence-cost-run'", [])
        .map_err(|error| FractalError::new("fence-cost", error.to_string()))?;
    Ok(())
}

#[test]
#[ignore = "same-host pending-run external mutation writer cost"]
fn measure_run_projection_fence_costs() {
    let worker = Store::open_memory("worker").unwrap();
    let iterations = setting("FRACTAL_FENCE_BENCH_ITERATIONS", 10_000);
    assert!(iterations > 0);
    for probe in [false, true] {
        let started = std::time::Instant::now();
        let (_, hold) = smallclaims::windows::holds::longest_during(|| {
            for _ in 0..iterations {
                let mut connection = worker.connection.write();
                let transaction = connection.transaction().unwrap();
                if probe {
                    assert!(!fractal_store::store::finish_all_run_projections_tx(&transaction).unwrap());
                }
                action(&transaction).unwrap();
                transaction.commit().unwrap();
            }
        });
        eprintln!("fence_cost empty_queue probe={probe} n={iterations} us_per_op={:.3} longest_hold_us={}",
            started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64, hold.as_micros());
    }
    let (small, step, runs) = pending_part(0, 1, 0, 0);
    RUN_TREE_REBUILDS.with(|count| count.set(0));
    let started = std::time::Instant::now();
    let (_, hold) = smallclaims::windows::holds::longest_during(|| {
        small.with_all_runs_fenced(action).unwrap();
    });
    eprintln!("fence_cost small_pending with_all_runs_fenced runs={runs} wall_ms={} longest_hold_ms={} rebuilds={}",
        started.elapsed().as_millis(), hold.as_millis(), RUN_TREE_REBUILDS.with(std::cell::Cell::get));
    assert!(!small.run_projection_pending(&step).unwrap());
    assert_eq!(RUN_TREE_REBUILDS.with(std::cell::Cell::get), 1);

    let children = setting("FRACTAL_HUGE_PART_CHILDREN", 64);
    let steps = setting("FRACTAL_HUGE_PART_STEPS", 32);
    let generations = setting("FRACTAL_HUGE_PART_GENERATIONS", 1);
    let history = setting("FRACTAL_HUGE_PART_HISTORY", 8);
    let (large, step, runs) = pending_part(children, steps, generations, history);
    RUN_TREE_REBUILDS.with(|count| count.set(0));
    RUN_PART_RUNS.with(|count| count.set(0));
    let started = std::time::Instant::now();
    let (_, hold) = smallclaims::windows::holds::longest_during(|| {
        large.with_run_fence(&step, action).unwrap();
    });
    eprintln!("fence_cost large_pending with_run_fence runs={runs} steps_per_run={steps} generations={} history={history} wall_ms={} longest_hold_ms={} rebuilds={}",
        generations + 1, started.elapsed().as_millis(), hold.as_millis(), RUN_TREE_REBUILDS.with(std::cell::Cell::get));
    assert_eq!(RUN_TREE_REBUILDS.with(std::cell::Cell::get), 1);
    assert_eq!(RUN_PART_RUNS.with(std::cell::Cell::get), runs);
    assert!(!large.run_projection_pending(&step).unwrap());
}
