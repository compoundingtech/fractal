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

#[test]
fn paged_capture_resolves_committed_writer_loan_before_certifying() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("graph.db"), "example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-writer-resolution", json!([]));
    let at = sent + 2;
    frontier(&store, at - 1);
    std::thread::scope(|scope| {
        let (write, write_now) = std::sync::mpsc::channel();
        let (committed, commit_done) = std::sync::mpsc::channel();
        let (finished, capture_done) = std::sync::mpsc::channel();
        let store = &store;
        let writer = scope.spawn(move || {
            write_now.recv().unwrap();
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            crate::store::mail_input_acceptance::record_frontier(&transaction,
                "agent/example/worker", "incarnation-one", at, at).unwrap();
            transaction.commit().unwrap();
            committed.send(()).unwrap();
            // Keep the already-committed loan alive. Its native generation has not
            // published yet; certification must wait for resolution, not just COMMIT.
            let returned_before_resolution = capture_done.recv_timeout(std::time::Duration::from_secs(1)).is_ok();
            drop(writer);
            returned_before_resolution
        });
        let mut requested = false;
        let snapshot = capture_with_progress(store, at, || {
            if requested { return; }
            requested = true;
            write.send(()).unwrap();
            commit_done.recv().unwrap();
        }).unwrap();
        let _ = finished.send(());
        assert!(!writer.join().unwrap(), "capture certified before the committed writer loan resolved");
        assert!(snapshot.unknown > 0, "the resolved generation must fence the mixed paged read");
    });
}

