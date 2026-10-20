#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::fleet_admission_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::fleet_admission_tests_shared_seats::{FLEET, admit, admitted, append, key, node, note, sync};
use super::*;
use crate::fleet::{MemberKey, MemberState, envelope_signature_message, verify_signature};


/// A member one build behind keeps a newer member's unknown claims, and only an unknown claim
/// that could bear on runtime authority makes a subject indeterminate. Seen live: a newer
/// member's `harness.limits` made every seat on it unreachable for terminal attach from an
/// older member, though its runtime observations were valid.
#[test]
fn an_unknown_non_authority_claim_leaves_runtime_routing_alone() {
    let anchor_key = key();
    let older_key = key();
    let current = node("current", Some(&anchor_key), Some(&anchor_key));
    admit(&current, "current", &anchor_key, "anchor", None);
    admit(&current, "older", &older_key, "invite", None);
    let older = node("older", Some(&older_key), Some(&anchor_key));
    let mut registry = st3_schema::registry().clone();
    for kind in ["harness.limits", "runtime.restart-window-reset"] {
        registry.claims.remove(kind).unwrap();
    }
    older.set_claim_registry(registry);
    sync(&current, &older);

    let subject = "agent/example/worker";
    append(
        &current,
        "runtime.observed",
        subject,
        json!({"status": "running", "runtime_id": "agent.example.worker",
               "incarnation_id": "worker-one"}),
    );
    append(
        &current,
        "harness.limits",
        subject,
        json!({"driver": "claude", "incarnation_id": "worker-one",
               "five_hour_percent": 12.5, "measured_at_unix_ms": 1_790_000_000_000_u64}),
    );
    let admission = sync(&current, &older);
    assert_eq!(admission.unknown, 1, "the newer kind is kept, not dropped");
    let status = older.status(Some(subject)).unwrap();
    let worker = &status.subjects[0];
    assert_eq!(worker.reachability, "reachable", "{:?}", worker.reason);
    assert_eq!(worker.actual_origin.as_deref(), Some("current"));
    assert_eq!(worker.reason, None);
    let authority = older.runtime_authority(subject).unwrap().unwrap();
    assert_eq!(authority.reachability, "reachable");
    assert_eq!(authority.actual_origin.as_deref(), Some("current"));
    assert_eq!(authority.actual, worker.actual);

    // An unknown kind in the runtime family could change the answer: stay conservative.
    append(
        &current,
        "runtime.restart-window-reset",
        subject,
        json!({"incarnation_id": "worker-one", "reason": "test"}),
    );
    sync(&current, &older);
    let status = older.status(Some(subject)).unwrap();
    assert_eq!(status.subjects[0].reachability, "indeterminate");
    assert_eq!(older.runtime_authority(subject).unwrap().unwrap().reachability, "indeterminate");
    assert!(
        status.subjects[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("runtime.restart-window-reset"))
    );
}

#[test]
fn runtime_authority_preserves_append_fields_nulls_and_other_actual_kinds() {
    let subject = "agent/runtime-fence-fields";
    for missing in ["status", "runtime_id", "incarnation_id", "terminal", "reachability"] {
        let store = Store::open_memory("owner").unwrap();
        let original = json!({"status":"running","runtime_id":"r","incarnation_id":"i",
                              "terminal":true,"reachability":"reachable"});
        append(&store, "runtime.observed", subject, original.clone());
        append(&store, "runtime.observed", subject, json!({"reason":"later partial report"}));
        let narrow = store.runtime_authority(subject).unwrap().unwrap();
        let status = store.status(Some(subject)).unwrap();
        for field in ["status", "runtime_id", "incarnation_id", "terminal", "reachability"] {
            assert_eq!(narrow.actual.as_ref().unwrap().get(field), status.subjects[0].actual.as_ref().unwrap().get(field));
            assert_eq!(narrow.actual.as_ref().unwrap().get(field), original.get(field));
        }
        append(&store, "runtime.observed", subject, json!({missing: null}));
        let narrow = store.runtime_authority(subject).unwrap().unwrap();
        let status = store.status(Some(subject)).unwrap();
        assert_eq!(narrow.actual.as_ref().unwrap().get(missing), Some(&Value::Null));
        assert_eq!(narrow.reachability, status.subjects[0].reachability);
        assert_eq!(narrow.actual.as_ref().unwrap().get(missing), status.subjects[0].actual.as_ref().unwrap().get(missing));
        append(&store, "runtime.reconcile-decision", subject,
               json!({"decision":"hold","reachability":"unreachable"}));
        let narrow = store.runtime_authority(subject).unwrap().unwrap();
        let status = store.status(Some(subject)).unwrap();
        assert_eq!(narrow.reachability, "unreachable");
        assert_eq!(narrow.actual, status.subjects[0].actual);
    }
}

