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

fn now() -> u64 {
    u64::try_from(now_ms() / 1000).unwrap()
}

fn laned(runtime: Arc<FakeRuntime>, store: &Arc<Store>) -> Reconciler<FakeRuntime> {
    Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()))
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside, burst_lane: None }))
}

fn seat(store: &Store, name: &str, lane: &str) -> MemberSpec {
    apply_source(store, &format!("version 2\nagent \"{name}\" {{ workspace \"/tmp\"; command \"true\"; {lane} }}"), name);
    let subject = format!("agent/node.{name}");
    observe(store, &subject, "running");
    store.desired_subject_with_writer(&subject).unwrap().unwrap().0.member.unwrap()
}

fn hold(reconciler: &Reconciler<FakeRuntime>, store: &Store, agent: &str, member: &MemberSpec) {
    reconciler.hold_step_lanes(agent, "inc-1", member, &store.work_for_reconcile(agent).unwrap()).unwrap();
}

#[test]
fn a_burst_that_fits_raises_the_seats_lane_and_comes_down_once_given_back() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = laned(runtime.clone(), &store);
    let member = seat(&store, "builder", "capacity cores=2 memory=\"4G\"");
    let builder = "agent/node.builder";
    let status = &reconciler.seat_admission.status;
    let burst = Capacity { cores: 8, memory: 24 * G };
    status.burst_hold(builder, burst, "st-test", now());
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted(builder), Some(burst));
    // Counted at once, before the seat's wake sets the slice: another seat's start sees the room taken.
    assert_eq!(status.state().lane_raised.get(builder), Some(&burst));
    assert_eq!(reconciler.held_lanes("agent/node.other", 0).unwrap(), 24 * G);
    hold(&reconciler, &store, builder, &member);
    assert_eq!(*runtime.lanes_resized.lock().unwrap(), [(member.runtime_id.clone(), burst)]);
    // Given back while the lane still holds more than the seat's own allows: cores now, memory later.
    assert!(status.burst_release(builder));
    reconciler.reconcile_lane_bursts().unwrap();
    *runtime.lane_held.lock().unwrap() = Some(10 * G);
    hold(&reconciler, &store, builder, &member);
    assert_eq!(runtime.lanes_resized.lock().unwrap().last(), Some(&(member.runtime_id.clone(), Capacity { cores: 2, memory: 24 * G })));
    *runtime.lane_held.lock().unwrap() = Some(G);
    hold(&reconciler, &store, builder, &member);
    assert_eq!(runtime.lanes_resized.lock().unwrap().last(), Some(&(member.runtime_id.clone(), member.capacity.unwrap())));
    assert!(!status.state().lane_raised.contains_key(builder));
}

#[test]
fn bursts_wait_in_the_order_they_asked_and_one_that_can_never_fit_is_refused() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = laned(Arc::new(FakeRuntime::default()), &store);
    seat(&store, "held", "capacity cores=8 memory=\"40G\"");
    seat(&store, "first", "capacity cores=2 memory=\"4G\"");
    seat(&store, "second", "capacity cores=2 memory=\"4G\"");
    seat(&store, "huge", "capacity cores=2 memory=\"4G\"");
    let status = &reconciler.seat_admission.status;
    let at = now();
    // 54G less the 40G lane, two 4G lanes and 2G outside: 4G free. The first asks 24G and waits; the
    // second asks 6G, which would fit beside its own 4G, but waits behind the first.
    status.burst_hold("agent/node.first", Capacity { cores: 2, memory: 24 * G }, "st-test", at);
    status.burst_hold("agent/node.second", Capacity { cores: 2, memory: 6 * G }, "st-test", at);
    status.burst_hold("agent/node.huge", Capacity { cores: 2, memory: 60 * G }, "st-test", at);
    reconciler.reconcile_lane_bursts().unwrap();
    let wait = |subject: &str| status.state().lane_bursts.iter().find(|burst| burst.subject == subject).unwrap().clone();
    assert_eq!(wait("agent/node.first").wait.as_deref(), Some(
        "burst lane 2 cores/24.0G for agent/node.first does not fit: 4.0G free of the 54.0G this host gives lanes (other lanes hold 48.0G, seats outside lanes 2.0G)"));
    assert_eq!(wait("agent/node.second").wait.as_deref(), Some(
        "burst lane 2 cores/6.0G for agent/node.second waits behind agent/node.first, first in line for room"));
    assert_eq!(wait("agent/node.huge").refused.as_deref(), Some(
        "burst lane 2 cores/60.0G for agent/node.huge can never fit on this host, which gives lanes 14 cores/54.0G"));
    assert!(!wait("agent/node.first").granted && !wait("agent/node.second").granted);
    // The 40G seat stops for good: the first is served, then the second beside it.
    apply_source(&store, "version 2\nstop \"agent/node.held\"", "stop-held");
    observe(&store, "agent/node.held", "exited");
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted("agent/node.first"), Some(Capacity { cores: 2, memory: 24 * G }));
    assert_eq!(status.burst_granted("agent/node.second"), Some(Capacity { cores: 2, memory: 6 * G }));
}