#[test]
fn runtime_identity_inputs_during_capture_keep_coverage_unknown() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-concurrent-graph", json!([]));
    frontier(&store, sent);
    let mut writes = 0;
    let snapshot = capture_with_progress(&store, sent, || {
        writes += 1;
        store.append_claim(&ClaimInput {
            subject: "agent/example-unrelated".into(), kind: "runtime.observed".into(),
            actor: Some("agent/example-unrelated".into()),
            fields: serde_json::from_value(json!({"status":"running",
                "runtime_id":"example-unrelated-runtime","incarnation_id":format!("unrelated-{writes}")})).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
    }).unwrap();
    assert!(writes > 0);
    assert!(store.index().unwrap() > snapshot.source_cut);
    assert_eq!(snapshot.source_work, [1, 0, 0]);
    assert_eq!(snapshot.unknown, 1);
    let publication = store.extension::<Publication>();
    let (_sender, receiver) = tokio::sync::watch::channel(0);
    *publication.changed.lock().unwrap() = Some(receiver);
    *publication.snapshot.lock().unwrap() = Some(snapshot);
    assert_eq!(rows(&store, sent).unwrap()[0]["status"], "no-data");
    // A later capture without overlapping writes can recover.
    assert_eq!(capture(&store, sent).unwrap().unknown, 0);
}

#[test]
fn incremental_mail_work_is_changed_message_only_and_frontiers_reuse_source_facts() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let mut at = 0;
    for index in 0..64 {
        let message = format!("message/example-incremental-{index}");
        at = send(&store, &message, json!([])).max(at);
    }
    at += 1;
    frontier(&store, at);
    store.seal_local_batches().unwrap();
    let mut objective = Incremental::default();
    let initial = objective.capture(&store, at).unwrap();
    assert_eq!(initial.samples.len(), 64);
    let baseline = work_counts(&store);
    assert_eq!(baseline.full_rebuilds, 1);
    assert_eq!(baseline.messages_evaluated, 64);

    drop(store.connection.write());
    store.append_claim(&ClaimInput {
        subject: "agent/example-unrelated-runtime".into(), kind: "runtime.observed".into(),
        actor: None, fields: serde_json::from_value(json!({"status":"running","incarnation_id":"unrelated"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.seal_local_batches().unwrap();
    for sql in [
        /* sqlite-large-value: allow-for-now lv-h264ac1d5dbbf owner=fractal_api_core::mail_delivery_objectives_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO desired(subject,kind,revision,claim_id,body) VALUES('resource/mail-unrelated','resource','revision/test',(SELECT id FROM claims LIMIT 1),'{}')",
        /* sqlite-large-value: allow-for-now lv-h16edf8e42749 owner=fractal_api_core::mail_delivery_objectives_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET body='{\"changed\":true}' WHERE subject='resource/mail-unrelated'",
        /* sqlite-large-value: allow-for-now lv-h08e8cdfb5399 owner=fractal_api_core::mail_delivery_objectives_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject='resource/mail-unrelated'",
    ] {
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h574ba13a3a53 owner=fractal_api_core::mail_delivery_objectives_tests TODO=trace-statement-and-value-representation */ sql, []).unwrap();
        objective.capture(&store, at).unwrap();
    }
    store.forget_current_views();
    objective.capture(&store, at).unwrap();
    let unrelated = work_counts(&store);
    assert_eq!(unrelated.full_rebuilds, baseline.full_rebuilds);
    assert_eq!(unrelated.messages_evaluated, baseline.messages_evaluated);
    assert_eq!(unrelated.source_rows, baseline.source_rows);
    assert_eq!(unrelated.owner_lookups, baseline.owner_lookups);
    assert_eq!(unrelated.lifecycle_rows, baseline.lifecycle_rows);

    let closed = step(&store, "message/example-incremental-17", "message.closed");
    at = at.max(closed);
    let changed = objective.capture(&store, at).unwrap();
    let related = work_counts(&store);
    assert_eq!(related.full_rebuilds, 1);
    assert_eq!(related.messages_evaluated - unrelated.messages_evaluated, 1);
    assert_eq!(related.source_rows - unrelated.source_rows, 1);
    assert_eq!(related.lifecycle_rows - unrelated.lifecycle_rows, 1);
    assert_eq!(changed.samples.iter().filter(|sample| matches!(sample.terminal, LifecycleTerminal::Failed(_))).count(), 1);

    frontier(&store, at + 1);
    let refreshed = objective.capture(&store, at + 1).unwrap();
    let heartbeat = work_counts(&store);
    assert_eq!(heartbeat.full_rebuilds, 1);
    assert_eq!(heartbeat.messages_evaluated - related.messages_evaluated, 64);
    assert_eq!(heartbeat.source_rows, related.source_rows);
    assert_eq!(heartbeat.owner_lookups - related.owner_lookups, 1);
    assert_eq!(heartbeat.acceptance_rows, related.acceptance_rows);
    assert_eq!(heartbeat.lifecycle_rows, related.lifecycle_rows);
    assert_eq!(refreshed.unknown, 0);
    assert_eq!(refreshed.source_through_ms, at + 1);
    assert_eq!(refreshed.samples.len(), 64);

    let stale = objective.capture(&store, at + FRESHNESS + 2).unwrap();
    assert!(stale.unknown > 0, "a retained reduction must not extend native coverage");
    let expired = work_counts(&store);
    assert_eq!(expired.source_rows, heartbeat.source_rows);
    assert_eq!(expired.acceptance_rows, heartbeat.acceptance_rows);
    assert_eq!(expired.lifecycle_rows, heartbeat.lifecycle_rows);
}

#[test]
fn incremental_retention_expires_completed_messages_but_preserves_old_open_mail() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let sent = send(&store, "message/example-incremental-retained", json!([]));
    send(&store, "message/example-incremental-completed", json!([]));
    let closed = step(&store, "message/example-incremental-completed", "message.closed");
    let at = closed.max(sent) + 1;
    frontier(&store, at);
    let mut objective = Incremental::default();
    assert_eq!(objective.capture(&store, at).unwrap().samples.len(), 2);
    let before = work_counts(&store);
    let expiry = at + DEADLINE + HORIZON + FRESHNESS + 1;
    frontier(&store, expiry);
    let snapshot = objective.capture(&store, expiry).unwrap();
    assert_eq!(snapshot.samples.len(), 1);
    assert!(matches!(snapshot.samples[0].terminal, LifecycleTerminal::Open));
    assert_eq!(snapshot.samples[0].onset_ms, sent);
    assert_eq!(snapshot.unknown, 0);
    let after = work_counts(&store);
    assert_eq!(after.full_rebuilds, before.full_rebuilds);
    assert_eq!(after.source_rows, before.source_rows);
    assert_eq!(after.lifecycle_rows, before.lifecycle_rows);
    assert_eq!(objective.messages.len(), 1);
}

#[test]
fn incremental_shard_coverage_fences_without_rebuilding_the_population() {
    let directory = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(Store::open(&directory.path().join("graph.db"), "example-host").unwrap());
    declared(&store);
    let shard = crate::store::messages_shard::open(&store, directory.path()).unwrap();
    let mut objective = Incremental::default();
    objective.capture(&store, now()).unwrap();
    let before = work_counts(&store);
    let sent = shard.append_claim(&ClaimInput {
        subject: "message/example-incremental-shard".into(), kind: "message.sent".into(),
        actor: Some("agent/example/sender".into()),
        fields: serde_json::from_value(json!({"from":"agent/example/sender","to":"agent/example/worker",
            "status":"sent","content":"synthetic shard input"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let at = u64::try_from(sent.accepted_at_unix_ms).unwrap() + 1;
    assert!(objective.capture(&store, at).unwrap().unknown > 0, "unmirrored admitted mail invalidates coverage immediately");
    let unmirrored = work_counts(&store);
    assert_eq!(unmirrored.full_rebuilds, before.full_rebuilds);
    assert_eq!(unmirrored.messages_evaluated, before.messages_evaluated);
    assert_eq!(unmirrored.source_rows, before.source_rows);
    crate::store::stream_mirror::mirror(&store).unwrap();
    frontier(&store, at);
    let mirrored = objective.capture(&store, at).unwrap();
    let after = work_counts(&store);
    assert_eq!(after.full_rebuilds, before.full_rebuilds);
    assert_eq!(after.messages_evaluated - before.messages_evaluated, 1);
    assert_eq!(after.source_rows - before.source_rows, 1);
    assert_eq!(mirrored.samples.len(), 1);
    assert_eq!(mirrored.unknown, 0);
}

#[test]
fn incremental_mail_known_deletions_only_recheck_the_affected_message() {
    let store = Store::open_memory("example-host").unwrap();
    declared(&store);
    let first = send(&store, "message/example-delete-first", json!([]));
    let second = send(&store, "message/example-delete-second", json!([]));
    let closed = step(&store, "message/example-delete-first", "message.closed");
    let at = first.max(second).max(closed) + 1;
    frontier(&store, at);
    let mut objective = Incremental::default();
    assert_eq!(objective.capture(&store, at).unwrap().samples.len(), 2);
    let before = work_counts(&store);
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hd7f010cccc43 owner=fractal_api_core::mail_delivery_objectives_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mail_delivery_steps_v1 WHERE message='message/example-delete-first'", [],
    ).unwrap();
    let after = objective.capture(&store, at).unwrap();
    let work = work_counts(&store);
    assert_eq!(work.full_rebuilds, before.full_rebuilds);
    assert_eq!(work.messages_evaluated - before.messages_evaluated, 1);
    assert_eq!(after.samples.len(), 2);
    assert!(after.samples.iter().all(|sample| matches!(sample.terminal, LifecycleTerminal::Open)));
}
