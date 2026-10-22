//! Receipt-link lookups: store-level tests that need the delivery store's mailbox operations.
use super::*;
use crate::conversation_delivery::NativeReceipt;
use crate::conversation_delivery::unique_links;
use crate::{ConversationReadsStore, DeliveryStore, MailboxLeaseStore};
use fractal_store::store::conversation_deliveries::QUERY;
use fractal_store::store::migrate_schema;

const SESSION: &str = "session/external-0123456789abcdef01234567";
const OWNER: &str = "agent/eval.worker";
fn input(subject: &str, kind: &str, actor: &str, fields: Value) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: Some(format!("delivery-fixture:{subject}:{kind}")),
    }
}
fn prepared(store: &Store, subject: &str) -> (crate::mailbox::Fence, ClaimInput) {
    crate::store::mailbox_tests::ready(store, "session-1");
    let fence = store
        .bind_mailbox(&crate::mailbox::Fence::new(OWNER, "session-1", "delivery"))
        .unwrap();
    store.append_claim(&input(subject, "message.sent", "person/fixture",
        json!({"status":"sent","from":"person/fixture","to":OWNER,"content":"Invented fixture"}))).unwrap();
    for status in ["staged", "delivered"] {
        store
            .append_mailbox_receipt_outcome(
                &input(
                    subject,
                    &format!("message.{status}"),
                    OWNER,
                    json!({"status":status}),
                ),
                &fence,
            )
            .unwrap();
    }
    (
        fence,
        input(subject, "message.read", OWNER, json!({"status":"read"})),
    )
}
fn proof(id: &str) -> NativeReceipt {
    NativeReceipt {
        session_id: SESSION.into(),
        message_id: id.into(),
    }
}
fn links(store: &Store, through: u64) -> BTreeMap<String, String> {
    unique_links(
        SESSION,
        store
            .conversation_delivery_receipts(
                SESSION,
                OWNER,
                &BTreeSet::from(["native-one".into()]),
                through,
            )
            .unwrap(),
    )
}

#[test]
fn native_delivery_receipt_is_atomic_and_replay_does_not_rebind_it() {
    let store = Store::open_memory("node").unwrap();
    let (fence, input) = prepared(&store, "message/one");
    let before = store.index().unwrap();
    let (receipt, appended) = store
        .append_native_mailbox_receipt_outcome(&input, &fence, Some(&proof("native-one")))
        .unwrap();
    assert!(appended);
    assert_eq!(receipt.body["fields"], json!({"status":"read"}));
    // Local provenance cannot change the replicated/shared logical digest.
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let before = projection_digest::tables(&tx).unwrap();
        tx.execute("UPDATE local_native_delivery_receipts_v2 SET native_id='scope-only-fixture' WHERE claim_id=?1",[&receipt.id]).unwrap();
        assert_eq!(before, projection_digest::tables(&tx).unwrap());
        tx.rollback().unwrap();
    }

    assert!(links(&store, before).is_empty());
    assert_eq!(
        links(&store, receipt.store_index)["native-one"],
        "message/one"
    );
    let (replayed, appended) = store
        .append_native_mailbox_receipt_outcome(&input, &fence, Some(&proof("different-record")))
        .unwrap();
    assert!(!appended);
    assert_eq!(receipt.id, replayed.id);
    assert_eq!(store.index().unwrap(), receipt.store_index);
    assert_eq!(
        links(&store, receipt.store_index)["native-one"],
        "message/one"
    );
}

