//! Releasing old envelope payloads. Once every claim of an envelope carries its writer's own
//! signature (`member_signatures`), the envelope's payload and its envelope signatures say nothing
//! a reader still needs: each claim authenticates by its writer's signature, its row is in
//! `claims`, its position in `claim_positions`, and its record is the one `replica_record_rows`
//! makes. This member then deletes the payload, the envelope signatures, any hold and the
//! remaining valid records, and keeps the envelope's identity in `released_envelopes`.
//!
//! What any reader or peer sees stays the same. The replication inventory lists a released
//! envelope without its payload, as it lists a checkpoint tombstone, so the authority digest does
//! not move and peers never send it again; this member no longer serves it either. Readers that
//! ask which envelope holds a claim, or whether a claim was admitted, read `envelope_rows`, which
//! lists both kinds. Readers of payloads (export, backup, wire archive, signature migration and
//! backfill, rename proofs) skip released envelopes: they have no payload.
//!
//! The pass is local and paced like `record_bytes`: a page of at most [`PAGE`] envelopes and
//! [`BUDGET`] of work on the batched writer, its cursor committed with the page. It stays off in
//! the daemon ([`ENVELOPE_RELEASE`]) until peers can be served released claims.
use super::*;
use std::time::{Duration, Instant};

/// An envelope is released only once it was accepted this long ago: 14 days.
pub const ENVELOPE_RELEASE_AGE_MS: u64 = 14 * 24 * 60 * 60 * 1000;

/// Whether the daemon releases envelopes. Off until the claim-set transport can serve a released
/// envelope's claims to a peer that lacks them: until then such a peer could not get them here.
pub const ENVELOPE_RELEASE: bool = false;

/// A released envelope's identity, and the view of every envelope this member holds or released.
/// `original_rowid` is the envelope's `replica_envelopes` rowid, so a capture bounded by a seal row
/// (`row_id <= ?`) sees a released envelope exactly where it saw the stored one. A released
/// envelope was validated and admitted (`receipt_state` 'validated').
pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h4f885dc5dbb7 owner=smallclaims::store::envelope_release TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS released_envelopes (
    writer TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    envelope_hash TEXT NOT NULL,
    previous_hash TEXT,
    accepted_at_unix_ms TEXT NOT NULL,
    batch_id TEXT,
    original_rowid INTEGER NOT NULL,
    released_at_unix_ms INTEGER NOT NULL,
    PRIMARY KEY(writer, sequence, envelope_hash)
);
CREATE INDEX IF NOT EXISTS released_envelopes_batch ON released_envelopes(batch_id);
CREATE UNIQUE INDEX IF NOT EXISTS released_envelopes_rowid ON released_envelopes(original_rowid);
CREATE VIEW IF NOT EXISTS envelope_rows(row_id, writer, sequence, envelope_hash, previous_hash,
        accepted_at_unix_ms, batch_id, receipt_state, released) AS
    SELECT rowid, writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms, batch_id,
           receipt_state, 0
    FROM replica_envelopes
    UNION ALL
    SELECT original_rowid, writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms,
           batch_id, 'validated', 1
    FROM released_envelopes;
";

const CURSOR: &str = "envelope_release_cursor";
/// The envelopes one page reads at most.
pub const PAGE: usize = 512;
/// One page's share of a writer batch, as in `record_bytes`.
pub const BUDGET: Duration = Duration::from_millis(10);

/// The next envelopes past the cursor, without their payloads.
const PAGE_SQL: &str = "SELECT rowid, writer, sequence, envelope_hash, accepted_at_unix_ms, batch_id,
        receipt_state='validated' AND batch_id IS NOT NULL
     FROM replica_envelopes WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

