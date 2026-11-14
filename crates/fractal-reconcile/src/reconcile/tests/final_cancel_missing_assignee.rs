//! A cancelled run whose finally step is assigned to a seat that is stopped or retired still ends.
//! A seat this host has not seen yet may still replicate, so its step keeps waiting.
use super::*;
use crate::RetirementsStore;

const SOURCE: &str = r#"version 2
resource "heron-work" { kind "custom.st3.document-source" }
mission "heron/close-out" state="ready" {
 goal "End a cancelled run whose close-out seat is gone."
 step "work" {
  agentless
  gate "work waits" { field "state" "resource/heron-work" is "ready" }
 }
 finally {
  step "close-out" { assigned-to "agent/node.heron-requester" }
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

const SEAT: &str = "agent/node.heron-requester";

/// Declare the close-out seat, then stop or retire it, as ending a requester does.
fn end_seat(store: &Store, retire: bool) {
    apply_source(
        store,
        r#"version 2
agent "heron-requester" { workspace "/tmp"; command "true"; restart "never" }"#,
        "heron-requester-seat",
    );
    if retire {
        let token = store.selected_desired_token(SEAT).unwrap().unwrap();
        store
            .retire_agent(SEAT, "person/operator", "Work finished", &token, None, "heron-retire", &[])
            .unwrap();
    } else {
        apply_source(
            store,
            r#"version 2
stop "agent/node.heron-requester""#,
            "heron-requester-stop",
        );
    }
    assert_eq!(
        store
            .selected_desired_kind(SEAT)
            .unwrap()
            .as_deref(),
        Some("stop")
    );
}

fn start(key: &str) -> (Arc<Store>, Reconciler<FakeRuntime>, String) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "heron-close-out");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "heron/close-out".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: key.into(),
        })
        .unwrap();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    assert_eq!(step(&store, &run.id, "work").status, "working");
    (store, reconciler, run.id)
}

#[test]
fn a_cancelled_run_cancels_a_finally_step_whose_assignee_is_stopped() {
    cancels_a_finally_step_whose_assignee_ended(false);
}

#[test]
fn a_cancelled_run_cancels_a_finally_step_whose_assignee_is_retired() {
    cancels_a_finally_step_whose_assignee_ended(true);
}

fn cancels_a_finally_step_whose_assignee_ended(retire: bool) {
    let (store, reconciler, run) = start("heron-cancel");
    end_seat(&store, retire);
    assert!(
        store
            .request_mission_run_cancellation(&run, "the work is no longer needed")
            .unwrap()
    );
    for _ in 0..6 {
        reconciler.reconcile_once().unwrap();
    }
    let close_out = step(&store, &run, "close-out");
    assert_eq!(close_out.status, "cancelled", "{close_out:?}");
    assert!(
        close_out
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("stopped or retired")
                && reason.contains("heron-requester`")),
        "the step records why it never ran: {close_out:?}"
    );
    let ended = store.mission_run(&run).unwrap().unwrap();
    assert_eq!(ended.status, "cancelled", "{ended:?}");
}

#[test]
fn a_cancelled_run_keeps_waiting_for_a_seat_this_host_has_not_seen() {
    // The seat may be declared on another host and not have replicated here yet.
    let (store, reconciler, run) = start("heron-unseen");
    assert!(
        store
            .selected_desired_kind(SEAT)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .request_mission_run_cancellation(&run, "the work is no longer needed")
            .unwrap()
    );
    for _ in 0..6 {
        reconciler.reconcile_once().unwrap();
    }
    let close_out = step(&store, &run, "close-out");
    assert_eq!(close_out.status, "blocked", "{close_out:?}");
    assert!(
        close_out
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("no eligible agent is present")),
        "{close_out:?}"
    );
    assert_eq!(store.mission_run(&run).unwrap().unwrap().phase, "final-cancelled");

    // Once the seat replicates and runs, the cleanup step is offered to it.
    apply_source(
        &store,
        r#"version 2
agent "heron-requester" { workspace "/tmp"; command "true"; restart "never" }"#,
        "heron-requester-seat",
    );
    reconciler.reconcile_once().unwrap();
    assert_eq!(step(&store, &run, "close-out").status, "ready");
}

#[test]
fn a_finished_run_still_waits_for_its_finally_steps_stopped_assignee() {
    let (store, reconciler, run) = start("heron-finish");
    end_seat(&store, false);
    store
        .append_claim(&ClaimInput {
            subject: "resource/heron-work".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("state".into(), Value::String("ready".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("heron-work-ready".into()),
        })
        .unwrap();
    for _ in 0..6 {
        reconciler.reconcile_once().unwrap();
    }
    assert_eq!(step(&store, &run, "work").status, "completed");
    let close_out = step(&store, &run, "close-out");
    assert_eq!(close_out.status, "blocked", "{close_out:?}");
    assert!(
        close_out
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("no eligible agent is present")),
        "{close_out:?}"
    );
    assert_eq!(store.mission_run(&run).unwrap().unwrap().phase, "final");

    // Cancelling the finished run while it waits on the missing seat ends it.
    assert!(
        store
            .request_mission_run_cancellation(&run, "the close-out seat was retired")
            .unwrap()
    );
    for _ in 0..6 {
        reconciler.reconcile_once().unwrap();
    }
    assert_eq!(step(&store, &run, "close-out").status, "cancelled");
    assert_eq!(
        store.mission_run(&run).unwrap().unwrap().status,
        "cancelled"
    );
}

#[test]
fn a_cancelled_run_still_runs_a_finally_step_that_declares_its_own_seat() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(
        &store,
        r#"version 2
resource "heron-work" { kind "custom.st3.document-source" }
mission "heron/own-seat" state="ready" {
 goal "Run close-out on the seat it brings."
 step "work" {
  agentless
  gate "work waits" { field "state" "resource/heron-work" is "ready" }
 }
 finally {
  step "close-out" {
   assigned-to "agent/${ST_MISSION_RUN}/closer"
   agent "closer" { workspace "/tmp"; command "true"; restart "never" }
  }
 }
}"#,
        "heron-own-seat",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "heron/own-seat".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "heron-own-seat-run".into(),
        })
        .unwrap();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    assert!(
        store
            .request_mission_run_cancellation(&run.id, "the work is no longer needed")
            .unwrap()
    );
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    let close_out = step(&store, &run.id, "close-out");
    assert_eq!(close_out.status, "ready", "{close_out:?}");
    assert_eq!(
        store.mission_run(&run.id).unwrap().unwrap().phase,
        "final-cancelled"
    );
}
