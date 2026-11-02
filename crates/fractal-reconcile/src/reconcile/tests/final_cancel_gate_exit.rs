//! A cancelled run's finally gate that exits while its step is evaluated still answers for it.
use super::*;

const SOURCE: &str = r#"version 2
resource "orchid-work" { kind "custom.st3.document-source" }
mission "orchid/late-answer" state="ready" {
 goal "Read a final gate's answer that arrives during cancellation."
 step "work" {
  agentless
  gate "work waits" { field "state" "resource/orchid-work" is "ready" }
 }
 finally {
  step "report" {
   agentless
   gate "the final status is posted" {
    exec "exit 7"
    host "node"
    workspace "/tmp"
    time-limit "1m"
   }
  }
 }
}"#;

fn step(store: &Store, run: &str, path: &str) -> crate::model::StepRunView {
    store
        .mission_run(run)
        .unwrap()
        .unwrap()
        .steps
        .into_iter()
        .find(|step| step.step == path)
        .unwrap()
}

#[test]
fn a_final_gate_that_exits_during_its_evaluation_fails_its_step_rather_than_cancelling_it() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "late-answer");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "orchid/late-answer".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "late-answer-run".into(),
        })
        .unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(
        store.clone(),
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    );
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    assert_eq!(step(&store, &run.id, "work").status, "working");
    assert!(
        store
            .request_mission_run_cancellation(&run.id, "the work is no longer needed")
            .unwrap()
    );
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    let gate = runtime
        .started_members
        .lock()
        .unwrap()
        .iter()
        .find(|member| member.driver.as_deref() == Some("mechanical-gate"))
        .expect("the final gate started")
        .runtime_id
        .clone();
    let running = RuntimeObservation {
        runtime_id: gate.clone(),
        terminal: false,
        status: "running".into(),
        exit_code: None,
        incarnation_id: Some("final-gate".into()),
    };
    runtime
        .execs
        .lock()
        .unwrap()
        .insert(gate.clone(), running.clone());
    reconciler.reconcile_once().unwrap();
    let report = step(&store, &run.id, "report");
    assert_eq!(report.status, "working", "{report:?}");

    // The gate's check exits right after the run's evaluation first observes it running. The
    // pass's own exec sweep reads it earlier, so the run is evaluated alone here.
    let exits = runtime.clone();
    let exited = RuntimeObservation {
        status: "exited".into(),
        exit_code: Some(7),
        ..running
    };
    let id = gate.clone();
    *runtime.after_observe_exec.lock().unwrap() = Some((
        gate,
        Box::new(move || {
            exits.execs.lock().unwrap().insert(id, exited);
        }),
    ));
    let (view, mission) = store.mission_run_for_evaluation(&run.id).unwrap();
    reconciler
        .evaluate_active_mission_run(&view, mission)
        .unwrap();
    assert!(runtime.after_observe_exec.lock().unwrap().is_none());
    let report = step(&store, &run.id, "report");
    assert_ne!(
        report.status, "cancelled",
        "the check's answer was never read: {report:?}"
    );
    // A pass that saw the check running leaves its answer to the next pass, which the exit wakes.
    reconciler.reconcile_once().unwrap();
    let report = step(&store, &run.id, "report");
    assert_eq!(report.status, "failed", "{report:?}");
    assert!(
        report
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("the final status is posted")),
        "{report:?}"
    );
}
