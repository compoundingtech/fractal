//! Each stored operation claim's operation and request digest, out of its body, for the
//! operation audit (#5171). The audit compares every operation's row with its claims each hour.
//! Reading the digest from each body read the whole claim log from disk, about 26 GB on a
//! 56 GB store; this table holds only the keys the audit compares.
//!
//! Nothing here runs during Store::open. The first page installs the table and its triggers;
//! from then on every claim write keeps it exact, under whichever binary makes the write, since
//! the triggers are part of the schema. The pages after it copy the claims already stored, in
//! the audit's own order. Each page reads its claims on a reader. The writer then re-reads only
//! those few bodies, which the reader has just brought into cache, to check that each claim
//! still holds the keys it read. The audit reads the table only once the last page has marked
//! it ready.
use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::sqlite::{ReadPool, WriterConnection};

pub const NAME: &str = "operation_claim_digests";
/// The profile operation and writer-holder label of one backfill page.
pub const LABEL: &str = "maintenance.operation_claim_digests";
pub const PAGE_SIZE: i64 = 32;
/// A page stops reading claims after this long, so its reader snapshot stays short.
pub const PAGE_READ_BUDGET: Duration = Duration::from_millis(20);
/// A page stops copying after this long under the writer and commits what it copied; the next
/// page reads on from the last copied claim. The commit's own sync is on top of it.
pub const PAGE_WRITE_BUDGET: Duration = Duration::from_millis(20);

