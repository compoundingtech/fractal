//! A local cut's capture, scoped to the kinds it drops (`LOCAL_TRIM_KINDS`) and the kinds judging
//! them reads (`LOCAL_TRIM_CONTEXT_KINDS`). A full capture
//! stages every sealed envelope, record and claim, and its planner keeps every other kind
//! anyway (`protect_all_but`). The scoped one stages only what judging a claim of a scoped kind
//! reads, so its plan drops the same claims and envelopes:
//!
//! - the envelopes that hold a record of such a claim, whole, with their records and claims, and
//!   the envelopes of the claims that share an operation with one, so slots, operations,
//!   duplicates, envelope and record-count rules see what they see today;
//! - the same for every claim of a context kind, so the status-history rule judges each seat's
//!   harness observations with its runtime resets and diagnostics, as a full capture does;
//! - the protections, looked up for the staged claims by each table's claim index, or read from
//!   the few rows a repair writes;
//! - each staged writer's newest envelope before the cut, for the writer's-newest guard, in place
//!   of every earlier checkpoint's tombstones (which only the drop digest reads);
//! - the citations of the claims the cut could drop, from the store's citation cache
//!   (`local_trim_citations`), caught up first, counting a citer only if it is present and
//!   sealed at the cut, as a full capture would stage it.
//!
//! Every envelope here is read from `replica_envelopes` and `released_envelopes` alike, by the
//! rowid it had: one this member released (`envelope_release`) is staged as the stored one was,
//! so its claims are still cut. Each table is read in its own branch and each record count by
//! its own seeks (`envelope_record_count_sql`): SQLite runs a view compound in full for a
//! correlated term.
use super::*;
use crate::store::checkpoint::sealed_envelope_records_sql;
use crate::store::record_bytes::{envelope_has_late_record_sql, envelope_record_count_sql};
use std::sync::LazyLock;

/// The envelopes up to the seal row that hold a record of claim `?1`, each with its accepted
/// time and record count: the stored ones, then those this member released.
static HELD: LazyLock<String> = LazyLock::new(|| {
    let branch = |table: &str, rowid: &str| {
        format!(
            "SELECT envelopes.writer, envelopes.sequence, envelopes.envelope_hash,
                    envelopes.accepted_at_unix_ms, {}
             FROM replica_record_rows records JOIN {table} envelopes
               ON envelopes.writer=records.writer AND envelopes.sequence=records.sequence
                 AND envelopes.envelope_hash=records.envelope_hash
             WHERE records.claim_id=?1 AND envelopes.{rowid} <= ?2",
            envelope_record_count_sql("envelopes")
        )
    };
    format!("{} UNION ALL {}", branch("replica_envelopes", "rowid"), branch("released_envelopes", "original_rowid"))
});

/// Writer `?1`'s newest envelope up to the seal row `?2` and before the cut `?3`, with its record
/// count, that holds no claim dated at or after the cut, stored or released.
static NEWEST_SEALED: LazyLock<String> = LazyLock::new(|| {
    let branch = |table: &str, rowid: &str| {
        format!(
            "SELECT * FROM (SELECT sequence, envelope_hash, accepted_at_unix_ms, {}
             FROM {table} envelopes
             WHERE writer=?1 AND {rowid} <= ?2 AND CAST(accepted_at_unix_ms AS INTEGER) < ?3 AND NOT {}
             ORDER BY sequence DESC LIMIT 1)",
            envelope_record_count_sql("envelopes"),
            envelope_has_late_record_sql("envelopes", "?3")
        )
    };
    format!(
        "{} UNION ALL {} ORDER BY 1 DESC LIMIT 1",
        branch("replica_envelopes", "rowid"),
        branch("released_envelopes", "original_rowid")
    )
});

