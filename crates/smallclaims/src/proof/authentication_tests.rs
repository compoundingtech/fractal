use super::*;
use super::super::history::tests::{FLEET, anchor, sealed};
use super::super::verify_sealed_history;
use crate::fleet::MemberKey;
use serde_json::json;

#[test]
fn anchored_principal_signature_is_independently_usable() {
    let (key, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&key);
    let (second, ids, _) = sealed(&key, 2, Some(hash), vec![("mission/example", "mission.published", json!({"fields":{}}))]);
    let mut history = verify_sealed_history(&[first, second], FLEET, key.public()).unwrap();
    history.verify_claim(&ids[0]).unwrap();
    assert!(history.verify_actor_alias("agent/example", "agent/example").unwrap());
    assert!(!history.verify_actor_alias("agent/unrelated", "agent/example").unwrap());
    history.records.get_mut(&ids[0]).unwrap().signatures[0].signature = "invalid".into();
    assert_eq!(history.verify_claim(&ids[0]).unwrap_err().code, "invalid-signature");
}

#[test]
fn naming_subjects_use_the_runtime_subject_grammar() {
    assert!(valid_subject("agent/example/worker", "agent"));
    assert!(valid_subject("mission/example", "mission"));
    for invalid in ["agent/", "agent//worker", "agent/example/", "agent/white space", "mission/example"] {
        assert!(!valid_subject(invalid, "agent"), "{invalid}");
    }
    assert!(!valid_subject(&format!("agent/{}", "x".repeat(512)), "agent"));
}

#[test]
fn verified_envelopes_do_not_bypass_naming_schema() {
    let (key, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&key);
    let (second, _, _) = sealed(&key, 2, Some(hash), vec![("mission/example", "mission.renamed", json!({"fields":{}}))]);
    let history = verify_sealed_history(&[first, second], FLEET, key.public()).unwrap();
    assert_eq!(history.validate_rename_claims().unwrap_err().code, "malformed-bundle");
}

fn replace_signature(
    envelope: &mut crate::claim::ReplicaEnvelope,
    id: &str,
    signature: principal::ClaimSignature,
    carrier: &MemberKey,
) {
    let mut payload: crate::claim::ReplicaEnvelopePayload =
        ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
    payload.claim_signatures.insert(id.into(), signature);
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes).unwrap();
    envelope.hash = crate::hash::replica_envelope_hash(
        &envelope.writer, envelope.sequence, envelope.previous_hash.as_deref(),
        envelope.accepted_at_unix_ms, &bytes,
    );
    envelope.signature = Some(carrier.sign(&crate::fleet::envelope_signature_message(
        FLEET, &envelope.writer, envelope.sequence, &envelope.hash,
    )));
    envelope.payload = bytes.into();
}

#[test]
fn ended_member_cannot_sign_a_new_claim_carried_by_an_active_member() {
    let (carrier, _) = MemberKey::generate().unwrap();
    let (retired, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&carrier);
    let (second, _, hash) = sealed(&carrier, 2, Some(hash), vec![
        ("host/retired", "fleet.member-admitted", json!({"fields":{
            "member_key":retired.public(),"via":"sponsor","writer_floor":0
        }})),
    ]);
    let (third, _, hash) = sealed(&carrier, 3, Some(hash), vec![
        ("host/retired", "fleet.member-removed", json!({"fields":{
            "member_key":retired.public(),"high_water":1
        }})),
    ]);
    let body = json!({"fields":{}});
    let (mut fourth, ids, _) = sealed(&carrier, 4, Some(hash), vec![
        ("mission/example", "mission.published", body.clone()),
    ]);
    let signature = principal::ClaimSignature::sign(
        &retired, &principal::content_digest(
            "mission/example", "mission.published", Some("person/operator"), &body,
        ), "host/retired", None, vec![], 0,
    );
    replace_signature(&mut fourth, &ids[0], signature, &carrier);
    let history = verify_sealed_history(&[first, second, third, fourth], FLEET, carrier.public()).unwrap();
    assert_eq!(history.verify_claim(&ids[0]).unwrap_err().code, "untrusted-membership");
}

#[test]
fn ended_member_keeps_its_historical_in_window_signatures() {
    let (key, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&key);
    let (second, ids, hash) = sealed(&key, 2, Some(hash), vec![
        ("mission/example", "mission.published", json!({"fields":{}})),
    ]);
    let (third, _, _) = sealed(&key, 3, Some(hash), vec![
        ("host/example", "fleet.member-left", json!({"fields":{
            "member_key":key.public(),"high_water":3
        }})),
    ]);
    let history = verify_sealed_history(&[first, second, third], FLEET, key.public()).unwrap();
    history.verify_claim(&ids[0]).unwrap();
}

#[test]
fn only_genuine_earlier_signatures_consume_a_nonce() {
    for genuine_replay in [false, true] {
        let (key, _) = MemberKey::generate().unwrap();
        let (first, _, hash) = anchor(&key);
        let body = json!({"fields":{"value":"legitimate"}});
        let earlier = if genuine_replay { body.clone() } else { json!({"fields":{"value":"different"}}) };
        let (mut second, earlier_ids, hash) = sealed(&key, 2, Some(hash), vec![
            ("mission/example", "mission.published", earlier),
        ]);
        let (third, ids, _) = sealed(&key, 3, Some(hash), vec![
            ("mission/example", "mission.published", body),
        ]);
        let payload: crate::claim::ReplicaEnvelopePayload =
            ciborium::from_reader(third.payload.bytes().unwrap()).unwrap();
        replace_signature(&mut second, &earlier_ids[0], payload.claim_signatures[&ids[0]].clone(), &key);
        let history = verify_sealed_history(&[first, second, third], FLEET, key.public()).unwrap();
        if genuine_replay {
            assert_eq!(history.verify_claim(&ids[0]).unwrap_err().code, "invalid-signature");
        } else {
            assert_eq!(history.verify_claim(&earlier_ids[0]).unwrap_err().code, "invalid-signature");
            history.verify_claim(&ids[0]).unwrap();
        }
    }
}

#[test]
fn lower_id_unauthenticated_candidate_does_not_hide_a_valid_actor_alias() {
    let (key, _) = MemberKey::generate().unwrap();
    let (first, _, hash) = anchor(&key);
    let (second, declarations, hash) = sealed(&key, 2, Some(hash), vec![
        ("agent/source", "intent.desired", json!({"kind":"agent","member":{"host":"example"}})),
    ]);
    let rename = |reason| json!({"fields":{
        "from":"agent/source","to":"agent/alias","owner_host":"example",
        "declaration":declarations[0],"reason":reason
    }});
    let (mut third, mut candidates, _) = sealed(&key, 3, Some(hash), vec![
        ("agent/source", "agent.renamed", rename("first candidate")),
        ("agent/source", "agent.renamed", rename("second candidate")),
    ]);
    candidates.sort();
    assert!(candidates[0] < candidates[1]);
    let payload: crate::claim::ReplicaEnvelopePayload =
        ciborium::from_reader(third.payload.bytes().unwrap()).unwrap();
    let mut invalid = payload.claim_signatures[&candidates[0]].clone();
    invalid.signature = "invalid".into();
    replace_signature(&mut third, &candidates[0], invalid, &key);
    let history = verify_sealed_history(&[first, second, third], FLEET, key.public()).unwrap();
    assert_eq!(history.verify_claim(&candidates[0]).unwrap_err().code, "invalid-signature");
    history.verify_claim(&candidates[1]).unwrap();
    assert!(history.verify_actor_alias("agent/alias", "agent/source").unwrap());
}