/// Whether the envelope `?1`, `?2`, `?3` (batch `?4`) can lose its payload, as far as the store's
/// rows tell: its batch has no other envelope, every record it has is valid and no repair names
/// one, it is not its writer's newest envelope here, no checkpoint lists it, no restore holds it
/// in quarantine, and every claim of its batch has its position stored.
const ELIGIBLE_SQL: &str = "SELECT
        (SELECT COUNT(*) FROM envelope_rows WHERE batch_id=?4)=1
    AND NOT EXISTS(SELECT 1 FROM replica_records
                   WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND state<>'valid')
    AND NOT EXISTS(SELECT 1 FROM claims INDEXED BY claims_batch_index
                   WHERE claims.batch_id=?4 AND EXISTS(SELECT 1 FROM replica_records
                       WHERE replica_records.claim_id=claims.id AND state<>'valid'))
    AND NOT EXISTS(SELECT 1 FROM claims AS repairs INDEXED BY claims_kind_index
                   WHERE repairs.kind='record.repaired'
                     AND json_extract(repairs.body,'$.fields.record') IN (
                         SELECT record_ref FROM replica_records
                         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3))
    AND ?2<(SELECT MAX(sequence) FROM replica_envelopes WHERE writer=?1)
    AND NOT EXISTS(SELECT 1 FROM checkpoint_envelopes
                   WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3)
    AND NOT EXISTS(SELECT 1 FROM replica_restore_quarantine
                   WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3)
    AND NOT EXISTS(SELECT 1 FROM claims INDEXED BY claims_batch_index
                   WHERE claims.batch_id=?4
                     AND NOT EXISTS(SELECT 1 FROM claim_positions WHERE claim_id=claims.id))
    AND EXISTS(SELECT 1 FROM batches WHERE id=?4 AND origin=?1 AND replica_sequence=?2)";

/// What one page of [`Store::release_envelopes`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnvelopeRelease {
    pub scanned: usize,
    pub released: usize,
    /// The page stopped at an envelope too young to release, or read every envelope.
    pub done: bool,
    /// Why nothing was read: what the pass waits for.
    pub waiting: Option<&'static str>,
    /// The page's work inside its transaction, before the commit.
    pub work: Duration,
}

/// What judging a claim's writer signature needs, read before the page takes the writer.
struct Authority {
    fleet_id: String,
    membership: crate::fleet::Membership,
}

impl Store {
    /// Release one bounded page of envelopes accepted at least `min_age_ms` ago whose claims
    /// all carry their writer's signature. The cursor commits with the page and stays where
    /// the pass stopped: at the first envelope too young to release, which a later call reads
    /// again once it is old enough, or after the last one. An envelope the page passed over for
    /// another reason (one of its claims is unsigned, or it was its writer's newest) is not read
    /// again.
    pub fn release_envelopes(&self, min_age_ms: u64) -> Result<EnvelopeRelease> {
        let waiting = |reason| Ok(EnvelopeRelease { done: true, waiting: Some(reason), ..Default::default() });
        // A principal's claim signatures move out of the payloads they arrived in; until that
        // migration is done a payload may still hold the only copy of one.
        if !self.signature_migration()?.done() {
            return waiting("the claim signature migration has not finished");
        }
        let Some(fleet_id) = self.signing_fleet_id(&self.readers.get())? else {
            return waiting("this store has no fleet");
        };
        let authority = Authority { fleet_id, membership: self.fleet_membership_sealed()? };
        let origin = self.origin.clone();
        let (report, released, changes) = self
            .connection
            .batched(move |tx| release_page(tx, &origin, &authority, min_age_ms))
            .map_err(anyhow::Error::msg)??;
        if !released.is_empty() {
            self.mark_payloads_released(&released, changes)?;
        }
        Ok(report)
    }

    /// How many envelopes this member released.
    pub fn released_envelopes(&self) -> Result<u64> {
        Ok(self.readers.get().query_row("SELECT COUNT(*) FROM released_envelopes", [], |row| row.get(0))?)
    }