/// Whether claim `?1` is present with a record that is not repaired, in an envelope up to the
/// seal row `?2` and before the cut `?3` that holds no claim dated at or after the cut.
static SEALED_CITER: LazyLock<String> = LazyLock::new(|| {
    let branch = |table: &str, rowid: &str| {
        format!(
            "EXISTS(
               SELECT 1 FROM replica_record_rows records JOIN {table} envelopes
                 ON envelopes.writer=records.writer AND envelopes.sequence=records.sequence
                   AND envelopes.envelope_hash=records.envelope_hash
               WHERE records.claim_id=?1 AND records.state<>'repaired'
                 AND envelopes.{rowid} <= ?2 AND CAST(envelopes.accepted_at_unix_ms AS INTEGER) < ?3
                 AND NOT {})",
            envelope_has_late_record_sql("envelopes", "?3")
        )
    };
    format!(
        "SELECT EXISTS(SELECT 1 FROM claims citer JOIN batches ON batches.id=citer.batch_id WHERE citer.id=?1)
             AND ({} OR {})",
        branch("replica_envelopes", "rowid"),
        branch("released_envelopes", "original_rowid")
    )
});

/// An envelope holding a record of a staged claim: its key, accepted time and record count.
type ClaimEnvelope = (EnvelopeKey, u128, i64);

impl StagedSet {
    fn scope_kinds(&self) -> &[String] {
        self.scope.as_deref().unwrap_or_default()
    }

