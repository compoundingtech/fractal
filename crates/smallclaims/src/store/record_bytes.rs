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
//! The row stays for now: its state, claim, position and repair link are what older builds read,
//! and an older build that runs this store after a rollback finds every row it expects. Readers
//! in this build no longer need it (`replica_record_rows`, `admitted_claim_sql`), so a later
//! release can delete valid records once older builds are fenced out. Only
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

/// Whether the claim `alias` has a record in a state other than valid: a record a repair
/// replaced, or one marked invalid with its envelope. Such a claim is not an accepted copy.
pub fn claim_record_not_valid_sql(alias: &str) -> String {
    format!("EXISTS(SELECT 1 FROM replica_records WHERE claim_id={alias}.id AND state<>'valid')")
}

/// Whether the claim `alias` was sealed or admitted and is an accepted copy: its batch names an
/// envelope this member holds or released (`envelope_rows`), and it has no record in another state ([`claim_record_not_valid_sql`]). It asks
/// what `EXISTS(... replica_records WHERE claim_id=... AND state='valid')` asked, whether or not
/// the claim's valid record is stored.
pub fn admitted_claim_sql(alias: &str) -> String {
    // Each table on its own: a view compound is not searched by a correlated term.
    format!(
        "((EXISTS(SELECT 1 FROM replica_envelopes WHERE batch_id={alias}.batch_id)
           OR EXISTS(SELECT 1 FROM released_envelopes WHERE batch_id={alias}.batch_id))
          AND NOT {})",
        claim_record_not_valid_sql(alias)
    )
}

/// How many records the envelope `alias` (a row of `replica_envelopes` or
/// `released_envelopes`) has in `replica_record_rows`: its stored records, and one for each claim
/// of its batch with a position that no stored record of it holds. SQLite runs a view compound
/// once for every row a correlated count reads, so counting through the view scanned every
/// record and claim; each term here seeks its own index.
pub fn envelope_record_count_sql(alias: &str) -> String {
    format!(
        "((SELECT COUNT(*) FROM replica_records AS counted
           WHERE counted.writer={alias}.writer AND counted.sequence={alias}.sequence
             AND counted.envelope_hash={alias}.envelope_hash)
          + (SELECT COUNT(*) FROM claims AS counted INDEXED BY claims_batch_index
             JOIN claim_positions AS positions ON positions.claim_id=counted.id
             WHERE counted.batch_id={alias}.batch_id AND NOT EXISTS (
                 SELECT 1 FROM replica_records AS stored
                 WHERE stored.writer={alias}.writer AND stored.sequence={alias}.sequence
                   AND stored.envelope_hash={alias}.envelope_hash AND stored.position=positions.position)))"
    )
}

/// Whether the envelope `alias` has a record in `replica_record_rows` that is not repaired, of a
/// claim accepted at or after `cut`: a late envelope, which no cut before then holds. Each term
/// seeks its own index, as in [`envelope_record_count_sql`].
pub fn envelope_has_late_record_sql(alias: &str, cut: &str) -> String {
    format!(
        "(EXISTS(SELECT 1 FROM replica_records AS other CROSS JOIN claims AS late ON late.id=other.claim_id
              WHERE other.writer={alias}.writer AND other.sequence={alias}.sequence
                AND other.envelope_hash={alias}.envelope_hash AND other.state<>'repaired'
                AND CAST(late.accepted_at_unix_ms AS INTEGER) >= {cut})
          OR EXISTS(SELECT 1 FROM claims AS late INDEXED BY claims_batch_index
              JOIN claim_positions AS positions ON positions.claim_id=late.id
              WHERE late.batch_id={alias}.batch_id AND CAST(late.accepted_at_unix_ms AS INTEGER) >= {cut}
                AND NOT EXISTS (
                    SELECT 1 FROM replica_records AS stored
                    WHERE stored.writer={alias}.writer AND stored.sequence={alias}.sequence
                      AND stored.envelope_hash={alias}.envelope_hash AND stored.position=positions.position)))"
    )
}

/// The table repairs wait in for a record that is gone (`await_repaired_record_tx`).
pub const VIEW_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h76ab6030e905 owner=smallclaims::store::record_bytes TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS record_repair_lookups (record_ref TEXT PRIMARY KEY) WITHOUT ROWID;
";

