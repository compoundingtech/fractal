use super::*;

const SOURCE: &str = "custom/attention/source/01900000-0000-7000-8000-000000000002";
const RECEIPT: &str = "custom/attention/receipt/01900000-0000-7000-8000-000000000003";

// Projection-only fixture injection. These tests are not owner-authentication evidence.
struct ReviewedFixture;
impl ReceiptAuthorizer for ReviewedFixture {
    fn assess(&self, _: &ClaimRecord, _: &Receipt) -> ReceiptAuthorization { ReceiptAuthorization::Authorized }
}

fn setup() -> (Store, ClaimRecord) {
    let store = Store::open_memory("example").unwrap();
    let source = setup_source(&store);
    (store, source)
}

fn setup_source(store: &Store) -> ClaimRecord {
    for manifest in notice::manifests() {
        store.register_custom_kind(&custom::RegistrationRequest { manifest: manifest.clone(), actor: "person/example".into() }).unwrap();
    }
    let fixture: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h881725e371d9 owner=fractal_store::store::notice_receipts_tests TODO=trace-statement-and-value-representation */ include_str!("../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
    let source = store.append_claim(&ClaimInput {
        subject: SOURCE.into(), kind: "custom.attention.notice-created".into(), actor: Some("agent/example/producer".into()),
        fields: serde_json::from_value(fixture["snapshot"]["input"].clone()).unwrap(),
        ..custom::tests_shared_delivery::empty_input()
    }).unwrap();
    source
}

fn receipt(source: &ClaimRecord) -> Value {
    json!({"source_id":SOURCE,"creation_claim":source.id,"producer":"agent/example/producer","recipient":"person/example",
        "owner_binding":"a".repeat(64),"surface":"tray","outcome":{"state":"delivered","completed_at":"2026-11-07T12:00:00Z","evidence":"surface acknowledged"},
        "_registration":notice::HASHES[3]})
}

// Raw actor-asserted history isolates projection structure. Production must never
// authorize these fixtures, even when ReviewedFixture approves their shape.
fn durable(store: &Store, subject: &str, kind: &str, fields: Value) {
    store.connection.batched(|tx| append_claim_tx(tx, &store.origin, subject, kind,
        Some("person/example"), &json!({"fields":fields}), &[], None)).unwrap().unwrap();
}
fn approved(store: &Store) -> Vec<SurfaceDeliveryEvidence> {
    project(&store.readers.get(), SOURCE, &ReviewedFixture).unwrap()
}
fn assert_unknown(rows: &[SurfaceDeliveryEvidence], reason: ReceiptUnknownReason) {
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.evidence == unknown(reason.clone())));
}

#[test]
fn notice_raw_actor_receipts_without_member_proof_remain_unknown() {
    let (store, source) = setup();
    durable(&store, RECEIPT, notice_receipt::KIND, receipt(&source));
    let rows = store.notice_receipts(SOURCE).unwrap();
    assert_authority_unknown(&rows[0], "invalid-notice-owner-proof");
    assert!(rows[1..].iter().all(|row| row.evidence == unknown(ReceiptUnknownReason::MissingEvidence)));
}

#[test]
fn notice_snapshot_delivery_cannot_invent_receipt_evidence() {
    let (store, source) = setup();
    durable(&store, "custom/attention/snapshot/01900000-0000-7000-8000-000000000004", "custom.attention.snapshot-created",
        json!({"source_id":SOURCE,"creation_claim":source.id,"delivery":[{"surface":"tray","state":"delivered"}]}));
    assert_unknown(&approved(&store), ReceiptUnknownReason::MissingEvidence);
    assert_unknown(&store.notice_receipts(SOURCE).unwrap(), ReceiptUnknownReason::MissingEvidence);
}