mod terminal_authority_idle {
use super::*;

fn append(store: &Store, subject: &str, at: u128, fields: Value) -> String {
    store.set_write_clock_at(at).unwrap();
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let claim = append_claim_tx(&transaction, store.origin(), subject, "resource.observed",
        None, &json!({"fields":fields}), &[], None).unwrap();
    transaction.commit().unwrap();
    claim.id
}
#[test]
fn terminal_authority_ignores_idempotent_message_traffic_but_wakes_on_shared_conflict() {
    let store = Store::open_memory("owner").unwrap();
    let subject = "agent/terminal-message-traffic";
    store.append_claim(&crate::model::ClaimInput {
        subject: subject.into(), kind: "runtime.observed".into(),
        actor: Some("person/robin".into()),
        fields: serde_json::from_value(json!({
            "status": "running", "runtime_id": "terminal-message-traffic",
            "incarnation_id": "one", "terminal": true,
        })).unwrap(),
        evidence: Vec::new(), expected_subject: None,
        idempotency_key: Some("terminal-operation".into()),
    }).unwrap();
    let mut changes = store.subscribe_runtime_authority(subject);
    assert_eq!(store.runtime_authority(subject).unwrap().unwrap().reachability, "reachable");
    changes.borrow_and_update();
    for index in 0..100 {
        // The durable message.send path: a fresh message and fresh idempotency operation.
        store.append_claim(&crate::model::ClaimInput {
            subject: format!("message/traffic-{index}"), kind: "message.sent".into(),
            actor: Some("person/robin".into()),
            fields: serde_json::from_value(json!({
                "from": "person/robin", "to": "person/blair",
                "content": format!("message {index}"), "status": "sent",
            })).unwrap(),
            evidence: Vec::new(), expected_subject: None,
            idempotency_key: Some(format!("message-traffic-{index}")),
        }).unwrap();
        let work = smallclaims::sqlite::work::SqliteWorkScope::start();
        // The stream's production read gate must stay closed after every committed message.
        if changes.has_changed().unwrap() {
            changes.borrow_and_update();
            store.runtime_authority(subject).unwrap();
        }
        assert_eq!(work.finish().statements, 0, "message {index} triggered authority SQL");
    }
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let operation: String = transaction.query_row(
            "SELECT json_extract(body, '$._operation.id') FROM claims WHERE subject=?1",
            [subject], |row| row.get(0),
        ).unwrap();
        // A second claim on another subject reuses the operation with a different digest.
        // Register it exactly as incremental receive does; no terminal claim changes.
        let rival = append_claim_tx(&transaction, store.origin(), "message/cross-subject-conflict",
            "message.sent", Some("person/robin"), &json!({
                "fields": {"from":"person/robin", "to":"person/blair", "content":"conflict", "status":"sent"},
                "_operation": {"id":operation, "request_digest":"different-request"},
            }), &[], None).unwrap();
        assert!(!smallclaims::store::register_operation_tx(&transaction, &rival).unwrap());
        transaction.commit().unwrap();
    }
    assert!(changes.has_changed().unwrap(), "shared-operation conflict must wake the terminal");
    assert_eq!(store.runtime_authority(subject).unwrap().unwrap().reachability, "indeterminate");
}

