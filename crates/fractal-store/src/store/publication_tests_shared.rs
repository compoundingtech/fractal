#![allow(unused_imports)]
use crate::store::*;
use crate::model::MemberSpec;
use crate::publication::{Interruption, interruption};
use crate::store::publication::*;
use crate::graph::parse_intent;

pub fn declaration(command: &str) -> NormalizedIntent {
        parse_intent(&format!("version 2\nagent \"example/worker\" {{ workspace \"/tmp\"; command \"{command}\"; }}"), "node").unwrap()
    }

pub fn publish(store: &Store, intent: &NormalizedIntent, key: &str) -> ApplyResponse {
        let preview = store
            .mission(
                intent,
                IntentInput {
                    kdl: String::new(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(intent, &preview.subject_tokens, key).unwrap()
    }

pub fn observe(store: &Store, incarnation: &str, token: Option<&str>) {
        if let Some(token) = token {
            store
                .append_claim(&ClaimInput {
                    subject: "agent/example/worker".into(),
                    kind: "runtime.action.succeeded".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("action".into(), json!("start")),
                        ("incarnation_id".into(), json!(incarnation)),
                        ("desired_token".into(), json!(token)),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        store
            .append_claim(&ClaimInput {
                subject: "agent/example/worker".into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("incarnation_id".into(), json!(incarnation)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
