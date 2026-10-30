//! A stop another host owns, and a seat declared on another host that was never declared here,
//! give this host nothing to do. Full passes leave them out, and they are evaluated again when
//! something that decides it changes: a claim on their subject, or for a stop the fleet's member
//! hosts.
use super::*;
use crate::fleet::MemberKey;
use serde_json::json;

const FLEET: &str = "6f1d2c1e-6c8e-4a7a-9d61-0b7f4f6e2a10";
const SEAT: &str = "agent/example/worker";
const STOP: &str = "stop:agent/example/worker";
const AWAY: &str = "away:agent/example/worker";
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

fn seat_on(host: &str) -> String {
    format!("version 2\nagent \"example/worker\" {{ host \"{host}\"; workspace \"/tmp\"; command \"true\" }}")
}

/// Host "ivory" in a fleet with member "jade", and a seat declared on jade.
fn fleet_with_seat_on_jade() -> Arc<Store> {
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
    apply_source(&store, &seat_on("jade"), "jade-seat");
    store
}

/// A reconciler that skips settled items, after its first pass: a full one, which finds `item`
/// has nothing for this host to do and that it read the seat's subject.
fn reconciler_finding_nothing_here(store: &Arc<Store>, item: &str) -> Reconciler<FakeRuntime> {
    let mut reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "ivory".into(),
        Arc::new(Notify::new()),
    );
    reconciler.skip_unneeded = true;
    reconciler.reconcile_once().unwrap();
    assert!(reconciler.nothing_here(item), "{item} was not found to give this host nothing to do");
    let reads = reconciler.incremental.reads_of(item);
    assert!(reads.contains(SEAT), "{reads:?}");
    reconciler
}

/// Host "ivory", and a stopped seat that ran on jade.
fn foreign_stop() -> (Arc<Store>, Reconciler<FakeRuntime>) {
    let store = fleet_with_seat_on_jade();
    apply_source(&store, "version 2\nstop \"agent/example/worker\"", "foreign-stop");
    let reconciler = reconciler_finding_nothing_here(&store, STOP);
    assert!(reconciler.incremental.reads_of(STOP).contains(FLEET_MEMBER_HOSTS));
    (store, reconciler)
}

/// Note that no pass has evaluated `item` since now, keeping what it read.
fn watch(reconciler: &Reconciler<FakeRuntime>, item: &str) {
    let mut reads = reconciler.incremental.reads_of(item);
    reads.insert(UNTOUCHED.into());
    reconciler.incremental.evaluated(item, reads, None);
}

/// Whether a pass evaluated `item` since [`watch`]: it is still an item, and what it read was
/// replaced. An item a pass dropped (its subject no longer one) reads nothing and was not.
fn evaluated_since_watch(reconciler: &Reconciler<FakeRuntime>, item: &str) -> bool {
    let reads = reconciler.incremental.reads_of(item);
    assert!(!reads.is_empty(), "{item} is no longer an item");
    !reads.contains(UNTOUCHED)
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
    watch(&reconciler, STOP);
    full_pass(&reconciler, &mut now);
    assert!(!evaluated_since_watch(&reconciler, STOP), "a full pass evaluated another host's stop");
    assert!(reconciler.nothing_here(STOP));
}

#[test]
fn a_claim_on_its_subject_has_another_hosts_stop_evaluated_again() {
    let _reset = ResetClock;
    let (store, reconciler) = foreign_stop();
    let mut now = now_ms();
    watch(&reconciler, STOP);
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "harness.observed".into(),
            actor: Some(SEAT.into()),
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
    assert!(evaluated_since_watch(&reconciler, STOP), "a claim on the subject did not mark the stop");
}

#[test]
fn its_owner_leaving_the_fleet_has_another_hosts_stop_evaluated_again() {
    let _reset = ResetClock;
    let (store, reconciler) = foreign_stop();
    let mut now = now_ms();
    watch(&reconciler, STOP);
    full_pass(&reconciler, &mut now);
    assert!(!evaluated_since_watch(&reconciler, STOP));
    store.remove_fleet_member("jade", "invented", "person/operator").unwrap();
    full_pass(&reconciler, &mut now);
    assert!(
        evaluated_since_watch(&reconciler, STOP),
        "the owner leaving the fleet did not mark the stop"
    );
}