#[test]
fn a_burst_whose_holder_stops_renewing_is_dropped_counted_and_its_lane_comes_down() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = laned(runtime.clone(), &store);
    let member = seat(&store, "builder", "capacity cores=2 memory=\"4G\"");
    let builder = "agent/node.builder";
    let status = &reconciler.seat_admission.status;
    let burst = Capacity { cores: 8, memory: 24 * G };
    let at = now() - fractal_runtime::seat_admission_status::BURST_LEASE_SECS + 30;
    status.burst_hold(builder, burst, "st-test", at);
    reconciler.reconcile_lane_bursts().unwrap();
    hold(&reconciler, &store, builder, &member);
    assert_eq!(runtime.lanes_resized.lock().unwrap().last(), Some(&(member.runtime_id.clone(), burst)));
    // Its holder renewed last 60s ago, then died: the lease runs out 30s later.
    status.state().lane_bursts.iter_mut().for_each(|held| held.expires = now() - 1);
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted(builder), None);
    assert_eq!(status.bursts_dropped_by_lease(), 1);
    *runtime.lane_held.lock().unwrap() = Some(G);
    hold(&reconciler, &store, builder, &member);
    assert_eq!(runtime.lanes_resized.lock().unwrap().last(), Some(&(member.runtime_id.clone(), member.capacity.unwrap())));
}

#[test]
fn a_seat_without_a_lane_or_a_host_without_lanes_holds_no_burst() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = laned(Arc::new(FakeRuntime::default()), &store);
    seat(&store, "plain", "");
    let status = &reconciler.seat_admission.status;
    status.burst_hold("agent/node.plain", Capacity { cores: 8, memory: 24 * G }, "st-test", now());
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.state().lane_bursts[0].refused.as_deref(), Some("agent/node.plain holds no lane on this host"));
    let off = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    seat(&store, "laned", "capacity cores=2 memory=\"4G\"");
    off.seat_admission.status.burst_hold("agent/node.laned", Capacity { cores: 8, memory: 24 * G }, "st-test", now());
    off.reconcile_lane_bursts().unwrap();
    assert_eq!(off.seat_admission.status.state().lane_bursts[0].refused.as_deref(), Some("lanes are off on this host"));
}

