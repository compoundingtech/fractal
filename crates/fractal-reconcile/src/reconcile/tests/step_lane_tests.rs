use super::*;
use super::seat_admission::LaneHost;
use fractal_runtime::Capacity;
use serde_json::json;

const G: u64 = 1 << 30;

fn two_gib_outside() -> Option<u64> {
    Some(2 * G)
}

fn observe(store: &Store, subject: &str, status: &str) {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: "runtime.observed".into(), actor: None,
        fields: serde_json::from_value(json!({"status": status, "host": "node", "incarnation_id": format!("{subject}-1")})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn run(store: &Store, source: &str) -> crate::model::MissionRunView {
    apply_source(store, source, "orchid");
    store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "orchid".into(), revision: None, workspace: "/tmp".into(), requester: Some("person/lichen".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: "orchid-run".into(),
    }).unwrap()
}

fn step(store: &Store, run: &crate::model::MissionRunView, path: &str) -> crate::model::StepRunView {
    store.work_for_reconcile_all().unwrap().into_iter()
        .find(|view| view.run == run.subject && view.step == path).unwrap()
}

#[test]
fn a_step_raises_its_seats_lane_while_the_seat_holds_it() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside }));
    let run = run(&store, r#"version 2
mission "orchid" state="ready" {
  goal "Build."
  completion { when "all-steps-exhausted" }
  agent "builder" { workspace "/tmp"; command "true"; capacity cores=2 memory="4G" }
  step "build" { assigned-to "agent/${ST_MISSION_RUN}/builder"; capacity cores=8 memory="16G" }
}"#);
    for _ in 0..4 {
        reconciler.reconcile_once().unwrap();
    }
    let build = step(&store, &run, "build");
    assert_eq!(build.status, "ready", "{:?}", build.blocked_reason);
    let builder = format!("agent/{}/builder", run.id);
    let member = store.desired_subject_with_writer(&builder).unwrap().unwrap().0.member.unwrap();
    let raised = Capacity { cores: 8, memory: 16 * G };
    reconciler.hold_step_lanes(&builder, "builder-1", &member, &store.work_for_reconcile(&builder).unwrap()).unwrap();
    assert_eq!(*runtime.lanes_resized.lock().unwrap(), [(member.runtime_id.clone(), raised)]);
    // The raised lane is what admission counts for the seat; nothing is set again while it holds.
    assert_eq!(reconciler.seat_admission.status.state().lane_raised.get(&builder), Some(&raised));
    observe(&store, &builder, "running");
    assert_eq!(reconciler.held_lanes("agent/node.other", 0).unwrap(), 16 * G);
    reconciler.hold_step_lanes(&builder, "builder-1", &member, &store.work_for_reconcile(&builder).unwrap()).unwrap();
    assert_eq!(runtime.lanes_resized.lock().unwrap().len(), 1);
    // A relaunch starts the slice at the seat's own lane, so the raise is set again for it.
    reconciler.hold_step_lanes(&builder, "builder-2", &member, &store.work_for_reconcile(&builder).unwrap()).unwrap();
    assert_eq!(runtime.lanes_resized.lock().unwrap().len(), 2);
    // Done with the step, the lane stays up while it still holds more than the seat's own lane
    // allows (a job the step left running), then goes back to the seat's own.
    store.set_step_state(&build.subject, "completed", None).unwrap();
    *runtime.lane_held.lock().unwrap() = Some(10 * G);
    reconciler.hold_step_lanes(&builder, "builder-2", &member, &store.work_for_reconcile(&builder).unwrap()).unwrap();
    // Its cores come back at once; CPU time is not taken from anything by lowering a quota.
    let held = Capacity { cores: 2, memory: 16 * G };
    assert_eq!(runtime.lanes_resized.lock().unwrap().last(), Some(&(member.runtime_id.clone(), held)));
    assert_eq!(reconciler.seat_admission.status.state().lane_raised.get(&builder), Some(&held));
    *runtime.lane_held.lock().unwrap() = Some(G);
    reconciler.hold_step_lanes(&builder, "builder-2", &member, &store.work_for_reconcile(&builder).unwrap()).unwrap();
    assert_eq!(runtime.lanes_resized.lock().unwrap().last(), Some(&(member.runtime_id.clone(), member.capacity.unwrap())));
    assert!(!reconciler.seat_admission.status.state().lane_raised.contains_key(&builder));
}

