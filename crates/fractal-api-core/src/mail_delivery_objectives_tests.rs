use super::*;
use crate::model::{ClaimInput, IntentInput};
use crate::DeliveryStore;

fn declared(store: &Store) {
    let source = r#"version 2
agent "example/worker" {
    rollout "manual"
    host "example-host"
    workspace "/tmp"
    harness "claude" { model "fixture"; }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "example-host").unwrap();
    let plan = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &plan.subject_tokens, "declare").unwrap();
    for (kind, fields) in [
        ("runtime.observed", json!({"status":"running","runtime_id":"example-worker","incarnation_id":"incarnation-one"})),
        ("harness.observed", json!({"state":"ready","driver":"claude","incarnation_id":"incarnation-one"})),
    ] {
        store.append_claim(&ClaimInput {
            subject: "agent/example/worker".into(), kind: kind.into(), actor: Some("agent/example/worker".into()),
            fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
            expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    store.bind_mailbox(&crate::client::Fence::new("agent/example/worker", "incarnation-one", "delivery")).unwrap();
}
fn send(store: &Store, message: &str, tags: Value) -> u64 {
    let sent = store.append_claim(&ClaimInput {
        subject: message.into(), kind: "message.sent".into(), actor: Some("agent/example/sender".into()),
        fields: serde_json::from_value(json!({"from":"agent/example/sender","to":"agent/example/worker",
            "status":"sent","content":"synthetic mail","tags":tags})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: Some(format!("fixture-send-{message}")),
    }).unwrap();
    u64::try_from(sent.accepted_at_unix_ms).unwrap()
}
fn step(store: &Store, message: &str, kind: &str) -> u64 {
    let mut fields = BTreeMap::from([("status".into(), json!(kind.trim_start_matches("message.")))]);
    if kind == "message.staged" {
        fields.insert("recipient".into(), json!("agent/example/worker"));
    }
    let receipt = store.append_claim(&ClaimInput {
        subject: message.into(), kind: kind.into(),
        actor: Some(if kind == "message.closed" { "daemon/runtime" } else { "agent/example/worker" }.into()),
        fields, evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    u64::try_from(receipt.accepted_at_unix_ms).unwrap()
}
fn frontier(store: &Store, through: u64) {
    crate::store::mail_input_acceptance::record_frontier(&store.connection.write(),
        "agent/example/worker", "incarnation-one", through, through).unwrap();
}
fn publish(store: &Store, at: u64) -> Value {
    frontier(store, at);
    let publication = store.extension::<Publication>();
    let (_sender, receiver) = tokio::sync::watch::channel(0);
    *publication.changed.lock().unwrap() = Some(receiver);
    *publication.snapshot.lock().unwrap() = Some(capture(store, at).unwrap());
    rows(store, at).unwrap().remove(0)
}

#[test]
fn open_deadline_and_late_native_acceptance_preserve_one_failure() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-open", json!([]));
    assert_eq!(publish(&store, sent + DEADLINE + 1)["evidence"]["bad"], 1.0);
    crate::store::mail_input_acceptance::record(&store.connection.write(), "message/example-open",
        "agent/example/worker", "incarnation-one", sent + DEADLINE + 2, sent + DEADLINE + 3).unwrap();
    let row = publish(&store, sent + DEADLINE + 4);
    assert_eq!(row["evidence"]["bad"], 1.0);
    assert_eq!(row["evidence"]["total"], 1.0);
}

#[test]
fn terminal_undelivered_is_bad_but_generic_delivery_is_unknown() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-closed", json!([]));
    step(&store, "message/example-closed", "message.closed");
    assert_eq!(publish(&store, sent + DEADLINE + 1)["evidence"]["bad"], 1.0);
    send(&store, "message/example-asserted", json!([]));
    step(&store, "message/example-asserted", "message.delivered");
    let row = publish(&store, sent + DEADLINE + 2);
    assert_eq!(row["status"], "no-data");
    assert_eq!(row["source"]["unknown"], 1);
}

#[test]
fn explicit_hold_and_unknown_owner_never_become_good_delivery() {
    let store = Store::open_memory("example-host").unwrap();
    let sent = send(&store, "message/example-held", json!(["st3-silent"]));
    assert_eq!(publish(&store, sent + DEADLINE + 1)["status"], "no-data");
    declared(&store);
    let row = publish(&store, sent + DEADLINE + 2);
    assert_eq!(row["source"]["held"], 1);
    assert_eq!(row["evidence"]["total"], 0.0);
    step(&store, "message/example-held", "message.closed");
    assert_eq!(publish(&store, sent + DEADLINE + 3)["evidence"]["bad"], 1.0);
}

#[test]
fn restart_replays_unfinished_source_and_does_not_create_acceptance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.db");
    let sent;
    {
        let store = Store::open(&path, "example-host").unwrap();
        declared(&store);
        sent = send(&store, "message/example-survivor", json!([]));
    }
    let store = Store::open(&path, "example-host").unwrap();
    assert_eq!(rows(&store, sent + DEADLINE + 1).unwrap()[0]["status"], "no-data");
    let row = publish(&store, sent + DEADLINE + 2);
    assert_eq!(row["evidence"]["bad"], 1.0);
    assert_eq!(row["source"]["coverage_known"], true);
}

#[test]
fn freshness_and_pending_source_changes_cannot_recover() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-freshness", json!([]));
    publish(&store, sent + 1);
    assert_eq!(rows(&store, sent + FRESHNESS + 2).unwrap()[0]["status"], "no-data");
    let publication = store.extension::<Publication>();
    let (sender, receiver) = tokio::sync::watch::channel(0);
    *publication.changed.lock().unwrap() = Some(receiver);
    sender.send_replace(1);
    assert_eq!(rows(&store, sent + 2).unwrap()[0]["status"], "no-data");
}

#[test]
fn uncertain_historical_delivery_has_a_one_shot_horizon_expiry() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-unknown-expiry", json!([]));
    step(&store, "message/example-unknown-expiry", "message.delivered");
    frontier(&store, sent + DEADLINE + 1);
    let snapshot = capture(&store, sent + DEADLINE + 1).unwrap();
    assert_eq!(snapshot.unknown, 1);
    let expiry = sent + DEADLINE + HORIZON + FRESHNESS + 1;
    assert!(snapshot.recapture_at_ms.is_some_and(|at| at <= expiry));
    assert_eq!(capture(&store, expiry).unwrap().unknown, 0);
}

