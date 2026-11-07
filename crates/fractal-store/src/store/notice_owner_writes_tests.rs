use super::*;
use crate::store::fleet_admission_tests_shared_seats::{admit, key, node, sync};
use fractal_schema::notice_receipt::{Outcome, Surface};
use smallclaims::fleet::MemberState;

const SOURCE: &str = "custom/attention/source/01900000-0000-7000-8000-000000000002";
const RECEIPT: &str = "custom/attention/receipt/01900000-0000-7000-8000-000000000003";

fn fixture() -> (Store, Arc<crate::fleet::MemberKey>, Acknowledgement, Receipt) {
    let owner = key();
    let store = node("example", Some(&owner), Some(&owner));
    admit(&store, "example", &owner, "anchor", None);
    let membership = smallclaims::store::fleet_membership_tx(&store.readers.get()).unwrap();
    let MemberState::Current(member) = membership.state("example") else { panic!("owner not admitted") };
    let intent = crate::graph::parse_test_intent(&format!(
        "notice-owner \"default\" member=\"host/example\" incarnation=\"{}\" always-on=true",
        member.admitted_claim), "example").unwrap();
    store.apply(&intent, &BTreeMap::new(), "writer-owner").unwrap();
    let runtime = crate::parse_intent(
        "version 2\nagent \"example/runtime\" { command \"true\"; notice-runtime; }", "example").unwrap();
    let expected = runtime.subjects.keys().map(|subject| (subject.clone(), Vec::new())).collect();
    store.apply_as(&runtime, &expected, "writer-runtime", Some("person/example")).unwrap();
    for manifest in notice::manifests() {
        store.register_custom_kind(&custom::RegistrationRequest {
            manifest: manifest.clone(), actor: "person/example".into(),
        }).unwrap();
    }
    let conformance: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h3d31c440280f owner=fractal_store::store::notice_owner_writes_tests TODO=trace-statement-and-value-representation */ include_str!(
        "../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
    let source = store.append_claim(&ClaimInput {
        subject: SOURCE.into(), kind: "custom.attention.notice-created".into(),
        actor: Some("agent/example/producer".into()),
        fields: serde_json::from_value(conformance["snapshot"]["input"].clone()).unwrap(),
        ..custom::tests_shared_delivery::empty_input()
    }).unwrap();
    let binding = notice_owner::resolve(&store.readers.get()).unwrap();
    let acknowledgement = Acknowledgement {
        owner_binding: binding.declaration_claim.clone(), previous_ack: None,
        status: Status::Active,
        runtime_binding: Some(notice_authority::binding(&store.readers.get()).unwrap().1),
    };
    let receipt = Receipt {
        source_id: SOURCE.into(), creation_claim: source.id,
        producer: "agent/example/producer".into(), recipient: "person/example".into(),
        owner_binding: binding.declaration_claim, surface: Surface::Tray,
        outcome: Outcome::Delivered {
            completed_at: "2026-11-07T12:00:00Z".into(), evidence: "surface acknowledged".into(),
        },
    };
    (store, owner, acknowledgement, receipt)
}

#[test]
fn notice_member_writers_produce_real_sealed_owner_proof_and_identical_receipt_is_idempotent() {
    let (store, owner, acknowledgement, receipt) = fixture();
    let ack = store.acknowledge_notice_owner(&acknowledgement).unwrap();
    assert!(ack.actor.is_none());
    let ack_position = super::super::notice_owner_proof::authenticate(
        &store.readers.get(), &ack, None, owner.public()).unwrap();
    let claim = store.record_notice_receipt(RECEIPT, &receipt).unwrap();
    assert!(claim.actor.is_none());
    let position = super::super::notice_owner_proof::authenticate(
        &store.readers.get(), &claim, None, owner.public()).unwrap();
    assert!((position.sequence, position.position) > (ack_position.sequence, ack_position.position));
    Chain::load(&store.readers.get(), None).unwrap()
        .authorize(&store.readers.get(), &claim, &receipt, None).unwrap();
    assert_eq!(store.record_notice_receipt(RECEIPT, &receipt).unwrap().id, claim.id);
    let mut conflicting = receipt;
    conflicting.outcome = Outcome::Failed { reason: "different evidence".into() };
    assert!(store.record_notice_receipt(RECEIPT, &conflicting).is_err());
    assert_eq!(custom::claims(&store.readers.get(), RECEIPT).unwrap().len(), 1);
}

#[test]
fn notice_another_real_admitted_member_cannot_acknowledge_or_write_owner_receipts() {
    let (store, owner, acknowledgement, receipt) = fixture();
    let other_key = key();
    admit(&store, "other", &other_key, "sponsor", None);
    let other = node("other", Some(&other_key), Some(&owner));
    sync(&store, &other);
    assert_eq!(other.acknowledge_notice_owner(&acknowledgement).unwrap_err().code,
        "invalid-notice-owner-proof");
    store.acknowledge_notice_owner(&acknowledgement).unwrap();
    sync(&store, &other);
    assert_eq!(other.record_notice_receipt(RECEIPT, &receipt).unwrap_err().code,
        "invalid-notice-owner-proof");
    assert!(custom::claims(&other.readers.get(), RECEIPT).unwrap().is_empty());
}

#[test]
fn notice_receipt_writer_does_not_require_current_runtime_and_ended_owner_cannot_append() {
    let (store, _, acknowledgement, receipt) = fixture();
    let active = store.acknowledge_notice_owner(&acknowledgement).unwrap();
    let runtime = "agent/example/runtime";
    let token = store.selected_desired_token(runtime).unwrap().unwrap();
    let stop = crate::parse_intent(&format!("version 2\nstop \"{runtime}\""), "example").unwrap();
    store.apply_as(&stop, &BTreeMap::from([(runtime.into(), vec![token])]),
        "writer-stop-runtime", Some("person/example")).unwrap();
    assert!(notice_authority::binding(&store.readers.get()).is_err());
    let claim = store.record_notice_receipt(RECEIPT, &receipt).unwrap();
    let ended = Acknowledgement {
        owner_binding: acknowledgement.owner_binding, previous_ack: Some(active.id),
        status: Status::Ended, runtime_binding: None,
    };
    store.acknowledge_notice_owner(&ended).unwrap();
    // Retrying identical historical evidence remains valid after the owner interval ends.
    assert_eq!(store.record_notice_receipt(RECEIPT, &receipt).unwrap().id, claim.id);
    let next_subject = RECEIPT.replace("000003", "000004");
    let mut next = receipt;
    next.surface = Surface::Speech;
    assert!(store.record_notice_receipt(&next_subject, &next).is_err());
}

#[test]
fn notice_acknowledgement_writer_rejects_stale_head_and_illegal_end_without_persisting() {
    let (store, _, mut acknowledgement, _) = fixture();
    let mut ended = acknowledgement.clone();
    ended.status = Status::Ended;
    ended.runtime_binding = None;
    assert!(store.acknowledge_notice_owner(&ended).is_err());
    let first = store.acknowledge_notice_owner(&acknowledgement).unwrap();
    assert!(store.acknowledge_notice_owner(&acknowledgement).is_err());
    acknowledgement.previous_ack = Some(first.id);
    store.acknowledge_notice_owner(&acknowledgement).unwrap();
    let count = custom::claims(&store.readers.get(), "notice-owner/default").unwrap()
        .into_iter().filter(|claim| claim.kind == fractal_schema::notice_owner_ack::KIND).count();
    assert_eq!(count, 2);
}
