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

#[test]
fn a_laned_seat_starts_only_when_its_lane_fits_and_seats_wait_for_room_in_order() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside, burst_lane: None }));
    let seat = |name: &str, lane: &str| format!("agent {name:?} {{ workspace \"/tmp\"; command \"true\"; {lane} }}\n");
    apply_source(&store, &format!("version 2\n{}{}{}{}{}",
        seat("held", r#"capacity cores=8 memory="24G""#),
        seat("big", r#"capacity cores=8 memory="30G""#),
        seat("small", r#"capacity cores=1 memory="4G""#),
        seat("huge", r#"capacity cores=64 memory="8G""#),
        seat("plain", "")), "seats");
    let desired = store.desired_subjects().unwrap().into_iter()
        .map(|desired| (desired.subject.clone(), desired)).collect::<BTreeMap<_, _>>();
    let admit = |name: &str| {
        let desired = &desired[&format!("agent/node.{name}")];
        reconciler.admit_seat_start(desired, desired.member.as_ref().unwrap(), None).unwrap()
    };
    let status = reconciler.seat_admission.status.clone();
    let reason = |name: &str| status.subject(&format!("agent/node.{name}"), u64::MAX).unwrap();
    observe(&store, "agent/node.held", "running");

    // 54G for lanes, less the 24G lane that runs and 2G outside lanes: 28G free.
    assert!(!admit("big"));
    assert_eq!(reason("big")["reason"],
        "lane 8 cores/30.0G does not fit: 28.0G free of the 54.0G this host gives lanes (other lanes hold 24.0G, seats outside lanes 2.0G)");
    assert_eq!(reason("big")["queuePosition"], 1);
    // A smaller lane that fits does not pass the seat first in line.
    assert!(!admit("small"));
    assert!(reason("small")["reason"].as_str().unwrap().starts_with("lane 1 cores/4.0G waits behind agent/node.big, first in line for room"));
    assert_eq!(reason("small")["queuePosition"], 2);
    // A lane larger than the host waits out of line, and a seat without a lane is not held.
    assert!(!admit("huge"));
    assert_eq!(reason("huge")["reason"], "lane 64 cores/8.0G can never fit on this host, which gives lanes 14 cores/54.0G");
    assert!(reason("huge")["queuePosition"].is_null());
    assert!(admit("plain"));

    // Room frees when the running seat ends; the first in line starts, then the next while the
    // first's lane holds its room before it is seen running.
    observe(&store, "agent/node.held", "exited");
    assert!(!admit("small"), "big is still first in line");
    assert!(admit("big"));
    assert!(admit("small"));
    assert!(status.subject("agent/node.big", 0).is_none());
    // 54 - 30 (big, starting) - 4 (small, starting) - 2 outside = 18G: the 24G lane must wait.
    assert!(!admit("held"));
    assert!(reason("held")["reason"].as_str().unwrap().starts_with("lane 8 cores/24.0G does not fit: 18.0G free"));
    // A seat that leaves this host's laned seats stops waiting.
    reconciler.cancel_invalid_admissions(&[]).unwrap();
    assert!(status.subject("agent/node.held", 0).is_none());
}

#[test]
fn without_lanes_on_a_laned_seat_is_not_held_for_room() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    apply_source(&store, r#"version 2
agent "huge" { workspace "/tmp"; command "true"; capacity cores=64 memory="1T" }"#, "seat");
    let desired = store.desired_subjects().unwrap().remove(0);
    assert!(reconciler.admit_seat_start(&desired, desired.member.as_ref().unwrap(), None).unwrap());
}

#[test]
fn a_seat_first_in_line_that_is_suspended_gives_up_its_place() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside, burst_lane: None }));
    apply_source(&store, r#"version 2
agent "held" { workspace "/tmp"; command "true"; capacity cores=8 memory="24G" }
agent "big" { workspace "/tmp"; command "true"; capacity cores=8 memory="30G" }
agent "small" { workspace "/tmp"; command "true"; capacity cores=1 memory="4G" }"#, "seats");
    let desired = store.desired_subjects().unwrap().into_iter()
        .map(|desired| (desired.subject.clone(), desired)).collect::<BTreeMap<_, _>>();
    let admit = |name: &str| {
        let desired = &desired[&format!("agent/node.{name}")];
        reconciler.admit_seat_start(desired, desired.member.as_ref().unwrap(), None).unwrap()
    };
    observe(&store, "agent/node.held", "running");
    assert!(!admit("big"));
    assert!(!admit("small"), "behind big");
    // Suspended, big is no longer tried for a start, so it must not hold the line.
    store.append_claim(&ClaimInput {
        subject: "agent/node.big".into(), kind: "runtime.action.requested".into(), actor: Some("person/test".into()),
        fields: serde_json::from_value(json!({"action": "suspend"})).unwrap(),
        evidence: vec![store.selected_desired_token("agent/node.big").unwrap().unwrap()],
        expected_subject: None, idempotency_key: None,
    }).unwrap();
    reconciler.cancel_invalid_admissions(&store.desired_subjects().unwrap()).unwrap();
    assert!(reconciler.seat_admission.status.subject("agent/node.big", 0).is_none());
    assert!(admit("small"));
}