#[test]
fn notice_fixture_injected_receipts_survive_runtime_handover_without_proving_authority() {
    let (store, source) = setup();
    durable(&store, RECEIPT, notice_receipt::KIND, receipt(&source));
    let mut failed = receipt(&source); failed["surface"] = json!("speech"); failed["outcome"] = json!({"state":"failed","reason":"surface rejected"});
    durable(&store, &RECEIPT.replace("000003", "000005"), notice_receipt::KIND, failed);
    let before = approved(&store);
    assert!(matches!(&before[0].evidence, DeliveryEvidence::Delivered { evidence, .. } if evidence == "surface acknowledged"));
    assert!(matches!(&before[1].evidence, DeliveryEvidence::Failed { reason, .. } if reason == "surface rejected"));
    for name in ["example/runtime-a", "example/runtime-b"] {
        let intent = crate::parse_intent(&format!("version 2\nagent \"{name}\" {{ command \"true\"; notice-runtime; }}"), "example").unwrap();
        let expected = intent.subjects.keys().map(|subject| (subject.clone(), Vec::new())).collect();
        store.apply_as(&intent, &expected, &format!("receipt-designate-{name}"), Some("person/example")).unwrap();
        assert_eq!(approved(&store), before);
        let subject = format!("agent/{name}");
        let token = store.selected_desired_token(&subject).unwrap().unwrap();
        let stop = crate::parse_intent(&format!("version 2\nstop \"{subject}\""), "example").unwrap();
        store.apply_as(&stop, &BTreeMap::from([(subject, vec![token])]), &format!("receipt-stop-{name}"), Some("person/example")).unwrap();
        assert_eq!(approved(&store), before);
    }
}

#[test]
fn notice_receipt_mismatched_lineage_is_unknown() {
    let (store, source) = setup();
    let mut fields = receipt(&source); fields["creation_claim"] = json!("b".repeat(64));
    durable(&store, RECEIPT, notice_receipt::KIND, fields);
    assert_unknown(&approved(&store), ReceiptUnknownReason::LineageMismatch);
}

#[test]
fn notice_conflicting_receipt_identities_are_unknown() {
    let (store, source) = setup();
    for subject in [RECEIPT.to_owned(), RECEIPT.replace("000003", "000004")] {
        durable(&store, &subject, notice_receipt::KIND, receipt(&source));
    }
    assert_unknown(&approved(&store), ReceiptUnknownReason::ConflictingEvidence);
}

#[test]
fn notice_receipt_updates_and_competing_creations_are_unknown() {
    for kind in [notice_receipt::KIND, "custom.attention.receipt-updated"] {
        let (store, source) = setup();
        durable(&store, RECEIPT, notice_receipt::KIND, receipt(&source));
        let mut fields = receipt(&source); fields["outcome"]["evidence"] = json!("competing evidence");
        durable(&store, RECEIPT, kind, fields);
        assert_unknown(&approved(&store), ReceiptUnknownReason::ConflictingEvidence);
    }
}

#[test]
fn notice_receipt_local_admission_rejects_actor_assertions() {
    let (store, source) = setup();
    let error = store.append_claim(&ClaimInput {
        subject: RECEIPT.into(), kind: notice_receipt::KIND.into(), actor: Some("person/example".into()),
        fields: serde_json::from_value(receipt(&source)).unwrap(),
        ..custom::tests_shared_delivery::empty_input()
    }).unwrap_err();
    assert_eq!(error.code, "invalid-notice-owner-proof");
    assert!(custom::claims(&store.readers.get(), RECEIPT).unwrap().is_empty());
}

fn assert_authority_unknown(row: &SurfaceDeliveryEvidence, reason: &str) {
    assert!(matches!(&row.evidence,
        DeliveryEvidence::Unknown { reason: ReceiptUnknownReason::AuthorityUnknown(message) }
        if message.contains(reason)), "{row:?}");
}

