#![allow(unused_imports)]
pub use fractal_store::store::reconcile_work_context::tests_shared_seats::*;
use crate::store::reconcile_work_context::*;
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::time::Instant;


pub fn oracle(store: &Store, actor: &str, at: u128) -> Result<Vec<StepRunView>> {
    // Exact existing per-step path, including owner/definition queries and all enrichments.
    store.read_snapshot(|_| store.work_at_snapshot_internal(Some(actor), true, at, false))
}

pub fn parity(store: &Store, actor: &str, at: u128) -> Vec<StepRunView> {
    let (old, old_keys) = smallclaims::touched::record(|| oracle(store, actor, at).unwrap());
    let (new, new_keys) = smallclaims::touched::record(|| read(store, actor, at).unwrap());
    assert!(
        old_keys.is_subset(&new_keys),
        "batched read dropped registered dependencies: {:?}",
        old_keys.difference(&new_keys).collect::<Vec<_>>()
    );
    println!(
        "registered work context keys: old={} new={} added={:?}",
        old_keys.len(),
        new_keys.len(),
        new_keys.difference(&old_keys).collect::<Vec<_>>()
    );
    assert_eq!(
        serde_json::to_value(&new).unwrap(),
        serde_json::to_value(&old).unwrap()
    );
    new
}

pub fn claim_for_handoff(store: &Store, subject: &str) {
    store.set_step_state(subject, "ready", None).unwrap();
    store
        .work_action(
            subject,
            "claim",
            &WorkRequest {
                wait_ms: None,
                actor: Some(ACTOR.into()),
                incarnation: Some("support-worker-one".into()),
                summary: None,
                reason: None,
                evidence: Vec::new(),
                idempotency_key: "support-worker-claim".into(),
            },
        )
        .unwrap();
}

pub fn handoff_to_peer(store: &Store, subject: &str) {
    store
        .handoff_work(
            subject,
            &crate::model::WorkHandoffRequest {
                actor: ACTOR.into(),
                incarnation: Some("support-worker-one".into()),
                to: "agent/alder.peer".into(),
                note: "Continue this exact step.".into(),
                evidence: Vec::new(),
                idempotency_key: "support-transfer".into(),
            },
        )
        .unwrap();
}
