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
