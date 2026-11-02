#![allow(unused_imports)]
use crate::store::*;

pub const MINUTE: u128 = 60_000;

pub struct Waiting {
    pub _directory: tempfile::TempDir,
    pub store: Store,
    pub subject: String,
}

pub fn request(key: &str, reason: Option<&str>, wait_ms: Option<u64>) -> WorkRequest {
    WorkRequest {
        wait_ms,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: reason.map(Into::into),
        evidence: Vec::new(),
        idempotency_key: key.into(),
    }
}

pub const HOUR: u128 = 60 * MINUTE;
