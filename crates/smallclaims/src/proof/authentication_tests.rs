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
