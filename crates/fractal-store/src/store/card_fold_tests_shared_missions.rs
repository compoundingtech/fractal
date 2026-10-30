#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::{agent_subjects, assert_card_fold_parity, assert_card_reads_parity, assert_card_reads_parity_with, per_subject_card_status};
use crate::store::*;

/// Every snapshot of `store` that a fold could read: each store index up to its head.
pub fn snapshots(store: &Store) -> Vec<u64> {
    (0..=store.index().unwrap()).collect()
}

pub fn append(store: &Store, subject: &str, kind: &str, actor: Option<&str>, fields: Value) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

pub fn running(store: &Store, subject: &str, incarnation: &str) {
    append(
        store,
        subject,
        "runtime.observed",
        Some(subject),
        json!({"status": "running", "runtime_id": subject, "incarnation_id": incarnation}),
    );
}

pub fn declare(store: &Store, intent: &NormalizedIntent, key: &str) -> u64 {
    let planned = store
        .mission(intent, IntentInput { kdl: key.into(), source_name: None })
        .unwrap();
    store.apply(intent, &planned.subject_tokens, key).unwrap().store_index
}

/// Host birch observes alder's plain seat too: a rival origin the card must weigh.
pub fn birch() -> Store {
    let store = Store::open_memory("birch").unwrap();
    running(&store, "agent/alder.plain", "plain-birch");
    running(&store, "agent/birch.solo", "solo-1");
    store
}
