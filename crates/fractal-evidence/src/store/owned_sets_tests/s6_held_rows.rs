//! Live-only owned selection regressions; parent adds `mod s6_held_rows;`.
//! Uses the integrator's parent signed_share_incrementally helper. No new seam:
//! try_project_simple_replication_tx controls a prefix of genuinely admitted input.
//! Physical last-good preservation here complements, not replaces, effective holds.
use super::*;

const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
const ORCHARD: &str = "agent/garden/orchard";
const MEADOW: &str = "agent/garden/meadow";
const MISSION: &str = "mission/garden/harvest";

fn mixed(command: &str, goal: &str, meadow: bool) -> NormalizedIntent {
    parse_intent(&format!(
        "version 2\nagent \"garden/orchard\" {{ command {command:?} }}\nmission \"garden/harvest\" state=\"ready\" {{ goal {goal:?}; step \"work\" {{ agentless }} }}\n{}",
        if meadow { "agent \"garden/meadow\" { command \"true\" }" } else { "" }
    ), "amber").unwrap()
}

/// Raw physical selections, including every stored desired/definition column.
/// Do not replace these reads with owned/effective accessors: those hide this bug.
fn physical(store: &Store) -> (Vec<String>, Vec<String>) {
    let connection = store.readers.get();
    let desired = connection.prepare(
        "SELECT json_object('subject',subject,'kind',kind,'revision',revision,'claim_id',claim_id,
         'body',body,'member',member,'owner_run',owner_run,'owner_generation',owner_generation,'owner_step',owner_step)
         FROM desired WHERE subject IN ('agent/garden/orchard','agent/garden/meadow') ORDER BY subject"
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<Vec<String>>>().unwrap();
    let missions = connection.prepare(
        "SELECT json_object('mission_id',mission_id,'revision',revision,'state',state,'claim_id',claim_id)
         FROM mission_definitions WHERE mission_id='garden/harvest' ORDER BY mission_id"
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<Vec<String>>>().unwrap();
    (desired, missions)
}

fn claim_for(store: &Store, subject: &str) -> ClaimRecord {
    store.claims_for(subject, Some(if subject.starts_with("mission/") {
        "mission.published"
    } else { "intent.desired" })).unwrap().pop().unwrap()
}

fn replace_reference(receipt: &mut owned_sets::Revision, subject: &str, claim: &ClaimRecord) {
    let member = receipt.members.get_mut(subject).expect("fixture includes subject");
    member.claim = claim.id.clone();
    member.revision = if claim.kind == "mission.published" {
        claim.body["revision"].as_str().unwrap().to_owned()
    } else {
        desired_revision(&serde_json::from_value::<DesiredSubject>(claim.body.clone()).unwrap())
    };
}

fn append_set(store: &Store, set: &str, receipt: &owned_sets::Revision) {
    store.append_claim(&ClaimInput {
        subject: set.into(), kind: "owned-set.revised".into(), actor: None,
        fields: serde_json::from_value(json!({"revision":canonical_hash(receipt).unwrap(),"body":receipt})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn snapshot(from: &Store, to: &Store) -> ReplicationExchange {
    from.export_replication_exchange_answering(FLEET, &to.replication_inventory().unwrap(),
        &to.replication_signature_requests().unwrap()).unwrap()
}

fn admit(target: &Store, relay: &str, exchange: &ReplicationExchange) {
    target.receive_replication_exchange(relay, FLEET, exchange).unwrap();
    let admitted = target.validate_replication_backlog().unwrap();
    assert_eq!(admitted.invalid, 0);
    assert_eq!(admitted.unknown, 0);
}

fn deliver(target: &Store, relay: &str, exchange: &ReplicationExchange) {
    let before = smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get);
    admit(target, relay, exchange);
    assert!(target.project_replication_backlog().unwrap());
    assert!(!target.replication_projection_recovery_required().unwrap());
    assert_eq!(smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get), before);
}

fn assert_lower(target: &Store, earlier: &ClaimRecord, later: &str) {
    let connection = target.readers.get();
    assert!(smallclaims::store::canonical::claim_key(&connection, &earlier.id).unwrap()
        < smallclaims::store::canonical::claim_key(&connection, later).unwrap());
}

fn assert_pending(store: &Store, subject: &str) {
    assert_eq!(store.owned_member_guard(subject).unwrap_err().code, "owned-set-pending");
    assert!(!store.replication_projection_recovery_required().unwrap(),
        "an incomplete set is a normal effective hold, not a terminal projection fault");
}

fn assert_mission_history(store: &Store, claim: &ClaimRecord) {
    assert!(store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM mission_revisions WHERE mission_id='garden/harvest' AND revision=?1)",
        [claim.body["revision"].as_str().unwrap()], |row| row.get::<_, bool>(0)).unwrap(),
        "holding the selected definition must not suppress revision history materialization");
}

/// R10 is complete; R30 references earlier A1/B1 but only A1 arrives.
/// The mission variant makes A1 a mission publication and B1 an agent declaration.
fn held_last_good(mission_first: bool) {
    let fleet = signed_fleet();
    // cobalt sorts below ivory even when both declarations share one millisecond.
    let (receiver, reference, publisher) = (&fleet[0], &fleet[1], &fleet[2]);
    let initial = if mission_first { mixed("initial", "Initial", false) } else { bundle("initial", true) };
    apply(publisher, &initial, 10);
    signed_share_incrementally(publisher, receiver);
    let last_good = physical(receiver);
    let a_subject = if mission_first { MISSION } else { ORCHARD };
    let b_subject = if mission_first { ORCHARD } else { MEADOW };
    let first = if mission_first {
        parse_intent("version 2\nmission \"garden/harvest\" state=\"ready\" { goal \"Replacement\"; step \"work\" { agentless } }", "amber").unwrap()
    } else { bundle("replacement", false) };
    direct(reference, &first, "earlier-A1").unwrap();
    let a = claim_for(reference, a_subject);
    let only_a = snapshot(reference, receiver);
    let second = if mission_first { bundle("replacement", false) } else { bundle("replacement", true) };
    direct(reference, &second, "earlier-B1").unwrap();
    let b = claim_for(reference, b_subject);
    let old = publisher.owned_sets().unwrap().remove(0);
    let mut receipt = old.receipt;
    receipt.previous = Some(format!("{}@{}", old.id, old.revision));
    receipt.source = options(publisher, 30).source;
    replace_reference(&mut receipt, a_subject, &a);
    replace_reference(&mut receipt, b_subject, &b);
    append_set(publisher, "owned-set/garden", &receipt);
    let higher = claim_for_set(publisher, "owned-set/garden");
    signed_share_incrementally(publisher, receiver);
    assert_eq!(physical(receiver), last_good);
    assert_pending(receiver, a_subject);
    deliver(receiver, &reference.origin, &only_a);
    assert!(receiver.claim_by_id(&b.id).unwrap().is_none(), "B1 must remain withheld");
    assert_lower(receiver, &a, &higher);
    assert_eq!(physical(receiver), last_good,
        "a lower-key member must not erase held last-good physical selections");
    assert_pending(receiver, a_subject);
    if mission_first { assert_mission_history(receiver, &a); }
    signed_share_incrementally(reference, receiver);
    assert!(receiver.owned_sets().unwrap()[0].blockers.is_empty());
    assert!(receiver.owned_member_guard(a_subject).is_ok());
    assert_ne!(physical(receiver), last_good, "replacement occurs only after B1 arrives");
    if mission_first {
        assert_eq!(receiver.readers.get().query_row(
            "SELECT claim_id FROM mission_definitions WHERE mission_id='garden/harvest'", [],
            |row| row.get::<_, String>(0)).unwrap(), a.id);
    } else {
        assert_eq!(receiver.selected_desired_token(ORCHARD).unwrap(), Some(a.id));
    }
}

fn claim_for_set(store: &Store, set: &str) -> String {
    store.claims_for(set, Some("owned-set.revised")).unwrap().pop().unwrap().id
}

#[test]
fn late_owned_desired_reference_preserves_held_physical_last_good_until_final_reference() {
    held_last_good(false);
}

#[test]
fn late_owned_mission_reference_preserves_held_definition_and_materializes_revision() {
    held_last_good(true);
}

#[test]
fn overlapping_historical_owners_preserve_both_desired_and_mission_without_parking() {
    let fleet = signed_fleet();
    let (receiver, reference, publisher) = (&fleet[0], &fleet[1], &fleet[2]);
    apply(publisher, &mixed("initial", "Initial", true), 10);
    signed_share_incrementally(publisher, receiver);
    let last_good = physical(receiver);
    direct(reference, &mixed("replacement", "Replacement", false), "overlap-A1").unwrap();
    let desired = claim_for(reference, ORCHARD);
    let mission = claim_for(reference, MISSION);
    let only_a = snapshot(reference, receiver);
    direct(reference, &mixed("replacement", "Replacement", true), "overlap-B1").unwrap();
    let meadow = claim_for(reference, MEADOW);
    let old = publisher.owned_sets().unwrap().remove(0);
    let mut receipt = old.receipt;
    receipt.previous = Some(format!("{}@{}", old.id, old.revision));
    receipt.source = options(publisher, 30).source;
    for (subject, claim) in [(ORCHARD, &desired), (MISSION, &mission), (MEADOW, &meadow)] {
        replace_reference(&mut receipt, subject, claim);
    }
    append_set(publisher, "owned-set/garden", &receipt);
    // A distinct, otherwise valid root receipt creates ordinary overlapping ownership.
    // Its lineage is not cross-set; the only conflict is shared historical ownership.
    let mut other = receipt.clone();
    other.previous = None;
    append_set(publisher, "owned-set/overlap", &other);
    signed_share_incrementally(publisher, receiver);
    let owners = owned_sets::owners_at(&receiver.readers.get(), None).unwrap();
    assert_eq!(owners[ORCHARD].len(), 2);
    assert_eq!(owners[MISSION].len(), 2);
    assert!(receiver.owned_sets().unwrap().iter().all(|set|
        set.blockers.iter().any(|reason| reason.contains("ownership conflict"))));
    let higher = claim_for_set(receiver, "owned-set/garden");
    deliver(receiver, &reference.origin, &only_a);
    assert!(receiver.claim_by_id(&meadow.id).unwrap().is_none());
    assert_lower(receiver, &desired, &higher);
    assert_lower(receiver, &mission, &higher);
    assert_eq!(physical(receiver), last_good,
        "ANY owner, not a unique-owner lookup, preserves both physical selections");
    assert_eq!(receiver.owned_member_guard(ORCHARD).unwrap_err().code, "owned-set-conflict");
    assert_eq!(receiver.owned_member_guard(MISSION).unwrap_err().code, "owned-set-conflict");
    assert!(!receiver.replication_projection_recovery_required().unwrap());
    assert_mission_history(receiver, &mission);
}

/// Admit all envelopes normally but choose a prefix before the owning receipt.
/// With a descendant, A2 also lies beyond through and must not leak into the fold.
fn consuming_prefix(future_descendant: bool) {
    let fleet = signed_fleet();
    let (receiver, reference, publisher) = (&fleet[0], &fleet[1], &fleet[2]);
    direct(reference, &mixed("earlier", "Earlier", false), "prefix-A1").unwrap();
    let a_desired = claim_for(reference, ORCHARD);
    let a_mission = claim_for(reference, MISSION);
    direct(publisher, &mixed("last-good", "Last good", false), "prefix-old").unwrap();
    let old_desired = claim_for(publisher, ORCHARD);
    let old_mission = claim_for(publisher, MISSION);
    let old_wire = publisher.export_replication(0).unwrap();
    signed_share_incrementally(publisher, receiver);
    let early_exchange = snapshot(reference, receiver);
    let replacement = if future_descendant {
        direct(publisher, &mixed("future-descendant", "Future descendant", false), "prefix-A2").unwrap();
        publisher
    } else { reference };
    let selected_desired = claim_for(replacement, ORCHARD);
    let selected_mission = claim_for(replacement, MISSION);
    let template = Store::open_memory("template").unwrap();
    apply(&template, &mixed("template", "Template", false), 10);
    let mut receipt = template.owned_sets().unwrap().remove(0).receipt;
    receipt.previous = None;
    receipt.source = options(publisher, 30).source;
    replace_reference(&mut receipt, ORCHARD, &selected_desired);
    replace_reference(&mut receipt, MISSION, &selected_mission);
    append_set(publisher, "owned-set/garden", &receipt);
    let future_exchange = snapshot(publisher, receiver);
    let before_replays = smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get);
    admit(receiver, &reference.origin, &early_exchange);
    let through = [receiver.claim_by_id(&a_desired.id).unwrap().unwrap().store_index,
        receiver.claim_by_id(&a_mission.id).unwrap().unwrap().store_index].into_iter().max().unwrap();
    admit(receiver, &publisher.origin, &future_exchange);
    let owning = receiver.claim_by_id(&claim_for_set(publisher, "owned-set/garden")).unwrap().unwrap();
    assert!(owning.store_index > through);
    let owners_at_prefix = owned_sets::owners_at(&receiver.readers.get(), Some(through)).unwrap();
    assert!(!owners_at_prefix.contains_key(ORCHARD));
    assert!(!owners_at_prefix.contains_key(MISSION));
    assert!(owned_sets::owners_at(&receiver.readers.get(), None).unwrap().contains_key(ORCHARD));
    assert_lower(receiver, &a_desired, &old_desired.id);
    assert_lower(receiver, &a_mission, &old_mission.id);
    // Independent oracle contains only the prefix, never A2 or the future receipt.
    let oracle = Store::open_memory("prefix-oracle").unwrap();
    oracle.import_replication(&publisher.origin, &old_wire).unwrap();
    share(reference, &oracle);
    let expected = physical(&oracle);
    assert!(!expected.0.is_empty() && !expected.1.is_empty());
    if future_descendant {
        assert!(receiver.claim_by_id(&selected_desired.id).unwrap().unwrap().store_index > through);
        assert!(receiver.claim_by_id(&selected_mission.id).unwrap().unwrap().store_index > through);
        assert_lower(receiver, &old_desired, &selected_desired.id);
        assert_lower(receiver, &old_mission, &selected_mission.id);
    }
    {
        let mut writer = receiver.connection.write();
        let transaction = writer.transaction().unwrap();
        assert_eq!(try_project_simple_replication_tx(&transaction, &receiver.origin, through).unwrap(),
            smallclaims::store::runtime::IncrementalProjection::Projected);
        transaction.commit().unwrap();
    }
    assert_eq!(physical(receiver), expected,
        "deletion, ownership lookup and consuming history must use exactly the same prefix");
    assert_eq!(smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get), before_replays);
    assert!(!receiver.replication_projection_recovery_required().unwrap());
    // Finish the actual backlog; future selection is permitted only now.
    assert!(receiver.project_replication_backlog().unwrap());
    assert_eq!(receiver.selected_desired_token(ORCHARD).unwrap(), Some(selected_desired.id));
    assert_eq!(receiver.readers.get().query_row(
        "SELECT claim_id FROM mission_definitions WHERE mission_id='garden/harvest'", [],
        |row| row.get::<_, String>(0)).unwrap(), selected_mission.id);
    assert_eq!(smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get), before_replays);
}

#[test]
fn same_prefix_consuming_selection_ignores_future_ownership_for_desired_and_mission() {
    consuming_prefix(false);
}

#[test]
fn prefix_straddle_does_not_select_already_admitted_future_descendants() {
    consuming_prefix(true);
}
