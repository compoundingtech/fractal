#![allow(unused_imports)]
use crate::store::*;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use crate::store::runtime_starts::*;

pub fn operation(id: &str, subject: &str, token: Option<String>) -> RuntimeStartOperation {
        RuntimeStartOperation {
            id: id.into(),
            subject: subject.into(),
            owner: "owner".into(),
            body: serde_json::json!({"subject":{"subject":subject,"kind":"agent","desired":null},"selected_token": token, "desired_token":"desired-test", "member":{"runtime_id":subject,"terminal":true}}),
            revoked_by: None,
            finished: false,
            result: None,
            applied: false,
            cleanup_verified: false,
            dispatched: false,
            receipted: false,
        }
    }

pub fn receipt(subject: &str) -> ClaimInput {
        ClaimInput {
            subject: subject.into(),
            kind: "runtime.action.succeeded".into(),
            actor: None,
            fields: BTreeMap::from([("action".into(), Value::from("start"))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("test-start:{subject}")),
        }
    }

pub fn publish(store: &Store, source: &str, key: &str) {
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                crate::model::IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &preview.subject_tokens, key).unwrap();
    }

pub fn format_cuts(connection: &Connection) -> (i64, i64) {
        let read_cut = |key| {
            connection
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",
                    [key],
                    |row| row.get(0),
                )
                .unwrap()
        };
        (
            read_cut("publication-operation-format-shared-cut"),
            read_cut("publication-operation-format-local-cut"),
        )
    }
