#![allow(unused_imports)]
use super::arrangements::*;
use super::*;
use st3_attention::store::arrangements_tests_shared_seats::*;
use st3_attention::store::arrangements_tests_shared_seats::{A, B, SUBJECT, append, create, input};
use st3_attention::store::directive_notes::tests_shared_seats::*;
use st3_attention::store::directive_notes::tests_shared_seats::{FLEET, advertise, claim, declare, enable, store};


#[test]
fn historical_status_reads_skip_arrangements_changed_after_the_frontier() {
    let store = Store::open_memory("historical-arrangements").unwrap();
    create(&store);
    store.append_claim(&ClaimInput {
        subject: "resource/historical".into(), kind: "resource.observed".into(), actor: None,
        fields: serde_json::from_value(json!({"kind":"custom.test.historical","value":"before"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let frontier = store.index().unwrap();
    append(&store, json!([{"op":"rename","name":"After frontier"}]));
    let status = store.status_at(None, None, Some(frontier)).unwrap();
    assert!(status.subjects.iter().all(|subject| subject.subject != SUBJECT));
    assert!(status.subjects.iter().any(|subject| subject.subject == "resource/historical"));
    let history = store.status_history(None, None, Some(frontier)).unwrap();
    assert!(history.subjects.iter().all(|subject| subject.subject != SUBJECT));
    assert!(history.subjects.iter().any(|subject| subject.subject == "resource/historical"));
    let prefix = store
        .status_for_subject_prefix_at(
            "arrangement/",
            Some(frontier),
            false,
            crate::store::ClaimsReduction::Summary,
        )
        .unwrap();
    assert!(prefix.subjects.is_empty());
}
