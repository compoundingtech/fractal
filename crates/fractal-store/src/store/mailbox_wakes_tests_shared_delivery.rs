#![allow(unused_imports)]
use crate::store::*;
use crate::model::DoctorCheck;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::watch;
use crate::store::mailbox_wakes::*;
use crate::mailbox::Fence;

pub fn sent(store: &Store, subject: &str, to: &str) {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "message.sent".into(),
                actor: Some("person/fixture".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("sent")),
                    ("from".into(), json!("person/fixture")),
                    ("to".into(), json!(to)),
                    ("content".into(), json!("Fixture note.")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
