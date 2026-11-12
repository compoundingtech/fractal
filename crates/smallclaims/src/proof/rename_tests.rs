use super::*;
use super::super::history::tests::{FLEET, anchor, sealed};
use crate::fleet::MemberKey;
use serde_json::json;

fn fixture() -> (RenameProofBundle, MemberKey) {
    let (key, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&key);
    let (second, declaration, hash) = sealed(&key, 2, Some(hash), vec![("mission/example", "mission.published", json!({"fields":{}}))]);
    let request = json!({"from":"mission/example","to":"mission/renamed","actor":"person/operator","reason":"Clarify purpose","idempotency_key":"rename-example"});
    let digest = crate::hash::canonical_hash(&("st3.identity-rename-request.v1", "mission.renamed", crate::hash::canonical_json_value(&request))).unwrap();
    let body = json!({"fields":{"from":"mission/example","to":"mission/renamed","reason":"Clarify purpose",
        "owner_host":"example","declaration":declaration[0],"request":request},
        "_operation":{"id":crate::store::operation_id_for_key("rename-example"),"request_digest":digest}});
    let (third, revision, _) = sealed(&key, 3, Some(hash), vec![("mission/example", "mission.renamed", body)]);
    (RenameProofBundle { schema: BUNDLE_SCHEMA.into(), fleet_id: FLEET.into(), fleet_anchor: key.public().into(),
        disclosure: "fleet-internal".into(),
        identity_kind: "mission".into(), storage_id: "mission/example".into(), canonical_id: "mission/renamed".into(),
        naming_revision: revision[0].clone(), declaration: declaration[0].clone(), chain: revision,
        envelopes: vec![first, second, third] }, key)
}

#[test]
fn verifies_real_signed_naming_history_offline() {
    let (bundle, key) = fixture();
    let verdict = verify_rename_proof(&bundle, key.public(), None).unwrap();
    assert!(verdict.ok);
    assert_eq!(verdict.storage_identity, "mission/example");
    assert_eq!(verdict.to, "mission/renamed");
    assert_eq!(verdict.chain[0].revision, bundle.naming_revision);
    assert_eq!(verdict.signer_key, key.public());
}

#[test]
fn duplicated_metadata_and_revision_progress_fail_closed() {
    let (mut bundle, key) = fixture();
    for token in [bundle.naming_revision.as_str(), "absent"] {
        assert_eq!(verify_rename_proof(&bundle, key.public(), Some(token)).unwrap_err().code, "stale-revision");
    }
    bundle.canonical_id = "mission/forged".into();
    assert_eq!(verify_rename_proof(&bundle, key.public(), None).unwrap_err().code, "malformed-bundle");
}

#[test]
fn schema_and_informational_anchor_cannot_override_caller() {
    let (mut bundle, key) = fixture();
    bundle.fleet_anchor = "attacker".into();
    assert_eq!(verify_rename_proof(&bundle, key.public(), None).unwrap_err().code, "untrusted-membership");
    bundle.schema = "unsupported".into();
    assert_eq!(verify_rename_proof(&bundle, key.public(), None).unwrap_err().code, "malformed-bundle");
}

fn append(bundle: &mut RenameProofBundle, key: &MemberKey, entries: Vec<(&str, &str, Value)>) -> Vec<String> {
    let last = bundle.envelopes.last().unwrap();
    let payload: crate::claim::ReplicaEnvelopePayload = ciborium::from_reader(last.payload.bytes().unwrap()).unwrap();
    let (envelope, ids, _) = sealed(key, last.sequence + 1, Some(payload.batch.hash), entries);
    bundle.envelopes.push(envelope);
    ids
}

fn rename_body(identity: &str, from: &str, to: &str, declaration: &str, previous: Option<&str>, key: &str) -> Value {
    let request = json!({"from":from,"to":to,"actor":"person/operator","reason":"Clarify purpose","idempotency_key":key});
    let digest = crate::hash::canonical_hash(&("st3.identity-rename-request.v1", "mission.renamed", crate::hash::canonical_json_value(&request))).unwrap();
    assert!(identity.starts_with("mission/"));
    let mut fields = json!({"from":from,"to":to,"reason":"Clarify purpose","owner_host":"example","declaration":declaration,"request":request});
    if let Some(previous) = previous { fields["previous_revision"] = json!(previous); }
    json!({"fields":fields,"_operation":{"id":crate::store::operation_id_for_key(key),"request_digest":digest}})
}