fn signed_setup() -> (Store, Arc<crate::fleet::MemberKey>, ClaimRecord, fractal_schema::notice_owner_ack::Acknowledgement) {
    use crate::store::fleet_admission_tests_shared_seats::{admit, key, node};
    let owner = key();
    let store = node("example", Some(&owner), Some(&owner));
    admit(&store, "example", &owner, "anchor", None);
    let membership = smallclaims::store::fleet_membership_tx(&store.readers.get()).unwrap();
    let crate::fleet::MemberState::Current(member) = membership.state("example") else {
        panic!("expected admitted owner");
    };
    let intent = crate::parse_intent(&format!(
        "version 2\nnotice-owner \"default\" member=\"host/example\" incarnation=\"{}\" always-on=true\n\
         agent \"example/runtime\" {{ command \"true\"; notice-runtime; }}", member.admitted_claim,
    ), "example").unwrap();
    store.apply_as(&intent, &BTreeMap::new(), "receipt-owner-runtime", Some("person/example")).unwrap();
    let binding = notice_owner::resolve(&store.readers.get()).unwrap();
    let (_, runtime) = notice_authority::binding(&store.readers.get()).unwrap();
    let source = setup_source(&store);
    let acknowledgement = fractal_schema::notice_owner_ack::Acknowledgement {
        owner_binding: binding.declaration_claim, previous_ack: None,
        status: fractal_schema::notice_owner_ack::Status::Active, runtime_binding: Some(runtime),
    };
    (store, owner, source, acknowledgement)
}

fn signed_receipt(source: &ClaimRecord, owner_binding: &str) -> Receipt {
    let mut fields = receipt(source);
    fields.as_object_mut().unwrap().remove("_registration");
    fields["owner_binding"] = json!(owner_binding);
    serde_json::from_value(fields).unwrap()
}

// Bypass only local write policy to test production projection of genuinely sealed
// member-authored history, including writes the supported local API must reject.
fn member_receipt(store: &Store, subject: &str, receipt: &Receipt) -> ClaimRecord {
    let mut fields = serde_json::to_value(receipt).unwrap();
    fields["_registration"] = json!(notice::HASHES[3]);
    let claim = store.connection.batched(|tx| append_claim_tx(tx, &store.origin, subject,
        notice_receipt::KIND, None, &json!({"fields":fields}), &[], None)).unwrap().unwrap();
    store.seal_local_batches().unwrap();
    claim
}

fn signed_envelope(store: &Store, claim: &ClaimRecord) -> smallclaims::ReplicaEnvelope {
    let id = store.readers.get().query_row(
        "SELECT writer,sequence,envelope_hash FROM replica_envelopes WHERE batch_id=?1",
        [&claim.batch_id], |row| Ok(smallclaims::ReplicaEnvelopeId {
            writer: row.get(0)?, sequence: row.get(1)?, hash: row.get(2)?,
        }),
    ).unwrap();
    store.replica_envelopes(vec![id]).unwrap().pop().unwrap()
}

fn assert_signed_by(store: &Store, claim: &ClaimRecord, key: &crate::fleet::MemberKey) {
    use crate::store::fleet_admission_tests_shared_seats::FLEET;
    let envelope = signed_envelope(store, claim);
    assert!(crate::fleet::verify_signature(key.public(),
        &crate::fleet::envelope_signature_message(FLEET, &envelope.writer, envelope.sequence, &envelope.hash),
        envelope.signature.as_deref().unwrap()));
}

#[test]
fn notice_real_owner_receipt_survives_latest_graph_withdrawal() {
    let (store, owner, source, acknowledgement) = signed_setup();
    let active = store.acknowledge_notice_owner(&acknowledgement).unwrap();
    let receipt = signed_receipt(&source, &acknowledgement.owner_binding);
    let claim = store.record_notice_receipt(RECEIPT, &receipt).unwrap();
    assert_signed_by(&store, &active, &owner);
    assert_signed_by(&store, &claim, &owner);
    let before = store.notice_receipts(SOURCE).unwrap();
    assert!(matches!(&before[0].evidence, DeliveryEvidence::Delivered { receipt_claim, .. } if receipt_claim == &claim.id));
    let token = store.selected_desired_token("notice-owner/default").unwrap().unwrap();
    let stop = crate::parse_intent("version 2\nstop \"notice-owner/default\"", "example").unwrap();
    store.apply_as(&stop, &BTreeMap::from([("notice-owner/default".into(), vec![token])]),
        "withdraw-receipt-owner", Some("person/example")).unwrap();
    assert!(notice_owner::resolve(&store.readers.get()).is_err());
    assert_eq!(store.notice_receipts(SOURCE).unwrap(), before);
}