/// The claims the audit counts: their operation and digest are both text. Every trigger and
/// the backfill copy exactly these, and the audit's body read selects exactly these.
const INSTALL: &str = /* sqlite-large-value: allow-for-now lv-h3bf56234d7c1 owner=smallclaims::store::operation_claim_digests TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS operation_claim_digests (
    store_index INTEGER PRIMARY KEY,
    claim_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    request_digest TEXT NOT NULL
);
-- This CREATE INDEX only ever sees the newly created, empty table.
CREATE INDEX IF NOT EXISTS operation_claim_digests_operation
ON operation_claim_digests(operation_id, store_index, request_digest, claim_id);
CREATE TABLE IF NOT EXISTS operation_claim_digests_backfill (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    operation TEXT NOT NULL,
    claim_rowid INTEGER NOT NULL,
    ready INTEGER NOT NULL CHECK(ready IN (0,1))
);
CREATE TRIGGER IF NOT EXISTS operation_claim_digests_insert AFTER INSERT ON claims
WHEN json_type(NEW.body, '$._operation.id')='text'
 AND json_type(NEW.body, '$._operation.request_digest')='text' BEGIN
    INSERT OR REPLACE INTO operation_claim_digests VALUES (NEW.store_index, NEW.id,
        json_extract(NEW.body, '$._operation.id'),
        json_extract(NEW.body, '$._operation.request_digest'));
END;
-- A delete seeks the claim's row by its store index, without reading the deleted body.
CREATE TRIGGER IF NOT EXISTS operation_claim_digests_delete AFTER DELETE ON claims BEGIN
    DELETE FROM operation_claim_digests WHERE store_index=OLD.store_index;
END;
CREATE TRIGGER IF NOT EXISTS operation_claim_digests_update
AFTER UPDATE OF store_index, id, body ON claims BEGIN
    DELETE FROM operation_claim_digests WHERE store_index=OLD.store_index;
    INSERT OR REPLACE INTO operation_claim_digests
    SELECT NEW.store_index, NEW.id, json_extract(NEW.body, '$._operation.id'),
        json_extract(NEW.body, '$._operation.request_digest')
    WHERE json_type(NEW.body, '$._operation.id')='text'
      AND json_type(NEW.body, '$._operation.request_digest')='text';
END;
INSERT OR IGNORE INTO operation_claim_digests_backfill VALUES (1, '', 0, 0);
"#;

/// The rest of one operation's claims after `claim_rowid`, then the operations after it: the
/// audit's own two seeks through `claims_operation_index`.
const REMAINDER_PAGE: &str =
    "SELECT store_index, id, json_extract(body,'$._operation.id'),
            json_extract(body,'$._operation.request_digest')
     FROM claims INDEXED BY claims_operation_index
     WHERE json_extract(body,'$._operation.id')=?1 AND rowid>?2
       AND json_type(body,'$._operation.id')='text'
       AND json_type(body,'$._operation.request_digest')='text'
     ORDER BY rowid LIMIT ?3";
const NEXT_PAGE: &str =
    "SELECT store_index, id, json_extract(body,'$._operation.id'),
            json_extract(body,'$._operation.request_digest')
     FROM claims INDEXED BY claims_operation_index
     WHERE json_extract(body,'$._operation.id')>?1 AND ?2 IS NOT NULL
       AND json_type(body,'$._operation.id')='text'
       AND json_type(body,'$._operation.request_digest')='text'
     ORDER BY json_extract(body,'$._operation.id'),rowid LIMIT ?3";

#[derive(Debug)]
pub struct Page {
    pub ready: bool,
    pub copied: usize,
    /// How long the page held the writer, begin through commit, excluding queue admission.
    pub write_hold: Duration,
}

/// Whether the backfill has finished, so the table holds every operation claim. Absent tables
/// read as not ready, so a store from before this table keeps the audit on bodies.
pub fn ready(connection: &Connection) -> Result<bool> {
    let present: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='operation_claim_digests_backfill')",
        [], |row| row.get(0),
    )?;
    if !present {
        return Ok(false);
    }
    Ok(connection.query_row(
        "SELECT ready FROM operation_claim_digests_backfill WHERE singleton=1",
        [], |row| row.get(0),
    ).optional()?.unwrap_or(false))
}

/// Copy the next page. Each page is its own profile operation, so its reads and writer hold are
/// named whether or not task metrics are on.
pub fn advance(readers: &ReadPool, writer: &WriterConnection) -> Result<Page> {
    crate::profile::task(LABEL, || advance_with_budget(readers, writer, PAGE_READ_BUDGET, PAGE_WRITE_BUDGET, &mut || {}))
}

fn advance_with_budget(
    readers: &ReadPool,
    writer: &WriterConnection,
    read_budget: Duration,
    write_budget: Duration,
    // Runs between a page's read and its write, where other writers may change its claims.
    between: &mut dyn FnMut(),
) -> Result<Page> {
    let installed: bool = readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='operation_claim_digests_backfill')",
        [], |row| row.get(0),
    )?;
    if !installed {
        // The triggers and the cursor come in one transaction, so every claim written after it
        // reaches the table by trigger, and every claim before it by the backfill.
        let mut writer = writer.write_background();
        let acquired = Instant::now();
        let transaction = writer.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h2f6df7b9a119 owner=smallclaims::store::operation_claim_digests TODO=trace-statement-and-value-representation */ INSTALL)?;
        transaction.commit()?;
        return Ok(Page { ready: false, copied: 0, write_hold: acquired.elapsed() });
    }
    let (operation, claim_rowid, complete): (String, i64, bool) = readers.get().query_row(
        "SELECT operation, claim_rowid, ready FROM operation_claim_digests_backfill WHERE singleton=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if complete {
        return Ok(Page { ready: true, copied: 0, write_hold: Duration::ZERO });
    }
    // Read the page on a reader: the bodies are what cost, and the writer never waits on them.
    let mut rows = Vec::new();
    let mut remainder = true;
    {
        let connection = readers.get();
        let started = Instant::now();
        let transaction = connection.unchecked_transaction()?;
        while (rows.len() as i64) < PAGE_SIZE {
            let (cursor, after) = rows.last().map_or((operation.as_str(), claim_rowid), |row: &(i64, String, String, String)| (row.2.as_str(), row.0));
            let (cursor, after) = (cursor.to_owned(), after);
            let left = PAGE_SIZE - rows.len() as i64;
            let page = transaction
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-hedc149804801 owner=smallclaims::store::operation_claim_digests TODO=trace-statement-and-value-representation */ if remainder { REMAINDER_PAGE } else { NEXT_PAGE })?
                .query_map(params![cursor, after, left], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if page.is_empty() {
                if !remainder {
                    break;
                }
                remainder = false;
                continue;
            }
            remainder = true;
            rows.extend(page);
            if started.elapsed() >= read_budget {
                break;
            }
        }
    }
    between();
    // An empty read after both seeks is the end: every claim before the triggers is copied.
    let done = rows.is_empty();
    let mut writer = writer.write_background();
    let acquired = Instant::now();
    let transaction = writer.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut copied = 0;
    let mut written = 0;
    {
        // The claim must still hold the keys the page read: one deleted since is gone from the
        // table by its trigger, and one rewritten since was copied or dropped by its trigger,
        // so neither may come back. The page's bodies are the ones its reader just read.
        let mut insert = transaction.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h22a91f01cbf8 owner=smallclaims::store::operation_claim_digests TODO=prepare-body-and-predecessor-values */
            "INSERT OR IGNORE INTO operation_claim_digests
             SELECT ?1, ?2, ?3, ?4 WHERE EXISTS(SELECT 1 FROM claims WHERE store_index=?1 AND id=?2
               AND json_type(body,'$._operation.id')='text'
               AND json_type(body,'$._operation.request_digest')='text'
               AND json_extract(body,'$._operation.id')=?3
               AND json_extract(body,'$._operation.request_digest')=?4)",
        )?;
        for (store_index, claim_id, operation_id, request_digest) in &rows {
            copied += insert.execute(/* sqlite-large-value: allow-for-now lv-h23892eda4b7e owner=smallclaims::store::operation_claim_digests TODO=trace-statement-and-value-representation */ params![store_index, claim_id, operation_id, request_digest])?;
            written += 1;
            if acquired.elapsed() >= write_budget {
                break;
            }
        }
    }
    let (operation, claim_rowid) = rows[..written]
        .last()
        .map_or((operation, claim_rowid), |row| (row.2.clone(), row.0));
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-hc8fbf8a65fc9 owner=smallclaims::store::operation_claim_digests TODO=bound-or-reference-values-and-reclamation */
        "UPDATE operation_claim_digests_backfill SET operation=?1, claim_rowid=?2, ready=?3 WHERE singleton=1",
        params![operation, claim_rowid, done],
    )?;
    transaction.commit()?;
    Ok(Page { ready: done, copied, write_hold: acquired.elapsed() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Store, runtime::Plain};
    use std::sync::Arc;

    #[test]
    fn exhausted_budgets_resume_at_each_copied_claim_without_gaps() {
        let store = Store::open_memory("invented-origin", Arc::new(Plain)).unwrap();
        {
            let connection = store.connection.write();
            for (store_index, body) in [
                (3, r#"{"_operation":{"id":"op/b","request_digest":"d/1"}}"#),
                (6, r#"{"_operation":{"id":"op/a","request_digest":"d/2"}}"#),
                (9, r#"{"note":"no operation"}"#),
                (12, r#"{"_operation":{"id":"op/b","request_digest":7}}"#),
                (15, r#"{"_operation":{"id":"op/a","request_digest":"d/3"}}"#),
                (18, r#"{"_operation":{"id":"op/b","request_digest":"d/4"}}"#),
            ] {
                connection.execute(
                    "INSERT INTO batches(id, origin, replica_sequence, hash, accepted_at_unix_ms)
                     VALUES (?1, 'local', ?2, ?1, '1')",
                    params![format!("invented-batch/{store_index}"), store_index],
                ).unwrap();
                connection.execute(
                    "INSERT INTO claims(id, batch_id, subject, kind, origin, body,
                     predecessors, accepted_at_unix_ms, store_index)
                     VALUES (?1, ?2, 'invented/subject', 'invented.kind', 'local', ?3, '[]', '1', ?4)",
                    params![format!("invented-claim/{store_index}"), format!("invented-batch/{store_index}"),
                        body, store_index],
                ).unwrap();
            }
        }
        let install = advance_with_budget(&store.readers, &store.connection, Duration::ZERO, Duration::ZERO, &mut || {}).unwrap();
        assert!(!install.ready);
        assert_eq!(install.copied, 0);
        // The audit's order: by operation, then by store index; one claim per page.
        for cursor in [("op/a", 6), ("op/a", 15), ("op/b", 3), ("op/b", 18)] {
            let page = advance_with_budget(&store.readers, &store.connection, Duration::ZERO, Duration::ZERO, &mut || {}).unwrap();
            assert_eq!((page.ready, page.copied), (false, 1));
            let stored: (String, i64) = store.readers.get().query_row(
                "SELECT operation, claim_rowid FROM operation_claim_digests_backfill", [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(stored, (cursor.0.to_owned(), cursor.1));
            assert!(!ready(&store.readers.get()).unwrap());
        }
        let last = advance_with_budget(&store.readers, &store.connection, Duration::ZERO, Duration::ZERO, &mut || {}).unwrap();
        assert_eq!((last.ready, last.copied), (true, 0));
        assert!(ready(&store.readers.get()).unwrap());
        let copied = store.readers.get()
            .prepare("SELECT store_index, claim_id, operation_id, request_digest FROM operation_claim_digests ORDER BY store_index")
            .unwrap()
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let expected = [(3, "op/b", "d/1"), (6, "op/a", "d/2"), (15, "op/a", "d/3"), (18, "op/b", "d/4")]
            .map(|(store_index, operation, digest)| {
                (store_index, format!("invented-claim/{store_index}"), operation.to_owned(), digest.to_owned())
            });
        assert_eq!(copied, expected);
    }

    #[test]
    fn a_claim_rewritten_between_a_pages_read_and_write_keeps_the_keys_its_trigger_left() {
        let store = Store::open_memory("invented-origin", Arc::new(Plain)).unwrap();
        {
            let connection = store.connection.write();
            for (store_index, body) in [
                (3, r#"{"_operation":{"id":"op/a","request_digest":"d/1"}}"#),
                (6, r#"{"_operation":{"id":"op/a","request_digest":"d/2"}}"#),
                (9, r#"{"_operation":{"id":"op/a","request_digest":"d/3"}}"#),
                (12, r#"{"_operation":{"id":"op/a","request_digest":"d/4"}}"#),
            ] {
                connection.execute(
                    "INSERT INTO batches(id, origin, replica_sequence, hash, accepted_at_unix_ms)
                     VALUES (?1, 'local', ?2, ?1, '1')",
                    params![format!("invented-batch/{store_index}"), store_index],
                ).unwrap();
                connection.execute(
                    "INSERT INTO claims(id, batch_id, subject, kind, origin, body,
                     predecessors, accepted_at_unix_ms, store_index)
                     VALUES (?1, ?2, 'invented/subject', 'invented.kind', 'local', ?3, '[]', '1', ?4)",
                    params![format!("invented-claim/{store_index}"), format!("invented-batch/{store_index}"),
                        body, store_index],
                ).unwrap();
            }
        }
        advance(&store.readers, &store.connection).unwrap();
        // The page reads all four, then each is rewritten or deleted before its copy.
        let page = advance_with_budget(&store.readers, &store.connection, PAGE_READ_BUDGET, PAGE_WRITE_BUDGET, &mut || {
            store.connection.write().execute_batch(
                "UPDATE claims SET body='{\"note\":\"no operation\"}' WHERE store_index=3;
                 UPDATE claims SET body=json_set(body, '$._operation.request_digest', 'd/8') WHERE store_index=6;
                 UPDATE claims SET body=json_set(body, '$._operation.id', 'op/b') WHERE store_index=9;
                 DELETE FROM claims WHERE store_index=12;",
            ).unwrap();
        }).unwrap();
        assert_eq!(page.copied, 0, "every claim the page read changed before its copy");
        while !advance(&store.readers, &store.connection).unwrap().ready {}
        let copied = store.readers.get()
            .prepare("SELECT store_index, operation_id, request_digest FROM operation_claim_digests ORDER BY store_index")
            .unwrap()
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(copied, [(6, "op/a".to_owned(), "d/8".to_owned()), (9, "op/b".to_owned(), "d/3".to_owned())]);
    }
}
