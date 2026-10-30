use super::*;
use crate::claim::ReplicaBatch;
use crate::fleet::MemberKey;
use crate::hash::{batch_header_hash, claim_hash};
use crate::principal::{ClaimSignature, content_digest};
use serde_json::json;

pub(crate) const FLEET: &str = "proof-example";

/// Real keys and the same batch/hash/signature constructors used by the store.
pub(crate) fn sealed(
    key: &MemberKey, sequence: u64, previous: Option<String>, entries: Vec<(&str, &str, Value)>,
) -> (ReplicaEnvelope, Vec<String>, String) {
    let hash = batch_header_hash("example", sequence, previous.as_deref(), u128::from(sequence)).unwrap();
    let batch_id = format!("batch/example/{sequence}/{hash}");
    let mut claims = Vec::new();
    let mut signatures = BTreeMap::new();
    for (subject, kind, body) in entries {
        let id = claim_hash(&batch_id, subject, kind, "example", Some("person/operator"), &body, &[]).unwrap();
        signatures.insert(id.clone(), ClaimSignature::sign(key,
            &content_digest(subject, kind, Some("person/operator"), &body), "host/example", None, vec![], sequence));
        claims.push(ClaimRecord { id, store_index: claims.len() as u64 + 1, batch_id: batch_id.clone(),
            subject: subject.into(), kind: kind.into(), origin: "example".into(), actor: Some("person/operator".into()),
            operation_id: None, request_digest: None, body, predecessors: vec![], accepted_at_unix_ms: u128::from(sequence) });
    }
    let ids = claims.iter().map(|claim| claim.id.clone()).collect();
    let payload = ReplicaEnvelopePayload { batch: ReplicaBatch { id: batch_id, origin: "example".into(),
        replica_sequence: sequence, previous_hash: previous.clone(), hash: hash.clone(),
        accepted_at_unix_ms: u128::from(sequence), claims }, blobs: BTreeMap::new(), claim_signatures: signatures };
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes).unwrap();
    let envelope_hash = replica_envelope_hash("example", sequence, previous.as_deref(), u128::from(sequence), &bytes);
    let signature = key.sign(&envelope_signature_message(FLEET, "example", sequence, &envelope_hash));
    (ReplicaEnvelope { writer: "example".into(), sequence, previous_hash: previous, hash: envelope_hash,
        accepted_at_unix_ms: u128::from(sequence), payload: bytes.into(), member_key: Some(key.public().into()), signature: Some(signature) }, ids, hash)
}

pub(crate) fn anchor(key: &MemberKey) -> (ReplicaEnvelope, String, String) {
    let (envelope, ids, hash) = sealed(key, 1, None, vec![("host/example", "fleet.member-admitted",
        json!({"fields":{"member_key":key.public(),"via":"anchor","writer_floor":0}}))]);
    (envelope, ids[0].clone(), hash)
}

#[test]
fn real_signed_anchor_history_is_independently_usable() {
    let (key, _) = MemberKey::generate().unwrap();
    let (envelope, id, _) = anchor(&key);
    let history = verify_sealed_history(&[envelope], FLEET, key.public()).unwrap();
    assert_eq!(history.claim(&id).unwrap().kind, "fleet.member-admitted");
}

#[test]
fn caller_anchor_and_exact_payload_are_required() {
    let (key, _) = MemberKey::generate().unwrap();
    let (mut envelope, _, _) = anchor(&key);
    assert!(matches!(verify_sealed_history(&[envelope.clone()], FLEET, "untrusted"), Err(error) if error.code == "untrusted-membership"));
    envelope.payload = vec![0].into();
    assert!(matches!(verify_sealed_history(&[envelope], FLEET, key.public()), Err(error) if error.code == "invalid-signature"));
}

#[test]
fn real_batch_hash_links_and_sequence_omissions_are_checked() {
    let (key, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&key);
    let (second, _, second_hash) = sealed(&key, 2, Some(hash), vec![]);
    let (third, _, _) = sealed(&key, 3, Some(second_hash), vec![]);
    assert!(verify_sealed_history(&[first.clone(), second, third.clone()], FLEET, key.public()).is_ok());
    assert!(matches!(verify_sealed_history(&[first, third], FLEET, key.public()), Err(error) if error.code == "broken-chain"));
}
