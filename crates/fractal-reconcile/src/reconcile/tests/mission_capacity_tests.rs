use super::*;

const G: u64 = 1 << 30;

#[test]
fn a_missions_capacity_lanes_each_agent_its_run_materializes_that_declares_none() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, r#"version 2
mission "orchid" state="ready" {
  goal "Grow a garden."
  completion { when "all-steps-exhausted" }
  capacity cores=2 memory="4G"
  agent "amber" { workspace "/tmp"; command "true"; restart "never" }
  step "start-seats" {
    agentless
    agent "birch" { workspace "/tmp"; command "true"; restart "never" }
    agent "cedar" { workspace "/tmp"; command "true"; restart "never"; capacity cores=4 memory="8G" }
  }
  step "amber" { assigned-to "agent/${ST_MISSION_RUN}/amber"; capacity memory="16G" }
}"#, "orchid");
    let run = store
        .create_mission_run(&crate::model::MissionRunRequest {
            mission: "orchid".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/lichen".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "orchid-run".into(),
        })
        .unwrap();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    for _ in 0..4 {
        reconciler.reconcile_once().unwrap();
    }
    let lanes = store
        .desired_subjects_for_owner_run(&run.subject)
        .unwrap()
        .into_iter()
        .filter(|desired| desired.kind == "agent")
        .map(|desired| (desired.subject.rsplit('/').next().unwrap().to_owned(), desired.member.unwrap().capacity))
        .collect::<BTreeMap<_, _>>();
    let lane = |cores, memory| Some(fractal_runtime::Capacity { cores, memory });
    assert_eq!(
        lanes,
        BTreeMap::from([
            ("amber".to_owned(), lane(2, 4 * G)),
            ("birch".to_owned(), lane(2, 4 * G)),
            ("cedar".to_owned(), lane(4, 8 * G)),
        ]),
        "the mission's lane goes to the agents that declare none; a step's raise is not the seat's own lane"
    );
    let started = reconciler.runtime.started_members.lock().unwrap();
    let launched = started
        .iter()
        .find(|member| member.runtime_id.ends_with("birch"))
        .expect("birch started");
    // What the runtime is handed is what set_lane gives the spawn.
    assert_eq!(launched.capacity, lane(2, 4 * G));
}

#[test]
fn a_nested_missions_steps_take_the_nested_missions_lane_not_the_roots() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, r#"version 2
mission "orchid" state="ready" {
  goal "Grow a garden."
  completion { when "all-steps-exhausted" }
  capacity cores=2 memory="4G"
  step "bare" {
    agentless
    mission "plain" {
      goal "Grow without a lane of its own."
      step "seats" {
        agentless
        agent "pine" { workspace "/tmp"; command "true"; restart "never" }
      }
    }
  }
  step "laned" {
    agentless
    mission "sized" {
      goal "Grow with a lane of its own."
      capacity cores=3 memory="6G"
      step "seats" {
        agentless
        agent "spruce" { workspace "/tmp"; command "true"; restart "never" }
      }
    }
  }
}"#, "orchid");
    let run = store
        .create_mission_run(&crate::model::MissionRunRequest {
            mission: "orchid".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/lichen".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "orchid-run".into(),
        })
        .unwrap();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    for _ in 0..6 {
        reconciler.reconcile_once().unwrap();
    }
    let lanes = store
        .desired_subjects_for_owner_run(&run.subject)
        .unwrap()
        .into_iter()
        .filter(|desired| desired.kind == "agent")
        .map(|desired| (desired.subject.rsplit('/').next().unwrap().to_owned(), desired.member.unwrap().capacity))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        lanes,
        BTreeMap::from([
            ("pine".to_owned(), None),
            ("spruce".to_owned(), Some(fractal_runtime::Capacity { cores: 3, memory: 6 * G })),
        ]),
        "a nested mission declares its own lane, as publication checks it"
    );
}

