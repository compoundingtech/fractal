use super::*;
use super::message_send_tests::operation_rows;
use super::tests_shared::{TEST_FLEET, exchange_from};
use super::tests_shared_seats::graph_digest_of;

#[test]
fn operation_conflict_receipts_match_full_replay_in_opposite_orders() {
    for kind in ["custom.test.observed", "intent.desired"] {
        let sources = [Store::open_memory("early").unwrap(), Store::open_memory("late").unwrap()];
        let mut ids = Vec::new();
        let subject = if kind == "intent.desired" { "exec/conflict" } else { "custom/test/conflict" };
        for (number, source) in sources.iter().enumerate() {
            let mut connection = source.connection.write();
            let tx = connection.transaction().unwrap();
            let mut body = if kind == "intent.desired" {
                json!({"subject":"exec/conflict","kind":"exec","desired":{"number":number}})
            } else { json!({"fields":{"number":number}}) };
            body["_operation"] = json!({"id":"op/conflict-parity",
                "request_digest":if number == 0 { "digest-z" } else { "digest-a" }});
            ids.push(append_claim_record_tx(&tx, &source.origin, subject, kind,
                None, &body, &[], None).unwrap().id);
            tx.commit().unwrap();
        }
        let mut answers = Vec::new();
        for reverse in [false, true] {
            let target = Store::open_memory(if reverse { "receiver-b" } else { "receiver-a" }).unwrap();
            target.bind_fleet(TEST_FLEET).unwrap();
            target.project_replication_backlog().unwrap();
            let mut logs = Vec::new();
            FULL_REPLAYS.with(|count| count.set(0));
            for index in if reverse { [1, 0] } else { [0, 1] } {
                let source = &sources[index];
                let exchange = exchange_from(source, &target.replication_inventory().unwrap());
                target.receive_replication_exchange(&source.origin, TEST_FLEET, &exchange).unwrap();
                target.validate_replication_backlog().unwrap();
                assert!(target.claim_by_id(&ids[index]).unwrap().is_some(), "{kind}: claim must be admitted");
                target.project_replication_backlog_with_log("conflict-test", |line| logs.push(line.to_owned())).unwrap();
            }
            assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "{kind}/{reverse}");
            let incremental = (operation_rows(&target.readers.get()), graph_digest_of(&target));
            target.replay_replication_graph().unwrap();
            assert_eq!(incremental, (operation_rows(&target.readers.get()), graph_digest_of(&target)));
            assert_eq!(target.idempotency_conflicts(1).unwrap().0, 1);
            if kind == "intent.desired" {
                let health = target.replication_status(false, None, &[]).unwrap();
                let conflict = health.unhealthy.iter().find(|row| row.error_code.as_deref() == Some("operation-conflict")).unwrap();
                let detail: Value = serde_json::from_str(conflict.error_message.as_ref().unwrap()).unwrap();
                assert_eq!(detail["projection_operation_id"], "op/conflict-parity");
                assert!(ids.iter().any(|id| detail["projection_claim_id"] == *id));
                let log = logs.iter().find_map(|line| line.strip_prefix("fractal: projection failure detail ")).unwrap();
                let log: Value = serde_json::from_str(log).unwrap();
                assert_eq!(log["code"], "operation-conflict");
                assert_eq!(log["operation_id"], "op/conflict-parity");
                assert!(ids.iter().any(|id| log["previous_claim_id"] == *id));
            } else {
                assert!(logs.iter().all(|line| !line.contains("projection failure detail")));
            }
            answers.push(incremental);
        }
        assert_eq!(answers[0], answers[1], "{kind}: arrival order changed the projection");
    }
}