    /// The envelopes holding a record of a claim of a scoped kind, or of a claim sharing its
    /// operation: kind by kind (`bound`), each in store-index order (`after`), by
    /// `claims_kind_index`, `claims_operation_index` and `replica_records_claim`.
    pub(super) fn capture_scoped_envelopes(&self, store: &Store, after: i64, bound: Option<i64>) -> Result<()> {
        let kind = usize::try_from(bound.unwrap_or(0))?;
        let Some(kind_name) = self.scope_kinds().get(kind) else {
            let transaction = self.staging.unchecked_transaction()?;
            Self::advance(&transaction, RECORDS, 0, None)?;
            return Ok(transaction.commit()?);
        };
        let page: Vec<(i64, Vec<ClaimEnvelope>)> = store.checkpoint_capture_page(self.epoch, || {
            let connection = store.readers.get();
            let claims = connection
                .prepare_cached(
                    "SELECT store_index, id FROM claims WHERE kind=?1 AND store_index>?2
                     ORDER BY store_index LIMIT ?3",
                )?
                .query_map(params![kind_name, after, STAGING_PAGE], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut held = connection.prepare_cached(&HELD)?;
            let mut partners = connection.prepare_cached(
                "SELECT partner.id FROM claims AS claim JOIN claims AS partner
                   ON json_extract(partner.body, '$._operation.id')=json_extract(claim.body, '$._operation.id')
                 WHERE claim.id=?1 AND json_extract(claim.body, '$._operation.id') IS NOT NULL
                   AND json_extract(partner.body, '$._operation.id') IS NOT NULL AND partner.id<>claim.id",
            )?;
            claims
                .into_iter()
                .map(|(index, id)| {
                    let mut ids = vec![id.clone()];
                    ids.extend(partners.query_map([&id], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
                    let mut envelopes = Vec::new();
                    for id in &ids {
                        envelopes.extend(
                            held.query_map(params![id, self.seal_rowid], |row| {
                                let accepted: String = row.get(3)?;
                                Ok((
                                    EnvelopeKey { writer: row.get(0)?, sequence: row.get(1)?, envelope_hash: row.get(2)? },
                                    checkpoint_accepted_time(&accepted, id, 3)?,
                                    row.get::<_, i64>(4)?,
                                ))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?,
                        );
                    }
                    Ok((index, envelopes))
                })
                .collect()
        })?;
        let transaction = self.staging.unchecked_transaction()?;
        let Some(last) = page.last().map(|(index, _)| *index) else {
            Self::advance(&transaction, ENVELOPES, 0, Some(i64::try_from(kind)? + 1))?;
            return Ok(transaction.commit()?);
        };
        {
            let mut insert = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-h6907f5361ba9 owner=smallclaims::store::checkpoint_staging::scoped TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO envelopes VALUES (?1, ?2, ?3, ?4, ?5, 1)")?;
            let mut number = transaction.prepare_cached(
                /* sqlite-large-value: allow-for-now lv-hc9f6acb432b0 owner=smallclaims::store::checkpoint_staging::scoped TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO scoped_envelopes(writer, sequence, envelope_hash) VALUES (?1, ?2, ?3)",
            )?;
            for (key, accepted_at, records) in page.into_iter().flat_map(|(_, envelopes)| envelopes) {
                if accepted_at < self.cut_unix_ms {
                    insert.execute(/* sqlite-large-value: allow-for-now lv-ha785421a488b owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ params![key.writer, key.sequence, key.envelope_hash, accepted(accepted_at)?, records])?;
                    number.execute(/* sqlite-large-value: allow-for-now lv-h578eae135705 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ params![key.writer, key.sequence, key.envelope_hash])?;
                }
            }
        }
        Self::advance(&transaction, ENVELOPES, last, Some(i64::try_from(kind)?))?;
        Ok(transaction.commit()?)
    }

    /// The records of the scoped envelopes, an envelope at a time by its key, as a full capture
    /// reads every record.
    pub(super) fn capture_scoped_records(&self, store: &Store, after: i64) -> Result<()> {
        let envelopes = self
            .staging
            .prepare_cached("SELECT n, writer, sequence, envelope_hash FROM scoped_envelopes WHERE n>?1 ORDER BY n LIMIT ?2")?
            .query_map(params![after, STAGING_PAGE], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    EnvelopeKey { writer: row.get(1)?, sequence: row.get(2)?, envelope_hash: row.get(3)? },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some(last) = envelopes.last().map(|(n, _)| *n) else {
            let transaction = self.staging.unchecked_transaction()?;
            Self::advance(&transaction, ORDER, 0, None)?;
            return Ok(transaction.commit()?);
        };
        let cut = accepted(self.cut_unix_ms)?;
        let sql = sealed_envelope_records_sql();
        let page: Vec<RecordRow> = store.checkpoint_capture_page(self.epoch, || {
            let connection = store.readers.get();
            let mut records = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hbb840ed5c2a8 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ &sql)?;
            let mut page = Vec::new();
            for (_, key) in &envelopes {
                page.extend(
                    records
                        .query_map(params![key.writer, key.sequence, cut, self.seal_rowid, key.envelope_hash], |row| {
                            Ok((
                                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                                row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                                row.get(10)?, row.get(11)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                );
            }
            Ok(page)
        })?;
        let transaction = self.staging.unchecked_transaction()?;
        self.stage_record_rows(&transaction, page)?;
        Self::advance(&transaction, RECORDS, last, None)?;
        Ok(transaction.commit()?)
    }

    /// A protection, for the staged claims only. The projection tables are read by their claim
    /// index, a page of staged claims at a time; repairs by the kind and state their rows carry.
    /// `documents` has no claim index and is small, so it is read as a full capture reads it.
    pub(super) fn capture_scoped_protection(&self, store: &Store, phase: i64, after: i64, bound: Option<i64>) -> Result<()> {
        let (table, column, _) = PROTECTIONS[usize::try_from(phase - PROTECTED)?];
        let sql = match table {
            "desired" | "mission_definitions" | "mission_revisions" => {
                let staged = self
                    .staging
                    .prepare_cached("SELECT pos, id FROM claims WHERE pos>?1 ORDER BY pos LIMIT ?2")?
                    .query_map(params![after, STAGING_PAGE], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let Some(last) = staged.last().map(|(pos, _)| *pos) else {
                    return self.end_protection_phase(phase);
                };
                let named = store.checkpoint_capture_page(self.epoch, || {
                    let connection = store.readers.get();
                    let mut named = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h1cd0c334c601 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column}=?1)"))?;
                    let mut ids = Vec::new();
                    for (_, id) in &staged {
                        if named.query_row(/* sqlite-large-value: allow-for-now lv-h230ac67996e0 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ [id], |row| row.get::<_, bool>(0))? {
                            ids.push(id.clone());
                        }
                    }
                    Ok(ids)
                })?;
                return self.stage_protected(phase, named, last, None);
            }
            "claims" => {
                "SELECT store_index, json_extract(body, '$.fields.replacement') FROM claims
                 WHERE kind='record.repaired' AND store_index>?1 ORDER BY store_index LIMIT ?2"
            }
            "replica_records" => {
                "SELECT rowid, replacement_claim_id FROM replica_records
                 WHERE state='repaired' AND replacement_claim_id IS NOT NULL AND rowid>?1
                 ORDER BY rowid LIMIT ?2"
            }
            _ => return self.capture_protection(store, phase, after, bound),
        };
        let page = store.checkpoint_capture_page(self.epoch, || {
            Ok(store
                .readers
                .get()
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-hf33fcb46317c owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ sql)?
                .query_map(params![after, STAGING_PAGE], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })?;
        let Some(last) = page.last().map(|(cursor, _)| *cursor) else {
            return self.end_protection_phase(phase);
        };
        self.stage_protected(phase, page.into_iter().filter_map(|(_, id)| id), last, None)
    }

    /// Each staged writer's newest envelope before the cut, sealed or tombstoned by an earlier
    /// checkpoint, as a row with no claims: the writer's-newest guard reads its sequence, and
    /// `finish` never drops an envelope it holds no claim of.
    pub(super) fn capture_writer_heads(&self, store: &Store) -> Result<()> {
        let writers = self
            .staging
            .prepare_cached("SELECT DISTINCT writer FROM claims")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let cut = accepted(self.cut_unix_ms)?;
        let heads = store.checkpoint_capture_page(self.epoch, || {
            let connection = store.readers.get();
            // Newest first; a late envelope (one holding a claim dated at or after the cut) is
            // not before the cut, as in a full capture.
            let mut sealed = connection.prepare_cached(&NEWEST_SEALED)?;
            let mut tombstoned = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h512fd1a2bdd1 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ &format!(
                "SELECT sequence, envelope_hash, CAST(accepted_at_unix_ms AS TEXT), 0 FROM checkpoint_envelopes
                 WHERE writer=?1 AND ?2 IS NOT NULL AND {RECORDED_TOMBSTONE_FILTER}
                 ORDER BY sequence DESC LIMIT 1"
            ))?;
            let mut heads = Vec::new();
            for writer in &writers {
                let read = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(u64, String, String, i64)> {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                };
                let newest = [
                    sealed.query_row(/* sqlite-large-value: allow-for-now lv-h2a96f5eac6fd owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ params![writer, self.seal_rowid, cut], read).optional()?,
                    tombstoned.query_row(/* sqlite-large-value: allow-for-now lv-h30ba373ba026 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ params![writer, self.seal_rowid, cut], read).optional()?,
                ]
                .into_iter()
                .flatten()
                .max_by_key(|(sequence, ..)| *sequence);
                if let Some((sequence, hash, accepted_at, records)) = newest {
                    let accepted_at = checkpoint_accepted_time(&accepted_at, &hash, 2)?;
                    heads.push((EnvelopeKey { writer: writer.clone(), sequence, envelope_hash: hash }, accepted_at, records));
                }
            }
            Ok(heads)
        })?;
        let transaction = self.staging.unchecked_transaction()?;
        {
            let mut insert = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-hf6107b65de89 owner=smallclaims::store::checkpoint_staging::scoped TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO envelopes VALUES (?1, ?2, ?3, ?4, ?5, 1)")?;
            for (key, accepted_at, records) in heads {
                insert.execute(/* sqlite-large-value: allow-for-now lv-h1a23eb0b9c1b owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ params![key.writer, key.sequence, key.envelope_hash, accepted(accepted_at)?, records])?;
            }
        }
        Self::advance(&transaction, EARLIER_CLAIMS, 0, None)?;
        Ok(transaction.commit()?)
    }

    /// The citations of the claims the cut could drop (older than the newest of their slot):
    /// the cache is caught up with every claim the store holds, then each candidate's citers
    /// are read from it, and one counts only if it is present and sealed at the cut.
    pub(super) fn capture_citers(&self, store: &Store) -> Result<()> {
        store.catch_up_local_trim_citations()?;
        let kinds = self.scope_kinds();
        let candidates = self
            .staging
            .prepare(/* sqlite-large-value: allow-for-now lv-h0353fd31e999 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ &format!(
                "SELECT id FROM claims AS claim
                 WHERE first=1 AND slot IS NOT NULL AND kind IN ({})
                   AND EXISTS(SELECT 1 FROM claims AS newer WHERE newer.slot=claim.slot AND newer.pos>claim.pos)",
                vec!["?"; kinds.len()].join(", ")
            ))?
            .query_map(rusqlite::params_from_iter(kinds), |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let cut = accepted(self.cut_unix_ms)?;
        let cited = store.checkpoint_capture_page(self.epoch, || {
            let connection = store.readers.get();
            // Sealed as a full capture stages it: present, with a record, not repaired, in an
            // envelope before the cut and the seal row that holds no claim dated at or after it.
            let mut sealed = connection.prepare_cached(&SEALED_CITER)?;
            let mut cited = Vec::new();
            for candidate in &candidates {
                for citer in Store::local_trim_citers(&connection, candidate)? {
                    if sealed.query_row(/* sqlite-large-value: allow-for-now lv-h720b85b352d2 owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ params![citer, self.seal_rowid, cut], |row| row.get::<_, bool>(0))? {
                        cited.push(candidate.clone());
                        break;
                    }
                }
            }
            Ok(cited)
        })?;
        let transaction = self.staging.unchecked_transaction()?;
        {
            let mut cite = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-h8d9eebe1e289 owner=smallclaims::store::checkpoint_staging::scoped TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO cited(id) VALUES (?1)")?;
            for id in cited {
                cite.execute(/* sqlite-large-value: allow-for-now lv-h7257d86e40ef owner=smallclaims::store::checkpoint_staging::scoped TODO=trace-statement-and-value-representation */ [id])?;
            }
        }
        Self::advance(&transaction, CAPTURED, 0, None)?;
        Ok(transaction.commit()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::runtime::Plain;
    use std::sync::Arc;

    /// Each statement the scoped capture runs per claim or writer seeks every table, released
    /// envelopes included: a correlated term on a view compound would scan it once per run.
    #[test]
    fn the_scoped_capture_seeks_stored_and_released_envelopes() {
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        let connection = store.connection.write();
        for sql in [&*HELD, &*NEWEST_SEALED, &*SEALED_CITER] {
            crate::store::envelope_release::assert_plan_seeks(&connection, sql);
        }
    }

    /// A local cut's file holds the marks `protect_all_but` set. Left behind, as after an error,
    /// a full capture of the same cut must start its own file, not resume those marks.
    #[test]
    fn a_full_capture_never_resumes_a_local_cuts_scoped_file() {
        let scratch = tempfile::tempdir().unwrap();
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        store
            .connection
            .write()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS desired(claim_id TEXT);
                 CREATE TABLE IF NOT EXISTS mission_definitions(claim_id TEXT);
                 CREATE TABLE IF NOT EXISTS mission_revisions(claim_id TEXT);
                 CREATE TABLE IF NOT EXISTS documents(binding_claim_id TEXT);",
            )
            .unwrap();
        for n in 0..3 {
            store
                .append_claim(&ClaimInput {
                    subject: format!("note/{n}"),
                    kind: "example.note".into(),
                    actor: None,
                    fields: BTreeMap::from([("text".into(), json!(n))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        store.seal_local_batches().unwrap();
        let cut = now_ms() + 1_000;
        let protected = |staged: &StagedSet| -> (i64, i64) {
            staged
                .staging
                .query_row("SELECT COUNT(*), COALESCE(SUM(protected), 0) FROM claims", [], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
        };
        let scoped = store.checkpoint_staged_set_scoped(cut, None, scratch.path(), Some(&["example.note"])).unwrap();
        scoped.protect_all_but(&["another.kind"]).unwrap();
        assert_eq!(protected(&scoped), (3, 3));
        let scoped_path = scoped.path.clone();
        drop(scoped);
        let full = store.checkpoint_staged_set(cut, None, scratch.path()).unwrap();
        assert_ne!(full.path, scoped_path, "a full capture has its own file");
        assert!(!scoped_path.exists(), "the scoped file is removed, not resumed");
        assert_eq!(protected(&full), (3, 0), "no mark a local cut set survives into a full capture");
    }
}
