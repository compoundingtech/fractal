#![allow(unused_imports)]
use crate::model::ClaimRecord;
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use crate::native_resume::{CONTINUE_ENV, CONTINUE_PATH_ENV, RESUME_ENV};
use crate::suspension::*;
use serde_json::json;

pub fn continuation_claim(store: &Store, kind: &str, fields: Value, key: Option<String>) {
        store.append_claim(&crate::model::ClaimInput {
            subject: "agent/example".into(), kind: kind.into(), actor: Some("agent/example".into()),
            fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
            expected_subject: None, idempotency_key: key,
        }).unwrap();
    }
