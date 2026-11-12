//! Signed admission retains one committed message/signature and no temporary expectation.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_send_tests_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_send_tests_shared_delivery::{assert_operation_replay_parity, operation_rows, request, signed};
use super::*;


#[test]
fn message_operation_reconciliation_matches_replay_for_orders_tombstones_and_repairs() {
    for reverse in [false, true] {
        for mode in ["stored", "checkpoint", "repaired"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("invented-parity.db");
            let store = Store::open(&path, "fixture").unwrap();
            store.project_replication_backlog().unwrap();
            let digests = if reverse {
                ["digest-b", "digest-a"]
            } else {
                ["digest-a", "digest-b"]
            };
            let mut first = None;
            for (number, digest) in digests.into_iter().enumerate() {
                let mut connection = store.connection.write();
                let tx = connection.transaction().unwrap();
                if number == 1 {
                    let claim: &ClaimRecord = first.as_ref().unwrap();
                    if mode == "checkpoint" {
                        tx.execute(/* sqlite-large-value: allow-for-now lv-hd7cd14b1d0a5 owner=fractal_store::store::message_send_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id,writer,sequence,envelope_hash,subject,kind,
                            actor,predecessors,operation_id,request_digest,accepted_at_unix_ms,checkpoint)
                            VALUES (?1,'invented-peer',1,'invented-envelope',?2,'message.sent',NULL,'[]',
                            'op/invented-parity',?3,0,'checkpoint/2026-10-01')",
                            params![claim.id, claim.subject, digests[0]]).unwrap();
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h0cfe32b4b70b owner=fractal_store::store::message_send_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations WHERE id='op/invented-parity'", [])
                            .unwrap();
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h9e78aaf35467 owner=fractal_store::store::message_send_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&claim.id])
                            .unwrap();
                    } else if mode == "repaired" {
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h1e6ea42f1255 owner=fractal_store::store::message_send_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,
                            raw,state,claim_id,updated_at_unix_ms) VALUES ('record/invented-repair','invented-peer',
                            1,'invented-envelope',0,X'','repaired',?1,'1')", [&claim.id]).unwrap();
                    }
                }
                let claim = append_claim_record_tx(&tx, "invented-peer", "message/invented-parity",
                    "message.sent", Some("person/example"), &json!({"fields":{
                        "from":"person/example", "to":"agent/invented", "content":"Invented receipt", "status":"sent"
                    }, "_operation":{"id":"op/invented-parity", "request_digest":digest}}), &[], None).unwrap();
                tx.commit().unwrap();
                drop(connection);
                if number == 0 {
                    first = Some(claim);
                }
                FULL_REPLAYS.with(|count| count.set(0));
                store.project_replication_backlog().unwrap();
                assert_eq!(
                    FULL_REPLAYS.with(std::cell::Cell::get),
                    0,
                    "{reverse}/{mode}/{number}"
                );
                assert_operation_replay_parity(&store);
            }
            let expected_state = if mode == "repaired" {
                "active"
            } else {
                "conflict"
            };
            assert_eq!(
                operation_tx(&store.readers.get(), "op/invented-parity")
                    .unwrap()
                    .unwrap()
                    .2,
                expected_state
            );
            let before = operation_rows(&store.readers.get());
            drop(store);
            let reopened = Store::open(&path, "fixture").unwrap();
            FULL_REPLAYS.with(|count| count.set(0));
            reopened.project_replication_backlog().unwrap();
            assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
            assert_eq!(operation_rows(&reopened.readers.get()), before);
            // Another receipt exercises the keyed reducer with a retained tombstone,
            // including a minimum digest for which no stored claim remains.
            {
                let mut connection = reopened.connection.write();
                let tx = connection.transaction().unwrap();
                append_claim_record_tx(&tx, "invented-other-peer", "message/invented-parity",
                    "message.sent", Some("person/example"), &json!({"fields":{
                        "from":"person/example", "to":"agent/invented", "content":"Another receipt", "status":"sent"
                    }, "_operation":{"id":"op/invented-parity", "request_digest":digests[1]}}), &[], None).unwrap();
                tx.commit().unwrap();
            }
            reopened.project_replication_backlog().unwrap();
            assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
            assert_eq!(
                operation_tx(&reopened.readers.get(), "op/invented-parity")
                    .unwrap()
                    .unwrap()
                    .2,
                expected_state
            );
            assert_operation_replay_parity(&reopened);
        }
    }
}

#[test]
fn message_operation_conflict_receipt_cost_depends_on_its_own_history() {
    use smallclaims::sqlite::work::SqliteWorkScope;
    let mut costs = Vec::new();
    for count in [2, 10, 100] {
        let store = Store::open_memory("fixture").unwrap();
        store.project_replication_backlog().unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            for number in 0..count {
                append_claim_record_tx(&tx, "invented-peer", "message/invented-cost", "message.sent",
                    Some("person/example"), &json!({"fields":{"from":"person/example",
                        "to":"agent/invented", "content":format!("Invented content {number}"), "status":"sent"},
                        "_operation":{"id":"op/invented-cost", "request_digest":format!("digest-{number:03}")}}),
                    &[], None).unwrap();
            }
            tx.commit().unwrap();
        }
        store.project_replication_backlog().unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            append_claim_record_tx(
                &tx,
                "invented-peer",
                "message/invented-cost",
                "message.sent",
                Some("person/example"),
                &json!({"fields":{"from":"person/example",
                    "to":"agent/invented", "content":"Another receipt", "status":"sent"},
                    "_operation":{"id":"op/invented-cost", "request_digest":"digest-001"}}),
                &[],
                None,
            )
            .unwrap();
            tx.commit().unwrap();
        }
        FULL_REPLAYS.with(|replays| replays.set(0));
        let work = SqliteWorkScope::start();
        store.project_replication_backlog().unwrap();
        let cost = work.finish();
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
        eprintln!(
            "message conflict receipt prior_claims={count} statements={} vm_steps={} fullscan_steps={}",
            cost.statements, cost.vm_steps, cost.fullscan_steps
        );
        assert!(cost.vm_steps > 0);
        costs.push(cost);
    }
    assert_eq!(costs[0].statements, costs[1].statements);
    assert_eq!(costs[1].statements, costs[2].statements);
    assert!(
        costs[2].vm_steps < costs[0].vm_steps * 50,
        "one indexed fold should grow linearly, not repeat a whole-store replay per receipt"
    );
}

#[test]
fn graph_neutral_operation_conflicts_match_replay_in_both_orders_and_after_checkpoint() {
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let subject = "github-event/github/acme/garden/pull-request/7/checks/failed";
    let sources = [
        Store::open_memory("node-a").unwrap(),
        Store::open_memory("node-b").unwrap(),
    ];
    let claims = sources.iter().enumerate().map(|(index, source)| {
        source.bind_fleet(FLEET).unwrap();
        source.append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "github.event".into(),
            actor: None,
            fields: BTreeMap::from([
                ("version".into(), json!(1)),
                ("item".into(), json!("resource/github/acme/garden/pull-request/7")),
                ("event".into(), json!("checks-failed")),
                ("observer".into(), json!(format!("observer/node-{}", index))),
                ("at".into(), json!(format!("2026-11-11T00:00:0{index}Z"))),
                ("detail".into(), json!({"check":format!("check-{index}")})),
                ("head_sha".into(), json!(format!("{index:040x}"))),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("github-event:{subject}")),
        }).unwrap()
    }).collect::<Vec<_>>();
    let (operation, digest) = operation_parts(&claims[0].body).unwrap();
    let (other_operation, other_digest) = operation_parts(&claims[1].body).unwrap();
    assert_eq!(operation, other_operation);
    assert_ne!(digest, other_digest);
    assert!(!replay_projects_kind("github.event"));
    assert!(!Store::simple_replication_kind("github.event"));
    let exchanges = sources.iter().map(|source| {
        source.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap()
    }).collect::<Vec<_>>();
    assert!(exchanges.iter().all(|exchange| !exchange.envelopes.is_empty()));
    for checkpointed in [false, true] {
        for order in [[0, 1], [1, 0]] {
            let store = Store::open_memory("target").unwrap();
            store.bind_fleet(FLEET).unwrap();
            store.project_replication_backlog().unwrap();
            let context = format!("order={order:?} checkpointed={checkpointed}");
            let mut admitted = [false; 2];
            for (position, index) in order.into_iter().enumerate() {
                if checkpointed && position == 1 {
                    let previous = &claims[order[0]];
                    let (_, previous_digest) = operation_parts(&previous.body).unwrap();
                    let mut connection = store.connection.write();
                    let tx = connection.transaction().unwrap();
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h0f43df342761 owner=fractal_store::store::message_send_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id,writer,sequence,envelope_hash,subject,kind,
                        actor,predecessors,operation_id,request_digest,accepted_at_unix_ms,checkpoint)
                        VALUES (?1,?2,1,'invented-envelope',?3,'github.event',NULL,'[]',
                        ?4,?5,0,'checkpoint/2026-11-11')",
                        params![previous.id, previous.origin, previous.subject, operation, previous_digest]).unwrap();
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h3c87df5ded70 owner=fractal_store::store::message_send_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations WHERE id=?1", [operation]).unwrap();
                    tx.execute(/* sqlite-large-value: allow-for-now lv-hede6c7531978 owner=fractal_store::store::message_send_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&previous.id]).unwrap();
                    tx.commit().unwrap();
                    assert!(operation_tx(&connection, operation).unwrap().is_none());
                }
                let before = FULL_REPLAYS.with(std::cell::Cell::get);
                store.receive_replication_exchange(&claims[index].origin, FLEET, &exchanges[index]).unwrap();
                // Receiving stores envelopes; admission materializes their claims.
                store.validate_replication_backlog().unwrap();
                assert!(store.claim_by_id(&claims[index].id).unwrap().is_some(),
                    "{context} position={position}: incoming claim must be admitted");
                admitted[index] = true;
                let prior_state = operation_tx(&store.readers.get(), operation).unwrap()
                    .map(|(_, _, state)| state);
                assert_eq!(prior_state.as_deref(),
                    if position == 0 || checkpointed { None } else { Some("active") },
                    "{context} position={position}: repair must still be pending");
                store.project_replication_backlog().unwrap();
                assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), before,
                    "order={order:?} checkpointed={checkpointed} position={position}");
                // An admitted digest mismatch (or tombstone-only history) must
                // actually transition to conflict here, without a replay.
                let state = operation_tx(&store.readers.get(), operation).unwrap()
                    .unwrap_or_else(|| panic!("{context} position={position}: missing repaired operation")).2;
                assert_eq!(state, if position == 0 { "active" } else { "conflict" },
                    "{context} position={position}: indexed operation repair must run");
            }
            // Never compare two empty projections: both claims were admitted,
            // and the second incremental projection already produced conflict.
            assert_eq!(admitted, [true, true], "{context}");
            let operations = operation_rows(&store.readers.get());
            let digests = store.replication_snapshot().unwrap().projection_digests.clone();
            assert!(digests.contains_key("claim_sources"));
            assert!(digests.contains_key("custom_registrations"));
            assert!(digests.contains_key("custom_sources"));
            assert!(digests.contains_key("custom_dependencies"));
            store.replay_replication_graph().unwrap();
            assert_eq!(operation_rows(&store.readers.get()), operations, "{context}");
            assert_eq!(store.replication_snapshot().unwrap().projection_digests, digests, "{context}");
        }
    }
}
