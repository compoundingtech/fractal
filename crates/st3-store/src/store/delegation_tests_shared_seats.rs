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

pub fn policy(store: &Store, actions: &[&str], key: &str) -> ClaimRecord {
        let decision = sent(
            store,
            &format!("policy-{key}"),
            "person/avery",
            "agent/alder.asker",
            "Use this delegation list.",
        );
        store
            .set_delegation_policy(&DelegationPolicyRequest {
                person: "person/avery".into(),
                actor: "person/avery".into(),
                actions: actions.iter().map(|action| (*action).into()).collect(),
                evidence: vec![decision.id],
                idempotency_key: key.into(),
            })
            .unwrap()
    }

pub fn proof(store: &Store, actions: &[&str], episode: String) -> DelegationProof {
        let policy = policy(store, actions, "allow");
        sent(
            store,
            "instruction",
            "person/avery",
            "agent/alder.asker",
            "Friday. Archive that message and read the update. Approve the release. Stop the release.",
        );
        DelegationProof {
            person: "person/avery".into(),
            policy: policy.id,
            message: "message/instruction".into(),
            quote: "Friday".into(),
            episode,
        }
    }