#[test]
fn a_step_whose_raised_lane_does_not_fit_waits_blocked_until_it_does() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside }));
    apply_source(&store, r#"version 2
agent "held" { workspace "/tmp"; command "true"; capacity cores=8 memory="40G" }"#, "held");
    observe(&store, "agent/node.held", "running");
    let run = run(&store, r#"version 2
mission "orchid" state="ready" {
  goal "Build."
  completion { when "all-steps-exhausted" }
  agent "builder" { workspace "/tmp"; command "true"; capacity cores=2 memory="4G" }
  step "build" { assigned-to "agent/${ST_MISSION_RUN}/builder"; capacity memory="16G" }
  step "huge" { assigned-to "agent/${ST_MISSION_RUN}/builder"; capacity memory="60G" }
}"#);
    for _ in 0..4 {
        reconciler.reconcile_once().unwrap();
    }
    let builder = format!("agent/{}/builder", run.id);
    // 54G less the 40G lane that runs and 2G outside: 12G, and the step needs 16G.
    let build = step(&store, &run, "build");
    assert_eq!(build.status, "blocked");
    assert_eq!(build.blocked_reason.as_deref(), Some(format!(
        "step lane 2 cores/16.0G for {builder} does not fit: 12.0G free of the 54.0G this host gives lanes (other lanes hold 40.0G, seats outside lanes 2.0G)").as_str()));
    let huge = step(&store, &run, "huge");
    assert_eq!(huge.blocked_reason.as_deref(), Some(format!(
        "step lane 2 cores/60.0G for {builder} can never fit on this host, which gives lanes 14 cores/54.0G").as_str()));
    // Room frees when the other seat stops for good (a seat that only exits is started again and
    // holds its room while it starts): the step is delivered.
    apply_source(&store, "version 2\nstop \"agent/node.held\"", "stop-held");
    observe(&store, "agent/node.held", "exited");
    for _ in 0..2 {
        reconciler.reconcile_once().unwrap();
    }
    let after = step(&store, &run, "build");
    assert_eq!(after.status, "ready", "{:?}", after.blocked_reason);
}

#[test]
fn a_step_below_the_lane_of_a_seat_declared_elsewhere_waits_with_why() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside }));
    // Publication cannot see this seat's lane: it is declared on its own.
    apply_source(&store, r#"version 2
agent "builder" { workspace "/tmp"; command "true"; capacity cores=4 memory="8G" }"#, "builder");
    let run = run(&store, r#"version 2
mission "orchid" state="ready" {
  goal "Build."
  completion { when "all-steps-exhausted" }
  step "build" { assigned-to "agent/node.builder"; capacity cores=2 }
}"#);
    for _ in 0..4 {
        reconciler.reconcile_once().unwrap();
    }
    let build = step(&store, &run, "build");
    assert_eq!(build.status, "blocked");
    assert_eq!(build.blocked_reason.as_deref(),
        Some("step lane for agent/node.builder: a step may only raise its seat's lane: cores=2 is below its 4"));
}

#[test]
fn two_steps_judged_in_one_pass_do_not_take_the_same_room() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside }));
    apply_source(&store, r#"version 2
agent "amber" { workspace "/tmp"; command "true"; capacity cores=2 memory="4G" }
agent "birch" { workspace "/tmp"; command "true"; capacity cores=2 memory="4G" }"#, "seats");
    observe(&store, "agent/node.amber", "running");
    observe(&store, "agent/node.birch", "running");
    // 54G less 2G outside and each seat's own 4G: room for one 30G raise, not two.
    let first = reconciler.step_lane_wait(Some(fractal_runtime::Raise { cores: None, memory: Some(30 * G) }), Some("agent/node.amber")).unwrap();
    assert_eq!(first, None);
    let second = reconciler.step_lane_wait(Some(fractal_runtime::Raise { cores: None, memory: Some(30 * G) }), Some("agent/node.birch")).unwrap();
    assert!(second.as_deref().is_some_and(|reason| reason.starts_with("step lane 2 cores/30.0G for agent/node.birch does not fit: 22.0G free")), "{second:?}");
}