#[test]
fn a_full_pass_leaves_out_a_seat_on_another_host_never_declared_here() {
    let _reset = ResetClock;
    let store = fleet_with_seat_on_jade();
    let reconciler = reconciler_finding_nothing_here(&store, AWAY);
    let mut now = now_ms();
    watch(&reconciler, AWAY);
    full_pass(&reconciler, &mut now);
    assert!(!evaluated_since_watch(&reconciler, AWAY), "a full pass read the seat's declarations again");
    assert!(reconciler.nothing_here(AWAY));
}

/// The seat is declared here and moved back to jade before any pass: this host may have a
/// process of it to retire, so the next pass evaluates the item, and full passes keep it.
#[test]
fn a_declaration_placing_the_seat_here_has_its_away_item_evaluated_again() {
    let _reset = ResetClock;
    let store = fleet_with_seat_on_jade();
    let reconciler = reconciler_finding_nothing_here(&store, AWAY);
    let mut now = now_ms();
    watch(&reconciler, AWAY);
    apply_source(&store, &seat_on("ivory"), "ivory-seat");
    apply_source(&store, &seat_on("jade"), "jade-seat-again");
    full_pass(&reconciler, &mut now);
    assert!(evaluated_since_watch(&reconciler, AWAY), "a declaration here did not mark the away item");
    assert!(!reconciler.nothing_here(AWAY), "a seat once declared here is still left out of full passes");
}

/// The same declarations written on jade long ago, arriving by replication: they are new to
/// this store, whatever their accepted time, and mark the item.
#[test]
fn a_replicated_declaration_placing_the_seat_here_marks_its_away_item_whatever_its_time() {
    let _reset = ResetClock;
    let store = fleet_with_seat_on_jade();
    let reconciler = reconciler_finding_nothing_here(&store, AWAY);
    let mut now = now_ms();
    watch(&reconciler, AWAY);
    let jade = Store::open_memory("jade").unwrap();
    let long_ago = now - 24 * 60 * 60 * 1000;
    jade.set_write_clock_at(long_ago).unwrap();
    apply_source(&jade, &seat_on("ivory"), "ivory-seat-from-jade");
    jade.set_write_clock_at(long_ago + 1_000).unwrap();
    apply_source(&jade, &seat_on("jade"), "jade-seat-from-jade");
    store.import_replication("jade", &jade.export_replication(0).unwrap()).unwrap();
    let selected = store.desired_subjects().unwrap().into_iter().find(|subject| subject.subject == SEAT).unwrap();
    assert_eq!(selected.member.map(|member| member.host).as_deref(), Some("jade"), "the seat is still away");
    let here = store
        .claims_for(SEAT, Some("intent.desired"))
        .unwrap()
        .into_iter()
        .filter(|claim| claim.body["member"]["host"] == "ivory")
        .collect::<Vec<_>>();
    assert_eq!(here.len(), 1, "the replicated declaration did not arrive");
    assert!(here[0].accepted_at_unix_ms < now - 60 * 60 * 1000, "{}", here[0].accepted_at_unix_ms);
    full_pass(&reconciler, &mut now);
    assert!(evaluated_since_watch(&reconciler, AWAY), "a replicated declaration did not mark the away item");
    assert!(!reconciler.nothing_here(AWAY));
}

/// What a reconciler found is kept in memory only: a restarted daemon evaluates every item in
/// its first pass, and finds again that it has nothing to do here.
#[test]
fn a_restarted_reconciler_evaluates_every_item_again() {
    let _reset = ResetClock;
    let (store, before) = foreign_stop();
    assert!(before.nothing_here(AWAY));
    drop(before);
    let mut after = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "ivory".into(),
        Arc::new(Notify::new()),
    );
    after.skip_unneeded = true;
    assert!(after.nothing_here.lock().unwrap().is_empty());
    assert!(after.incremental.reads_of(STOP).is_empty() && after.incremental.reads_of(AWAY).is_empty());
    after.reconcile_once().unwrap();
    for item in [STOP, AWAY] {
        assert!(after.incremental.reads_of(item).contains(SEAT), "{item} was not evaluated");
        assert!(after.nothing_here(item), "{item}");
    }
}
