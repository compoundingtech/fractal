//! Rebuildable transport-only seek index. Nothing here runs during Store::open.
//! Install capture before taking the backfill high-water mark, then yield the SQLite
//! writer after each bounded page. Reads keep the original query until completion.
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{canonical, canonical_sql};
use crate::sqlite::WriterConnection;

pub const NAME: &str = "transport_observation_seek";
pub const PAGE_SIZE: i64 = 128;
// Yield the writer after a short work slice; one cold read or commit can still exceed it.
pub const PAGE_WORK_BUDGET: Duration = Duration::from_millis(20);
pub const ORIGINAL_QUERY: &str =
    "SELECT body, accepted_at_unix_ms FROM claims
     WHERE subject=?1 AND kind='transport.observed' AND origin=?2
     ORDER BY CANONICAL_DESC(claims) LIMIT 1";

static ORIGINAL_SQL: LazyLock<String> = LazyLock::new(|| canonical_sql(ORIGINAL_QUERY));

// Capture copies claim identifiers, subject, origin and timestamp into TEXT columns
// and seek keys without a byte-size proof; it never copies body or predecessors.
const INSTALL: &str = /* sqlite-large-value: allow-for-now lv-hb806bfc38d25 owner=smallclaims::store::transport_observation_index TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS transport_observation_index (
    claim_id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    origin TEXT NOT NULL,
    accepted_at_unix_ms TEXT NOT NULL
);
-- This CREATE INDEX only ever sees the newly created, empty table. It is never
-- created over the claim log or deferred until after the historical copy.
CREATE INDEX IF NOT EXISTS transport_observation_seek
ON transport_observation_index(subject, origin, length(accepted_at_unix_ms), accepted_at_unix_ms, claim_id);
CREATE TABLE IF NOT EXISTS transport_observation_backfill (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    high_water INTEGER NOT NULL,
    cursor INTEGER NOT NULL,
    ready INTEGER NOT NULL CHECK(ready IN (0,1))
);
CREATE TRIGGER IF NOT EXISTS transport_observation_insert AFTER INSERT ON claims
WHEN NEW.kind='transport.observed' BEGIN
    INSERT OR REPLACE INTO transport_observation_index
    VALUES (NEW.id, NEW.subject, NEW.origin, NEW.accepted_at_unix_ms);
END;
CREATE TRIGGER IF NOT EXISTS transport_observation_delete AFTER DELETE ON claims
WHEN OLD.kind='transport.observed' BEGIN
    DELETE FROM transport_observation_index WHERE claim_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS transport_observation_update
AFTER UPDATE OF id, subject, kind, origin, accepted_at_unix_ms ON claims
WHEN OLD.kind='transport.observed' OR NEW.kind='transport.observed' BEGIN
    DELETE FROM transport_observation_index WHERE claim_id=OLD.id;
    INSERT OR REPLACE INTO transport_observation_index
    SELECT NEW.id, NEW.subject, NEW.origin, NEW.accepted_at_unix_ms
    WHERE NEW.kind='transport.observed';
END;
-- The same write transaction installs the triggers and captures this boundary.
INSERT OR IGNORE INTO transport_observation_backfill
SELECT 1, COALESCE(MAX(store_index),0), 0, 0 FROM claims;
"#;

#[derive(Debug)]
pub struct Page {
    pub ready: bool,
    pub copied: usize,
    /// Begin-through-commit upper bound on write hold, including managed finalizers
    /// and any SQLite acquisition wait, but excluding background queue admission.
    pub write_hold: Duration,
}

/// Inspect schema presence before preparing SQL against optional migration tables.
/// The marker is committed atomically with the last page, never before it.
pub fn ready(connection: &Connection) -> Result<bool> {
    let present: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)
         AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='transport_observation_backfill')",
        [NAME], |row| row.get(0),
    )?;
    if !present {
        return Ok(false);
    }
    Ok(connection.query_row(
        "SELECT ready FROM transport_observation_backfill WHERE singleton=1",
        [], |row| row.get(0),
    ).optional()?.unwrap_or(false))
}

static INDEXED_SQL: LazyLock<String> = LazyLock::new(|| {
    let ties = canonical::components("claims").into_iter().skip(2)
        .map(|component| format!("{component} DESC"))
        .collect::<Vec<_>>().join(", ");
    // CROSS JOIN keeps the timestamp-ordered side-index seek outside the claim-id
    // lookup. Only the newest timestamp's canonical ties need partial sorting.
    format!(
        "SELECT claims.body, claims.accepted_at_unix_ms
         FROM transport_observation_index AS observation
         CROSS JOIN claims ON claims.id=observation.claim_id
         WHERE observation.subject=?1 AND observation.origin=?2
         ORDER BY length(observation.accepted_at_unix_ms) DESC,
         observation.accepted_at_unix_ms DESC, {ties} LIMIT 1"
    )
});

pub fn indexed_query() -> &'static str {
    &INDEXED_SQL
}

pub fn latest(connection: &Connection, subject: &str, origin: &str) -> Result<Option<(String, String)>> {
    let query = if ready(connection)? { indexed_query() } else { &ORIGINAL_SQL };
    // Both query choices fetch one full inline body and timestamp. LIMIT 1 bounds
    // rows, not bytes: this hot-path read has no body-size proof.
    Ok(connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h389042a7d9e8 owner=smallclaims::store::transport_observation_index TODO=trace-statement-and-value-representation */ query)?.query_row(/* sqlite-large-value: allow-for-now lv-h7e34d2a4ccc6 owner=smallclaims::store::transport_observation_index TODO=trace-statement-and-value-representation */ params![subject, origin], |row| {
        Ok((row.get(0)?, row.get(1)?))
    }).optional()?)
}