#[test]
fn the_wake_stage_of_a_seat_raises_its_lane_for_the_step_it_holds() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside }));
    let run = run(&store, r#"version 2
mission "orchid" state="ready" {
  goal "Build."
  completion { when "all-steps-exhausted" }
  agent "builder" { workspace "/tmp"; command "true"; capacity cores=2 memory="4G" }
  step "build" { assigned-to "agent/${ST_MISSION_RUN}/builder"; capacity cores=8 memory="16G" }
}"#);
    for _ in 0..4 {
        reconciler.reconcile_once().unwrap();
    }
    let builder = format!("agent/{}/builder", run.id);
    let member = store.desired_subject_with_writer(&builder).unwrap().unwrap().0.member.unwrap();
    // The stage each pass runs for a live seat, before its wakes.
    reconciler.reconcile_work_messages(&builder, "builder-1", Some(&member)).unwrap();
    assert_eq!(*runtime.lanes_resized.lock().unwrap(), [(member.runtime_id.clone(), Capacity { cores: 8, memory: 16 * G })]);
}

#[test]
fn a_daemon_that_restarts_lowers_a_lane_a_step_left_raised_and_counts_it_until_then() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, r#"version 2
agent "amber" { workspace "/tmp"; command "true"; capacity cores=2 memory="4G" }"#, "seats");
    observe(&store, "agent/node.amber", "running");
    let member = store.desired_subject_with_writer("agent/node.amber").unwrap().unwrap().0.member.unwrap();
    let base = member.capacity.unwrap();
    // A fresh daemon, whose slice for the seat still carries a raise from a step it no longer holds.
    let restarted = |held: Option<u64>| {
        let runtime = Arc::new(FakeRuntime::default());
        *runtime.lane_live.lock().unwrap() = Some(Capacity { cores: 8, memory: 16 * G });
        *runtime.lane_held.lock().unwrap() = held;
        let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()))
            .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside }));
        reconciler.hold_step_lanes("agent/node.amber", "amber-1", &member, &store.work_for_reconcile("agent/node.amber").unwrap()).unwrap();
        (reconciler, runtime)
    };
    let (reconciler, runtime) = restarted(None);
    assert_eq!(*runtime.lanes_resized.lock().unwrap(), [(member.runtime_id.clone(), base)]);
    assert_eq!(reconciler.held_lanes("agent/node.other", 0).unwrap(), 4 * G);
    // While the lane holds more than the seat's own allows, its memory stays up and admission
    // counts the raised lane.
    let (reconciler, runtime) = restarted(Some(10 * G));
    let held = Capacity { cores: 2, memory: 16 * G };
    assert_eq!(*runtime.lanes_resized.lock().unwrap(), [(member.runtime_id.clone(), held)]);
    assert_eq!(reconciler.held_lanes("agent/node.other", 0).unwrap(), 16 * G);
    *runtime.lane_held.lock().unwrap() = Some(G);
    reconciler.hold_step_lanes("agent/node.amber", "amber-1", &member, &store.work_for_reconcile("agent/node.amber").unwrap()).unwrap();
    assert_eq!(runtime.lanes_resized.lock().unwrap().last(), Some(&(member.runtime_id.clone(), base)));
    assert_eq!(reconciler.held_lanes("agent/node.other", 0).unwrap(), 4 * G);
}