#[test]
fn operation_conflict_above_refold_cap_preserves_graph_and_reports_degraded() {
    let store = Store::open_memory("fixture").unwrap();
    let operation_id = format!("op/large-conflict-{}", "x".repeat(300));
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..=OPERATION_CONFLICT_REFOLD_CLAIMS {
            let mut body = json!({"subject":"exec/large-conflict","kind":"exec","desired":{"number":number}});
            if number == 0 {
                body["_operation"] = json!({"id":operation_id,"request_digest":"digest-z"});
            }
            append_claim_record_tx(&tx, "peer", "exec/large-conflict", "intent.desired", None,
                &body, &[], None).unwrap();
        }
        append_claim_record_tx(&tx, "peer", "custom/later-conflict", "custom.test.observed", None,
            &json!({"fields":{},"_operation":{"id":"op/later-event","request_digest":"a"}}), &[], None).unwrap();
        tx.commit().unwrap();
    }
    store.replay_replication_graph().unwrap();
    let desired = || store.readers.get().query_row(
        "SELECT claim_id,body FROM desired WHERE subject='exec/large-conflict'",
        [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).unwrap();
    let before = desired();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "exec/large-conflict", "intent.desired", None,
            &json!({"subject":"exec/large-conflict","kind":"exec","desired":{"number":"new"},
                "_operation":{"id":operation_id,"request_digest":"digest-a"}}), &[], None).unwrap();
        append_claim_record_tx(&tx, "peer", "custom/later-conflict", "custom.test.observed", None,
            &json!({"fields":{},"_operation":{"id":"op/later-event","request_digest":"b"}}), &[], None).unwrap();
        tx.commit().unwrap();
    }
    FULL_REPLAYS.with(|count| count.set(0));
    let mut logs = Vec::new();
    store.project_replication_backlog_with_log("large-conflict", |line| logs.push(line.to_owned())).unwrap();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert_eq!(before, desired());
    let health = store.replication_status(false, None, &[]).unwrap();
    assert!(health.unhealthy.iter().any(|row| row.status == "stale"
        && row.error_code.as_deref() == Some("operation-conflict-refold-limit")
        && row.aggregate.contains(&operation_id)));
    assert_eq!(store.idempotency_conflicts(2).unwrap().0, 2);
    assert!(health.unhealthy.iter().all(|row| !row.aggregate.contains("op/later-event")));
    let detail = logs.iter().find_map(|line| line.strip_prefix("fractal: projection failure detail ")).unwrap();
    let detail: Value = serde_json::from_str(detail).unwrap();
    assert_eq!(detail["projection_degraded"], true);
    assert_eq!(detail["refold_error_code"], "operation-conflict-refold-limit");
    store.project_replication_backlog().unwrap();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    let stale_key = health.unhealthy.iter().find(|row| row.status == "stale").unwrap().aggregate.clone();
    let stale_message = health.unhealthy.iter().find(|row| row.aggregate == stale_key).unwrap().error_message.clone();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "exec/another-subject", "intent.desired", None,
            &json!({"subject":"exec/another-subject","kind":"exec","desired":{},
                "_operation":{"id":operation_id,"request_digest":"digest-b"}}), &[], None).unwrap();
        tx.commit().unwrap();
    }
    store.project_replication_backlog().unwrap();
    let health = store.replication_status(false, None, &[]).unwrap();
    let original = health.unhealthy.iter().find(|row| row.aggregate == stale_key).unwrap();
    assert_eq!(original.status, "stale");
    assert_eq!(original.error_message, stale_message);
    assert_eq!(before, desired());
}

#[test]
fn operation_conflict_run_chunk_dispatch_never_defers_an_unbounded_refold() {
    for same_receive in [false, true] {
        assert_run_conflict_remains_bounded(same_receive);
    }
}