#[test]
fn uses_mission_capacity_waiters_follow_each_holder_without_full_passes() {
    const CONTENDERS: usize = 8;
    let workspace = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, &format!(r#"version 2
agent "capacity-worker" {{ workspace {:?}; command "true"; restart "never" }}
mission "serialized" state="ready" {{
  goal "Admit one child at a time."
  concurrent-runs max=1
  step "work" {{ assigned-to "agent/capacity-worker"; goal "Wait for the worker's completion." }}
}}"#, workspace.path().display().to_string()), "serialized");
    let revision = store.mission_spec("serialized", None).unwrap().unwrap().revision;
    apply_source(&store, &format!(r#"version 2
mission "contender" state="ready" {{
  goal "Wait for shared child capacity."
  concurrent-runs max={CONTENDERS}
  step "use" {{ uses-mission "serialized@{revision}"; }}
}}"#), "contender");
    let parents = (0..CONTENDERS).map(|index| {
        store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "contender".into(),
            revision: None,
            workspace: workspace.path().display().to_string(),
            requester: Some("person/test".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: format!("capacity-contender-{index}"),
        }).unwrap()
    }).collect::<Vec<_>>();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    // Materialize the initial declarations and ready steps before testing capacity handoffs.
    // After this seed only the post-commit follower evaluates the parents. A newly admitted
    // child gets its own initial evaluation, never a full pass over its waiting parents.
    for _ in 0..8 {
        reconciler.reconcile_once().unwrap();
    }
    let settle_changes = || {
        for _ in 0..64 {
            if !reconciler.evaluate_changed_mission_runs().unwrap().1 {
                return;
            }
        }
        panic!("capacity handoffs did not settle within bounded reactive evaluations");
    };
    settle_changes();
    for completed in 0..CONTENDERS {
        let holders = store.active_mission_runs_for_mission("serialized").unwrap();
        assert_eq!(holders.len(), 1, "one child must hold capacity after {completed} completions; parents: {:#?}", parents.iter().map(|parent| store.mission_run(&parent.subject).unwrap().unwrap()).collect::<Vec<_>>());
        let holder = &holders[0];
        for parent in &parents {
            if store.mission_run_status(&parent.subject).unwrap().as_deref() == Some("completed") {
                continue;
            }
            if let Some(child) = store.mission_run_for_parent_step(&parent.steps[0].subject).unwrap() {
                assert!(
                    reconciler.incremental.reads_of(&parent.subject).contains(&child.subject),
                    "an admitted parent must follow its child's outcome immediately",
                );
            } else {
                assert!(
                    reconciler.incremental.reads_of(&parent.subject).contains(&holder.subject),
                    "a capacity waiter must follow the current holder, including after a handoff",
                );
            }
        }
        reconciler.evaluate_one_mission_run(
            &holder.id, true, true, &mut BTreeSet::new(), &mut BTreeSet::new(),
        );
        settle_changes();
        store.set_step_state(&holder.steps[0].subject, "completed", None).unwrap();
        settle_changes();
        assert_eq!(
            parents.iter().filter(|parent| {
                store.mission_run_status(&parent.subject).unwrap().as_deref() == Some("completed")
            }).count(),
            completed + 1,
            "releasing capacity must complete its parent and admit the next without a full pass; states: {:#?}",
            parents.iter().map(|parent| {
                let current = store.mission_run(&parent.subject).unwrap().unwrap();
                let child = store.mission_run_for_parent_step(&parent.steps[0].subject).unwrap();
                (current.subject, current.status, current.phase, current.steps[0].status.clone(),
                    child.map(|child| (child.subject.clone(), child.status.clone(), child.phase.clone(),
                        child.steps[0].status.clone(), store.run_projection_pending(&child.subject).unwrap())))
            }).collect::<Vec<_>>(),
        );
    }
    assert!(store.active_mission_runs_for_mission("serialized").unwrap().is_empty());
}
