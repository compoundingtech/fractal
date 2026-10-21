//! Mailbox tests that bind and replay through the store; they live here because the mailbox module sits below the store.

#![allow(unused_imports, dead_code)]

use crate::mailbox::*;
use crate::store::Store;
use crate::model::{DesiredSubject, MessageView};
use anyhow::{Context as _, Result};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::client::Fence;
use crate::model::ClaimInput;
use serde_json::json;
use std::collections::BTreeMap;
use axum::{Json, Router, routing::post};
use axum::http::StatusCode;
use std::sync::{Arc, Mutex};


pub fn claim(subject: &str, kind: &str, fields: Value, key: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some("agent/eval.worker".into()),
        fields: serde_json::from_value::<BTreeMap<String, Value>>(fields).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(key.into()),
    }
}

pub fn ready(store: &Store, incarnation: &str) {
    store
        .append_claim(&claim(
            "agent/eval.worker",
            "runtime.observed",
            json!({"status":"running",
        "runtime_id":"eval.worker","incarnation_id":incarnation}),
            &format!("runtime:{incarnation}"),
        ))
        .unwrap();
    store
        .append_claim(&claim(
            "agent/eval.worker",
            "harness.observed",
            json!({"state":"ready",
        "driver":"omp","incarnation_id":incarnation}),
            &format!("harness:{incarnation}"),
        ))
        .unwrap();
}

#[test]
fn claude_attachment_fences_idle_hooks_and_clears_only_on_attach_or_replacement() {
    let store = Store::open_memory("node").unwrap();
    ready(&store, "current");
    let diagnostic = |code: &str, status: &str, incarnation: &str, key: &str| {
        store
            .append_claim(&claim(
                "agent/eval.worker",
                "harness.diagnostic",
                json!({"severity":"warning","code":code,"status":status,
                "reason":"attachment probe","incarnation_id":incarnation}),
                key,
            ))
            .unwrap();
    };
    diagnostic(
        "claude-channel-unattached",
        "starting",
        "current",
        "starting",
    );
    assert_eq!(
        store
            .current_harness("agent/eval.worker")
            .unwrap()
            .unwrap()
            .state,
        "starting"
    );
    diagnostic("claude-channel-unattached", "blocked", "current", "blocked");
    store
        .append_claim(&claim(
            "agent/eval.worker",
            "harness.observed",
            json!({"state":"idle","driver":"claude","incarnation_id":"current"}),
            "idle-hook",
        ))
        .unwrap();
    let blocked = store.current_harness("agent/eval.worker").unwrap().unwrap();
    assert_eq!(blocked.state, "blocked");
    assert_eq!(blocked.reason.as_deref(), Some("claude-channel-unattached"));
    diagnostic(
        "claude-channel-attached",
        "recovered",
        "previous",
        "foreign-recovery",
    );
    assert_eq!(
        store
            .current_harness("agent/eval.worker")
            .unwrap()
            .unwrap()
            .state,
        "blocked"
    );
    diagnostic(
        "claude-channel-attached",
        "recovered",
        "current",
        "recovered",
    );
    assert_eq!(
        store
            .current_harness("agent/eval.worker")
            .unwrap()
            .unwrap()
            .state,
        "idle"
    );
    diagnostic(
        "claude-channel-unattached",
        "blocked",
        "current",
        "blocked-again",
    );
    ready(&store, "replacement");
    assert_eq!(
        store
            .current_harness("agent/eval.worker")
            .unwrap()
            .unwrap()
            .state,
        "ready"
    );
}

#[test]
fn legacy_recipient_receipt_replay_after_close_settles_without_backward_claims() {
    let store = Store::open_memory("node").unwrap();
    store.append_claim(&claim("message/native", "message.sent",
        json!({"status":"sent","from":"person/eval","to":"agent/eval.worker","content":"QUARTZ SIGNAL"}), "sent")).unwrap();
    for status in ["delivered", "read"] {
        store.append_claim(&claim("message/native", &format!("message.{status}"), json!({"status":status}), status)).unwrap();
    }
    let closed = store.append_claim(&claim("message/native", "message.closed", json!({"status":"closed"}), "closed")).unwrap();
    let before = store.claims_for("message/native", None).unwrap().len();
    for status in ["staged", "delivered", "read"] {
        let input = claim("message/native", &format!("message.{status}"), json!({"status":status}), &format!("legacy-replay-{status}"));
        assert_eq!(store.append_claim(&input).unwrap().id, closed.id);
        let mut foreign = input;
        foreign.actor = Some("agent/eval.other".into());
        foreign.idempotency_key = Some(format!("foreign-{status}"));
        assert_eq!(store.append_claim(&foreign).unwrap_err().code, "invalid-message-transition");
    }
    assert_eq!(store.claims_for("message/native", None).unwrap().len(), before);
    assert_eq!(store.message("message/native").unwrap().unwrap().status, "closed");
}