#[test]
fn notice_signed_receipt_without_acknowledgement_is_unknown() {
    let (store, owner, source, acknowledgement) = signed_setup();
    let claim = member_receipt(&store, RECEIPT, &signed_receipt(&source, &acknowledgement.owner_binding));
    assert_signed_by(&store, &claim, &owner);
    assert_authority_unknown(&store.notice_receipts(SOURCE).unwrap()[0], "notice-owner-proof-unavailable");
}

#[test]
fn notice_receipt_signed_by_another_admitted_member_is_unknown() {
    use crate::store::fleet_admission_tests_shared_seats::{admit, key, node, sync};
    let (store, owner, source, acknowledgement) = signed_setup();
    store.acknowledge_notice_owner(&acknowledgement).unwrap();
    let other_key = key();
    admit(&store, "other", &other_key, "sponsor", None);
    let other = node("other", Some(&other_key), Some(&owner));
    // Registration projection precedes source admission; the following pass retries
    // claims only after their real prerequisites have become available.
    for _ in 0..3 { sync(&store, &other); }
    assert!(other.claim_by_id(&source.id).unwrap().is_some());
    let claim = member_receipt(&other, RECEIPT, &signed_receipt(&source, &acknowledgement.owner_binding));
    assert_signed_by(&other, &claim, &other_key);
    assert_authority_unknown(&other.notice_receipts(SOURCE).unwrap()[0], "invalid-notice-owner-proof");
}

#[test]
fn notice_missing_receipt_or_acknowledgement_signature_is_unknown() {
    for missing in ["receipt", "acknowledgement"] {
        let (store, _, source, acknowledgement) = signed_setup();
        let active = store.acknowledge_notice_owner(&acknowledgement).unwrap();
        let receipt = signed_receipt(&source, &acknowledgement.owner_binding);
        let claim = store.record_notice_receipt(RECEIPT, &receipt).unwrap();
        let unsigned = if missing == "receipt" { &claim } else { &active };
        store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-had4e059529fe owner=fractal_store::store::notice_receipts_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures WHERE writer='example' AND sequence=(
             SELECT replica_sequence FROM batches WHERE id=?1)",
            [&unsigned.batch_id],
        ).unwrap();
        assert_authority_unknown(&store.notice_receipts(SOURCE).unwrap()[0], "notice-owner-proof-unavailable");
    }
}

#[test]
fn notice_receipt_authority_uses_position_around_ended_not_completed_at() {
    let (store, owner, source, acknowledgement) = signed_setup();
    let active = store.acknowledge_notice_owner(&acknowledgement).unwrap();
    let mut before = signed_receipt(&source, &acknowledgement.owner_binding);
    before.outcome = Outcome::Delivered {
        completed_at: "2099-01-01T00:00:00Z".into(), evidence: "causally before ended".into(),
    };
    let mut after = before.clone();
    after.surface = Surface::Speech;
    after.outcome = Outcome::Delivered {
        completed_at: "2000-01-01T00:00:00Z".into(), evidence: "backdated after ended".into(),
    };
    let ended = fractal_schema::notice_owner_ack::Acknowledgement {
        previous_ack: Some(active.id), status: fractal_schema::notice_owner_ack::Status::Ended,
        runtime_binding: None, ..acknowledgement
    };
    let (before_claim, ended_claim, after_claim) = store.connection.batched(|tx| -> Result<_> {
        let mut fields = serde_json::to_value(&before)?;
        fields["_registration"] = json!(notice::HASHES[3]);
        let before = append_claim_tx(tx, &store.origin, RECEIPT, notice_receipt::KIND,
            None, &json!({"fields":fields}), &[], None)?;
        let ended = append_claim_tx(tx, &store.origin, "notice-owner/default",
            fractal_schema::notice_owner_ack::KIND, None, &json!({"fields":ended}), &[], Some(&before.batch_id))?;
        let mut fields = serde_json::to_value(&after)?;
        fields["_registration"] = json!(notice::HASHES[3]);
        let after = append_claim_tx(tx, &store.origin, &RECEIPT.replace("000003", "000005"),
            notice_receipt::KIND, None, &json!({"fields":fields}), &[], Some(&before.batch_id))?;
        Ok((before, ended, after))
    }).unwrap().unwrap();
    store.seal_local_batches().unwrap();
    for claim in [&before_claim, &ended_claim, &after_claim] { assert_signed_by(&store, claim, &owner); }
    assert_eq!(before_claim.batch_id, after_claim.batch_id);
    let rows = store.notice_receipts(SOURCE).unwrap();
    assert!(matches!(&rows[0].evidence, DeliveryEvidence::Delivered { completed_at, .. }
        if completed_at == "2099-01-01T00:00:00Z"));
    assert_authority_unknown(&rows[1], "invalid-notice-owner-proof");
}

