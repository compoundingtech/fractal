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
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h789631746b77 owner=st3_store::store::message_send_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id,writer,sequence,envelope_hash,subject,kind,
                            actor,predecessors,operation_id,request_digest,accepted_at_unix_ms,checkpoint)
                            VALUES (?1,'invented-peer',1,'invented-envelope',?2,'message.sent',NULL,'[]',
                            'op/invented-parity',?3,0,'checkpoint/2026-10-01')",
                            params![claim.id, claim.subject, digests[0]]).unwrap();
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h0e91a591e430 owner=st3_store::store::message_send_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations WHERE id='op/invented-parity'", [])
                            .unwrap();
                        tx.execute(/* sqlite-large-value: allow-for-now lv-he720c27162e7 owner=st3_store::store::message_send_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&claim.id])
                            .unwrap();
                    } else if mode == "repaired" {
                        tx.execute(/* sqlite-large-value: allow-for-now lv-he798b60449d9 owner=st3_store::store::message_send_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,
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

