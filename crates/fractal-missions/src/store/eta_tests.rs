use super::*;

fn publish(store: &Store, goal: &str) {
    let source = format!(
        "version 2\nmission \"eta\" state=\"ready\" {{ concurrent-runs max=100; goal \"Estimate.\"; step \"work\" {{ agentless; goal {goal:?} }} }}"
    );
    let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
    let plan = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store.apply(&intent, &plan.subject_tokens, goal).unwrap();
}

fn run(store: &Store, key: &str) -> MissionRunView {
    store
        .create_mission_run(&MissionRunRequest {
            mission: "eta".into(),
            revision: None,
            workspace: ".".into(),
            requester: None,
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: key.into(),
        })
        .unwrap()
}

#[test]
fn eta_samples_are_incremental_definition_scoped_and_null_when_finished() {
    let store = Store::open_memory("node").unwrap();
    publish(&store, "Initial work.");
    let historical = run(&store, "prior");
    store
        .set_step_state(&historical.steps[0].subject, "working", None)
        .unwrap();
    store
        .set_step_state(&historical.steps[0].subject, "completed", None)
        .unwrap();
    store
        .set_mission_run_state(&historical.id, "completed", "terminal", None)
        .unwrap();
    let current = run(&store, "current");
    let eta = store
        .mission_run_eta(&current.subject, 123_456)
        .unwrap()
        .unwrap();
    assert_eq!(eta["as_of"], 123_456);
    assert_eq!(eta["basis"], json!({"method":"per-step", "samples":1}));
    assert!(eta["remaining_low_ms"].is_u64());
    assert_eq!(eta["remaining_low_ms"], eta["remaining_high_ms"]);
    assert_eq!(
        store.mission_run_eta(&current.subject, 123_456).unwrap(),
        Some(eta)
    );
    assert!(
        store
            .mission_run_eta(&historical.subject, 123_456)
            .unwrap()
            .is_none()
    );
    store
        .set_mission_run_state(&current.id, "cancelled", "terminal", None)
        .unwrap();
    assert!(
        store
            .mission_run_eta(&current.subject, 123_456)
            .unwrap()
            .is_none()
    );
    publish(&store, "Changed work.");
    let changed = run(&store, "changed");
    assert!(
        store
            .mission_run_eta(&changed.subject, 123_456)
            .unwrap()
            .is_none()
    );
}

#[test]
fn eta_sample_projection_is_bounded_and_reads_do_not_repopulate_history() {
    let store = Store::open_memory("node").unwrap();
    publish(&store, "Bounded work.");
    for index in 0..66 {
        let prior = run(&store, &format!("prior-{index}"));
        store
            .set_step_state(&prior.steps[0].subject, "working", None)
            .unwrap();
        store
            .set_step_state(&prior.steps[0].subject, "completed", None)
            .unwrap();
        store
            .set_mission_run_state(&prior.id, "completed", "terminal", None)
            .unwrap();
    }
    let current = run(&store, "current");
    let count = || {
        store
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM local_run_eta_samples", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap()
    };
    assert_eq!(count(), 64);
    let before = store.connection.lock().unwrap().total_changes();
    for at in 0..10 {
        let eta = store
            .mission_run_eta(&current.subject, at)
            .unwrap()
            .unwrap();
        assert_eq!(eta["basis"]["samples"], 64);
        assert_eq!(eta["as_of"].as_u64(), u64::try_from(at).ok());
    }
    assert_eq!(count(), 64);
    assert_eq!(store.connection.lock().unwrap().total_changes(), before);
}

#[test]
fn eta_uses_declared_critical_paths_and_minimum_sample_count() {
    for critical in [false, true] {
        let store = Store::open_memory("node").unwrap();
        let dependency = if critical {
            "depends-on { step \"a\" completed; step \"b\" completed }"
        } else {
            ""
        };
        let source = format!(
            "version 2\nmission \"eta\" state=\"ready\" {{ goal \"Estimate.\"; step \"a\" {{ agentless }}; step \"b\" {{ agentless }}; step \"c\" {{ agentless; {dependency} }} }}"
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &plan.subject_tokens, "graph").unwrap();
        let current = run(&store, "current");
        {
            let connection = store.connection.lock().unwrap();
            for (index, step) in current.steps.iter().enumerate() {
                connection.execute(
                    /* sqlite-large-value: allow-for-now lv-h84d691feb23a owner=fractal_missions::store::eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_run_eta_samples SELECT ?1,'historical',definition_hash,step_path,1,?2 FROM step_runs WHERE subject=?3",
                    params![format!("sample-{index}"), (index as u64 + 1)*10, step.subject]).unwrap();
            }
            connection.execute(/* sqlite-large-value: allow-for-now lv-h88d82d562560 owner=fractal_missions::store::eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_run_eta_samples SELECT 'extra',run_id,definition_hash,step_path,2,duration FROM local_run_eta_samples WHERE subject='sample-0'", []).unwrap();
        }
        let eta = store
            .mission_run_eta(&current.subject, 500)
            .unwrap()
            .unwrap();
        assert_eq!(eta["basis"]["samples"], 1);
        assert_eq!(
            eta["basis"]["method"],
            if critical {
                "critical-path"
            } else {
                "per-step"
            }
        );
        assert_eq!(eta["remaining_low_ms"], if critical { 50 } else { 60 });
        assert_eq!(eta["remaining_high_ms"], eta["remaining_low_ms"]);
    }
}

#[test]
fn step_estimates_keep_the_sampled_steps_when_another_has_no_samples() {
    let store = Store::open_memory("node").unwrap();
    let source = "version 2\nmission \"eta\" state=\"ready\" { concurrent-runs max=10; goal \"Estimate.\"; step \"a\" { agentless }; step \"b\" { agentless } }";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store
        .mission(&intent, IntentInput { kdl: source.into(), source_name: None })
        .unwrap();
    store.apply(&intent, &plan.subject_tokens, "graph").unwrap();
    let prior = run(&store, "prior");
    let a = prior.steps.iter().find(|step| step.step == "a").unwrap();
    store.set_step_state(&a.subject, "working", None).unwrap();
    store.set_step_state(&a.subject, "completed", None).unwrap();
    store.set_mission_run_state(&prior.id, "completed", "terminal", None).unwrap();
    let current = run(&store, "current");
    assert!(store.mission_run_eta(&current.subject, 1).unwrap().is_none(), "b has no samples");
    let steps = store.mission_run_step_etas(&current.subject).unwrap();
    assert_eq!(steps.keys().collect::<Vec<_>>(), ["a"]);
    let (low, high, samples) = steps["a"];
    assert_eq!((samples, low), (1, high));
    assert!(store.mission_run_step_etas(&prior.subject).unwrap().is_empty(), "a finished run");
}
