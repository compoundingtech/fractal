#![allow(unused_imports)]
use crate::store::*;
use crate::model::{DelegationPolicyRequest, DelegationProof};
use crate::store::delegation::*;
use crate::model::PersonStepResponse;

pub fn sent(store: &Store, id: &str, from: &str, to: &str, text: &str) -> ClaimRecord {
        store
            .append_claim(&ClaimInput {
                subject: format!("message/{id}"),
                kind: "message.sent".into(),
                actor: Some(from.into()),
                fields: BTreeMap::from([
                    ("from".into(), json!(from)),
                    ("to".into(), json!(to)),
                    ("content".into(), json!(text)),
                    ("status".into(), json!("sent")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(id.into()),
            })
            .unwrap()
    }

