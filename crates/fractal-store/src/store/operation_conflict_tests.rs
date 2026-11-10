use super::*;
use super::message_send_tests::operation_rows;
use super::tests_shared::{TEST_FLEET, exchange_from};
use super::tests_shared_seats::graph_digest_of;

#[test]
fn operation_conflict_receipts_match_full_replay_in_opposite_orders() {
    for kind in ["custom.test.observed", "intent.desired"] {
        let sources = [Store::open_memory("early").unwrap(), Store::open_memory("late").unwrap()];
        let mut ids = Vec::new();
        for (number, source) in sources.iter().enumerate() {
            let mut connection = source.connection.write();
            let tx = connection.transaction().unwrap();
            let mut body = if kind == "intent.desired" {
                json!({"subject":"exec/conflict","kind":"exec","desired":{"number":number}})
            } else { json!({"fields":{"number":number}}) };
            body["_operation"] = json!({"id":"op/conflict-parity",
                "request_digest":if number == 0 { "digest-z" } else { "digest-a" }});
            ids.push(append_claim_record_tx(&tx, &source.origin, "exec/conflict", kind,
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
                target.project_replication_backlog_with_log("conflict-test", |line| logs.push(line.to_owned())).unwrap();
            }
            assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "{kind}/{reverse}");
            let incremental = (operation_rows(&target.readers.get()), graph_digest_of(&target));
            target.replay_replication_graph().unwrap();
            assert_eq!(incremental, (operation_rows(&target.readers.get()), graph_digest_of(&target)));
            let health = target.replication_status(false, None, &[]).unwrap();
            let conflict = health.unhealthy.iter().find(|row| row.error_code.as_deref() == Some("operation-conflict")).unwrap();
            let detail: Value = serde_json::from_str(conflict.error_message.as_ref().unwrap()).unwrap();
            assert_eq!(detail["operation_conflict_count"], 1);
            assert_eq!(detail["projection_operation_id"], "op/conflict-parity");
            assert!(ids.iter().any(|id| detail["projection_claim_id"] == *id));
            let log = logs.iter().find_map(|line| line.strip_prefix("fractal: projection failure detail ")).unwrap();
            let log: Value = serde_json::from_str(log).unwrap();
            assert_eq!(log["code"], "operation-conflict");
            assert_eq!(log["operation_conflict_count"], 1);
            assert_eq!(log["operation_id"], "op/conflict-parity");
            assert!(ids.iter().any(|id| log["previous_claim_id"] == *id));
            answers.push(incremental);
        }
        assert_eq!(answers[0], answers[1], "{kind}: arrival order changed the projection");
    }
}

#[test]
fn operation_conflict_above_refold_cap_preserves_graph_and_reports_degraded() {
    let store = Store::open_memory("fixture").unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..=OPERATION_CONFLICT_REFOLD_CLAIMS {
            let mut body = json!({"subject":"exec/large-conflict","kind":"exec","desired":{"number":number}});
            if number == 0 {
                body["_operation"] = json!({"id":"op/large-conflict","request_digest":"digest-z"});
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
                "_operation":{"id":"op/large-conflict","request_digest":"digest-a"}}), &[], None).unwrap();
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
        && row.error_message.as_deref().unwrap().contains("op/large-conflict")));
    assert!(health.unhealthy.iter().any(|row| row.aggregate.ends_with("op/later-event")
        && row.status == "indeterminate" && row.error_code.as_deref() == Some("operation-conflict")));
    let detail = logs.iter().find_map(|line| line.strip_prefix("fractal: projection failure detail ")).unwrap();
    let detail: Value = serde_json::from_str(detail).unwrap();
    assert_eq!(detail["projection_degraded"], true);
    assert_eq!(detail["refold_error_code"], "operation-conflict-refold-limit");
    store.project_replication_backlog().unwrap();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
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
                tx.execute(/* sqlite-large-value: allow-for-now lv-hc0649a7fe6c7 owner=fractal_store::store::operation_conflict_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id,writer,sequence,envelope_hash,subject,kind,
                    actor,predecessors,operation_id,request_digest,accepted_at_unix_ms,checkpoint)
                    VALUES ('tombstone','peer',1,'envelope','custom/conflict','custom.test.observed',NULL,'[]',
                    'op/guard-conflict','a',0,'checkpoint/2026-10-01')", []).unwrap();
            } else {
                append_claim_record_tx(&tx, "peer", "custom/conflict", "custom.test.observed",
                    None, &json!({"fields":{},"_operation":{"id":"op/guard-conflict","request_digest":"a"}}),
                    &[], None).unwrap();
            }
            append_claim_record_tx(&tx, "peer", "custom/conflict", "custom.test.observed",
                None, &json!({"fields":{},"_operation":{"id":"op/guard-conflict","request_digest":"b"}}),
                &[], None).unwrap();
            tx.execute(/* sqlite-large-value: allow-for-now lv-hf161f602d783 owner=fractal_store::store::operation_conflict_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
            tx.commit().unwrap();
        }
        FULL_REPLAYS.with(|count| count.set(0));
        assert!(!store.project_replication_backlog().unwrap());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
        assert!(store.replication_status(false, None, &[]).unwrap().unhealthy.iter().any(|row|
            row.error_code.as_deref() == Some("operation-conflict-projection-degraded")));
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
                ("detail".into(), json!(format!("observed by {}", source.origin))),
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