fn select(bundle: &mut RenameProofBundle, revision: &str, target: &str, chain: Vec<String>) {
    bundle.naming_revision = revision.into();
    bundle.canonical_id = target.into();
    bundle.chain = chain;
}

#[test]
fn initial_sibling_branches_reject_either_signed_head() {
    let (mut bundle, key) = fixture();
    let original = bundle.naming_revision.clone();
    let body = rename_body("mission/example", "mission/example", "mission/sibling", &bundle.declaration, None, "sibling");
    let sibling = append(&mut bundle, &key, vec![("mission/example", "mission.renamed", body)])[0].clone();
    assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
    select(&mut bundle, &sibling, "mission/sibling", vec![sibling.clone()]);
    assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
    assert_ne!(original, sibling);
}

#[test]
fn child_sibling_branches_reject_either_signed_head() {
    let (mut bundle, key) = fixture();
    let parent = bundle.naming_revision.clone();
    let left = rename_body("mission/example", "mission/renamed", "mission/left", &bundle.declaration, Some(&parent), "left");
    let right = rename_body("mission/example", "mission/renamed", "mission/right", &bundle.declaration, Some(&parent), "right");
    let mut children = append(&mut bundle, &key, vec![("mission/example", "mission.renamed", left)]);
    children.extend(append(&mut bundle, &key, vec![("mission/example", "mission.renamed", right)]));
    for (revision, target) in children.iter().zip(["mission/left", "mission/right"]) {
        select(&mut bundle, revision, target, vec![parent.clone(), revision.clone()]);
        assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
    }
}

#[test]
fn authenticated_storage_declarations_reserve_their_names() {
    let (mut bundle, key) = fixture();
    append(&mut bundle, &key, vec![("mission/renamed", "mission.published", json!({"fields":{}}))]);
    assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
}

#[test]
fn smallest_claim_wins_cross_identity_target_and_fences_loser_descendants() {
    let (mut bundle, key) = fixture();
    let first = bundle.naming_revision.clone();
    let first_declaration = bundle.declaration.clone();
    let declaration = append(&mut bundle, &key, vec![("mission/other", "mission.published", json!({"fields":{}}))])[0].clone();
    let body = rename_body("mission/other", "mission/other", "mission/renamed", &declaration, None, "other");
    let other = append(&mut bundle, &key, vec![("mission/other", "mission.renamed", body)])[0].clone();
    let first_wins = first < other;
    assert_eq!(verify_rename_proof(&bundle, key.public(), None).is_ok(), first_wins);
    bundle.storage_id = "mission/other".into();
    bundle.declaration = declaration;
    select(&mut bundle, &other, "mission/renamed", vec![other.clone()]);
    assert_eq!(verify_rename_proof(&bundle, key.public(), None).is_ok(), !first_wins);
    let loser = if first_wins { other } else {
        bundle.storage_id = "mission/example".into();
        bundle.declaration = first_declaration;
        first
    };
    let identity = bundle.storage_id.clone();
    let child = rename_body(&identity, "mission/renamed", "mission/descendant", &bundle.declaration, Some(&loser), "descendant");
    let child = append(&mut bundle, &key, vec![(&identity, "mission.renamed", child)])[0].clone();
    select(&mut bundle, &child, "mission/descendant", vec![loser, child.clone()]);
    assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
}

#[test]
fn unrelated_signed_conflict_does_not_invalidate_independent_proof() {
    let (mut bundle, key) = fixture();
    let declaration = append(&mut bundle, &key, vec![("mission/other", "mission.published", json!({"fields":{}}))])[0].clone();
    let left = rename_body("mission/other", "mission/other", "mission/left", &declaration, None, "left");
    let right = rename_body("mission/other", "mission/other", "mission/right", &declaration, None, "right");
    append(&mut bundle, &key, vec![("mission/other", "mission.renamed", left)]);
    append(&mut bundle, &key, vec![("mission/other", "mission.renamed", right)]);
    assert!(verify_rename_proof(&bundle, key.public(), None).is_ok());
}