#[test]
fn stream_source_and_terminal_clock_replay_through_the_mirror() {
    let directory = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(Store::open(&directory.path().join("graph.db"), "example-host").unwrap());
    declared(&store);
    let shard = crate::store::messages_shard::open(&store, directory.path()).unwrap();
    let sent = shard.append_claim(&ClaimInput {
        subject: "message/example-stream".into(), kind: "message.sent".into(),
        actor: Some("agent/example/sender".into()),
        fields: serde_json::from_value(json!({"status":"sent","from":"agent/example/sender",
            "to":"agent/example/worker","content":"synthetic stream input"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    crate::store::messages_shard::record_lifecycle(&store, &sent.subject, "message.closed",
        Some("daemon/runtime"), &BTreeMap::from([("status".into(), json!("closed"))])).unwrap();
    crate::store::stream_mirror::mirror(&store).unwrap();
    let at = u64::try_from(sent.accepted_at_unix_ms).unwrap() + DEADLINE + 1;
    frontier(&store, at);
    let snapshot = capture(&store, at).unwrap();
    assert_eq!(snapshot.unknown, 0);
    assert_eq!(snapshot.samples.len(), 1);
    assert!(matches!(snapshot.samples[0].terminal, LifecycleTerminal::Failed(_)));
    let row = publish(&store, at);
    assert_eq!(row["evidence"]["bad"], 1.0);
    assert_eq!(row["evidence"]["total"], 1.0);
    crate::store::stream_mirror::mirror(&store).unwrap();
    assert_eq!(publish(&store, at)["evidence"]["total"], 1.0);
}

#[test]
fn retried_send_retains_one_onset_and_never_stores_message_content() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-retry", json!([]));
    assert_eq!(send(&store, "message/example-retry", json!([])), sent);
    frontier(&store, sent + DEADLINE + 1);
    let snapshot = capture(&store, sent + DEADLINE + 1).unwrap();
    assert_eq!(snapshot.samples.len(), 1);
    assert_eq!(snapshot.samples[0].onset_ms, sent);
    let fields: String = store.readers.get().query_row(
        "SELECT fields FROM local_mail_delivery_source_v1 WHERE message='message/example-retry'",
        [], |row| row.get(0),
    ).unwrap();
    assert!(!fields.contains("synthetic mail"));
    assert!(serde_json::from_str::<Value>(&fields).unwrap().get("content").is_none());
}

#[test]
fn acceptance_with_an_inconsistent_source_clock_is_unknown_not_a_failure() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-clock", json!([]));
    crate::store::mail_input_acceptance::record(&store.connection.write(), "message/example-clock",
        "agent/example/worker", "incarnation-one", sent - 1, sent + 1).unwrap();
    let row = publish(&store, sent + DEADLINE + 1);
    assert_eq!(row["status"], "no-data");
    assert_eq!(row["evidence"]["bad"], 0.0);
    assert_eq!(row["source"]["unknown"], 1);
}

#[test]
fn a_held_message_becomes_eligible_at_its_durable_release_edge() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-release", json!(["st3-silent"]));
    frontier(&store, sent + 1);
    assert_eq!(capture(&store, sent + 1).unwrap().held, 1);
    let released = step(&store, "message/example-release", "message.staged");
    frontier(&store, released + DEADLINE + 1);
    let snapshot = capture(&store, released + DEADLINE + 1).unwrap();
    assert_eq!(snapshot.held, 0);
    assert_eq!(snapshot.samples.len(), 1);
    assert_eq!(snapshot.samples[0].onset_ms, sent);
    assert!(matches!(snapshot.samples[0].terminal, LifecycleTerminal::Open));
}

#[test]
fn missing_driver_frontier_cannot_manufacture_an_overdue_failure() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-no-frontier", json!([]));
    let snapshot = capture(&store, sent + DEADLINE + 1).unwrap();
    assert_eq!(snapshot.unknown, 1);
    assert!(snapshot.samples.is_empty());
    frontier(&store, sent - 1);
    let lagging = capture(&store, sent + DEADLINE + 1).unwrap();
    assert!(lagging.unknown > 0);
    assert!(lagging.samples.is_empty());
}

#[test]
fn lagging_fresh_frontier_keeps_the_accounting_horizon_failure() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-horizon-edge", json!([]));
    let accepted = sent + DEADLINE + 1;
    crate::store::mail_input_acceptance::record(&store.connection.write(),
        "message/example-horizon-edge", "agent/example/worker", "incarnation-one",
        accepted, accepted).unwrap();
    let at = sent + DEADLINE + HORIZON + 30_000;
    let cut = at - 60_000;
    frontier(&store, cut);
    let snapshot = capture(&store, at).unwrap();
    assert_eq!(snapshot.source_through_ms, cut);
    assert_eq!(snapshot.samples.len(), 1);
    let publication = store.extension::<Publication>();
    let (_sender, receiver) = tokio::sync::watch::channel(0);
    *publication.changed.lock().unwrap() = Some(receiver);
    *publication.snapshot.lock().unwrap() = Some(snapshot);
    let row = rows(&store, at).unwrap().remove(0);
    assert_eq!(row["evidence"]["bad"], 1.0);
    assert_eq!(row["evidence"]["total"], 1.0);
    assert_eq!(row["status"], "exhausted");
}

