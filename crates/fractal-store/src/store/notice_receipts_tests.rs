use super::*;

const SOURCE: &str = "custom/attention/source/01900000-0000-7000-8000-000000000002";
const RECEIPT: &str = "custom/attention/receipt/01900000-0000-7000-8000-000000000003";

// Test-only authority decision. Production deliberately has no implementation.
struct ReviewedFixture;
impl ReceiptAuthorizer for ReviewedFixture {
    fn assess(&self, _: &ClaimRecord, _: &Receipt) -> ReceiptAuthorization { ReceiptAuthorization::Authorized }
}

fn setup() -> (Store, ClaimRecord) {
    let store = Store::open_memory("example").unwrap();
    for manifest in notice::manifests() {
        store.register_custom_kind(&custom::RegistrationRequest { manifest: manifest.clone(), actor: "person/example".into() }).unwrap();
    }
    let fixture: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h881725e371d9 owner=fractal_store::store::notice_receipts_tests TODO=trace-statement-and-value-representation */ include_str!("../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
    let source = store.append_claim(&ClaimInput {
        subject: SOURCE.into(), kind: "custom.attention.notice-created".into(), actor: Some("agent/example/producer".into()),
        fields: serde_json::from_value(fixture["snapshot"]["input"].clone()).unwrap(),
        ..custom::tests_shared_delivery::empty_input()
    }).unwrap();
    (store, source)
}

fn receipt(source: &ClaimRecord) -> Value {
    json!({"source_id":SOURCE,"creation_claim":source.id,"producer":"agent/example/producer","recipient":"person/example",
        "owner_binding":"a".repeat(64),"surface":"tray","outcome":{"state":"delivered","completed_at":"2026-11-07T12:00:00Z","evidence":"surface acknowledged"},
        "_registration":notice::HASHES[3]})
}

// Model durable admitted facts supplied by a reviewed future authority, bypassing the
// intentionally unavailable write-admission seam, not substituting snapshot state.
fn durable(store: &Store, subject: &str, kind: &str, fields: Value) {
    store.connection.batched(|tx| append_claim_tx(tx, &store.origin, subject, kind,
        Some("person/example"), &json!({"fields":fields}), &[], None)).unwrap().unwrap();
}
fn approved(store: &Store) -> Vec<SurfaceDeliveryEvidence> {
    project(&store.readers.get(), SOURCE, Some(&ReviewedFixture)).unwrap()
}
fn assert_unknown(rows: &[SurfaceDeliveryEvidence], reason: ReceiptUnknownReason) {
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.evidence == unknown(reason.clone())));
}

#[test]
fn notice_receipts_without_authorizer_or_runtime_are_unknown() {
    let (store, source) = setup();
    durable(&store, RECEIPT, notice_receipt::KIND, receipt(&source));
    assert_unknown(&store.notice_receipts(SOURCE).unwrap(), ReceiptUnknownReason::AuthorityUnavailable);
}

#[test]
fn notice_snapshot_delivery_cannot_invent_receipt_evidence() {
    let (store, source) = setup();
    durable(&store, "custom/attention/snapshot/01900000-0000-7000-8000-000000000004", "custom.attention.snapshot-created",
        json!({"source_id":SOURCE,"creation_claim":source.id,"delivery":[{"surface":"tray","state":"delivered"}]}));
    assert_unknown(&approved(&store), ReceiptUnknownReason::MissingEvidence);
    assert_unknown(&store.notice_receipts(SOURCE).unwrap(), ReceiptUnknownReason::AuthorityUnavailable);
}

#[test]
fn notice_approved_receipts_survive_runtime_handover() {
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
fn notice_receipt_local_admission_has_no_implicit_runtime_authorizer() {
    let (store, source) = setup();
    let error = store.append_claim(&ClaimInput {
        subject: RECEIPT.into(), kind: notice_receipt::KIND.into(), actor: Some("person/example".into()),
        fields: serde_json::from_value(receipt(&source)).unwrap(),
        ..custom::tests_shared_delivery::empty_input()
    }).unwrap_err();
    assert!(error.message.contains("durable receipt owner authority is not configured"));
    assert!(custom::claims(&store.readers.get(), RECEIPT).unwrap().is_empty());
}
