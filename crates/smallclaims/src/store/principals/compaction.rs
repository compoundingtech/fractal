//! Reader side of storage version 24: a compacted claim keeps its facade as its only candidate.
//!
//! A settled claim with one candidate keeps its `claim_signatures` facade (inline text), its
//! `claim_signature_counts` row (candidates = 1), its verdict, nonce use and dependency links,
//! and drops its candidate and source rows. A counted claim without candidate rows is
//! therefore compacted: readers take the facade as its single candidate, and any write of a
//! candidate first expands the facade back into a candidate and a source, so an exact
//! duplicate still adds nothing and a second candidate never loses the first.
//!
//! The writer compacts a claim when its verdict settles and, a short page at a time, every
//! settled claim an earlier release kept whole. It turns on by its own meta flag and fences the
//! store at version 24 in the transaction that compacts its first claim; a build that reads only
//! up to 23 then refuses the store instead of misreading it.
use super::*;

/// The reader fence of compacted claims. It also carries version 23's meaning.
pub const COMPACTION_STORAGE_VERSION: u32 = 24;
/// The compaction writer's own switch (`user_version` is a reader fence only).
pub const COMPACTION_WRITER_FLAG: &str = "claim_signature_compaction_writer";
/// The last claim the compaction of earlier releases' settled claims has passed, or `done`.
pub(super) const COMPACTION_CURSOR: &str = "claim_signature_compaction";
/// Candidate rows the paced compaction looks at in one writer loan, and the time after which it
/// commits early.
pub const COMPACTION_PAGE: usize = 512;
pub const COMPACTION_BUDGET: std::time::Duration = std::time::Duration::from_millis(10);

/// A settled claim that compaction may keep as its facade alone: one candidate, a published
/// verdict that is not held, an inline text facade, a nonce use (so the signature verified on
/// this claim), nothing queued or fresh for it, no judging row (a settled fold keeps none, so a
/// row left means pending or large; queueing a claim also gives it one) and no version 21
/// originals. `judging::unavailable` adds
/// the requeue cursors.
const SETTLED: &str = "n.candidates=1 AND v.verdict<>'held' AND typeof(s.signature)='text'
   AND EXISTS(SELECT 1 FROM claim_signature_nonce_uses u WHERE u.claim_id=d.claim_id)
   AND NOT EXISTS(SELECT 1 FROM claim_verdict_queue q WHERE q.claim_id=d.claim_id)
   AND NOT EXISTS(SELECT 1 FROM claim_verdict_fresh f WHERE f.claim_id=d.claim_id)
   AND NOT EXISTS(SELECT 1 FROM claim_signature_judging j WHERE j.claim_id=d.claim_id)
   AND NOT EXISTS(SELECT 1 FROM claim_signature_originals o WHERE o.claim_id=d.claim_id)";
const SETTLED_FROM: &str = "FROM claim_signature_candidates d
   JOIN claim_signature_counts n USING(claim_id)
   JOIN claim_verdicts v USING(claim_id)
   JOIN claim_signatures s USING(claim_id)";

/// Whether this store compacts: fenced at 24 or later with the writer's flag set.
pub(super) fn enabled(connection: &Connection) -> Result<bool> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version >= COMPACTION_STORAGE_VERSION
        && super::super::fleet_meta(connection, COMPACTION_WRITER_FLAG)?.as_deref() == Some("1"))
}

/// Fence the store at 24 and set the writer's flag, in the caller's transaction. Never lowers
/// a later fence.
fn activate_tx(connection: &Connection) -> Result<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < COMPACTION_STORAGE_VERSION {
        connection.pragma_update(None, "user_version", COMPACTION_STORAGE_VERSION)?;
    }
    set_meta_tx(connection, COMPACTION_WRITER_FLAG, "1")
}

/// Compact `claim_id` if it is settled. Returns whether it did. Its candidate row goes, the
/// existing trigger takes its source and count with it, and the count comes back at 1.
pub(super) fn compact_settled_tx(connection: &Connection, claim_id: &str) -> Result<bool> {
    let settled: bool = connection
        .prepare_cached(&format!(
            "SELECT EXISTS(SELECT 1 {SETTLED_FROM} WHERE d.claim_id=?1 AND {SETTLED})"
        ))?
        .query_row([claim_id], |row| row.get(0))?;
    if !settled || judging::unavailable(connection, claim_id)?.is_some() {
        return Ok(false);
    }
    if !enabled(connection)? {
        activate_tx(connection)?;
    }
    connection
        .prepare_cached("DELETE FROM claim_signature_candidates WHERE claim_id=?1")?
        .execute([claim_id])?;
    connection
        .prepare_cached("INSERT INTO claim_signature_counts(claim_id,candidates) VALUES (?1,1)")?
        .execute([claim_id])?;
    Ok(true)
}