#[test]
fn a_restart_that_ignores_the_fit_starts_a_waiting_seat_and_holds_its_room() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside, burst_lane: None }));
    apply_source(&store, r#"version 2
agent "held" { workspace "/tmp"; command "true"; capacity cores=8 memory="24G" }
agent "operations" { workspace "/tmp"; command "true"; capacity cores=2 memory="40G" }"#, "seats");
    observe(&store, "agent/node.held", "running");
    let operations = store.desired_subjects().unwrap().into_iter().find(|desired| desired.subject == "agent/node.operations").unwrap();
    let member = operations.member.as_ref().unwrap();
    assert!(!reconciler.admit_seat_start(&operations, member, None).unwrap());
    let request = |fields: Value| store.append_claim(&ClaimInput {
        subject: operations.subject.clone(), kind: "runtime.action.requested".into(), actor: Some("person/avery".into()),
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    // An ordinary restart still waits for room.
    let plain = request(json!({"action": "restart"}));
    assert!(!reconciler.admit_seat_start(&operations, member, Some(&plain)).unwrap());
    let ignoring = request(json!({"action": "restart", "ignore_fit": true, "reason": "the measurement is stale and Operations must run"}));
    assert!(reconciler.admit_seat_start(&operations, member, Some(&ignoring)).unwrap());
    let status = &reconciler.seat_admission.status;
    assert!(status.subject("agent/node.operations", 0).is_none());
    assert!(status.state().lane_starts.contains_key("agent/node.operations"), "its room counts once it starts");
}

#[test]
fn a_step_waiting_for_its_lane_is_tried_again_and_becomes_ready_where_nothing_holds_it() {
    // A later build blocks a step with a `step lane` reason; this one, and any that follows, tries it
    // again, so a step a newer member blocked is never stranded after a rollback to this build.
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    apply_source(&store, r#"version 2
mission "orchid" state="ready" {
  goal "Build."
  completion { when "all-steps-exhausted" }
  agent "builder" { workspace "/tmp"; command "true" }
  step "build" { assigned-to "agent/${ST_MISSION_RUN}/builder" }
}"#, "orchid");
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "orchid".into(), revision: None, workspace: "/tmp".into(), requester: Some("person/lichen".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: "orchid-run".into(),
    }).unwrap();
    let step = |store: &Store| store.work_for_reconcile_all().unwrap().into_iter()
        .find(|view| view.run == run.subject && view.step == "build").unwrap();
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    assert_eq!(step(&store).status, "ready");
    store.set_step_state(&step(&store).subject, "blocked", Some("step lane 2 cores/16.0G for agent/x does not fit: 1.0G free")).unwrap();
    for _ in 0..2 {
        reconciler.reconcile_once().unwrap();
    }
    assert_eq!(step(&store).status, "ready");
}

fn reserving(store: &Arc<Store>) -> Reconciler<FakeRuntime> {
    Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside, burst_lane: Some(super::seat_admission::BURST_LANE) }))
}

#[test]
fn a_start_that_would_take_the_room_kept_for_a_burst_waits_and_a_burst_may_use_it() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = reserving(&store);
    let seat = |name: &str| format!("agent {name:?} {{ workspace \"/tmp\"; command \"true\"; capacity cores=4 memory=\"8G\" }}\n");
    apply_source(&store, &format!("version 2\n{}{}{}{}{}", seat("alder"), seat("birch"), seat("cedar"), seat("dogwood"), seat("elm")), "seats");
    for name in ["alder", "birch", "cedar", "dogwood"] {
        observe(&store, &format!("agent/node.{name}"), "running");
    }
    let elm = store.desired_subjects().unwrap().into_iter().find(|desired| desired.subject == "agent/node.elm").unwrap();
    let status = reconciler.seat_admission.status.clone();
    // 54G for lanes, less four 8G lanes and 2G outside: 20G, of which a burst to 24G on an 8G seat
    // needs 16G. The fifth seat's 8G would leave a burst no room, so it waits.
    assert!(!reconciler.admit_seat_start(&elm, elm.member.as_ref().unwrap(), None).unwrap());
    assert_eq!(status.subject("agent/node.elm", u64::MAX).unwrap()["reason"],
        "lane 4 cores/8.0G does not fit: 4.0G free of the 54.0G this host gives lanes (other lanes hold 32.0G, seats outside lanes 2.0G, kept for a burst 16.0G)");
    assert_eq!(status.snapshot(0)["laneBurstReserve"], json!({"subject": "agent/node.elm", "full": 16 * G, "kept": 16 * G}));
    // A burst may use it: the run's 24G is granted.
    let now = u64::try_from(now_ms() / 1000).unwrap();
    status.burst_hold("agent/node.alder", Capacity { cores: 8, memory: 24 * G }, "st-test", now);
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted("agent/node.alder"), Some(Capacity { cores: 8, memory: 24 * G }));
    // While it holds its raise, the reserve keeps nothing more, and the start still waits.
    assert!(!reconciler.admit_seat_start(&elm, elm.member.as_ref().unwrap(), None).unwrap());
    assert_eq!(status.subject("agent/node.elm", u64::MAX).unwrap()["reason"],
        "lane 4 cores/8.0G does not fit: 4.0G free of the 54.0G this host gives lanes (other lanes hold 48.0G, seats outside lanes 2.0G, kept for a burst 0.0G)");
    // With a seat stopped there is room beside the reserve once the burst is given back.
    assert!(status.burst_release("agent/node.alder"));
    reconciler.reconcile_lane_bursts().unwrap();
    status.state().lane_raised.remove("agent/node.alder");
    observe(&store, "agent/node.dogwood", "exited");
    assert!(reconciler.admit_seat_start(&elm, elm.member.as_ref().unwrap(), None).unwrap());
}

