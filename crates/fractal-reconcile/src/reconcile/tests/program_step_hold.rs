//! A program step's outcome is the program engine's: no reconciler pass completes it.
use super::*;

#[test]
fn a_published_program_step_stays_working_until_the_engine_decides() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let hash = store.put_blob(b"export default {}").unwrap();
    apply_source(
        &store,
        &format!(
            r#"version 2
mission "program-hold" state="ready" {{
  goal "Run a program."
  step "program" {{ program "/bundles/hold.mjs" sha256="{hash}" }}
}}"#
        ),
        "program-hold-definition",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "program-hold".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: None,
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "program-hold-run".into(),
        })
        .unwrap();
    let step = run.steps.iter().find(|step| step.step == "program").unwrap().subject.clone();
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    for _ in 0..4 {
        reconciler.reconcile_once().unwrap();
    }
    assert_eq!(store.step_run(&step).unwrap().unwrap().status, "working");
    assert_eq!(store.mission_run_status(&run.subject).unwrap().as_deref(), Some("running"));
}
