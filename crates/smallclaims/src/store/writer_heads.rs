//! Each writer's highest batch, kept apart from `batches`.
//!
//! A checkpoint trim deletes a batch once none of its claims or envelopes remain, and that can be
//! a writer's newest batch: a member offline past the cut returns with its head trimmed. Sequence
//! allocation and the chain's previous hash read `batches`, so they would then reuse a sequence
//! peers have already seen. `writer_heads` keeps the highest (sequence, hash) per writer, raised
//! by trigger on every batch insert and stored before any batch delete, so trimming never lowers
//! it. Rows are never deleted. Heads trimmed before this table existed leave only their envelope
//! tombstones, which sequence allocation also respects, including through the retired-envelope
//! ledger after adoption withdraws them; their hash is not recoverable.

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-hdade9b126123 owner=smallclaims::store::writer_heads TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS writer_heads (
    writer TEXT NOT NULL PRIMARY KEY,
    sequence INTEGER NOT NULL,
    hash TEXT NOT NULL
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS writer_heads_batch_insert_v1 AFTER INSERT ON batches BEGIN
    INSERT INTO writer_heads(writer, sequence, hash) VALUES (NEW.origin, NEW.replica_sequence, NEW.hash)
    ON CONFLICT(writer) DO UPDATE SET sequence=excluded.sequence, hash=excluded.hash
    WHERE excluded.sequence>writer_heads.sequence;
END;
CREATE TRIGGER IF NOT EXISTS writer_heads_batch_delete_v1 BEFORE DELETE ON batches BEGIN
    INSERT INTO writer_heads(writer, sequence, hash) VALUES (OLD.origin, OLD.replica_sequence, OLD.hash)
    ON CONFLICT(writer) DO UPDATE SET sequence=excluded.sequence, hash=excluded.hash
    WHERE excluded.sequence>writer_heads.sequence;
END;
";

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::SCHEMA;
    use crate::store::runtime::Plain;
    use std::sync::Arc;

    fn append(store: &Store, label: &str) -> ClaimRecord {
        store
            .append_claim(&crate::claim::ClaimInput {
                subject: format!("note/{label}"),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    }

    fn next(store: &Store) -> (u64, Option<String>) {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        (
            next_replica_sequence(&transaction, "writer").unwrap(),
            previous_batch_hash(&transaction, "writer").unwrap(),
        )
    }

    #[test]
    fn trimming_a_writers_newest_batch_never_reuses_its_sequence_or_breaks_its_chain() {
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        append(&store, "one");
        let newest = append(&store, "two");
        let before = next(&store);
        let head: (u64, String) = store
            .readers
            .get()
            .query_row(
                "SELECT replica_sequence, hash FROM batches WHERE id=?1",
                [&newest.batch_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(before, (head.0 + 1, Some(head.1.clone())));
        // A trim removes the newest batch with its only claim.
        store
            .connection
            .write()
            .execute_batch(&format!(
                "DELETE FROM claims WHERE id='{}'; DELETE FROM batches WHERE id='{}';",
                newest.id, newest.batch_id
            ))
            .unwrap();
        assert_eq!(next(&store), before);
        let following = append(&store, "three");
        let sequence: u64 = store
            .readers
            .get()
            .query_row(
                "SELECT replica_sequence FROM batches WHERE id=?1",
                [&following.batch_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sequence, head.0 + 1);
    }

    #[test]
    fn a_head_trimmed_before_the_ledger_still_floors_the_next_sequence() {
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        append(&store, "one");
        {
            let writer = store.connection.write();
            // An earlier build trimmed sequences 2 to 5 and kept only their envelope tombstones.
            writer.execute("DELETE FROM writer_heads", []).unwrap();
            for sequence in 2..=5 {
                writer
                    .execute(
                        "INSERT INTO checkpoint_envelopes VALUES('writer', ?1, ?2, 1, 'checkpoint/old')",
                        params![sequence, format!("{sequence:064x}")],
                    )
                    .unwrap();
            }
        }
        assert_eq!(next(&store).0, 6);
        // Another writer's tombstones do not move this one.
        store
            .connection
            .write()
            .execute(
                "INSERT INTO checkpoint_envelopes VALUES('other', 50, 'hash', 1, 'checkpoint/old')",
                [],
            )
            .unwrap();
        assert_eq!(next(&store).0, 6);
        // Adoption keeping only a manifest that lacks them withdraws the tombstones; the
        // retired-envelope ledger keeps them, so the floor holds.
        store
            .connection
            .write()
            .execute("DELETE FROM checkpoint_envelopes WHERE writer='writer'", [])
            .unwrap();
        assert_eq!(next(&store).0, 6);
    }

    #[test]
    fn heads_only_rise_whatever_order_batches_arrive_or_leave() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER, hash TEXT);",
            )
            .unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection
            .execute_batch(
                "INSERT INTO batches VALUES('b3','peer',3,'h3'),('b1','peer',1,'h1'),('b2','peer',2,'h2');
                 DELETE FROM batches WHERE id IN ('b3','b2');
                 INSERT INTO batches VALUES('b2-again','peer',2,'h2');",
            )
            .unwrap();
        let head: (u64, String) = connection
            .query_row("SELECT sequence, hash FROM writer_heads WHERE writer='peer'", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(head, (3, "h3".into()));
    }
}
