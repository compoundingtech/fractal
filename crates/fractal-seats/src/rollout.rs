//! The part of `rollout` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::model::{ClaimInput, DesiredSubject, MemberSpec};
use crate::store::{Store, owned_sets::Source};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
pub use fractal_missions::rollout::*;
pub use fractal_missions::rollout::{Mode, Operation, Policy, Selection, blockers, hold_render, launched_member, manual, pending_manual_launch, pending_manual_launch_at, status, target};

pub const OPERATION_ENV: &str = "ST3_ROLLOUT_OPERATION";

pub const PREDECESSOR_ENV: &str = "ST3_ROLLOUT_PREDECESSOR";

pub const RESUME_PATH_ENV: &str = "ST3_NATIVE_RESUME_PATH";

pub fn phase(
    store: &Store,
    subject: &str,
    operation: &Operation,
    phase: &str,
    reason: Option<&str>,
    blocking: &[String],
) -> Result<()> {
    let mut state = serde_json::to_value(operation)?;
    state["reason"] = json!(reason);
    state["blocking"] = json!(blocking);
    store.append_claim(&ClaimInput {
        subject: subject.into(),
        kind: if matches!(phase, "failed" | "blocked") {
            "runtime.action.failed"
        } else if phase == "held" {
            "runtime.action.deadline-reached"
        } else {
            "runtime.action.succeeded"
        }
        .into(),
        actor: operation.requested_by.clone(),
        fields: BTreeMap::from([
            ("action".into(), json!("rollout")),
            ("operation".into(), json!(operation.id)),
            ("operation_status".into(), json!(phase)),
            ("rollout".into(), state),
        ]),
        evidence: vec![operation.id.clone()],
        expected_subject: None,
        idempotency_key: Some(format!("seat-rollout:{}:{phase}", operation.id)),
    })?;
    Ok(())
}

pub fn bound_account(store: &Store, subject: &str, incarnation: &str) -> Result<Option<String>> {
    Ok(store
        .claims_for(subject, Some("harness.session-file"))?
        .into_iter()
        .rev()
        .find(|claim| {
            claim
                .body
                .pointer("/fields/incarnation_id")
                .and_then(serde_json::Value::as_str)
                == Some(incarnation)
        })
        .and_then(|claim| {
            claim
                .body
                .pointer("/fields/account_ref")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        }))
}

pub fn binding(
    store: &Store,
    subject: &str,
    incarnation: &str,
) -> Result<Option<(String, String, Option<String>)>> {
    Ok(store
        .claims_for(subject, Some("harness.session-file"))?
        .into_iter()
        .rev()
        .find_map(|claim| {
            let fields = &claim.body["fields"];
            if fields["incarnation_id"].as_str() != Some(incarnation) {
                return None;
            }
            Some((
                fields["harness"].as_str()?.into(),
                fields["session_id"].as_str()?.into(),
                fields["path"].as_str().map(str::to_owned),
            ))
        }))
}
