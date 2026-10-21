#![allow(unused_imports)]
use crate::store::*;
use crate::model::{WorkAcknowledgeRequest, WorkHandoffRequest, WorkStartRequest};
use crate::store::adhoc_work::*;

pub fn request(actor: &str, key: &str) -> WorkRequest {
        WorkRequest {
            wait_on: None,
            wait_ms: None,
            actor: Some(actor.into()),
            incarnation: Some(format!("{actor}-one")),
            summary: None,
            reason: None,
            evidence: vec![],
            idempotency_key: key.into(),
        }
    }
