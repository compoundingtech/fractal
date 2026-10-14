//! Bounded lookups of newly authored, fenced native receipt identities. No historical
//! fold, read-path initialization or transcript content. Native identities are stored
//! only beside a newly appended, authenticated owner-local consumption receipt.
use super::*;
use crate::conversation_delivery::NativeReceipt;

const IDS: usize = 200;
const BODY_BYTES: usize = 4096;

const QUERY: &str = "WITH candidates AS (
    SELECT requested.value AS native_id, substr(claims.id,1,513) AS id, substr(claims.subject,1,513) AS subject,
           substr(CAST(claims.body AS BLOB),1,4097) AS body,
           EXISTS(SELECT 1 FROM main.replica_records INDEXED BY replica_records_repaired_claim
                  WHERE claim_id=claims.id AND state='repaired') AS repaired
    FROM json_each(?1) AS requested
    CROSS JOIN main.claims AS claims WHERE claims.id IN (
        SELECT claim_id FROM main.local_native_delivery_receipts_v1
            INDEXED BY local_native_delivery_identity_v1
        WHERE session_id=?2 AND native_id=requested.value AND actor=?3 AND origin=?4
            AND store_index<=?5 ORDER BY store_index DESC LIMIT 3
    )
)
SELECT candidates.native_id, candidates.id, candidates.subject, candidates.body, candidates.repaired,
       substr(operations.request_digest,1,65), substr(operations.canonical_claim_id,1,513), substr(operations.state,1,33)
FROM candidates LEFT JOIN main.operations AS operations
ON operations.id=CASE WHEN json_valid(candidates.body)
                     THEN json_extract(candidates.body,'$._operation.id') END";

impl Store {
    /// At most three physical receipts per selected native id. Hitting that limit
    /// cannot prove uniqueness, so the optional link stays absent. Two qualified
    /// receipts are ambiguous even if their graph subjects happen to be identical.
    pub(crate) fn conversation_delivery_receipts(
        &self,
        session: &str,
        owner: &str,
        ids: &BTreeSet<String>,
        through: u64,
    ) -> Result<Vec<(String, NativeReceipt)>> {
        if ids.is_empty() || ids.len() > IDS || ids.iter().any(|id| id.len() > 256) {
            return Ok(Vec::new());
        }
        let connection = self.readers.get();
        let mut statement = connection.prepare(QUERY)?;
        let requested = serde_json::to_string(ids)?;
        let rows = statement.query_map(
            params![requested, session, owner, self.origin(), through],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )?;
        let mut counts = BTreeMap::<String, usize>::new();
        let mut qualified = Vec::new();
        for row in rows {
            let (native, id, subject, bytes, repaired, digest, canonical, state) = row?;
            *counts.entry(native.clone()).or_default() += 1;
            if repaired || bytes.len() > BODY_BYTES {
                continue;
            }
            let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if body.get("_operation").is_some() {
                let Some((_, request_digest)) = operation_parts(&body) else {
                    continue;
                };
                if digest.as_deref() != Some(request_digest)
                    || canonical.as_deref() != Some(id.as_str())
                    || state.as_deref() != Some("active")
                {
                    continue;
                }
            }
            if body.pointer("/fields/status").and_then(Value::as_str) != Some("read") {
                continue;
            }
            let receipt = NativeReceipt {
                session_id: session.into(),
                message_id: native,
            };
            if receipt.valid() && subject.len() <= 512 && id.len() <= 512 {
                qualified.push((subject, receipt));
            }
        }
        qualified.retain(|(_, receipt)| {
            counts
                .get(&receipt.message_id)
                .is_some_and(|count| *count < 3)
        });
        Ok(qualified)
    }

    /// A new receipt for this owner can alter a previously delivered native turn.
    /// Conservatively resync its native conversation, with one owner/cut index seek.
    pub(crate) fn conversation_delivery_changed(
        &self,
        owner: &str,
        after: u64,
        through: u64,
    ) -> Result<bool> {
        Ok(self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM main.local_native_delivery_receipts_v1
             INDEXED BY local_native_delivery_owner_cut_v1
             WHERE actor=?1 AND store_index>?2 AND store_index<=?3 LIMIT 1)",
            params![owner, after, through],
            |row| row.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation_delivery::unique_links;

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
        crate::mailbox::tests::ready(store, "session-1");
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
                    "SELECT count(*) FROM local_native_delivery_receipts_v1",
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
        store.connection.write().execute_batch("CREATE TRIGGER reject_fixture_proof BEFORE INSERT ON local_native_delivery_receipts_v1 BEGIN SELECT RAISE(ABORT,'fixture refusal'); END;").unwrap();
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
                tx.execute("INSERT INTO local_native_delivery_receipts_v1 VALUES (?1,?2,?3,?4,'node',?5)",
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
            assert!(plan.contains("local_native_delivery_identity_v1"), "{plan}");
            assert!(!plan.contains("SCAN claims"), "{plan}");
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
}