#[test]
fn accounting_clock_stops_at_the_native_frontier_not_the_daemon_heartbeat() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-accounting-cut", json!([]));
    let cut = sent + DEADLINE - 1;
    let at = sent + DEADLINE + 1;
    frontier(&store, cut);
    let publication = store.extension::<Publication>();
    let (_sender, receiver) = tokio::sync::watch::channel(0);
    *publication.changed.lock().unwrap() = Some(receiver);
    *publication.snapshot.lock().unwrap() = Some(capture(&store, at).unwrap());
    let row = rows(&store, at).unwrap().remove(0);
    assert_eq!(row["source"]["coverage_known"], true);
    assert_eq!(row["source"]["covered_through_unix_ms"], cut);
    assert_eq!(row["evidence"]["bad"], 0.0);
    assert_eq!(row["evidence"]["as_of_unix_ms"], cut);
    let later = cut + FRESHNESS + 1;
    publication.snapshot.lock().unwrap().as_mut().unwrap().observed_ms = later;
    let stale = rows(&store, later).unwrap().remove(0);
    assert_eq!(stale["status"], "no-data");
    assert_eq!(stale["source"]["coverage_known"], false);
}

#[test]
fn expired_audit_history_is_not_materialized_on_unrelated_events() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-archived", json!([]));
    let accepted = step(&store, "message/example-archived", "message.delivered");
    {
        let connection = store.connection.write();
        for index in 0..512 {
            let message = format!("message/example-audit-{index}");
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h7f3a847a2da6 owner=fractal_api_core::mail_delivery_objectives_tests TODO=bound-or-reference-values-and-reclamation */
                "INSERT INTO local_mail_delivery_source_v1 VALUES(?1,'agent/example/worker',?2,
                 '{\"from\":\"agent/example/sender\",\"tags\":[],\"sent_clock_valid\":1}')",
                rusqlite::params![message, sent],
            ).unwrap();
            crate::store::mail_input_acceptance::record(
                &connection, &message, "agent/example/worker", "incarnation-one", accepted, accepted,
            ).unwrap();
            crate::store::mail_delivery_source::stream_step(
                &connection, &message, "agent/example/worker", "message.delivered", accepted,
            ).unwrap();
        }
    }
    let expired = sent + DEADLINE + HORIZON + FRESHNESS + 1;
    assert_eq!(capture(&store, expired).unwrap().source_work, [0, 0, 0]);
    store.append_claim(&ClaimInput {
        subject: "agent/example-unrelated".into(), kind: "runtime.observed".into(),
        actor: Some("agent/example-unrelated".into()),
        fields: serde_json::from_value(json!({"status":"running",
            "runtime_id":"example-unrelated-runtime","incarnation_id":"unrelated-one"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert_eq!(capture(&store, expired).unwrap().source_work, [0, 0, 0]);
    let current = send(&store, "message/example-current-population", json!([]));
    frontier(&store, current);
    let snapshot = capture(&store, current).unwrap();
    assert_eq!(snapshot.source_work[0], 514);
    // At the real current clock these fixtures are in-window; advancing the accounting
    // horizon discards all completed audit history but retains the unresolved message.
    let snapshot = capture(&store, expired).unwrap();
    assert_eq!(snapshot.source_work, [1, 0, 0]);
}

#[tokio::test]
async fn unrelated_notifications_and_claims_do_not_recapture() {
    let directory = tempfile::tempdir().unwrap();
    let state = crate::api::tests_shared::state(directory.path());
    let publication = state.store.extension::<Publication>();
    start(&state);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while publication.snapshot.lock().unwrap().is_none() {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert!(publication.snapshot.lock().unwrap().as_ref().unwrap().error.is_none());
    let generation = *publication.changed.lock().unwrap().as_ref().unwrap().borrow();
    for _ in 0..128 {
        crate::api::signal_local_change(&state);
    }
    state.store.append_claim(&ClaimInput {
        subject: "resource/example/unrelated".into(), kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.test.unrelated"))]), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap();
    for _ in 0..16 { tokio::task::yield_now().await; }
    {
        let changed = publication.changed.lock().unwrap();
        assert_eq!(*changed.as_ref().unwrap().borrow(), generation);
        assert!(!changed.as_ref().unwrap().has_changed().unwrap(),
            "unrelated input cannot schedule a capture even after this task yields");
        assert_eq!(publication.captures.load(Ordering::Acquire), 1);
    }
    send(&state.store, "message/example-real-source-wake", json!([]));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let complete = publication.snapshot.lock().unwrap().as_ref()
                .is_some_and(|snapshot| snapshot.source_work[0] == 1);
            if complete { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert_eq!(publication.captures.load(Ordering::Acquire), 2,
        "a genuine mail source change wakes without a global notification");
}

#[test]
fn capture_releases_readers_between_pages_and_lifecycle_messages() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("graph.db"), "example-host").unwrap();
    declared(&store);
    let mut sent = 0;
    for index in 0..SOURCE_PAGE + 1 {
        sent = send(&store, &format!("message/checkpoint-{index:04}"), json!([]));
    }
    frontier(&store, sent);
    let mut checkpoints = 0;
    let snapshot = capture_with_progress(&store, sent, || {
        // A real write after each read ensures RESTART must pass the active
        // reader, rather than succeeding vacuously on an empty WAL.
        let connection = store.connection.write();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h5024471c81ae owner=fractal_api_core::mail_delivery_objectives_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('mail_checkpoint_test',?1)",
            [checkpoints.to_string()]).unwrap();
        let (busy, frames, checkpointed): (i64, i64, i64) = connection.query_row(
            "PRAGMA wal_checkpoint(RESTART)", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(busy, 0, "capture pinned a reader at boundary {checkpoints}");
        assert!(frames > 0);
        assert_eq!(frames, checkpointed);
        checkpoints += 1;
    }).unwrap();
    assert_eq!(snapshot.samples.len(), SOURCE_PAGE + 1);
    assert_eq!(snapshot.unknown, 0);
    assert!(checkpoints >= SOURCE_PAGE + 5, "must traverse multiple source pages and every message");
}

#[test]
fn paged_capture_uses_mail_input_cut_not_unrelated_graph_index() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-source-cut", json!([]));
    let at = sent + 2;
    frontier(&store, at - 1);
    let mut inserted = false;
    let snapshot = capture_with_progress(&store, at, || {
        if inserted { return; }
        inserted = true;
        store.append_claim(&ClaimInput {
            subject: "resource/example/capture-unrelated".into(), kind: "resource.observed".into(), actor: None,
            fields: BTreeMap::from([("kind".into(), json!("custom.test.unrelated"))]), evidence: Vec::new(),
            expected_subject: None, idempotency_key: None,
        }).unwrap();
    }).unwrap();
    assert!(inserted);
    assert_eq!(snapshot.unknown, 0, "unrelated graph writes cannot poison mail coverage");
    let mut advanced = false;
    let snapshot = capture_with_progress(&store, at, || {
        if advanced { return; }
        advanced = true;
        frontier(&store, at);
    }).unwrap();
    assert!(advanced);
    assert!(snapshot.unknown > 0, "relevant writes still fence a mixed paged capture");
}
