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
        .with_lane_host(Some(LaneHost { room: Capacity { cores: 14, memory: 54 * G }, outside: two_gib_outside }));
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