/// One page of the compaction of settled claims that earlier releases kept whole: walk the
/// candidate rows in claim order from the cursor and compact each settled claim. Returns
/// `(examined, compacted, finished)`. A claim that settles behind the cursor is compacted when
/// it settles. Once done, only unsettled claims keep candidate rows, so a cheap scan of that
/// remainder finds a settled one a release without this writer left after a rollback, and the
/// walk starts over.
pub(super) fn compact_page_tx(connection: &Connection) -> Result<(usize, usize, bool)> {
    let mut cursor = super::super::fleet_meta(connection, COMPACTION_CURSOR)?.unwrap_or_default();
    if cursor == "done" {
        let left: bool = connection
            .prepare_cached(&format!("SELECT EXISTS(SELECT 1 {SETTLED_FROM} WHERE {SETTLED})"))?
            .query_row([], |row| row.get(0))?;
        if !left {
            return Ok((0, 0, true));
        }
        cursor = String::new();
    }
    let started = std::time::Instant::now();
    let claims = connection
        .prepare_cached(
            "SELECT DISTINCT claim_id FROM claim_signature_candidates WHERE claim_id>?1 ORDER BY claim_id LIMIT ?2",
        )?
        .query_map(params![cursor, COMPACTION_PAGE as i64], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (mut examined, mut compacted, mut last) = (0, 0, None);
    for claim_id in &claims {
        if examined != 0 && started.elapsed() >= COMPACTION_BUDGET {
            break;
        }
        examined += 1;
        compacted += usize::from(compact_settled_tx(connection, claim_id)?);
        last = Some(claim_id.as_str());
    }
    let finished = claims.len() < COMPACTION_PAGE && examined == claims.len();
    let next = if finished { "done" } else { last.unwrap_or(cursor.as_str()) };
    set_meta_tx(connection, COMPACTION_CURSOR, next)?;
    Ok((examined, compacted, finished))
}

/// The facade of a compacted claim: counted, without candidate rows.
pub(super) fn facade(connection: &Connection, claim_id: &str) -> Result<Option<String>> {
    let value = connection
        .prepare_cached(
            "SELECT s.signature FROM claim_signatures s
              WHERE s.claim_id=?1
                AND EXISTS(SELECT 1 FROM claim_signature_counts c WHERE c.claim_id=s.claim_id)
                AND NOT EXISTS(SELECT 1 FROM claim_signature_candidates d WHERE d.claim_id=s.claim_id)",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-ha5155cd52ec4 owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get::<_, SqlValue>(0))
        .optional()?;
    value
        .map(|value| references::resolve(connection, claim_id, value))
        .transpose()
}

/// Expand a compacted claim back into its candidate and source rows before a candidate write.
/// The source is the claim's batch origin, which is what wrote the single candidate.
pub(super) fn expand_tx(connection: &Connection, claim_id: &str) -> Result<bool> {
    let Some(text) = facade(connection, claim_id)? else {
        return Ok(false);
    };
    let signature: ClaimSignature = serde_json::from_str(&text)?;
    let writer: String = connection
        .prepare_cached(
            "SELECT b.origin FROM claims c JOIN batches b ON b.id=c.batch_id WHERE c.id=?1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h1c1e2c4418af owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0))
        .optional()?
        .unwrap_or_else(|| "unknown-legacy".to_owned());
    // The candidate trigger counts the restored row, so drop the compacted count first.
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h92b0e591751c owner=smallclaims::store::principals::compaction TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_signature_counts WHERE claim_id=?1")?
        .execute(/* sqlite-large-value: allow-for-now lv-h428772d36dee owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ [claim_id])?;
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h35cb2b585420 owner=smallclaims::store::principals::compaction TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO claim_signature_candidates(claim_id,signature,key,nonce) VALUES (?1,?2,?3,?4)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h2a1fb85fc3c3 owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ params![claim_id, text, signature.key, signature.nonce])?;
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hc5e3af7efbd3 owner=smallclaims::store::principals::compaction TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signature_sources(claim_id,signature,writer) VALUES (?1,?2,?3)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-hfddd7253665c owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ params![claim_id, text, writer])?;
    Ok(true)
}