pub fn advance(connection: &WriterConnection) -> Result<Page> {
    advance_with_budget(connection, PAGE_WORK_BUDGET)
}

fn advance_with_budget(connection: &WriterConnection, work_budget: Duration) -> Result<Page> {
    // All claim writers, including deferred read-then-write transactions, share
    // this queue. An independent SQLite writer would invalidate their snapshots.
    let mut writer = connection.write_background();
    // The first page installs empty tables and triggers; no history scan on startup.
    // Subsequent calls never redo DDL or restart the backfill after a crash.
    let installed: bool = writer.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='transport_observation_backfill')",
        [], |row| row.get(0),
    )?;
    let acquired = Instant::now();
    let transaction = writer.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !installed {
        // Executes the schema/capture template above, including its variable keys.
        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h0efe90e42a9b owner=smallclaims::store::transport_observation_index TODO=trace-statement-and-value-representation */ INSTALL)?;
    }
    let (high_water, mut cursor, complete): (i64, i64, bool) = transaction.query_row(
        "SELECT high_water, cursor, ready FROM transport_observation_backfill WHERE singleton=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if complete {
        anyhow::ensure!(ready(&transaction)?, "transport side index is missing; original lookup remains active");
        transaction.commit()?;
        return Ok(Page { ready: true, copied: 0, write_hold: acquired.elapsed() });
    }
    let mut copied = 0;
    let mut done = cursor >= high_water;
    {
        let mut source = transaction.prepare(
            "SELECT store_index FROM claims WHERE store_index>?1 AND store_index<=?2
             ORDER BY store_index",
        )?;
        let mut rows = source.query(params![cursor, high_water])?;
        let mut insert = transaction.prepare(
            // The row range copies variable TEXT keys, never bodies or predecessors.
            /* sqlite-large-value: allow-for-now lv-hd43bf62e6984 owner=smallclaims::store::transport_observation_index TODO=prepare-body-and-predecessor-values */
            "INSERT OR IGNORE INTO transport_observation_index
             SELECT id, subject, origin, accepted_at_unix_ms FROM claims
             WHERE store_index>?1 AND store_index<=?2 AND kind='transport.observed'",
        )?;
        for processed in 0..PAGE_SIZE {
            // Always process one row so even an exhausted budget makes forward progress.
            if processed > 0 && acquired.elapsed() >= work_budget {
                break;
            }
            let Some(row) = rows.next()? else {
                done = true;
                break;
            };
            let end: i64 = row.get(0)?;
            // Two i64 bounds bind the prepared copy statement above; copied keys remain unbounded.
            copied += insert.execute(/* sqlite-large-value: allow-for-now lv-hd341b6569140 owner=smallclaims::store::transport_observation_index TODO=trace-statement-and-value-representation */ params![cursor, end])?;
            cursor = end;
            if cursor >= high_water {
                done = true;
                break;
            }
        }
    }
    transaction.execute(
        // Only an i64 cursor and boolean readiness flag in the singleton row.
        /* sqlite-large-value: allow-for-now lv-hcaf37daca2bb owner=smallclaims::store::transport_observation_index TODO=bound-or-reference-values-and-reclamation */
        "UPDATE transport_observation_backfill SET cursor=?1, ready=?2 WHERE singleton=1",
        params![cursor, done],
    )?;
    transaction.commit()?;
    Ok(Page { ready: done, copied, write_hold: acquired.elapsed() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicU64};

    #[test]
    fn exhausted_work_budget_resumes_at_each_processed_row_without_gaps() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(super::super::SCHEMA).unwrap();
        for sequence in 1..=6 {
            connection.execute(
                "INSERT INTO batches(id, origin, replica_sequence, hash, accepted_at_unix_ms)
                 VALUES (?1, 'local', ?2, ?1, '1')",
                params![sequence.to_string(), sequence],
            ).unwrap();
            connection.execute(
                "INSERT INTO claims(id, batch_id, subject, kind, origin, body,
                 predecessors, accepted_at_unix_ms, store_index)
                 VALUES (?1, ?1, 'host/peer', ?2, 'local', '{}', '[]', '1', ?3)",
                params![sequence.to_string(),
                    if sequence % 2 == 0 { "example.note" } else { "transport.observed" },
                    sequence * 3],
            ).unwrap();
        }
        let open_writer = || WriterConnection::new(
            Connection::open(&path).unwrap(), Arc::new(AtomicU64::new(0)),
        );
        for processed in 1..=6 {
            // A new managed writer each time also proves persisted, not in-memory, resume.
            let page = advance_with_budget(&open_writer(), Duration::ZERO).unwrap();
            let (cursor, high_water): (i64, i64) = connection.query_row(
                "SELECT cursor, high_water FROM transport_observation_backfill",
                [], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(cursor, processed * 3, "stop at the last processed source row");
            assert_eq!(high_water, 18);
            assert_eq!(page.copied, usize::from(processed % 2 != 0));
            assert_eq!(page.ready, processed == 6);
        }
        assert!(ready(&connection).unwrap());
        // Compare both directions to the single-shot source projection, including key values.
        let differences: i64 = connection.query_row(
            "SELECT
             (SELECT COUNT(*) FROM (
              SELECT id, subject, origin, accepted_at_unix_ms FROM claims WHERE kind='transport.observed'
              EXCEPT SELECT claim_id, subject, origin, accepted_at_unix_ms FROM transport_observation_index))
             + (SELECT COUNT(*) FROM (
              SELECT claim_id, subject, origin, accepted_at_unix_ms FROM transport_observation_index
              EXCEPT SELECT id, subject, origin, accepted_at_unix_ms FROM claims WHERE kind='transport.observed'))",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(differences, 0);
    }
}