fn assert_run_conflict_remains_bounded(same_receive: bool) {
    fn ordinary(tx: &Transaction<'_>, subject: &str) {
        let claim = append_claim_record_tx(tx, "peer", subject, "mission-run.state", None,
            &json!({"fields":{"status":"failed"}}), &[], None).unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h02c178e28e3f owner=fractal_store::store::operation_conflict_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='0' WHERE id=?1", [&claim.id]).unwrap();
    }
    let store = Store::open_memory("fixture").unwrap();
    super::tests_shared_seats::publish_mission(&store, r#"version 2
mission "bounded-run" state="ready" {
  goal "Exercise conflict dispatch."
  step "program" { agentless }
}"#, "bounded-run");
    let run = store.create_mission_run_inner(&crate::model::MissionRunRequest {
        mission: "bounded-run".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/operator".into()), mode: None,
        inputs: Default::default(), idempotency_key: "bounded-run-creation".into(),
    }, None, None, None).unwrap();
    let operation = "op/conflicting-run-final-drain";
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..=OPERATION_CONFLICT_REFOLD_CLAIMS {
            append_claim_record_tx(&tx, "peer", &run.subject, "custom.test.observed", None,
                &json!({"fields":{"number":number}}), &[], None).unwrap();
        }
        append_claim_record_tx(&tx, "peer", &run.subject, "mission-run.state", None,
            &json!({"fields":{"state":"running"},"_operation":{"id":operation,"request_digest":"z"}}),
            &[], None).unwrap();
        tx.commit().unwrap();
    }
    store.replay_replication_graph().unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", &run.subject, "mission-run.state", None,
            &json!({"fields":{"state":"running"},"_operation":{"id":operation,"request_digest":"a"}}),
            &[], None).unwrap();
        if same_receive {
            for number in 0..smallclaims::store::PROJECTION_CHUNK_CLAIMS {
                append_claim_record_tx(&tx, "peer", "custom/test/run-conflict-padding", "custom.test.observed",
                    None, &json!({"fields":{"number":number}}), &[], None).unwrap();
            }
            ordinary(&tx, &run.subject);
        }
        tx.commit().unwrap();
    }
    FULL_REPLAYS.with(|count| count.set(0));
    // The production driver dispatches project_incremental_chunk, then final-drain work.
    // A conflict run must finish its bounded/savepoint attempt before that generic queue.
    let projected_tables = || {
        let mut tables = smallclaims::store::projection_digest::tables(&store.readers.get()).unwrap();
        // Admission changes claim_sources even when every projected row stays frozen.
        // Compare every derived table, not the root that also commits those new claims.
        tables.remove("claim_sources");
        tables
    };
    let observed = std::cell::RefCell::new(None);
    let between = || {
        if observed.borrow().is_none() {
            let health = store.replication_status(false, None, &[]).unwrap();
            if let Some(row) = health.unhealthy.into_iter().find(|row| row.aggregate.contains(operation)
                && row.error_code.as_deref() == Some("operation-conflict-refold-limit")) {
                *observed.borrow_mut() = Some((projected_tables(), row.error_message));
                RUN_TREE_REBUILDS.with(|count| count.set(0));
            }
        }
    };
    store.project_replication_backlog_with_yield(between).unwrap();
    between();
    if !same_receive {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        ordinary(&tx, &run.subject);
        tx.commit().unwrap();
        drop(connection);
        store.project_replication_backlog_with_yield(between).unwrap();
    }
    between();
    assert_eq!(RUN_TREE_REBUILDS.with(std::cell::Cell::get), 0,
        "later ordinary claims must not rebuild an already-degraded run");
    let (before_graph, before_diagnostic) = observed.into_inner().unwrap();
    assert_eq!(projected_tables(), before_graph);
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    let health = store.replication_status(false, None, &[]).unwrap();
    assert!(health.unhealthy.iter().any(|row| row.aggregate.contains(operation)
        && row.error_code.as_deref() == Some("operation-conflict-refold-limit")));
    assert_eq!(health.unhealthy.iter().find(|row| row.aggregate.contains(operation)).unwrap().error_message,
        before_diagnostic);
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    assert!(!projection_work_pending_tx(&tx).unwrap());
    finish_projection_work_tx(&tx).unwrap();
    assert!(!projection_work_pending_tx(&tx).unwrap());
    assert_eq!(rebuild_run_tree_tx(&tx, &run.id).unwrap_err().code, "operation-conflict-run-degraded");
    assert_eq!(rebuild_run_part_tx(&tx, &run.id).unwrap_err().code, "operation-conflict-run-degraded");
    tx.rollback().unwrap();
    drop(connection);
    store.replay_replication_graph().unwrap();
    assert!(!run_conflict_degraded_tx(&store.readers.get(), &run.id).unwrap());
}

