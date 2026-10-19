#![allow(unused_imports)]
use crate::store::*;
use crate::store::seat_status::*;

pub fn input(kind: &str, fields: Value) -> ClaimInput {
        ClaimInput {
            subject: "agent/cedar".into(),
            kind: kind.into(),
            actor: Some("agent/cedar".into()),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }
    }

pub fn runtime(store: &Store, incarnation: &str) {
        store
            .append_claim(&input(
                "runtime.observed",
                json!({
                    "status":"running", "incarnation_id":incarnation, "runtime_id":"native"
                }),
            ))
            .unwrap();
    }

pub fn observe(
        store: &Store,
        incarnation: &str,
        state: &str,
        at: u128,
        reason: &str,
    ) -> ClaimRecord {
        store.append_latest_observation(&input("harness.observed", json!({
            "state":state, "incarnation_id":incarnation, "observed_at_ms":at as u64, "reason":reason
        })), at).unwrap().0
    }