#[test]
fn terminal_authority_failed_coverage_wakes_until_verified_recovery() {
    let store = Store::open_memory("owner").unwrap();
    let subject = "resource/terminal-coverage";
    let id = append(&store, subject, 10, json!({"state": "open"}));
    let mut changes = store.subscribe_runtime_authority(subject);
    assert!(store.runtime_authority(subject).unwrap().unwrap().actual.is_some());
    changes.borrow_and_update();
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h7f624abd90cf owner=st3_missions::store::fleet_admission_tests TODO=chunk-or-reference-variable-columns */
        "DROP TABLE projection_digest_repaired_claims;
         CREATE TABLE projection_digest_repaired_claims(id TEXT PRIMARY KEY) WITHOUT ROWID;"
    ).unwrap();
    assert!(store.runtime_authority_coverage_failed());
    assert!(changes.has_changed().unwrap(), "coverage failure must immediately wake readers");
    changes.borrow_and_update();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h3eee87cb9c9b owner=st3_missions::store::fleet_admission_tests TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO projection_digest_repaired_claims VALUES (?1)", [&id],
    ).unwrap();
    assert!(changes.has_changed().unwrap(), "WITHOUT ROWID repair must wake despite disabled publication");
    changes.borrow_and_update();
    assert!(store.runtime_authority(subject).unwrap().unwrap().actual.is_none());
    changes.borrow_and_update();
    append(&store, "resource/other", 20, json!({"state": "closed"}));
    assert!(changes.has_changed().unwrap(), "disabled coverage needs conservative resolution wakes");
    changes.borrow_and_update();
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h0b633c2fede3 owner=st3_missions::store::fleet_admission_tests TODO=chunk-or-reference-variable-columns */
        "DROP TABLE projection_digest_repaired_claims;
         CREATE TABLE projection_digest_repaired_claims(id TEXT PRIMARY KEY);"
    ).unwrap();
    assert!(!store.runtime_authority_coverage_failed());
    assert!(changes.has_changed().unwrap(), "verified recovery must publish invalidation");
    changes.borrow_and_update();
    append(&store, "resource/other", 30, json!({"state": "open"}));
    assert!(!changes.has_changed().unwrap(), "verified coverage restores selective waking");
}

#[test]
fn terminal_authority_operation_conflict_wakes_without_subject_claim_changes() {
    let store = Store::open_memory("owner").unwrap();
    let subject = "agent/terminal-operation";
    store.append_claim(&crate::model::ClaimInput {
        subject: subject.into(), kind: "runtime.observed".into(),
        actor: Some(subject.into()),
        fields: serde_json::from_value(json!({
            "status": "running", "runtime_id": "terminal-operation",
            "incarnation_id": "one", "terminal": true,
        })).unwrap(),
        evidence: Vec::new(), expected_subject: None,
        idempotency_key: Some("shared-operation".into()),
    }).unwrap();
    let mut changes = store.subscribe_runtime_authority(subject);
    assert_eq!(store.runtime_authority(subject).unwrap().unwrap().reachability, "reachable");
    changes.borrow_and_update();
    let before = store.index().unwrap();
    // A cross-subject idempotency collision repairs this shared operation projection.
    // Reproduce just that repair: no new terminal claim or graph-index advance.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hdfd751863d9a owner=st3_missions::store::fleet_admission_tests TODO=prepare-body-and-predecessor-values */
        "UPDATE operations SET state='conflict' WHERE id IN
         (SELECT json_extract(body, '$._operation.id') FROM claims WHERE subject=?1)", [subject],
    ).unwrap();
    assert_eq!(store.index().unwrap(), before);
    assert!(changes.has_changed().unwrap());
    assert_eq!(store.runtime_authority(subject).unwrap().unwrap().reachability, "indeterminate");
}

#[test]
fn terminal_authority_idle_subscription_skips_reads_and_delivers_changes() {
    let store = Store::open_memory("owner").unwrap();
    store.set_write_clock_at(10).unwrap();
    let subject = "agent/terminal-idle";
    let observe = |incarnation: &str| {
        store.append_claim(&crate::model::ClaimInput {
            subject: subject.into(),
            kind: "runtime.observed".into(),
            actor: Some(subject.into()),
            fields: serde_json::from_value(json!({
                "status": "running", "runtime_id": "terminal-idle",
                "incarnation_id": incarnation, "terminal": true,
            })).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
    };
    observe("one");
    let mut changes = store.subscribe_runtime_authority(subject);
    let mut second_viewer = store.subscribe_runtime_authority(subject);
    assert_eq!(store.runtime_authority(subject).unwrap().unwrap().actual.unwrap()["incarnation_id"], "one");
    changes.borrow_and_update();
    second_viewer.borrow_and_update();
    let work = smallclaims::sqlite::work::SqliteWorkScope::start();
    for _ in 0..100 {
        assert!(!changes.has_changed().unwrap(), "idle streams must not recheck authority");
    }
    assert_eq!(work.finish().statements, 0);
    append(&store, "resource/other", 20, json!({"state": "open"}));
    assert!(!changes.has_changed().unwrap(), "unrelated subjects must not wake a terminal");
    observe("two");
    assert!(changes.has_changed().unwrap());
    assert!(second_viewer.has_changed().unwrap());
    changes.borrow_and_update();
    assert_eq!(store.runtime_authority(subject).unwrap().unwrap().actual.unwrap()["incarnation_id"], "two");
    assert!(!changes.has_changed().unwrap());
}

}
