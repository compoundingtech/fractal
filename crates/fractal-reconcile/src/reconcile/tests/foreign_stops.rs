//! A stop another host owns is left out of full passes, and evaluated again when something that
//! decides its owner changes: a claim on its subject, or the fleet's member hosts.
use super::*;
use crate::fleet::MemberKey;
use serde_json::json;

const FLEET: &str = "6f1d2c1e-6c8e-4a7a-9d61-0b7f4f6e2a10";
const ITEM: &str = "stop:agent/example/worker";
/// A read no evaluation records: while an item still carries it, no pass evaluated it.
const UNTOUCHED: &str = "test:not-evaluated-since";

fn append(store: &Store, subject: &str, kind: &str, fields: Value) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// Host "ivory" in a fleet with member "jade", and a stopped seat that ran on jade.
fn foreign_stop() -> (Arc<Store>, Reconciler<FakeRuntime>) {
    let store = Arc::new(Store::open_memory("ivory").unwrap());
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    store.set_member_key(Some(anchor.clone())).unwrap();
    store.admit_fleet_anchor(FLEET, anchor.public(), "listening").unwrap();
    let jade = MemberKey::generate().unwrap().0.public().to_owned();
    append(
        &store,
        "host/jade",
        "fleet.member-admitted",
        json!({"fleet_id": FLEET, "member_key": jade, "via": "invite", "sponsor": "host/ivory", "mode": "listening"}),
    );
    apply_source(
        &store,
        "version 2\nagent \"example/worker\" { host \"jade\"; workspace \"/tmp\"; command \"true\" }",
        "foreign-seat",
    );
    apply_source(&store, "version 2\nstop \"agent/example/worker\"", "foreign-stop");
    let mut reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "ivory".into(),
        Arc::new(Notify::new()),
    );
    reconciler.skip_unneeded = true;
    // The first pass is a full one: it finds the stop is jade's.
    reconciler.reconcile_once().unwrap();
    assert!(
        reconciler.foreign_stops.lock().unwrap().contains(ITEM),
        "the stop was not found to be another host's"
    );
    let reads = reconciler.incremental.reads_of(ITEM);
    assert!(reads.contains("agent/example/worker"), "{reads:?}");
    assert!(reads.contains(FLEET_MEMBER_HOSTS), "{reads:?}");
    (store, reconciler)
}

/// Note that no pass has evaluated the stop since now, keeping what it read.
fn watch(reconciler: &Reconciler<FakeRuntime>) {
    let mut reads = reconciler.incremental.reads_of(ITEM);
    reads.insert(UNTOUCHED.into());
    reconciler.incremental.evaluated(ITEM, reads, None);
}

fn evaluated_since_watch(reconciler: &Reconciler<FakeRuntime>) -> bool {
    !reconciler.incremental.reads_of(ITEM).contains(UNTOUCHED)
}

/// A pass after the full-pass interval, so every section that takes full passes takes one.
fn full_pass(reconciler: &Reconciler<FakeRuntime>, after: &mut u128) {
    *after += crate::incremental::FULL_PASS_INTERVAL_MS;
    smallclaims::store::set_thread_clock(Some(*after));
    reconciler.reconcile_once().unwrap();
}

struct ResetClock;

impl Drop for ResetClock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

#[test]
fn a_full_pass_leaves_out_a_settled_stop_another_host_owns() {
    let _reset = ResetClock;
    let (_store, reconciler) = foreign_stop();
    let mut now = now_ms();
    watch(&reconciler);
    full_pass(&reconciler, &mut now);
    assert!(!evaluated_since_watch(&reconciler), "a full pass evaluated another host's stop");
    assert!(reconciler.foreign_stops.lock().unwrap().contains(ITEM));
}

#[test]
fn a_claim_on_its_subject_has_another_hosts_stop_evaluated_again() {
    let _reset = ResetClock;
    let (store, reconciler) = foreign_stop();
    let mut now = now_ms();
    watch(&reconciler);
    store
        .append_claim(&ClaimInput {
            subject: "agent/example/worker".into(),
            kind: "harness.observed".into(),
            actor: Some("agent/example/worker".into()),
            fields: BTreeMap::from([
                ("state".into(), json!("ready")),
                ("driver".into(), json!("codex")),
                ("incarnation_id".into(), json!("invented")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    full_pass(&reconciler, &mut now);
    assert!(evaluated_since_watch(&reconciler), "a claim on the subject did not mark the stop");
}

#[test]
fn its_owner_leaving_the_fleet_has_another_hosts_stop_evaluated_again() {
    let _reset = ResetClock;
    let (store, reconciler) = foreign_stop();
    let mut now = now_ms();
    watch(&reconciler);
    full_pass(&reconciler, &mut now);
    assert!(!evaluated_since_watch(&reconciler));
    store.remove_fleet_member("jade", "invented", "person/operator").unwrap();
    full_pass(&reconciler, &mut now);
    assert!(
        evaluated_since_watch(&reconciler),
        "the owner leaving the fleet did not mark the stop"
    );
}
