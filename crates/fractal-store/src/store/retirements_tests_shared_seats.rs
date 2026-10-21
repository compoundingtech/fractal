#![allow(unused_imports)]
use crate::store::*;
use crate::model::{LifecycleUnavailableReason, MessageSendRefusal};
use crate::store::retirements::*;
use std::sync::Barrier;

pub const SUBJECT: &str = "agent/example/retiring";

pub const ACTOR: &str = "person/avery";

pub fn send(key: &str) -> ClaimInput {
        ClaimInput {
            subject: format!("message/{key}"),
            kind: "message.sent".into(),
            actor: Some(ACTOR.into()),
            fields: BTreeMap::from([
                ("from".into(), json!(ACTOR)),
                ("to".into(), json!(SUBJECT)),
                ("content".into(), json!("An invented note.")),
                ("status".into(), json!("sent")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(key.into()),
        }
    }