/// Assert that `sql` seeks every table it reads: no step scans a table or builds an automatic
/// index. A view compound or a subquery read as it is produced (`SCAN records`) is not a table.
#[cfg(test)]
pub(crate) fn assert_plan_seeks(connection: &Connection, sql: &str) {
    let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let count = statement.parameter_count();
    let plan = statement
        .query_map(rusqlite::params_from_iter(vec![rusqlite::types::Null; count]), |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let produced = |step: &str| {
        step == "SCAN CONSTANT ROW"
            || step == "SCAN records"
            || step == "SCAN replica_record_rows"
            || step.starts_with("SCAN (subquery-")
    };
    let scans: Vec<&String> = plan
        .iter()
        .filter(|step| (step.starts_with("SCAN ") && !produced(step)) || step.contains("AUTOMATIC"))
        .collect();
    assert!(scans.is_empty(), "{scans:?} in {plan:#?} for {sql}");
}

/// Every record a reader of `replica_records` used to see: the stored rows (a record that is not
/// valid, a repaired one, or a valid one a store has not yet deleted), and for each valid claim
/// whose record is gone the row it had: its envelope, found by the claim's batch among the
/// envelopes this member holds or released (`envelope_release`), its position and its claim. A
/// valid claim needs no stored record (`record_bytes`), so a reader that asks which envelope holds
/// a claim, or how many records an envelope holds, reads this view instead. It holds no data of
/// its own. The two envelope tables are joined each in its own branch: a term on the view then
/// reaches every table's index, which a nested view compound would not.
pub const RECORD_ROWS_VIEW: &str = /* sqlite-large-value: allow-for-now lv-hcb990d31fe61 owner=smallclaims::store::record_bytes TODO=chunk-or-reference-exact-wire-values */ "
CREATE VIEW IF NOT EXISTS replica_record_rows(writer, sequence, envelope_hash, position, state, claim_id) AS
    SELECT writer, sequence, envelope_hash, position, state, claim_id FROM replica_records
    UNION ALL
    SELECT envelopes.writer, envelopes.sequence, envelopes.envelope_hash, positions.position,
           'valid', claims.id
    FROM claims
    JOIN replica_envelopes AS envelopes ON envelopes.batch_id=claims.batch_id
    JOIN claim_positions AS positions ON positions.claim_id=claims.id
    WHERE NOT EXISTS (
        SELECT 1 FROM replica_records AS stored
        WHERE stored.writer=envelopes.writer AND stored.sequence=envelopes.sequence
          AND stored.envelope_hash=envelopes.envelope_hash AND stored.position=positions.position)
    UNION ALL
    SELECT envelopes.writer, envelopes.sequence, envelopes.envelope_hash, positions.position,
           'valid', claims.id
    FROM claims
    JOIN released_envelopes AS envelopes ON envelopes.batch_id=claims.batch_id
    JOIN claim_positions AS positions ON positions.claim_id=claims.id
    WHERE NOT EXISTS (
        SELECT 1 FROM replica_records AS stored
        WHERE stored.writer=envelopes.writer AND stored.sequence=envelopes.sequence
          AND stored.envelope_hash=envelopes.envelope_hash AND stored.position=positions.position);
";

/// What a trigger on `replica_records` that fires on DELETE adds to its condition: a valid
/// record's deletion changes no claim's state or position, since its claim keeps its row and its
/// position stays in `claim_positions`, so only a record that is not valid wakes it.
pub const RECORD_DELETE_MATTERS: &str = "OLD.state<>'valid'";

/// Replace `name`, a trigger or view, when the store holds a definition other than `sql` (a
/// `CREATE ... IF NOT EXISTS` would keep the old one), so a reopen with the same definition
/// changes nothing. An older build that runs the store again puts its own definition back the
/// same way.
pub fn replace_if_changed(connection: &Connection, kind: &str, name: &str, sql: &str) -> Result<()> {
    let tokens = |sql: &str| {
        sql.replace("IF NOT EXISTS ", "").trim().trim_end_matches(';').split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let installed: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type=lower(?1) AND name=?2",
            [kind, name],
            |row| row.get(0),
        )
        .optional()?;
    if installed.as_deref().is_some_and(|installed| tokens(installed) != tokens(sql)) {
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hb6e94116e7da owner=smallclaims::store::record_bytes TODO=trace-statement-and-value-representation */ &format!("DROP {kind} {name}"))?;
    }
    Ok(())
}

