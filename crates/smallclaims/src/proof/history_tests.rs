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
    sealed_writer(key, "example", sequence, previous, entries)
}

fn sealed_writer(
    key: &MemberKey, writer: &str, sequence: u64, previous: Option<String>,
    entries: Vec<(&str, &str, Value)>,
) -> (ReplicaEnvelope, Vec<String>, String) {
    let hash = batch_header_hash(writer, sequence, previous.as_deref(), u128::from(sequence)).unwrap();
    let batch_id = format!("batch/{writer}/{sequence}/{hash}");
    let mut claims = Vec::new();
    let mut signatures = BTreeMap::new();
    for (subject, kind, body) in entries {
        let id = claim_hash(&batch_id, subject, kind, writer, Some("person/operator"), &body, &[]).unwrap();
        signatures.insert(id.clone(), ClaimSignature::sign(key,
            &content_digest(subject, kind, Some("person/operator"), &body), &format!("host/{writer}"), None, vec![], sequence));
        claims.push(ClaimRecord { id, store_index: claims.len() as u64 + 1, batch_id: batch_id.clone(),
            subject: subject.into(), kind: kind.into(), origin: writer.into(), actor: Some("person/operator".into()),
            operation_id: None, request_digest: None, body, predecessors: vec![], accepted_at_unix_ms: u128::from(sequence) });
    }
    let ids = claims.iter().map(|claim| claim.id.clone()).collect();
    let payload = ReplicaEnvelopePayload { batch: ReplicaBatch { id: batch_id, origin: writer.into(),
        replica_sequence: sequence, previous_hash: previous.clone(), hash: hash.clone(),
        accepted_at_unix_ms: u128::from(sequence), claims }, blobs: BTreeMap::new(), claim_signatures: signatures };
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes).unwrap();
    let envelope_hash = replica_envelope_hash(writer, sequence, previous.as_deref(), u128::from(sequence), &bytes);
    let signature = key.sign(&envelope_signature_message(FLEET, writer, sequence, &envelope_hash));
    (ReplicaEnvelope { writer: writer.into(), sequence, previous_hash: previous, hash: envelope_hash,
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

struct Rejoin {
    anchor: MemberKey,
    old: MemberKey,
    fresh: MemberKey,
    history: Vec<ReplicaEnvelope>,
    old_hash: String,
    fresh_hash: String,
}

fn rejoin() -> Rejoin {
    let (anchor_key, _) = MemberKey::generate().unwrap();
    let (old, _) = MemberKey::generate().unwrap();
    let (fresh, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&anchor_key);
    let (admit, _, hash) = sealed(&anchor_key, 2, Some(hash), vec![("host/rejoined", "fleet.member-admitted",
        json!({"fields":{"member_key":old.public(),"via":"sponsor","writer_floor":0}}))]);
    let (remove, _, hash) = sealed(&anchor_key, 3, Some(hash), vec![("host/rejoined", "fleet.member-removed",
        json!({"fields":{"member_key":old.public(),"high_water":2}}))]);
    let (readmit, _, _) = sealed(&anchor_key, 4, Some(hash), vec![("host/rejoined", "fleet.member-admitted",
        json!({"fields":{"member_key":fresh.public(),"via":"sponsor","writer_floor":2}}))]);
    let (old_first, _, hash) = sealed_writer(&old, "rejoined", 1, None, vec![]);
    let (old_second, _, old_hash) = sealed_writer(&old, "rejoined", 2, Some(hash), vec![]);
    let (fresh_first, _, fresh_hash) = sealed_writer(&fresh, "rejoined", 3, None,
        vec![("person/operator", "test.claim", json!({"fields":{}}))]);
    Rejoin { anchor: anchor_key, old, fresh,
        history: vec![first, admit, remove, readmit, old_first, old_second, fresh_first],
        old_hash, fresh_hash }
}

#[test]
fn authenticated_fresh_incarnation_floor_without_predecessor_is_valid() {
    let fixture = rejoin();
    assert!(verify_sealed_history(&fixture.history, FLEET, fixture.anchor.public()).is_ok());
    // A rejoin which retained its old batch ledger can also link across the boundary.
    let mut linked = fixture.history.clone();
    linked[6] = sealed_writer(&fixture.fresh, "rejoined", 3, Some(fixture.old_hash), vec![]).0;
    assert!(verify_sealed_history(&linked, FLEET, fixture.anchor.public()).is_ok());
}

#[test]
fn authenticated_floor_does_not_admit_forged_floor_or_interior_gaps() {
    let fixture = rejoin();
    for sequence in [2, 4] {
        let mut forged = fixture.history.clone();
        forged[6] = sealed_writer(&fixture.fresh, "rejoined", sequence, None, vec![]).0;
        assert!(verify_sealed_history(&forged, FLEET, fixture.anchor.public()).is_err());
    }
    let mut old_gap = fixture.history.clone();
    old_gap.remove(4);
    assert!(matches!(verify_sealed_history(&old_gap, FLEET, fixture.anchor.public()),
        Err(error) if error.code == "broken-chain"));
    let mut fresh_gap = fixture.history.clone();
    fresh_gap.push(sealed_writer(&fixture.fresh, "rejoined", 5, Some(fixture.fresh_hash), vec![]).0);
    assert!(matches!(verify_sealed_history(&fresh_gap, FLEET, fixture.anchor.public()),
        Err(error) if error.code == "broken-chain"));
}

#[test]
fn wrong_predecessor_and_removed_key_beyond_high_water_are_rejected() {
    let fixture = rejoin();
    let mut wrong = fixture.history.clone();
    wrong.push(sealed_writer(&fixture.fresh, "rejoined", 4, Some("wrong".into()), vec![]).0);
    assert!(matches!(verify_sealed_history(&wrong, FLEET, fixture.anchor.public()),
        Err(error) if error.code == "broken-chain"));
    let mut removed = fixture.history;
    removed[6] = sealed_writer(&fixture.old, "rejoined", 3, Some(fixture.old_hash), vec![]).0;
    assert!(matches!(verify_sealed_history(&removed, FLEET, fixture.anchor.public()),
        Err(error) if error.code == "untrusted-membership"));
}
