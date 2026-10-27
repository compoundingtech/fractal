#![allow(unused_imports)]
use crate::store::*;
use crate::store::conversation_reads::*;

pub fn append(store: &Store, subject: &str, kind: &str, fields: Value) -> ClaimRecord {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: None,
                fields: fields
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    }