#[test]
fn after_a_daemon_restart_a_raised_slice_counts_and_its_renewed_burst_holds_it_again() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let runtime = Arc::new(FakeRuntime::default());
    let member = seat(&store, "builder", "capacity cores=2 memory=\"4G\"");
    let builder = "agent/node.builder";
    let burst = Capacity { cores: 8, memory: 24 * G };
    // A new daemon: no record of the burst, but the slice is still raised and holds a build.
    *runtime.lane_live.lock().unwrap() = Some(burst);
    *runtime.lane_held.lock().unwrap() = Some(20 * G);
    let reconciler = laned(runtime.clone(), &store);
    hold(&reconciler, &store, builder, &member);
    assert_eq!(reconciler.seat_admission.status.state().lane_raised.get(builder).map(|lane| lane.memory), Some(24 * G));
    // The holder's renewal asks again: its own raised lane does not stand in its way.
    reconciler.seat_admission.status.burst_hold(builder, burst, "st-test", now());
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(reconciler.seat_admission.status.burst_granted(builder), Some(burst));
    hold(&reconciler, &store, builder, &member);
    assert_eq!(reconciler.seat_admission.status.state().lane_raised.get(builder), Some(&burst));
}

/// A seat declared on this host that is not running, waiting for room since `since`.
fn waiting_seat(reconciler: &Reconciler<FakeRuntime>, store: &Store, name: &str, lane: &str, since: u64, at: u64) {
    apply_source(store, &format!("version 2\nagent \"{name}\" {{ workspace \"/tmp\"; command \"true\"; capacity {lane} }}"), name);
    let status = &reconciler.seat_admission.status;
    status.lane_wait(&format!("agent/node.{name}"), "lane does not fit".into(), true, since);
    status.state().lane_waits[0].checked = at;
}

#[test]
fn a_burst_waits_behind_a_seat_that_has_waited_long_to_start() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = laned(Arc::new(FakeRuntime::default()), &store);
    seat(&store, "builder", "capacity cores=2 memory=\"4G\"");
    let status = &reconciler.seat_admission.status;
    let at = now();
    // A seat waiting for room since 10 minutes ago: a burst that fits is still granted.
    waiting_seat(&reconciler, &store, "big", "cores=8 memory=\"40G\"", at - 600, at);
    status.burst_hold("agent/node.builder", Capacity { cores: 2, memory: 6 * G }, "st-test", at);
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted("agent/node.builder"), Some(Capacity { cores: 2, memory: 6 * G }));
    assert!(status.burst_release("agent/node.builder"));
    reconciler.reconcile_lane_bursts().unwrap();
    // Once it has waited 15 minutes, a new burst waits behind it, so the host drains to fit for it.
    status.state().lane_waits[0].since = at - 15 * 60;
    status.burst_hold("agent/node.builder", Capacity { cores: 2, memory: 6 * G }, "st-test", at);
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted("agent/node.builder"), None);
    assert_eq!(status.state().lane_bursts[0].wait.as_deref(), Some(
        "burst lane 2 cores/6.0G for agent/node.builder waits behind agent/node.big, which has waited 15 min to start"));
    // Its start leaves the line: the burst is served beside it.
    status.lane_admitted("agent/node.big", Capacity { cores: 8, memory: 40 * G }, at);
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted("agent/node.builder"), Some(Capacity { cores: 2, memory: 6 * G }));
}

#[test]
fn a_burst_does_not_wait_behind_a_seat_that_would_not_fit_once_every_raise_is_given_back() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = laned(Arc::new(FakeRuntime::default()), &store);
    seat(&store, "builder", "capacity cores=2 memory=\"4G\"");
    seat(&store, "other", "capacity cores=2 memory=\"20G\"");
    let status = &reconciler.seat_admission.status;
    let at = now();
    // 54G for lanes, less the two seats' own 24G and 2G outside lanes, leaves 28G: the 40G seat
    // waits for a seat to stop, not for bursts to end.
    waiting_seat(&reconciler, &store, "big", "cores=8 memory=\"40G\"", at - 15 * 60, at);
    status.burst_hold("agent/node.builder", Capacity { cores: 2, memory: 6 * G }, "st-test", at);
    reconciler.reconcile_lane_bursts().unwrap();
    assert_eq!(status.burst_granted("agent/node.builder"), Some(Capacity { cores: 2, memory: 6 * G }));
}
