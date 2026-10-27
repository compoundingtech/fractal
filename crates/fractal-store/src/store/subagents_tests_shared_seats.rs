#![allow(unused_imports)]
use crate::store::*;
use crate::store::subagents::*;

pub const SEAT: &str = "agent/alder.parent";

pub fn runtime(store: &Store, seat: &str, status: &str, incarnation: &str) {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: "runtime.observed".into(),
                actor: Some(seat.into()),
                fields: BTreeMap::from([
                    ("status".into(), json!(status)),
                    ("runtime_id".into(), json!("runtime-1")),
                    ("incarnation_id".into(), json!(incarnation)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }

pub fn claim(
        store: &Store,
        kind: &str,
        id: &str,
        extra: &[(&str, Value)],
    ) -> Result<(ClaimRecord, bool), St3Error> {
        let mut fields = BTreeMap::from([("subagent_id".into(), json!(id))]);
        for (name, value) in extra {
            fields.insert((*name).into(), value.clone());
        }
        store.append_client_claim_outcome(&ClaimInput {
            subject: SEAT.into(),
            kind: kind.into(),
            actor: Some(SEAT.into()),
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
    }

pub fn appear(store: &Store, id: &str, lease: u64) -> Result<(ClaimRecord, bool), St3Error> {
        claim(
            store,
            "subagent.appeared",
            id,
            &[
                ("subagent_type", json!("Explore")),
                ("description", json!("map the code")),
                ("driver", json!("claude")),
                ("session_id", json!("session-1")),
                ("incarnation_id", json!("inc-1")),
                ("step_run", json!("step-run/run-1/claims")),
                ("started_at_unix_ms", json!(1_000)),
                ("lease_expires_at_unix_ms", json!(lease)),
            ],
        )
    }

pub fn ids(store: &Store) -> Vec<String> {
        store
            .open_subagents(SEAT)
            .unwrap()
            .into_iter()
            .map(|subagent| subagent.subagent_id)
            .collect()
    }
