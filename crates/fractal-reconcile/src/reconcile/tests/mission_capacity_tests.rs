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
