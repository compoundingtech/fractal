#![allow(unused_imports)]
use crate::store::*;
use crate::store::publication_facts::*;

pub fn input(kind: &str, incarnation: &str) -> ClaimInput {
        ClaimInput {
            subject: "agent/example/worker".into(),
            kind: kind.into(),
            actor: Some("agent/example/writer".into()),
            fields: if kind == "runtime.observed" {
                BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("incarnation_id".into(), json!(incarnation)),
                ])
            } else {
                BTreeMap::from([
                    ("action".into(), json!("start")),
                    ("incarnation_id".into(), json!(incarnation)),
                    ("desired_token".into(), json!("not-retained")),
                ])
            },
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        }
    }