#[test]
fn a_lane_that_fits_only_in_the_room_kept_for_a_burst_waits_out_of_line() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = reserving(&store);
    apply_source(&store, r#"version 2
agent "small" { workspace "/tmp"; command "true"; capacity cores=1 memory="4G" }
agent "big" { workspace "/tmp"; command "true"; capacity cores=8 memory="40G" }"#, "seats");
    observe(&store, "agent/node.small", "running");
    let big = store.desired_subjects().unwrap().into_iter().find(|desired| desired.subject == "agent/node.big").unwrap();
    // A burst to 24G on the running 4G seat needs 20G of the 54G: 34G is left for starts.
    assert!(!reconciler.admit_seat_start(&big, big.member.as_ref().unwrap(), None).unwrap());
    let wait = reconciler.seat_admission.status.subject("agent/node.big", u64::MAX).unwrap();
    assert_eq!(wait["reason"], "lane 8 cores/40.0G does not fit beside the 20.0G this host keeps from starts for a burst, of the 54.0G it gives lanes");
    assert!(wait["queuePosition"].is_null(), "it holds no one back");
}

#[test]
fn a_host_that_could_never_grant_a_burst_keeps_no_room_for_one_and_stopped_seats_do_not_size_it() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let small_host = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 6, memory: 20 * G }, outside: two_gib_outside,
            burst_lane: Some(super::seat_admission::BURST_LANE) }));
    apply_source(&store, r#"version 2
agent "idle" { workspace "/tmp"; command "true"; capacity cores=1 memory="2G" }
agent "seat" { workspace "/tmp"; command "true"; capacity cores=4 memory="8G" }"#, "seats");
    let seat = store.desired_subjects().unwrap().into_iter().find(|desired| desired.subject == "agent/node.seat").unwrap();
    // 20G for lanes can never hold a 24G burst: nothing is kept, and the 8G seat starts.
    assert!(small_host.admit_seat_start(&seat, seat.member.as_ref().unwrap(), None).unwrap());
    // On a host that can, the stopped 2G seat asks no burst: the reserve is sized from the seat
    // starting, 24G - 8G.
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = reserving(&store);
    apply_source(&store, r#"version 2
agent "idle" { workspace "/tmp"; command "true"; capacity cores=1 memory="2G" }
agent "seat" { workspace "/tmp"; command "true"; capacity cores=4 memory="8G" }"#, "seats");
    let seat = store.desired_subjects().unwrap().into_iter().find(|desired| desired.subject == "agent/node.seat").unwrap();
    assert!(reconciler.admit_seat_start(&seat, seat.member.as_ref().unwrap(), None).unwrap());
    assert_eq!(reconciler.seat_admission.status.snapshot(0)["laneBurstReserve"], json!({"subject": "agent/node.seat", "full": 16 * G, "kept": 16 * G}));
}

#[test]
fn the_burst_lane_admission_keeps_room_for_is_what_fractal_test_asks_for() {
    let path = test_env!("CARGO_MANIFEST_DIR", "/../../.st/lane-bursts.json");
    let config: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let memory = config["build"]["memory"].as_str().unwrap();
    let gib = memory.strip_suffix('G').and_then(|gib| gib.parse::<u64>().ok()).unwrap();
    let lane = super::seat_admission::BURST_LANE;
    assert_eq!((config["build"]["cores"].as_u64(), gib << 30), (Some(u64::from(lane.cores)), lane.memory),
        "the build burst fractal test asks for ({memory}) and BURST_LANE must agree");
}