    /// Mark the envelopes a release committed payloadless in the cached replication snapshot, as a
    /// load from nothing would. The release counted one change per envelope for the saved
    /// inventory (`changes`: the count before and after it); a snapshot that took in every change
    /// before the release has now taken in these too, so a stop can still save it.
    fn mark_payloads_released(&self, released: &[ReplicaEnvelopeId], changes: (i64, i64)) -> Result<()> {
        {
            let _building = crate::read_budget::lock_sync(&self.replication_snapshot_build)?;
            let mut cached = crate::read_budget::lock_sync(&self.replication_snapshot)?;
            if let Some(previous) = cached.take() {
                let mut snapshot = Arc::try_unwrap(previous).unwrap_or_else(|shared| (*shared).clone());
                for identity in released {
                    if snapshot.inventory.mark_payloadless(identity) {
                        snapshot.envelope_rows = snapshot.envelope_rows.saturating_sub(1);
                    }
                }
                if snapshot.inventory_changes == Some(changes.0) {
                    snapshot.inventory_changes = Some(changes.1);
                }
                *cached = Some(Arc::new(snapshot));
            }
        }
        self.replica_inventory_changed();
        Ok(())
    }
}

fn cursor(connection: &Connection) -> Result<i64> {
    let value: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE key=?1", [CURSOR], |row| row.get(0))
        .optional()?;
    Ok(value.map(|value| value.parse::<i64>()).transpose()?.unwrap_or(0))
}

type Released = (EnvelopeRelease, Vec<ReplicaEnvelopeId>, (i64, i64));

fn release_page(tx: &Transaction<'_>, origin: &str, authority: &Authority, min_age_ms: u64) -> Result<Released> {
    let started = Instant::now();
    let mut after = cursor(tx)?;
    let changes_before = saved_inventory::changes(tx)?;
    let newest = max_envelope_rowid(tx)?;
    let oldest_accepted = now_ms().saturating_sub(u128::from(min_age_ms));
    let page = tx
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hb977b8b2d181 owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ PAGE_SQL)?
        .query_map(params![after, PAGE], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                ReplicaEnvelopeId { writer: row.get(1)?, sequence: row.get(2)?, hash: row.get(3)? },
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut report = EnvelopeRelease { done: page.len() < PAGE, ..Default::default() };
    let mut released = Vec::new();
    for (rowid, envelope, accepted, batch, admitted) in page {
        // Rows are in arrival order, so what follows a young envelope is young too. The newest
        // row stays as well: deleting it would let the next envelope take its rowid.
        if accepted.parse::<u128>().unwrap_or(u128::MAX) > oldest_accepted || rowid >= newest {
            report.done = true;
            break;
        }
        report.scanned += 1;
        after = rowid;
        if let (true, Some(batch)) = (admitted, batch)
            && releasable(tx, origin, authority, &envelope, &batch)?
            && release_tx(tx, rowid, &envelope)?
        {
            released.push(envelope);
            report.released += 1;
        }
        if started.elapsed() >= BUDGET {
            report.done = false;
            break;
        }
    }
    if report.scanned > 0 {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h71e733dedc2b owner=smallclaims::store::envelope_release TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![CURSOR, after.to_string()],
        )?;
    }
    report.work = started.elapsed();
    Ok((report, released, (changes_before, saved_inventory::changes(tx)?)))
}