/// [`replace_if_changed`] for each trigger `names` defines in `schema`, a batch of
/// `CREATE TRIGGER IF NOT EXISTS name ... END;` statements, before the batch runs.
pub fn replace_changed_triggers(connection: &Connection, schema: &str, names: &[&str]) -> Result<()> {
    for name in names {
        let marker = format!(/* sqlite-large-value: allow-for-now lv-h00a91f12b797 owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER IF NOT EXISTS {name} ");
        let start = schema.find(&marker).with_context(|| format!("missing trigger {name}"))?;
        let length = schema[start..].find("\nEND;").with_context(|| format!("unterminated trigger {name}"))?;
        replace_if_changed(connection, "TRIGGER", name, &schema[start..start + length + "\nEND;".len()])?;
    }
    Ok(())
}

/// The storage version from which a store keeps no valid record at all: it commits with the
/// first deletion of one (`release_valid_records`). Below it, admission and sealing still store
/// a valid record (without bytes), so a member rolled back before then finds a record for every
/// claim; from it, a build that reads only up to version 26 refuses the store. (23 and 24 are retired, 25 and
/// 26 belong to the blob file and signature compaction.)
pub const VALID_RECORDS_GONE_VERSION: u32 = 27;

/// Whether the daemon deletes valid records. Off until the signature-reference writer turns on by
/// its own flag rather than at storage version 21 and above (#6765): committing version 27 would
/// otherwise switch that writer on too.
pub const VALID_RECORD_DELETION: bool = false;

/// Whether this store still stores a record for each valid claim.
pub fn valid_records_kept(connection: &Connection) -> Result<bool> {
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version < VALID_RECORDS_GONE_VERSION)
}

/// Remove the record an earlier attempt kept for a claim or blob that is valid now. A repaired
/// record stays: the repair is what it records.
pub(crate) fn drop_kept_record_tx(connection: &Connection, record_ref: &str, previous_state: Option<&str>) -> Result<()> {
    if previous_state.is_some_and(|state| state != "repaired") {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h6c76a8d11254 owner=smallclaims::store::record_bytes TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_records WHERE record_ref=?1 AND state<>'repaired'",
            [record_ref],
        )?;
    }
    Ok(())
}

const RECORDS_CURSOR: &str = "valid_records_cursor";
const FIRST_RUN: &str = "valid_records_first_run_unix_ms";
/// The records past the cursor one deleting page reads.
const DELETE_PAGE_SQL: &str = "SELECT rowid,
        state='valid' AND claim_id IS NOT NULL
        -- A forked writer sequence (two envelopes of one batch) keeps its records, so the
        -- view never shows one envelope's claims under the other.
        AND (SELECT COUNT(*) FROM replica_envelopes WHERE batch_id=
              (SELECT batch_id FROM claims WHERE id=replica_records.claim_id))
          + (SELECT COUNT(*) FROM released_envelopes WHERE batch_id=
              (SELECT batch_id FROM claims WHERE id=replica_records.claim_id))=1
        -- A repair names its record; keep any record a repair claim names.
        AND NOT EXISTS (SELECT 1 FROM claims repairs INDEXED BY claims_kind_index
                        WHERE repairs.kind='record.repaired'
                          AND json_extract(repairs.body,'$.fields.record')=replica_records.record_ref)
     FROM replica_records NOT INDEXED WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

/// What one page of [`Store::release_valid_records`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ValidRecordsRelease {
    pub scanned: usize,
    pub deleted: usize,
    /// Nothing is deleted before this time (unix ms): the rollback window.
    pub waiting_until_unix_ms: Option<u128>,
    pub done: bool,
    pub work: Duration,
}

impl Store {
    /// Delete one bounded page of valid replica records, once `delete_after_ms` has passed since
    /// this store first ran a build that does this. A valid claim needs no record: its row, its
    /// position and its envelope say everything the record did (`replica_record_rows`). The first
    /// deletion commits storage version [`VALID_RECORDS_GONE_VERSION`] in its own transaction, so
    /// from then on admission and sealing store no valid record either and an older build that
    /// would read one refuses the store. The cursor commits with each page and stays after the
    /// pass, as the bytes pass's does.
    pub fn release_valid_records(&self, delete_after_ms: u64) -> Result<ValidRecordsRelease> {
        self.connection
            .batched(|tx| release_records_page(tx, delete_after_ms))
            .map_err(anyhow::Error::msg)?
    }
}

fn release_records_page(tx: &Connection, delete_after_ms: u64) -> Result<ValidRecordsRelease> {
    let now = now_ms();
    let first: Option<String> = tx
        .query_row("SELECT value FROM meta WHERE key=?1", [FIRST_RUN], |row| row.get(0))
        .optional()?;
    let first = match first {
        Some(value) => value.parse::<u128>()?,
        None => {
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-heef38b6b9ef7 owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)",
                params![FIRST_RUN, now.to_string()],
            )?;
            now
        }
    };
    let opens = first + u128::from(delete_after_ms);
    if now < opens && valid_records_kept(tx)? {
        return Ok(ValidRecordsRelease { waiting_until_unix_ms: Some(opens), ..Default::default() });
    }
    let mut after: i64 = tx
        .query_row("SELECT value FROM meta WHERE key=?1", [RECORDS_CURSOR], |row| row.get::<_, String>(0))
        .optional()?
        .map(|value| value.parse::<i64>())
        .transpose()?
        .unwrap_or(0);
    let started = Instant::now();
    let mut report = ValidRecordsRelease::default();
    let page = tx
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h357fd48bba84 owner=smallclaims::store::record_bytes TODO=trace-statement-and-value-representation */ DELETE_PAGE_SQL)?
        .query_map(params![after, PAGE], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut delete = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h900de0bb74f9 owner=smallclaims::store::record_bytes TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_records WHERE rowid=?1")?;
    for (rowid, deletable) in &page {
        if *deletable {
            if report.deleted == 0 && valid_records_kept(tx)? {
                tx.pragma_update(None, "user_version", VALID_RECORDS_GONE_VERSION)?;
            }
            delete.execute(/* sqlite-large-value: allow-for-now lv-h6fab9c5ca933 owner=smallclaims::store::record_bytes TODO=trace-statement-and-value-representation */ [rowid])?;
            report.deleted += 1;
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
            /* sqlite-large-value: allow-for-now lv-h0348474f786c owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![RECORDS_CURSOR, after.to_string()],
        )?;
    }
    Ok(report)
}

const LOOKUP_CURSOR: &str = "record_repair_lookup_cursor";

/// A repair names the record it replaces by `record_ref`, a hash of the record's envelope and
/// position. Once a valid record is gone that hash names no row, so a repair that finds none
/// waits here until [`Store::resolve_record_repairs`] or admission puts the record back.
pub(crate) fn await_repaired_record_tx(connection: &Connection, record_ref: &str) -> Result<()> {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hfa5c35e151c6 owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO record_repair_lookups(record_ref) VALUES (?1)",
        [record_ref],
    )?;
    Ok(())
}

/// Store the record of a valid claim a repair waits for (`await_repaired_record_tx`), so the
/// repair finds it. True when one was waiting.
pub(crate) fn restore_awaited_record_tx(
    connection: &Connection,
    record_ref: &str,
    writer: &str,
    sequence: u64,
    envelope_hash: &str,
    position: u64,
    claim_id: &str,
) -> Result<bool> {
    if connection.execute(/* sqlite-large-value: allow-for-now lv-hd6ff7b9acb84 owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM record_repair_lookups WHERE record_ref=?1", [record_ref])? == 0 {
        return Ok(false);
    }
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h210a8491b266 owner=smallclaims::store::record_bytes TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO replica_records(record_ref, writer, sequence, envelope_hash, position, raw,
             state, claim_id, subject_hint, kind_hint, updated_at_unix_ms)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, 'valid', id, subject, kind, ?7 FROM claims WHERE id=?8",
        params![record_ref, writer, sequence, envelope_hash, position, VALID_RECORD_RAW, now_ms().to_string(), claim_id],
    )?;
    Ok(true)
}

/// What one page of [`Store::resolve_record_repairs`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecordRepairResolution {
    pub waiting: usize,
    pub scanned: usize,
    pub restored: usize,
    pub done: bool,
}

impl Store {
    /// Look for the records repairs wait for, one bounded page of the store's valid claims at a
    /// time, by computing each one's record reference. A page that restores one applies the
    /// repairs again. A walk that ends with repairs still waiting starts over at the next call:
    /// their records have not arrived, and admission restores one that does.
    pub fn resolve_record_repairs(&self) -> Result<RecordRepairResolution> {
        let resolution = self
            .connection
            .batched(resolve_page)
            .map_err(anyhow::Error::msg)??;
        if resolution.restored > 0 {
            self.apply_replication_repairs()?;
        }
        Ok(resolution)
    }
}

fn resolve_page(tx: &Transaction<'_>) -> Result<RecordRepairResolution> {
    let mut report = RecordRepairResolution {
        waiting: tx.query_row("SELECT COUNT(*) FROM record_repair_lookups", [], |row| row.get(0))?,
        ..Default::default()
    };
    if report.waiting == 0 {
        report.done = true;
        return Ok(report);
    }
    let mut after: i64 = tx
        .query_row("SELECT value FROM meta WHERE key=?1", [LOOKUP_CURSOR], |row| row.get::<_, String>(0))
        .optional()?
        .map(|value| value.parse::<i64>())
        .transpose()?
        .unwrap_or(0);
    let started = Instant::now();
    let rows = tx
        .prepare_cached(
            // A claim's batch has the envelopes this member holds or the one it released
            // (`envelope_release`), never both.
            "SELECT claims.store_index, claims.id, COALESCE(held.writer, released.writer),
                    COALESCE(held.sequence, released.sequence),
                    COALESCE(held.envelope_hash, released.envelope_hash), positions.position
             FROM claims
             JOIN claim_positions AS positions ON positions.claim_id=claims.id
             LEFT JOIN replica_envelopes AS held ON held.batch_id=claims.batch_id
             LEFT JOIN released_envelopes AS released ON released.batch_id=claims.batch_id
             WHERE claims.store_index>?1 AND (held.writer IS NOT NULL OR released.writer IS NOT NULL)
             ORDER BY claims.store_index LIMIT ?2",
        )?
        .query_map(params![after, PAGE], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?, row.get::<_, String>(4)?, row.get::<_, u64>(5)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (index, claim, writer, sequence, hash, position) in &rows {
        let record_ref = crate::hash::replica_record_ref(writer, *sequence, hash, *position);
        if restore_awaited_record_tx(tx, &record_ref, writer, *sequence, hash, *position, claim)? {
            report.restored += 1;
        }
        after = *index;
        report.scanned += 1;
        if started.elapsed() >= BUDGET {
            break;
        }
    }
    // A finished walk starts over: what still waits may arrive later.
    report.done = rows.is_empty();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-hce4ffb6fc1bd owner=smallclaims::store::record_bytes TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![LOOKUP_CURSOR, if report.done { "0".to_owned() } else { after.to_string() }],
    )?;
    Ok(report)
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

#[cfg(test)]
mod view_plan_tests {
    use super::*;
    use crate::store::runtime::Plain;

    /// Without statistics the planner assumes large tables, as on a real store.
    #[test]
    fn envelope_record_counts_and_late_checks_seek_and_scan_nothing() {
        let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
        let connection = store.connection.write();
        for sql in [
            format!(
                "SELECT {} FROM replica_envelopes envelopes WHERE rowid>?1 AND rowid<=?2",
                envelope_record_count_sql("envelopes")
            ),
            format!(
                "SELECT {} FROM replica_envelopes envelopes WHERE writer=?1",
                envelope_has_late_record_sql("envelopes", "?2")
            ),
            "SELECT COUNT(*) FROM replica_record_rows WHERE claim_id=?1".into(),
            crate::store::checkpoint::sealed_envelope_records_sql(),
        ] {
            assert_plan_seeks(&connection, &sql);
        }
    }
}