#[test]
fn notice_signed_replication_receipt_retries_missing_prerequisites_and_survives_replay() {
    use crate::store::fleet_admission_tests_shared_seats::{node, FLEET};
    let (store, owner, source, acknowledgement) = signed_setup();
    store.acknowledge_notice_owner(&acknowledgement).unwrap();
    let claim = store.record_notice_receipt(RECEIPT,
        &signed_receipt(&source, &acknowledgement.owner_binding)).unwrap();
    let reader = node("reader", None, Some(&owner));
    let exchange = store.export_replication_exchange(FLEET, &reader.replication_inventory().unwrap()).unwrap();
    let mut early = exchange.clone();
    early.envelopes = vec![signed_envelope(&store, &claim)];
    reader.receive_replication_exchange("example", FLEET, &early).unwrap();
    reader.validate_replication_backlog().unwrap();
    reader.project_replication_backlog().unwrap();
    assert!(reader.claim_by_id(&claim.id).unwrap().is_none());
    assert!(reader.notice_receipts(SOURCE).unwrap().iter()
        .all(|row| matches!(row.evidence, DeliveryEvidence::Unknown { .. })));
    reader.receive_replication_exchange("example", FLEET, &exchange).unwrap();
    // Real receive/classify/project retries, not authorizer injection. Registration,
    // source, and acknowledgement dependencies become admitted in causal stages.
    for _ in 0..6 {
        reader.validate_replication_backlog().unwrap();
        reader.project_replication_backlog().unwrap();
        if reader.claim_by_id(&claim.id).unwrap().is_some() { break; }
    }
    assert!(reader.claim_by_id(&claim.id).unwrap().is_some());
    let expected = store.notice_receipts(SOURCE).unwrap();
    assert!(matches!(&expected[0].evidence, DeliveryEvidence::Delivered { .. }));
    assert_eq!(reader.notice_receipts(SOURCE).unwrap(), expected);
    reader.replay_replication_graph().unwrap();
    assert_eq!(reader.notice_receipts(SOURCE).unwrap(), expected);
}

#[test]
fn notice_signed_forked_acknowledgement_chain_is_unknown() {
    let (store, owner, source, acknowledgement) = signed_setup();
    store.acknowledge_notice_owner(&acknowledgement).unwrap();
    store.record_notice_receipt(RECEIPT,
        &signed_receipt(&source, &acknowledgement.owner_binding)).unwrap();
    let fork = store.connection.batched(|tx| append_claim_tx(tx, &store.origin, "notice-owner/default",
        fractal_schema::notice_owner_ack::KIND, None, &json!({"fields":acknowledgement}), &[], None))
        .unwrap().unwrap();
    store.seal_local_batches().unwrap();
    assert_signed_by(&store, &fork, &owner);
    assert_authority_unknown(&store.notice_receipts(SOURCE).unwrap()[0], "forked acknowledgement chain");
}
