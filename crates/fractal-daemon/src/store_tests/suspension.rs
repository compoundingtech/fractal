//! The suspension move test that reads the seat snapshot; it lives in fractal-daemon because seat_snapshot does.

#![allow(unused_imports)]

use crate::SeatsStore;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::*;
use crate::store::Store;
use super::common::*;
use anyhow::Result;
use rusqlite::params;
use smallclaims::store::now_ms;
use crate::suspension::*;

#[test]
fn a_move_survives_a_crash_after_placement_and_a_later_label_change() {
    use crate::model::ClaimInput;
    use serde_json::json;
    let store = Store::open_memory("amber").unwrap();
    let intent = crate::parse_intent("version 2\nagent \"sample\" { host \"amber\"; workspace \"/sample\"; harness \"omp\" { model \"canary\" } }", "amber").unwrap();
    let preview = store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: String::new(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "declare",
            Some("person/test"),
        )
        .unwrap();
    let subject = "agent/amber.sample";
    let token = store.selected_desired_token(subject).unwrap().unwrap();
    let append = |kind: &str, fields: Value, evidence: Vec<String>, key: Option<String>| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some("person/test".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence,
                expected_subject: None,
                idempotency_key: key,
            })
            .unwrap()
    };
    let suspend = append(
        "runtime.action.requested",
        json!({"action": "suspend", "incarnation_id": "source-one"}),
        vec![token.clone()],
        None,
    );
    append(
        "runtime.action.succeeded",
        json!({"action": "suspend", "operation_status": "snapshotting", "harness": "omp", "native_session_id": "native", "source_host": "amber"}),
        vec![suspend.id.clone()],
        Some(suspend_snapshot_key(&suspend.id)),
    );
    append(
        "runtime.action.succeeded",
        json!({"action": "suspend", "operation_status": "suspended"}),
        vec![suspend.id.clone()],
        Some(suspend_completed_key(&suspend.id)),
    );
    let resume = append(
        "runtime.action.requested",
        json!({"action": "resume", "host": "jade", "source_host": "amber"}),
        vec![token.clone(), suspend.id],
        None,
    );
    let transferring = current(&store, subject).unwrap().unwrap();
    crate::seat_snapshot::phase(&store, subject, &transferring, "transfer", "transferring")
        .unwrap();
    assert_eq!(
        current(&store, subject).unwrap().unwrap().phase,
        "transferring"
    );
    store
        .place_resumed_seat(subject, &token, &resume.id, "jade", "person/test")
        .unwrap();
    // No restored receipt exists: this models a crash immediately after placement publication.
    let restored = current(&store, subject).unwrap().unwrap();
    assert_eq!(restored.phase, "restoring");
    assert_eq!(restored.native_session_id.as_deref(), Some("native"));
    let desired = store
        .desired_subject_with_writer(subject)
        .unwrap()
        .unwrap()
        .0;
    let kdl = crate::graph::render_agent_desired_kdl(&desired.desired, &std::collections::BTreeMap::new()).unwrap();
    assert!(
        crate::parse_intent(&kdl, "jade")
            .unwrap()
            .subjects
            .contains_key(subject)
    );
    store
        .rename_agent(subject, Some("Renamed sample"), "rename")
        .unwrap();
    assert_eq!(
        current(&store, subject).unwrap().unwrap().phase,
        "restoring"
    );
}

#[test]
fn suspension_optional_metadata_survives_replication_and_reopen() {
    for (cause, mode) in [(None, None), (Some("idle"), Some("in-place")), (Some("future-cause"), Some("future-mode"))] {
        let source = Store::open_memory("source").unwrap();
        let append = |fields: Value, key: Option<String>| {
            source.append_claim(&ClaimInput {
                subject: "agent/sample".into(),
                kind: if key.is_some() { "runtime.action.succeeded" } else { "runtime.action.requested" }.into(),
                actor: Some("person/test".into()), fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: key,
            }).unwrap()
        };
        let mut fields = json!({"action":"suspend"});
        if let Some(cause) = cause { fields["cause"] = json!(cause); }
        let request = append(fields.clone(), None);
        let mut snapshot = json!({"action":"suspend", "native_session_id":"native"});
        if let Some(mode) = mode { snapshot["mode"] = json!(mode); }
        append(snapshot, Some(suspend_snapshot_key(&request.id)));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("suspension.sqlite3");
        let replica = Store::open(&path, "replica").unwrap();
        let mut registry = fractal_schema::registry().clone();
        registry.claims.get_mut("runtime.action.requested").unwrap().fields.remove("cause");
        registry.claims.get_mut("runtime.action.succeeded").unwrap().fields.remove("mode");
        replica.set_claim_registry(registry);
        replica.bind_fleet(TEST_FLEET).unwrap();
        replica.receive_replication_exchange(&source.origin, TEST_FLEET,
            &exchange_from(&source, &ReplicationInventory::default())).unwrap();
        let admission = replica.validate_replication_backlog().unwrap();
        assert_eq!(admission.unknown, if cause.is_some() { 2 } else { 0 });
        replica.project_replication_backlog().unwrap();
        drop(replica);
        let reopened = Store::open(&path, "replica").unwrap();
        reopened.validate_replication_backlog().unwrap();
        reopened.project_replication_backlog().unwrap();
        let stored = reopened.claim_by_id(&request.id).unwrap().unwrap();
        assert_eq!(stored.body["fields"], fields);
        let state = suspend_state(&reopened, &stored, None).unwrap();
        assert_eq!(state.cause(), cause.unwrap_or("manual"));
        assert_eq!(state.mode(), mode.unwrap_or("git"));
        assert_eq!(state.native_session_id.as_deref(), Some("native"));
    }
}