#[test]
fn native_delivery_ambiguity_is_global_and_legacy_receipts_have_no_link() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/one");
    store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    let first_cut = store.index().unwrap();
    let (fence, read) = prepared(&store, "message/two-outside-selected-page");
    store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    assert_eq!(links(&store, first_cut).len(), 1);
    assert!(links(&store, store.index().unwrap()).is_empty());
    assert!(
        store
            .conversation_delivery_changed(OWNER, first_cut, store.index().unwrap())
            .unwrap()
    );
    assert!(
        store
            .conversation_changed_at(OWNER, first_cut, store.index().unwrap(), 0, 0)
            .unwrap()
    );
    let (fence, read) = prepared(&store, "message/legacy");
    store.append_mailbox_receipt_outcome(&read, &fence).unwrap();
    assert_eq!(
        store
            .readers
            .get()
            .query_row(
                "SELECT count(*) FROM local_native_delivery_receipts_v2",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn native_delivery_metadata_failure_rolls_back_the_receipt_and_operation() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/rollback");
    let before = store.index().unwrap();
    store.connection.write().execute_batch("CREATE TRIGGER reject_fixture_proof BEFORE INSERT ON local_native_delivery_receipts_v2 BEGIN SELECT RAISE(ABORT,'fixture refusal'); END;").unwrap();
    assert!(
        store
            .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
            .is_err()
    );
    assert_eq!(store.index().unwrap(), before);
    assert_eq!(
        store.message(&read.subject).unwrap().unwrap().status,
        "delivered"
    );
    assert!(links(&store, before).is_empty());
    store
        .connection
        .write()
        .execute_batch("DROP TRIGGER reject_fixture_proof;")
        .unwrap();
    assert!(
        store
            .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
            .unwrap()
            .1
    );
    assert_eq!(links(&store, store.index().unwrap()).len(), 1);
}

#[test]
fn native_delivery_missing_operation_proof_is_absent_and_not_repaired_by_read() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/unprovable");
    let (receipt, _) = store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    assert_eq!(links(&store, store.index().unwrap()).len(), 1);
    let operation = operation_parts(&receipt.body).unwrap().0;
    store
        .connection
        .write()
        .execute("DELETE FROM operations WHERE id=?1", [operation])
        .unwrap();
    assert!(links(&store, store.index().unwrap()).is_empty());
    assert_eq!(
        store
            .readers
            .get()
            .query_row(
                "SELECT count(*) FROM operations WHERE id=?1",
                [operation],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn native_delivery_timestamp_updates_keep_proof_and_meaning_updates_drop_it() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/timestamp");
    let (receipt, _) = store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    store
        .connection
        .write()
        .execute(
            "UPDATE claims SET accepted_at_unix_ms='1792000000000' WHERE id=?1",
            [&receipt.id],
        )
        .unwrap();
    assert_eq!(
        links(&store, store.index().unwrap())["native-one"],
        "message/timestamp"
    );
    store
        .connection
        .write()
        .execute(
            "UPDATE claims SET body=json_set(body,'$.fields.changed_fixture',1) WHERE id=?1",
            [&receipt.id],
        )
        .unwrap();
    assert!(links(&store, store.index().unwrap()).is_empty());
    assert_eq!(
        store
            .readers
            .get()
            .query_row(
                "SELECT count(*) FROM local_native_delivery_receipts_v2",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn native_delivery_provenance_survives_real_projection_and_batch_rebuilds() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/rebuild");
    store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    let before = links(&store, store.index().unwrap());
    assert_eq!(before.len(), 1);
    store.rebuild_claim_projections().unwrap();
    store.replay_replication_graph().unwrap();
    assert_eq!(links(&store, store.index().unwrap()), before);
    // The cited migration swaps batches, not claims. Run that exact migration
    // against this isolated fixture; all claim ids and metadata must survive.
    {
        let connection = store.connection.write();
        connection.execute_batch("PRAGMA user_version=11;").unwrap();
        migrate_schema(&connection).unwrap();
        connection.execute_batch("PRAGMA user_version=19;").unwrap();
        let triggers:u64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name IN ('local_native_delivery_update_v2','local_native_delivery_delete_v2')",[],|r|r.get(0)).unwrap();
        assert_eq!(triggers, 2);
    }
    assert_eq!(links(&store, store.index().unwrap()), before);
}

#[test]
fn native_delivery_foreign_keys_off_identity_change_and_missing_rows_never_rebind() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/id-change");
    let (receipt, _) = store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    {
        let connection = store.connection.write();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        let error = connection
            .execute(
                "UPDATE claims SET id='replacement-fixture-id' WHERE id=?1",
                [&receipt.id],
            )
            .expect_err(
                "the production source-identity guard also applies with foreign keys off",
            );
        assert!(
            error
                .to_string()
                .contains("claim source identity is immutable")
        );
    }
    assert_eq!(links(&store, store.index().unwrap()).len(), 1);
    // Fault injection only: if a table-copy tool lost the source-identity guard,
    // our independent local-provenance update guard still retires the old link.
    {
        let connection = store.connection.write();
        connection
            .execute_batch("DROP TRIGGER projection_digest_claims_identity_update;")
            .unwrap();
        connection
            .execute(
                "UPDATE claims SET id='replacement-fixture-id' WHERE id=?1",
                [&receipt.id],
            )
            .unwrap();
    }
    assert!(links(&store, store.index().unwrap()).is_empty());
    assert_eq!(
        store
            .readers
            .get()
            .query_row(
                "SELECT count(*) FROM local_native_delivery_receipts_v2",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn native_delivery_fingerprint_refuses_changed_meaning_even_without_the_guard_trigger() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/copied-id");
    let (receipt, _) = store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    assert_eq!(links(&store, store.index().unwrap()).len(), 1);
    {
        let connection = store.connection.write();
        connection
            .execute_batch("DROP TRIGGER local_native_delivery_update_v2;")
            .unwrap();
        connection.execute("UPDATE claims SET body=json_set(body,'$.fields.changed_after_copy',1) WHERE id=?1",[&receipt.id]).unwrap();
    }
    assert!(links(&store, store.index().unwrap()).is_empty());
    assert_eq!(
        store
            .readers
            .get()
            .query_row(
                "SELECT count(*) FROM local_native_delivery_receipts_v2",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn native_delivery_missing_physical_claims_still_count_towards_the_lookup_bound() {
    let store = Store::open_memory("node").unwrap();
    let mut receipts = Vec::new();
    for number in 0..4 {
        let (fence, read) = prepared(&store, &format!("message/raw-bound-{number}"));
        receipts.push(
            store
                .append_native_mailbox_receipt_outcome(
                    &read,
                    &fence,
                    Some(&proof("native-one")),
                )
                .unwrap()
                .0,
        );
    }
    // Simulate a foreign-keys-off table-copy/fault that lost the guard trigger.
    // Two newest missing rows must not expose only one of the bounded candidates
    // and hide the fourth qualified receipt outside that physical window.
    {
        let connection = store.connection.write();
        connection
            .execute_batch(
                "PRAGMA foreign_keys=OFF; DROP TRIGGER local_native_delivery_delete_v2;",
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM claims WHERE id IN (?1,?2)",
                params![receipts[3].id, receipts[2].id],
            )
            .unwrap();
    }
    assert!(links(&store, store.index().unwrap()).is_empty());
}

#[test]
fn native_delivery_query_work_does_not_scan_unrelated_receipt_history() {
    let store = Store::open_memory("node").unwrap();
    let (fence, read) = prepared(&store, "message/selected");
    store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    let mut costs = Vec::new();
    store.connection.batched(|tx| -> Result<()> {
        for number in 0..10_000 {
            let claim = append_claim_tx(tx, "node", &format!("message/other-{number}"),
                "message.read", Some(OWNER), &json!({"fields":{"status":"read"}}), &[], None)?;
            tx.execute("INSERT INTO local_native_delivery_receipts_v2 VALUES (?1,'unrelated-fixture',?2,?3,?4,'node',?5)",
                params![claim.id,SESSION,format!("other-native-{number}"),OWNER,claim.store_index])?;
            if number==99 || number==9999 {
                let mut query = tx.prepare(QUERY)?;
                query.reset_status(rusqlite::StatementStatus::VmStep);
                let rows = query.query_map(params!["[\"native-one\"]",SESSION,OWNER,"node",i64::MAX], |_| Ok(()))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert_eq!(rows.len(),1);
                costs.push(query.get_status(rusqlite::StatementStatus::VmStep));
            }
        }
        let plan = tx.prepare(&format!("EXPLAIN QUERY PLAN {QUERY}"))?
            .query_map(params!["[\"native-one\"]",SESSION,OWNER,"node",i64::MAX], |row| row.get::<_,String>(3))?
            .collect::<rusqlite::Result<Vec<_>>>()?.join("\n");
        assert!(plan.contains("local_native_delivery_identity_v2"), "{plan}");
        assert!(!plan.contains("SCAN claims"), "{plan}");
        assert!(!plan.contains("SCAN proof"), "{plan}");
        Ok(())
    }).unwrap().unwrap();
    assert!(costs[1] <= costs[0] + 20, "{costs:?}");
    assert!(costs[1] < 1000, "{costs:?}");
}

#[test]
fn native_delivery_binding_survives_reopen_and_stale_fence_adds_nothing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let (fence, read) = prepared(&store, "message/persisted");
    let mut stale = fence.clone();
    stale.epoch += 1;
    assert!(
        store
            .append_native_mailbox_receipt_outcome(&read, &stale, Some(&proof("native-one")))
            .is_err()
    );
    assert!(links(&store, store.index().unwrap()).is_empty());
    store
        .append_native_mailbox_receipt_outcome(&read, &fence, Some(&proof("native-one")))
        .unwrap();
    drop(store);
    let reopened = Store::open(&path, "node").unwrap();
    assert_eq!(
        links(&reopened, reopened.index().unwrap())["native-one"],
        "message/persisted"
    );
}