/// Whether `envelope`, of batch `batch`, can lose its payload: its rows allow it
/// ([`ELIGIBLE_SQL`]), no claim of its batch is of a kind whose readers need the envelope
/// ([`kept_kind`]), and every one carries its writer's signature by a key membership admits for
/// that writer and sequence, and has the id its stored row hashes to.
fn releasable(tx: &Connection, origin: &str, authority: &Authority, envelope: &ReplicaEnvelopeId, batch: &str) -> Result<bool> {
    if retired::envelope_refused(tx, &envelope.writer, envelope.sequence, &envelope.hash)? {
        return Ok(false);
    }
    let eligible: bool = tx
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h6c3e7cfc9c6d owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ ELIGIBLE_SQL)?
        .query_row(/* sqlite-large-value: allow-for-now lv-haf548599f1ef owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ params![envelope.writer, envelope.sequence, envelope.hash, batch], |row| row.get(0))?;
    if !eligible {
        return Ok(false);
    }
    let keys = match authority.membership.window(&envelope.writer, envelope.sequence) {
        crate::fleet::Window::Keyed(keys) => keys,
        crate::fleet::Window::Legacy | crate::fleet::Window::Fenced => return Ok(false),
    };
    let claims = tx
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-ha6d430f1b6cf owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ &format!(
            "SELECT {CLAIM_COLUMNS} FROM claims INDEXED BY claims_batch_index
             WHERE claims.batch_id=?1 ORDER BY claims.store_index"
        ))?
        .query_map([batch], claim_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut signatures = tx.prepare_cached(
        "SELECT member_key, signature FROM claim_member_signatures WHERE claim_id=?1",
    )?;
    for claim in &claims {
        if kept_kind(origin, claim) || claim.origin != envelope.writer || !crate::hash::claim_id_is_content_hash(claim)? {
            return Ok(false);
        }
        let message = crate::fleet::keys::claim_signature_message(
            &authority.fleet_id, &envelope.writer, envelope.sequence, &claim.id,
        );
        let signed = signatures
            .query_map([&claim.id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|(key, signature)| keys.contains(key) && crate::fleet::verify_signature(key, &message, signature));
        if !signed {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A claim whose envelope this member keeps whatever its age, because a reader of it reads the
/// envelope itself: the membership fold and every fleet barrier read `fleet.*` claims by their
/// envelope signatures, principals and pairing their own provenance, the declared-admission
/// barrier a member's `daemon.started` advertisement by its envelope signature, and idle wakes
/// another writer's `message.sent` by the time its envelope arrived here.
fn kept_kind(origin: &str, claim: &ClaimRecord) -> bool {
    ["fleet.", "principal.", "pairing.", "custom.client.pairing"]
        .iter()
        .any(|prefix| claim.kind.starts_with(prefix))
        || claim.kind == "daemon.started"
        || (claim.kind == "message.sent" && claim.origin != origin)
}

/// Keep `envelope`'s identity and delete its payload row, its envelope signatures, any hold and
/// its valid records, in the page's transaction. False when another identity already holds its
/// rowid in `released_envelopes`, as after a trim deleted the newest rows and an envelope took one
/// of their rowids again; it then stays.
fn release_tx(tx: &Connection, rowid: i64, envelope: &ReplicaEnvelopeId) -> Result<bool> {
    let kept = tx
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hf6c7e0f4d53c owner=smallclaims::store::envelope_release TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO released_envelopes(writer, sequence, envelope_hash, previous_hash,
                 accepted_at_unix_ms, batch_id, original_rowid, released_at_unix_ms)
             SELECT writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms, batch_id, rowid, ?2
             FROM replica_envelopes WHERE rowid=?1",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h5ae5cf1b36cc owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ params![rowid, i64::try_from(now_ms())?])?;
    if kept == 0 {
        return Ok(false);
    }
    for table in ["replica_envelope_signatures", "replica_envelope_holds", "replica_records"] {
        tx.prepare_cached(&format!(
            /* sqlite-large-value: allow-for-now lv-h5e86b51b796f owner=smallclaims::store::envelope_release TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM {table} WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3"
        ))?
        .execute(/* sqlite-large-value: allow-for-now lv-hb7a4188d5c54 owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ params![envelope.writer, envelope.sequence, envelope.hash])?;
    }
    tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h51ac46e59165 owner=smallclaims::store::envelope_release TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=?1")?
        .execute(/* sqlite-large-value: allow-for-now lv-hf1e1d4fedb7a owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ [rowid])?;
    Ok(true)
}

/// Whether this member released the envelope `writer`, `sequence`, `envelope_hash`. Admission
/// refuses it as it refuses a tombstoned one: its identity already stands for it.
pub fn envelope_released(connection: &Connection, writer: &str, sequence: u64, envelope_hash: &str) -> Result<bool> {
    Ok(connection
        .prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM released_envelopes WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3)",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-hbd866c8c542b owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ params![writer, sequence, envelope_hash], |row| row.get(0))?)
}

/// The writer signatures of claim `claim_id` whose admission came from an envelope this member
/// released, with the writer and sequence of that envelope's batch: what a check that read the
/// envelope's signature reads in its place. Only a claim with no record in another state counts
/// (`record_bytes::claim_record_not_valid_sql`). With `writer`, only that writer's.
pub fn released_claim_signers(
    connection: &Connection,
    claim_id: &str,
    writer: Option<&str>,
) -> Result<Vec<(String, u64, String, String)>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT batches.origin, batches.replica_sequence, signatures.member_key, signatures.signature
             FROM claims
             JOIN batches ON batches.id=claims.batch_id
             JOIN released_envelopes AS released ON released.batch_id=claims.batch_id
                 AND released.writer=batches.origin AND released.sequence=batches.replica_sequence
             JOIN claim_member_signatures AS signatures ON signatures.claim_id=claims.id
             WHERE claims.id=?1 AND (?2 IS NULL OR batches.origin=?2) AND NOT {}",
            record_bytes::claim_record_not_valid_sql("claims")
        )
    });
    Ok(connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hc79e38407965 owner=smallclaims::store::envelope_release TODO=trace-statement-and-value-representation */ &QUERY)?
        .query_map(params![claim_id, writer], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Whether one of [`released_claim_signers`]' signatures verifies for claim `claim_id` in fleet
/// `fleet_id`, by a key `admits(writer, sequence, key)` accepts.
pub fn released_claim_signed(
    connection: &Connection,
    fleet_id: &str,
    claim_id: &str,
    writer: Option<&str>,
    mut admits: impl FnMut(&str, u64, &str) -> Result<bool>,
) -> Result<bool> {
    for (writer, sequence, key, signature) in released_claim_signers(connection, claim_id, writer)? {
        let message = crate::fleet::keys::claim_signature_message(fleet_id, &writer, sequence, claim_id);
        if admits(&writer, sequence, &key)? && crate::fleet::verify_signature(&key, &message, &signature) {
            return Ok(true);
        }
    }
    Ok(false)
}

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
            || step == "SCAN envelope_rows"
            || step.starts_with("SCAN (subquery-")
            || (step == "SCAN envelopes" && sql.contains("envelope_rows envelopes"))
    };
    let scans: Vec<&String> = plan
        .iter()
        .filter(|step| (step.starts_with("SCAN ") && !produced(step)) || step.contains("AUTOMATIC"))
        .collect();
    assert!(scans.is_empty(), "{scans:?} in {plan:#?} for {sql}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::runtime::Plain;

    /// Without statistics the planner assumes large tables, as on a real store.
    #[test]
    fn what_reads_released_envelopes_seeks_and_scans_nothing() {
        let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
        let connection = store.connection.write();
        for sql in [
            format!("SELECT COUNT(*) FROM claims WHERE id=?1 AND {}", record_bytes::admitted_claim_sql("claims")),
            format!(
                "SELECT {} FROM replica_envelopes envelopes WHERE rowid>?1 AND rowid<=?2",
                record_bytes::envelope_record_count_sql("envelopes")
            ),
            format!(
                "SELECT {} FROM released_envelopes envelopes WHERE writer=?1",
                record_bytes::envelope_has_late_record_sql("envelopes", "?2")
            ),
            "SELECT COUNT(*) FROM replica_record_rows WHERE claim_id=?1".into(),
            "SELECT COUNT(*) FROM replica_record_rows WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3".into(),
            crate::store::checkpoint::sealed_envelope_records_sql(),
            ELIGIBLE_SQL.into(),
            PAGE_SQL.into(),
        ] {
            assert_plan_seeks(&connection, &sql);
        }
    }
}
