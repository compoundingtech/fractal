//! A valid replica record keeps no bytes. Its claim is in `claims`, its position in
//! `claim_positions`, and the signed bytes it came from in its envelope's payload. Nothing reads
//! a valid record's `raw` again: admission retries, repairs and diagnostics read only records
//! that are not valid, and a retry re-reads the envelope. Admission and sealing store an empty
//! `raw` for a valid record, and this pass empties the copies a store kept before, in bounded
//! pages on the writer queue, so the hourly free-page pass can give their pages back.
//!
//! Only a record that was never admitted as valid keeps its bytes. A valid record that a repair
//! later replaces, or that is marked invalid with its envelope, keeps the empty `raw`: no reader
//! takes a record's bytes for those either.
//!
//! The row stays: its state, claim, position and repair link are what readers and triggers use,
//! and an older build that runs this store after a rollback finds every row it expects. Only
//! the triggers on `replica_records` that fire on any UPDATE notice the change, so they list the
//! columns they read (`RECORD_COLUMNS_BUT_RAW`) and an emptied `raw` wakes none of them.
use super::*;
use std::time::{Duration, Instant};

/// What a valid record keeps in `raw`.
pub const VALID_RECORD_RAW: &[u8] = &[];

/// Every column of `replica_records` except `raw`: what a trigger that must see any other change
/// lists in its `UPDATE OF`, so emptying a valid record's bytes fires nothing.
pub const RECORD_COLUMNS_BUT_RAW: &str = "record_ref,writer,sequence,envelope_hash,position,state,\
    claim_id,subject_hint,kind_hint,error_code,error_message,replacement_claim_id,updated_at_unix_ms";

const CURSOR: &str = "valid_record_bytes_cursor";
/// The next page of valid records that still keep bytes. NOT INDEXED keeps the walk on the
/// rowid range: the state index would gather every valid record and sort them for each page.
/// It reads every record past the cursor, not only those with bytes, so a page's reads stay
/// bounded however few records past the cursor still hold bytes.
const PAGE_SQL: &str = "SELECT rowid, state='valid' AND length(raw)>0, length(raw)
     FROM replica_records NOT INDEXED WHERE rowid>?1 ORDER BY rowid LIMIT ?2";
const PAGE: usize = 512;
/// One page's share of a writer batch. A row's update rewrites its leaf page and frees its
/// overflow pages, and the commit writes them, so the page stops early to keep the whole
/// writer hold well under 100 ms.
const BUDGET: Duration = Duration::from_millis(10);
/// The free pages one write of the pass gives back: a quarter of the hourly pass's chunk, since
/// this pass gives back thousands in a row.
pub const FREE_PAGES_PER_WRITE: u64 = 128;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecordBytesRelease {
    pub scanned: usize,
    pub emptied: usize,
    pub bytes: u64,
    pub done: bool,
    /// The page's work inside its transaction, before the commit.
    pub work: Duration,
}

impl Store {
    /// Empty one bounded page of valid records' bytes. The cursor commits with the page, so an
    /// interrupted pass resumes at the committed row. It is the last record the pass read, and
    /// it stays after the pass finishes: the next start reads only the records added since,
    /// which this build stored without bytes, or an older build stored with them after a
    /// rollback.
    pub fn release_valid_record_bytes(&self) -> Result<RecordBytesRelease> {
        self.connection
            .batched(|tx| release_page(tx))
            .map_err(anyhow::Error::msg)?
    }

    /// Whether the pass has read every record this store holds.
    pub fn valid_record_bytes_released(&self) -> Result<bool> {
        let connection = self.readers.get();
        let newest: i64 = connection.query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM replica_records",
            [],
            |row| row.get(0),
        )?;
        Ok(cursor(&connection)? >= newest)
    }
}

fn cursor(connection: &Connection) -> Result<i64> {
    let value: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE key=?1", [CURSOR], |row| row.get(0))
        .optional()?;
    Ok(value.map(|value| value.parse::<i64>()).transpose()?.unwrap_or(0))
}

fn release_page(tx: &Connection) -> Result<RecordBytesRelease> {
    let mut after = cursor(tx)?;
    let started = Instant::now();
    let mut report = RecordBytesRelease::default();
    let mut rows = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hbdff2ded9fa8 owner=smallclaims::store::record_bytes TODO=trace-statement-and-value-representation */ PAGE_SQL)?;
    let mut empty = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h4e2a24c654f7 owner=smallclaims::store::record_bytes TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET raw=?1 WHERE rowid=?2")?;
    let page = rows
        .query_map(params![after, PAGE], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?, row.get::<_, u64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (rowid, holds_bytes, bytes) in &page {
        if *holds_bytes {
            empty.execute(/* sqlite-large-value: allow-for-now lv-he251c91cb160 owner=smallclaims::store::record_bytes TODO=trace-statement-and-value-representation */ params![VALID_RECORD_RAW, rowid])?;
            report.emptied += 1;
            report.bytes += bytes;
        }
        after = *rowid;
        report.scanned += 1;
        if started.elapsed() >= BUDGET {
            break;
        }
    }
    report.done = page.is_empty();
    report.work = started.elapsed();
    if report.scanned > 0 {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h2d7339b795d8 owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![CURSOR, after.to_string()],
        )?;
    }
    Ok(report)
}

/// The event and name suffix of a trigger that fires on UPDATE of `table`. On `replica_records`
/// it lists every column but `raw`, under a new name so that opening a store that has the
/// unlisted trigger replaces it (`replaced_update_trigger`).
pub fn update_event(table: &str) -> (String, &'static str) {
    if table == "replica_records" {
        (format!("UPDATE OF {RECORD_COLUMNS_BUT_RAW}"), "_columns")
    } else {
        ("UPDATE".to_owned(), "")
    }
}

/// Drop `name`, a trigger that fired on any UPDATE of `replica_records`, before its listed
/// replacement is created. An older build that runs this store again creates it beside the
/// replacement; both only mark a claim pending, so that marks nothing twice.
pub fn replaced_update_trigger(connection: &Connection, table: &str, name: &str) -> Result<()> {
    if table == "replica_records" {
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h3b2315a6c7ee owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS {name}"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::runtime::Plain;

    #[test]
    fn a_page_seeks_the_rowid_range_and_sorts_nothing() {
        let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
        let connection = store.connection.write();
        // Statistics that favour the state index, as a large store's do.
        connection.execute_batch("ANALYZE").unwrap();
        let plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {PAGE_SQL}"))
            .unwrap()
            .query_map(params![0, PAGE], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(plan, ["SEARCH replica_records USING INTEGER PRIMARY KEY (rowid>?)"]);
    }
}