#[test]
fn operation_conflict_refuses_a_guard_requested_full_replay() {
    for checkpoint_only in [false, true] {
        let store = Store::open_memory("fixture").unwrap();
        store.project_replication_backlog().unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            if checkpoint_only {
                tx.execute(/* sqlite-large-value: allow-for-now lv-hed1452e59184 owner=fractal_store::store::operation_conflict_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id,writer,sequence,envelope_hash,subject,kind,
                    actor,predecessors,operation_id,request_digest,accepted_at_unix_ms,checkpoint)
                    VALUES ('tombstone','peer',1,'envelope','exec/conflict','intent.desired',NULL,'[]',
                    'op/guard-conflict','a',0,'checkpoint/2026-10-01')", []).unwrap();
            } else {
                append_claim_record_tx(&tx, "peer", "exec/conflict", "intent.desired",
                    None, &json!({"subject":"exec/conflict","kind":"exec","desired":{},"_operation":{"id":"op/guard-conflict","request_digest":"a"}}),
                    &[], None).unwrap();
            }
            append_claim_record_tx(&tx, "peer", "exec/conflict", "intent.desired",
                None, &json!({"subject":"exec/conflict","kind":"exec","desired":{},"_operation":{"id":"op/guard-conflict","request_digest":"b"}}),
                &[], None).unwrap();
            tx.execute(/* sqlite-large-value: allow-for-now lv-hf161f602d783 owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
            tx.commit().unwrap();
        }
        FULL_REPLAYS.with(|count| count.set(0));
        assert!(store.project_replication_backlog().unwrap());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
        assert!(store.replication_status(false, None, &[]).unwrap().unhealthy.iter().any(|row|
            row.error_code.as_deref() == Some("operation-conflict-replay-deferred")));
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            append_claim_record_tx(&tx, "peer", "exec/after-checkpoint-conflict", "intent.desired", None,
                &json!({"subject":"exec/after-checkpoint-conflict","kind":"exec","desired":{}}),
                &[], None).unwrap();
            tx.commit().unwrap();
        }
        assert!(store.project_replication_backlog().unwrap());
        assert!(store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM desired WHERE subject='exec/after-checkpoint-conflict')",
            [], |row| row.get::<_, bool>(0),
        ).unwrap());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
        assert!(store.replication_projection_needs_recovery().unwrap());
        let health = store.replication_status(false, None, &[]).unwrap();
        assert!(health.unhealthy.iter().any(|row| row.aggregate == "graph" && row.status == "stale"));
        assert!(health.unhealthy.iter().any(|row| row.aggregate == "projection:deferred-replay"));
        let before_repair = (operation_rows(&store.readers.get()), graph_digest_of(&store));
        store.replay_replication_graph().unwrap();
        assert!(!store.replication_projection_needs_recovery().unwrap());
        assert!(!store.replication_status(false, None, &[]).unwrap().unhealthy.iter()
            .any(|row| row.aggregate == "projection:deferred-replay"));
        assert_eq!(before_repair, (operation_rows(&store.readers.get()), graph_digest_of(&store)));
        // A repaired historical conflict must not poison a later legitimate fallback.
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            assert!(!tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate GLOB 'projection:operation-conflict-fence:*')",
                [], |row| row.get::<_, bool>(0),
            ).unwrap());
            append_claim_record_tx(&tx, "peer", "exec/after-full-repair", "intent.desired", None,
                &json!({"subject":"exec/after-full-repair","kind":"exec","desired":{}}),
                &[], None).unwrap();
            require_replay_after_repair(&tx);
            tx.commit().unwrap();
        }
        let mut logs = Vec::new();
        assert!(store.project_replication_backlog_with_log("after-full-repair", |line| logs.push(line.to_owned())).unwrap());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 1);
        assert!(!store.replication_projection_needs_recovery().unwrap());
        assert!(!logs.iter().any(|line| line.contains("operation-conflict-projection-degraded")));
        assert!(!store.replication_status(false, None, &[]).unwrap().unhealthy.iter()
            .any(|row| row.aggregate == "projection:deferred-replay"));
    }
}

#[test]
fn operation_conflict_github_event_observers_never_replay_during_receipt() {
    let subject = "github-event/github/acme/garden/pull-request/7/checks/abc/fail";
    let sources = [Store::open_memory("node-a").unwrap(), Store::open_memory("node-b").unwrap()];
    let mut claims = Vec::new();
    for (number, source) in sources.iter().enumerate() {
        claims.push(source.append_claim(&ClaimInput {
            subject: subject.into(), kind: fractal_schema::github_plugin::EVENT.into(), actor: None,
            fields: BTreeMap::from([
                ("version".into(), json!(fractal_schema::github_plugin::VERSION)),
                ("item".into(), json!("resource/github/acme/garden/pull-request/7")),
                ("event".into(), json!("checks-failed")),
                ("observer".into(), json!(format!("observer/github/{}", source.origin))),
                ("at".into(), json!(format!("2026-11-10T23:0{number}:00.000Z"))),
                ("detail".into(), json!({"observer":source.origin})),
                ("head_sha".into(), json!(format!("head-{number}"))),
            ]),
            evidence: Vec::new(), expected_subject: None,
            idempotency_key: Some(format!("github-event:{subject}")),
        }).unwrap());
    }
    assert_eq!(claims[0].operation_id, claims[1].operation_id);
    let mut answers = Vec::new();
    for reverse in [false, true] {
        let target = Store::open_memory("receiver").unwrap();
        target.bind_fleet(TEST_FLEET).unwrap();
        target.project_replication_backlog().unwrap();
        FULL_REPLAYS.with(|count| count.set(0));
        for index in if reverse { [1, 0] } else { [0, 1] } {
            let source = &sources[index];
            let exchange = exchange_from(source, &target.replication_inventory().unwrap());
            target.receive_replication_exchange(&source.origin, TEST_FLEET, &exchange).unwrap();
            target.validate_replication_backlog().unwrap();
            target.project_replication_backlog().unwrap();
        }
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
        let before = (operation_rows(&target.readers.get()), graph_digest_of(&target));
        target.replay_replication_graph().unwrap();
        assert_eq!(before, (operation_rows(&target.readers.get()), graph_digest_of(&target)));
        answers.push(before);
    }
    assert_eq!(answers[0], answers[1]);
}

#[test]
fn operation_conflict_in_second_receipt_chunk_fences_first_chunk_fallback() {
    let source = Store::open_memory("source").unwrap();
    {
        let mut connection = source.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..smallclaims::store::PROJECTION_CHUNK_CLAIMS {
            append_claim_record_tx(&tx, "source", "custom/test/multi-chunk", "custom.test.observed",
                None, &json!({"fields":{"number":number}}), &[], None).unwrap();
        }
        for digest in ["a", "b"] {
            append_claim_record_tx(&tx, "source", "exec/multi-chunk-conflict", "intent.desired",
                None, &json!({"subject":"exec/multi-chunk-conflict","kind":"exec","desired":{},"_operation":{"id":"op/second-chunk","request_digest":digest}}),
                &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    let target = Store::open_memory("target").unwrap();
    target.bind_fleet(TEST_FLEET).unwrap();
    // Replication exports bounded pages; admit every page before testing projection chunks.
    loop {
        let exchange = exchange_from(&source, &target.replication_inventory().unwrap());
        if exchange.envelopes.is_empty() { break; }
        target.receive_replication_exchange("source", TEST_FLEET, &exchange).unwrap();
        target.validate_replication_backlog().unwrap();
    }
    assert!(target.index().unwrap() > smallclaims::store::PROJECTION_CHUNK_CLAIMS as u64);
    target.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hf0bbe1f00bc2 owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'", [],
    ).unwrap();
    FULL_REPLAYS.with(|count| count.set(0));
    let scanned_target = target.index().unwrap();
    let injected = std::cell::Cell::new(false);
    target.project_replication_backlog_with_yield(|| {
        if injected.replace(true) { return; }
        let mut connection = target.connection.write();
        let tx = connection.transaction().unwrap();
        for digest in ["a", "b"] {
            append_claim_record_tx(&tx, "peer", "exec/after-target", "intent.desired", None,
                &json!({"subject":"exec/after-target","kind":"exec","desired":{},"_operation":{"id":"op/after-target","request_digest":digest}}),
                &[], None).unwrap();
        }
        tx.commit().unwrap();
    }).unwrap();
    let watermark = || target.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='projection:operation-conflict-scan'",
        [], |row| row.get::<_, u64>(0),
    ).unwrap();
    let after_target_discovered = || target.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate='projection:operation-conflict-fence:op/after-target')",
        [], |row| row.get::<_, bool>(0),
    ).unwrap();
    assert_eq!(watermark(), scanned_target);
    assert!(!after_target_discovered());
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert!(target.replication_status(false, None, &[]).unwrap().unhealthy.iter().any(|row|
        row.error_code.as_deref() == Some("operation-conflict-replay-deferred")));
    {
        let mut connection = target.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "exec/after-deferred", "intent.desired", None,
            &json!({"subject":"exec/after-deferred","kind":"exec","desired":{"value":"projected"}}),
            &[], None).unwrap();
        tx.commit().unwrap();
    }
    target.project_replication_backlog().unwrap();
    assert_eq!(watermark(), target.index().unwrap());
    assert!(after_target_discovered());
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert!(target.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM desired WHERE subject='exec/after-deferred')",
        [], |row| row.get::<_, bool>(0),
    ).unwrap());
}

#[test]
fn operation_conflict_refold_failure_restores_every_prior_desired_column() {
    let store = Store::open_memory("fixture").unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "exec/refold-failure", "intent.desired", None,
            &json!({"subject":"exec/refold-failure","kind":"exec","desired":{"value":"prior"},
                "_operation":{"id":"op/refold-failure","request_digest":"z"}}), &[], None).unwrap();
        append_claim_record_tx(&tx, "peer", "exec/refold-failure", "intent.desired",
            None, &json!({}), &[], None).unwrap();
        tx.commit().unwrap();
    }
    store.replay_replication_graph().unwrap();
    let bytes = || store.readers.get().query_row(
        "SELECT json_array(subject,kind,revision,claim_id,body,member,owner_run,owner_generation,owner_step)
         FROM desired WHERE subject='exec/refold-failure'", [], |row| row.get::<_, String>(0),
    ).unwrap().into_bytes();
    let before = bytes();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "exec/refold-failure", "intent.desired", None,
            &json!({"subject":"exec/refold-failure","kind":"exec","desired":{"value":"new"},
                "_operation":{"id":"op/refold-failure","request_digest":"a"}}), &[], None).unwrap();
        tx.commit().unwrap();
    }
    FULL_REPLAYS.with(|count| count.set(0));
    store.project_replication_backlog().unwrap();
    assert_eq!(before, bytes());
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert!(store.replication_status(false, None, &[]).unwrap().unhealthy.iter().any(|row|
        row.status == "stale" && row.aggregate.contains("op/refold-failure")));
}

#[test]
fn operation_conflict_owned_set_exclusion_survives_later_receipt_chunk() {
    let store = Store::open_memory("fixture").unwrap();
    store.project_replication_backlog().unwrap();
    {
        let connection = store.readers.get();
        let mut plan = connection.prepare(
            "EXPLAIN QUERY PLAN SELECT aggregate FROM projection_health INDEXED BY projection_health_degraded_operation_subject
             WHERE status='stale' AND error_code='operation-conflict-no-bounded-refold'
             AND aggregate GLOB 'projection:operation-conflict:*'",
        ).unwrap();
        let steps = plan.query_map([], |row| row.get::<_, String>(3)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        assert!(steps.iter().any(|step| step.contains("projection_health_degraded_operation_subject")));
    }
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let body = json!({"subject":"exec/excluded-member","kind":"exec","desired":{"value":"excluded"},
            "owned_set":"owned-set/excluded"});
        let desired: DesiredSubject = serde_json::from_value(body.clone()).unwrap();
        let member = append_claim_record_tx(&tx, "peer", "exec/excluded-member", "intent.desired",
            None, &body, &[], None).unwrap();
        let revision = owned_sets::Revision {
            rollout: None, previous: None, policy: None,
            source: owned_sets::Source { repository:"acme/garden".into(), r#ref:"main".into(), sha:"abc".into(), sequence:1 },
            bundle_digest:"bundle".into(),
            members:BTreeMap::from([("exec/excluded-member".into(), owned_sets::Member {
                manual_rollout:false, one_shot:false, kind:"exec".into(), claim:member.id,
                revision:desired_revision(&desired),
            })]),
            retired:BTreeMap::new(), adoptions:BTreeMap::new(),
        };
        for digest in ["a", "b"] {
            append_claim_record_tx(&tx, "peer", "owned-set/excluded", "owned-set.revised", None,
                &json!({"fields":{"revision":canonical_hash(&revision).unwrap(),"body":revision},
                    "_operation":{"id":"op/excluded-set","request_digest":digest}}), &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    FULL_REPLAYS.with(|count| count.set(0));
    let absent = || !store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM desired WHERE subject='exec/excluded-member')",
        [], |row| row.get::<_, bool>(0),
    ).unwrap();
    assert!(store.project_replication_backlog().unwrap());
    assert!(absent());
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "custom/unrelated-set-event", "custom.test.observed",
            None, &json!({"fields":{}}), &[], None).unwrap();
        tx.commit().unwrap();
    }
    assert!(store.project_replication_backlog().unwrap());
    assert!(absent(), "an unrelated later chunk must not repair the excluded set");
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    // The set really is projectable: only an explicit repair may apply its member.
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    owned_sets::project_tx(&tx).unwrap();
    tx.commit().unwrap();
    assert!(!absent());
}

#[test]
fn operation_conflict_checkpoint_adoption_invalidates_discovery_during_projection() {
    let store = Store::open_memory("fixture").unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "exec/late-checkpoint", "intent.desired", None,
            &json!({"subject":"exec/late-checkpoint","kind":"exec","desired":{},"_operation":{"id":"op/late-checkpoint","request_digest":"b"}}),
            &[], None).unwrap();
        tx.commit().unwrap();
    }
    store.project_replication_backlog().unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..=smallclaims::store::PROJECTION_CHUNK_CLAIMS {
            append_claim_record_tx(&tx, "peer", "custom/checkpoint-race", "custom.test.observed", None,
                &json!({"fields":{"number":number}}), &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    let target = store.index().unwrap();
    let adopted = std::cell::Cell::new(false);
    FULL_REPLAYS.with(|count| count.set(0));
    store.project_replication_backlog_with_yield(|| {
        if adopted.replace(true) { return; }
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h386f61abc319 owner=fractal_store::store::operation_conflict_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id,writer,sequence,envelope_hash,subject,kind,
            actor,predecessors,operation_id,request_digest,accepted_at_unix_ms,checkpoint)
            VALUES ('late-tombstone','peer',1,'envelope','exec/late-checkpoint','intent.desired',
                NULL,'[]','op/late-checkpoint','a',0,'checkpoint/2026-10-01')", []).unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h69417713b2cb owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
        tx.commit().unwrap();
    }).unwrap();
    assert!(adopted.get());
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert!(!smallclaims::store::operation_conflict_discovery_valid(&store.readers.get(), target).unwrap());
    assert!(store.replication_projection_needs_recovery().unwrap());
    // The next reader pass invalidates the old watermark and discovers the new tombstone,
    // even though no new claim index was admitted after the preceding scan.
    store.project_replication_backlog().unwrap();
    assert!(smallclaims::store::operation_conflict_discovery_valid(&store.readers.get(), target).unwrap());
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
}

#[test]
fn operation_conflict_neutral_history_does_not_fence_unrelated_replay() {
    let store = Store::open_memory("fixture").unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for digest in ["a", "b"] {
            append_claim_record_tx(&tx, "peer", "agent/example", "harness.observed", None,
                &json!({"fields":{"state":"ready"},"_operation":{"id":"op/neutral-replay","request_digest":digest}}),
                &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    store.project_replication_backlog().unwrap();
    assert_eq!(store.idempotency_conflicts(1).unwrap().0, 1);
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, "peer", "exec/needs-replay", "intent.desired", None,
            &json!({"subject":"exec/needs-replay","kind":"exec","desired":{}}), &[], None).unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h0cfeef50b49a owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
        tx.commit().unwrap();
    }
    FULL_REPLAYS.with(|count| count.set(0));
    assert!(store.project_replication_backlog().unwrap());
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 1);
    assert!(!store.replication_projection_needs_recovery().unwrap());
    let projected = (operation_rows(&store.readers.get()), graph_digest_of(&store));
    store.replay_replication_graph().unwrap();
    assert_eq!(projected, (operation_rows(&store.readers.get()), graph_digest_of(&store)));
}

#[test]
fn operation_conflict_partial_rediscovery_below_graph_frontier_keeps_receipt_moving() {
    let store = Store::open_memory("fixture").unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..smallclaims::store::PROJECTION_CHUNK_CLAIMS {
            append_claim_record_tx(&tx, "peer", "custom/baseline", "custom.test.observed", None,
                &json!({"fields":{"number":number}}), &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    store.project_replication_backlog().unwrap();
    let frontier = || store.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| row.get::<_, u64>(0),
    ).unwrap();
    assert!(frontier() > 1);
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..=smallclaims::store::PROJECTION_CHUNK_CLAIMS {
            let subject = format!("exec/partial-proof-{number}");
            append_claim_record_tx(&tx, "peer", &subject, "intent.desired", None,
                &json!({"subject":subject,"kind":"exec","desired":{}}), &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    let target = store.index().unwrap();
    let interrupted = std::cell::Cell::new(false);
    FULL_REPLAYS.with(|count| count.set(0));
    assert!(store.project_replication_backlog_with_yield(|| {
        if interrupted.replace(true) { return; }
        assert!(frontier() > 1);
        // Model rediscovery of a new checkpoint generation committing only its early page,
        // below the graph frontier. That partial proof cannot authorize a whole-log replay.
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h792cc1f9b935 owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='operation_conflict_source_generation';
             UPDATE projection_health SET last_good_store_index=1,
               error_message=(SELECT value FROM meta WHERE key='operation_conflict_source_generation')
             WHERE aggregate='projection:operation-conflict-scan';
             UPDATE projection_health SET status='stale' WHERE aggregate='graph';",
        ).unwrap();
        tx.commit().unwrap();
    }).unwrap());
    assert!(interrupted.get());
    assert_eq!(frontier(), target, "receipt must advance beyond the partial discovery watermark");
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert!(store.replication_projection_needs_recovery().unwrap());
    assert!(store.replication_status(false, None, &[]).unwrap().unhealthy.iter()
        .any(|row| row.aggregate == "projection:deferred-replay"));
    // Rediscovery now finishes without graph-projecting conflicts. One authorized full
    // repair consumes the deferred row; a quiet subsequent catch-up must not replay again.
    FULL_REPLAYS.with(|count| count.set(0));
    assert!(store.project_replication_backlog().unwrap());
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 1);
    assert!(!store.replication_projection_needs_recovery().unwrap());
    assert!(!store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate='projection:deferred-replay')",
        [], |row| row.get::<_, bool>(0),
    ).unwrap());
    FULL_REPLAYS.with(|count| count.set(0));
    store.project_replication_backlog().unwrap();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
}

fn require_replay_after_repair(tx: &Transaction<'_>) {
    tx.execute(/* sqlite-large-value: allow-for-now lv-h7772a8dd1107 owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
}

#[test]
fn degraded_run_settles_sibling_pending_work_and_does_not_block_startup_repair() {
    let store = Store::open_memory("fixture").unwrap();
    super::tests_shared_seats::publish_mission(&store, r#"version 2
mission "pending-conflict" state="ready" {
  concurrent-runs max=4
  authority { start "pending-conflict"; }
  goal "Drain independent run work."
  step "program" { agentless }
}"#, "publish-pending-conflict");
    let start = |key: &str, parent: Option<&MissionRunView>| {
        let child = parent.map(|parent| ChildMissionContext {
            root_revision: parent.root_revision.clone(),
            root_run_id: parent.id.clone(),
            parent_step_run: parent.steps[0].subject.clone(),
            default_selector: None, started_by: Some(parent.subject.clone()),
        });
        store.create_mission_run_inner(&crate::model::MissionRunRequest {
            mission: "pending-conflict".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("person/operator".into()), mode: None, inputs: Default::default(),
            idempotency_key: key.into(),
        }, child, None, None).unwrap()
    };
    let root = start("pending-root", None);
    let first = start("pending-ordinary", Some(&root));
    let second = start("pending-conflicting", Some(&root));
    // Run IDs are hashes, not creation-ordered. Dispatch sorts parts by their IDs:
    // force ordinary work to enqueue before its sibling establishes the root fence.
    let (ordinary, conflicting) = if first.id < second.id { (first, second) } else { (second, first) };
    assert!(ordinary.id < conflicting.id, "pending sibling must dispatch before conflicting sibling");
    let unrelated = start("pending-unrelated", None);
    let operation = "op/pending-sibling-conflict";
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..=OPERATION_CONFLICT_REFOLD_CLAIMS {
            append_claim_record_tx(&tx, "peer", &conflicting.subject, "custom.test.observed", None,
                &json!({"fields":{"number":number}}), &[], None).unwrap();
        }
        append_claim_record_tx(&tx, "peer", &conflicting.subject, "mission-run.state", None,
            &json!({"fields":{"state":"running"},"_operation":{"id":operation,"request_digest":"z"}}),
            &[], None).unwrap();
        tx.commit().unwrap();
    }
    store.replay_replication_graph().unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        // Ordinary sibling work enters the pending queue before the bounded conflict
        // attempt fences their shared root. Independent root work must still drain.
        for run in [&ordinary, &unrelated] {
            let claim = append_claim_record_tx(&tx, "peer", &run.subject, "mission-run.state", None,
                &json!({"fields":{"status":"running"}}), &[], None).unwrap();
            tx.execute(/* sqlite-large-value: allow-for-now lv-hf851bfc39d1f owner=fractal_store::store::operation_conflict_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='0' WHERE id=?1", [&claim.id]).unwrap();
        }
        append_claim_record_tx(&tx, "peer", &conflicting.subject, "mission-run.state", None,
            &json!({"fields":{"state":"running"},"_operation":{"id":operation,"request_digest":"a"}}),
            &[], None).unwrap();
        tx.commit().unwrap();
    }
    RUN_TREE_REBUILDS.with(|count| count.set(0));
    let saw_fenced_pending_sibling = std::cell::Cell::new(false);
    store.project_replication_backlog_with_yield(|| {
        if !saw_fenced_pending_sibling.get() {
            let connection = store.readers.get();
            let pending = pending_run_rebuilds_tx(&connection).unwrap();
            if run_conflict_degraded_tx(&connection, &root.id).unwrap() && !pending.is_empty() {
                assert!(pending.contains(&run_part_of_tx(&connection, &ordinary.subject).unwrap().unwrap()));
                assert!(pending.contains(&run_part_of_tx(&connection, &unrelated.subject).unwrap().unwrap()));
                saw_fenced_pending_sibling.set(true);
            }
        }
    }).unwrap();
    assert!(saw_fenced_pending_sibling.get(), "exercise the already-queued sibling path");
    assert_eq!(RUN_TREE_REBUILDS.with(std::cell::Cell::get), 2,
        "only the bounded conflict attempt and independent root rebuild may run");
    assert!(store.replication_status(false, None, &[]).unwrap().unhealthy.iter().any(|row|
        row.aggregate.contains(operation) && row.error_code.as_deref() == Some("operation-conflict-refold-limit")));
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    assert!(!projection_work_pending_tx(&tx).unwrap());
    assert!(run_conflict_degraded_tx(&tx, &root.id).unwrap());
    let body: String = tx.query_row(
        "SELECT body FROM claims WHERE subject=?1 AND kind='mission-run.created' LIMIT 1",
        [&root.subject], |row| row.get(0),
    ).unwrap();
    let mut body: Value = serde_json::from_str(&body).unwrap();
    body["startup_fixture"] = json!(true);
    append_claim_record_tx(&tx, "peer", &root.subject, "mission-run.created", None, &body, &[], None).unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-hec42c211f7cb owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='occurrence_creation_projection_rules'", []).unwrap();
    RUN_TREE_REBUILDS.with(|count| count.set(0));
    migrate_occurrence_creation_projections_tx(&tx).unwrap();
    assert_eq!(RUN_TREE_REBUILDS.with(std::cell::Cell::get), 0);
    assert!(run_conflict_degraded_tx(&tx, &root.id).unwrap());
    tx.rollback().unwrap();
}