#[test]
fn historical_ancestor_remains_valid_after_nonconflicting_descendant() {
    let (mut bundle, key) = fixture();
    let parent = bundle.naming_revision.clone();
    let body = rename_body("mission/example", "mission/renamed", "mission/descendant", &bundle.declaration, Some(&parent), "descendant");
    let child = append(&mut bundle, &key, vec![("mission/example", "mission.renamed", body)])[0].clone();
    assert!(verify_rename_proof(&bundle, key.public(), None).is_ok());
    select(&mut bundle, &child, "mission/descendant", vec![parent.clone(), child.clone()]);
    assert!(verify_rename_proof(&bundle, key.public(), Some(&parent)).is_ok());
}

#[test]
fn unsigned_competing_fact_cannot_fence_authenticated_identity() {
    let (mut bundle, key) = fixture();
    let body = rename_body("mission/example", "mission/example", "mission/sibling", &bundle.declaration, None, "unsigned");
    append(&mut bundle, &key, vec![("mission/example", "mission.renamed", body)]);
    let envelope = bundle.envelopes.last_mut().unwrap();
    let mut payload: crate::claim::ReplicaEnvelopePayload = ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
    payload.claim_signatures.clear();
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes).unwrap();
    envelope.hash = crate::hash::replica_envelope_hash(&envelope.writer, envelope.sequence, envelope.previous_hash.as_deref(), envelope.accepted_at_unix_ms, &bytes);
    envelope.signature = Some(key.sign(&crate::fleet::envelope_signature_message(FLEET, &envelope.writer, envelope.sequence, &envelope.hash)));
    envelope.payload = bytes.into();
    assert!(verify_rename_proof(&bundle, key.public(), None).is_ok());
}

#[test]
fn disclosure_classification_is_required_and_preserved() {
    let (mut bundle, key) = fixture();
    assert_eq!(verify_rename_proof(&bundle, key.public(), None).unwrap().disclosure, "fleet-internal");
    for classification in ["", "public", "private"] {
        bundle.disclosure = classification.into();
        assert_eq!(verify_rename_proof(&bundle, key.public(), None).unwrap_err().code, "malformed-bundle");
    }
}

#[test]
fn authenticated_operation_digest_conflict_fences_rename() {
    let (mut bundle, key) = fixture();
    let operation = crate::store::operation_id_for_key("rename-example");
    append(&mut bundle, &key, vec![("mission/operation-competitor", "mission.published",
        json!({"fields":{},"_operation":{"id":operation,"request_digest":"different"}}))]);
    assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
}

#[test]
fn authenticated_repair_excludes_replaced_naming_fact() {
    let (mut bundle, key) = fixture();
    let original = bundle.envelopes.last().unwrap();
    let reference = crate::hash::replica_record_ref(&original.writer, original.sequence, &original.hash, 0);
    let replacement = append(&mut bundle, &key, vec![("mission/replacement", "mission.published", json!({"fields":{}}))])[0].clone();
    append(&mut bundle, &key, vec![("repair/example", "record.repaired",
        json!({"fields":{"record":reference,"replacement":replacement}}))]);
    assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
}

#[test]
fn signed_malformed_sibling_cannot_fence_valid_rename_or_its_operation() {
    for key_name in ["malformed-sibling", "rename-example"] {
        let (mut bundle, key) = fixture();
        let mut body = rename_body("mission/example", "mission/example", "mission/requested", &bundle.declaration, None, key_name);
        body["fields"]["to"] = json!("mission/different");
        let sibling = append(&mut bundle, &key, vec![("mission/example", "mission.renamed", body)])[0].clone();
        let history = verify_sealed_history(&bundle.envelopes, FLEET, key.public()).unwrap();
        history.verify_claim(&sibling).unwrap();
        assert!(!crate::naming::admits_replicated(history.claim(&sibling).unwrap()).unwrap());
        assert!(verify_rename_proof(&bundle, key.public(), None).is_ok());
        select(&mut bundle, &sibling, "mission/different", vec![sibling.clone()]);
        assert!(verify_rename_proof(&bundle, key.public(), None).is_err());
    }
}
