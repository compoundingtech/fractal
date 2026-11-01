//! Claim signatures in the store: signing this node's claims as their batches are sealed,
//! minting the keys it holds for people and agents, and the verdict each claim's signature gets.
//!
//! Verdicts are a fold over the claims in canonical order (see [`crate::principal::judge`]). The
//! store keeps it current incrementally and caches it in local tables that are derived only from
//! admitted claims, never synced, and outside every digest:
//!
//! - `claim_verdicts`: one row per signed claim, keyed by the claim ID (a content hash), so a
//!   different claim never reuses a verdict.
//! - `claim_verdict_links`: a conservative dependency superset for retained candidates
//!   (delegations, keys, nonces and roots), maintained by each judgment page. Relevant changes
//!   queue linked verdicts; claim removal or explicit recheck owns dependency cleanup.
//! - `claim_verdict_queue`: claims to judge again.
//! - `claim_verdict_fresh`: claims whose signature was just stored, at sealing or admission; a
//!   pass judges them and queues whatever they change. Nothing scans the claims.
//!
//! An unsigned claim has no row: its verdict is `unsigned`. [`Store::recheck_claim_verdicts`]
//! recomputes every verdict from the claims and reports any cached one that differed.

use std::cell::RefCell;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, BTreeSet};

use rusqlite::types::Value as SqlValue;

use super::*;
use crate::principal::{
    ClaimSignature, Delegation, Facts, Family, HeldKey, KEY_GRANTED, KEY_REVOKED, KeyGrant, Role,
    Verdict, content_digest, judge,
};

/// The tables of this module, created with the store's schema.
pub const PRINCIPAL_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h56821e2a865f owner=smallclaims::store::principals TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS claim_signatures (
    claim_id TEXT PRIMARY KEY,
    signer TEXT NOT NULL,
    key TEXT NOT NULL,
    nonce TEXT NOT NULL,
    signature TEXT NOT NULL
);
-- The legacy nonce index remains with the renamed legacy table during online bootstrap.
CREATE TABLE IF NOT EXISTS claim_signature_candidates (
    claim_id TEXT NOT NULL,
    signature TEXT NOT NULL,
    key TEXT NOT NULL,
    nonce TEXT NOT NULL,
    PRIMARY KEY(claim_id, signature)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS claim_signature_sources (
    claim_id TEXT NOT NULL,
    signature TEXT NOT NULL,
    writer TEXT NOT NULL,
    PRIMARY KEY(claim_id, signature, writer)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS claim_signature_counts (
    claim_id TEXT PRIMARY KEY,
    candidates INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS claim_signature_counts_alert_index ON claim_signature_counts(candidates,claim_id) WHERE candidates>=64;
CREATE TABLE IF NOT EXISTS claim_signature_writer_counts (
    writer TEXT PRIMARY KEY,
    candidates INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS claim_signature_candidate_added AFTER INSERT ON claim_signature_candidates BEGIN
    INSERT INTO claim_signature_counts(claim_id,candidates) VALUES (NEW.claim_id,1)
    ON CONFLICT(claim_id) DO UPDATE SET candidates=candidates+1;
END;
CREATE TRIGGER IF NOT EXISTS claim_signature_candidate_removed AFTER DELETE ON claim_signature_candidates BEGIN
    DELETE FROM claim_signature_sources WHERE claim_id=OLD.claim_id AND signature=OLD.signature;
    UPDATE claim_signature_counts SET candidates=candidates-1 WHERE claim_id=OLD.claim_id;
    DELETE FROM claim_signature_counts WHERE claim_id=OLD.claim_id AND candidates=0;
END;
CREATE TRIGGER IF NOT EXISTS claim_signature_source_added AFTER INSERT ON claim_signature_sources BEGIN
    INSERT INTO claim_signature_writer_counts(writer,candidates) VALUES (NEW.writer,1)
    ON CONFLICT(writer) DO UPDATE SET candidates=candidates+1;
END;
CREATE TRIGGER IF NOT EXISTS claim_signature_source_removed AFTER DELETE ON claim_signature_sources BEGIN
    UPDATE claim_signature_writer_counts SET candidates=candidates-1 WHERE writer=OLD.writer;
    DELETE FROM claim_signature_writer_counts WHERE writer=OLD.writer AND candidates=0;
END;
CREATE TABLE IF NOT EXISTS claim_signature_migration (
    id INTEGER PRIMARY KEY CHECK(id=1),
    phase TEXT NOT NULL,
    cursor INTEGER NOT NULL,
    legacy_through INTEGER NOT NULL,
    envelopes_through INTEGER NOT NULL,
    fault TEXT,
    legacy_fault TEXT,
    envelope_row INTEGER NOT NULL DEFAULT 0,
    claim_offset INTEGER NOT NULL DEFAULT 0,
    serving_legacy INTEGER NOT NULL DEFAULT 0 CHECK(serving_legacy IN (0,1)),
    legacy_present INTEGER NOT NULL DEFAULT 0 CHECK(legacy_present IN (0,1)),
    started_at_unix_ms INTEGER NOT NULL DEFAULT 0,
    advanced_at_unix_ms INTEGER NOT NULL DEFAULT 0,
    completed_work INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS claim_signature_nonce_uses (
    claim_id TEXT NOT NULL,
    key TEXT NOT NULL,
    nonce TEXT NOT NULL,
    PRIMARY KEY(claim_id,key,nonce)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS claim_signature_nonce_uses_key ON claim_signature_nonce_uses(key,nonce,claim_id);
CREATE TABLE IF NOT EXISTS claim_signature_judging (
    claim_id TEXT PRIMARY KEY,
    candidate_count INTEGER NOT NULL,
    roots_digest TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    last_signature TEXT NOT NULL,
    best_signature TEXT,
    best_verdict TEXT,
    best_reason TEXT,
    complete INTEGER NOT NULL,
    dirty INTEGER NOT NULL,
    notified INTEGER NOT NULL DEFAULT 0,
    blocked INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS claim_signature_judging_pending ON claim_signature_judging(claim_id) WHERE complete=0 OR dirty=1 OR blocked=1;
CREATE TABLE IF NOT EXISTS claim_verdicts (
    claim_id TEXT PRIMARY KEY,
    verdict TEXT NOT NULL,
    reason TEXT,
    signer TEXT NOT NULL,
    on_behalf TEXT
);
CREATE INDEX IF NOT EXISTS claim_candidate_verdicts_verdict_index ON claim_verdicts(verdict);
CREATE TABLE IF NOT EXISTS claim_verdict_links (
    link TEXT NOT NULL,
    claim_id TEXT NOT NULL,
    PRIMARY KEY(link, claim_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS claim_candidate_verdict_links_claim_index ON claim_verdict_links(claim_id);
CREATE TABLE IF NOT EXISTS claim_verdict_queue (
    claim_id TEXT PRIMARY KEY
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS claim_verdict_fresh (
    claim_id TEXT PRIMARY KEY
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS expected_claim_signatures (
    subject TEXT NOT NULL,
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    signature TEXT NOT NULL,
    PRIMARY KEY(subject, kind, actor)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS held_keys (
    public_key TEXT PRIMARY KEY,
    principal TEXT NOT NULL,
    role TEXT NOT NULL,
    chain TEXT NOT NULL
);
"#;

/// Queued claims judged per writer loan, so queued writes run between chunks.
const JUDGE_CHUNK: usize = 32;
const CANDIDATE_PAGE: usize = 32;
/// The longest chain admission follows: a device, its person's root, and the node that vouched.
const MAX_CHAIN_DEPTH: usize = 8;
const VERDICT_ROOTS: &str = "claim_candidate_verdict_roots";
/// While the verdicts linked to LINK are still being queued, `meta` holds `REQUEUE` + LINK with
/// the last claim queued so far. A trust-root change once queued every verdict relying on the
/// roots in one writer transaction (about 905k claims, 117 s, on a large member), and a changed
/// long-lived delegation its dependents (up to 23 s); now each loan queues one page.
const REQUEUE: &str = "claim_candidate_requeue:";
/// The first key after every `REQUEUE` key.
const REQUEUE_END: &str = "claim_candidate_requeue;";
const REQUEUE_PAGE: usize = 1024;
/// Loans one ordinary verdict pass takes (projection tail, admission, sealed export, startup).
/// The rest stays due for the next pass and for the paced signature job, so a large backlog
/// never holds the projection lock or the writer for long.
pub const VERDICT_SLICE_LOANS: usize = 8;
pub(super) const OWN_NODE_KEY: &str = "principal_node_key";

/// A claim's position in the canonical order, as comparable values.
#[derive(Clone, Debug)]
struct Position(Vec<SqlValue>);

impl Position {
    fn compare(&self, other: &Self) -> CmpOrdering {
        use crate::principal::facts::{OrderComponent, compare_order};
        fn component(value: &SqlValue) -> OrderComponent<'_> {
            match value {
                SqlValue::Integer(value) => OrderComponent::Integer(i128::from(*value)),
                SqlValue::Text(value) => OrderComponent::Text(value),
                SqlValue::Null => OrderComponent::Null,
                _ => OrderComponent::Other,
            }
        }
        compare_order(self.0.iter().map(component), other.0.iter().map(component))
    }
}

fn position(connection: &Connection, claim_id: &str) -> Result<Option<Position>> {
    let components = super::canonical::components("claims");
    let count = components.len();
    let sql = format!("SELECT {} FROM claims WHERE id=?1", components.join(", "));
    Ok(connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h9aea6e0e7e48 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ &sql)?
        .query_row(/* sqlite-large-value: allow-for-now lv-hd6bc18dda34a owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| {
            (0..count)
                .map(|index| row.get::<_, SqlValue>(index))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .optional()?
        .map(Position))
}

fn stored_signature(connection: &Connection, claim_id: &str) -> Result<Option<ClaimSignature>> {
    let text = connection
        .prepare_cached("SELECT signature FROM claim_signatures WHERE claim_id=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-hccb25d68371e owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get::<_, String>(0))
        .optional()?;
    text.map(|text| Ok(serde_json::from_str(&text)?))
        .transpose()
}

/// Keep each distinct carried signature as current state: authority facts arriving later
/// can make any candidate relevant. Exact duplicates neither add state nor queue judging.
pub fn store_claim_signature_tx(
    connection: &Connection,
    claim_id: &str,
    signature: &ClaimSignature,
) -> Result<()> {
    let writer: Option<String> = connection.prepare_cached(
        "SELECT batches.origin FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.id=?1"
    )?.query_row(/* sqlite-large-value: allow-for-now lv-ha3c5feea65a0 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0)).optional()?;
    store_carried_signature_tx(
        connection,
        claim_id,
        signature,
        writer.as_deref().unwrap_or("unknown-legacy"),
    )
}

pub(crate) fn store_carried_signature_tx(
    connection: &Connection,
    claim_id: &str,
    signature: &ClaimSignature,
    writer: &str,
) -> Result<()> {
    store_candidate_tx(
        connection,
        claim_id,
        signature,
        writer,
        legacy::active(connection)?,
    )
}

pub(super) fn store_inventory_signature_tx(
    connection: &Connection,
    claim_id: &str,
    signature: &ClaimSignature,
    writer: Option<&str>,
) -> Result<()> {
    let origin = if writer.is_none() {
        connection.prepare_cached("SELECT batches.origin FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.id=?1")?.query_row(/* sqlite-large-value: allow-for-now lv-h134e5aef5050 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get::<_, String>(0)).optional()?
    } else {
        None
    };
    store_candidate_tx(
        connection,
        claim_id,
        signature,
        writer.or(origin.as_deref()).unwrap_or("unknown-legacy"),
        false,
    )
}

fn store_candidate_tx(
    connection: &Connection,
    claim_id: &str,
    signature: &ClaimSignature,
    writer: &str,
    update_legacy: bool,
) -> Result<()> {
    // Historical inventory feeds only the shadow projection. New live inputs maintain
    // both projections until activation; duplicate candidates still reach the legacy path.
    if update_legacy {
        legacy::store_claim_signature_tx(connection, claim_id, signature)?;
    }
    let text = serde_json::to_string(signature)?;
    let added = connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h16586c1e56da owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signature_candidates(claim_id,signature,key,nonce)
         VALUES (?1,?2,?3,?4)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h7609ad2c9eca owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![claim_id, text, signature.key, signature.nonce])?;
    connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h8b1d8ca0d39f owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signature_sources(claim_id,signature,writer) VALUES (?1,?2,?3)"
    )?.execute(/* sqlite-large-value: allow-for-now lv-h6519e4b72b76 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![claim_id,text,writer])?;
    if added == 0 {
        return Ok(());
    }
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h7650541fb6ca owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_judging SET dirty=1,notified=0 WHERE claim_id=?1")?
        .execute(/* sqlite-large-value: allow-for-now lv-ha2e9111d7319 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
    let row = claim_row(connection, claim_id)?;
    if let Some(row) = row.as_ref()
        && signature_matches(row, signature)
    {
        let used=connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hfd04ef25b0f3 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signature_nonce_uses(claim_id,key,nonce) VALUES (?1,?2,?3)")?
            .execute(/* sqlite-large-value: allow-for-now lv-h23f9772ef4a9 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![claim_id,signature.key,signature.nonce])?;
        if used != 0 {
            queue_linked_tx(
                connection,
                &format!("nonce:{}|{}", signature.key, signature.nonce),
            )?;
        }
    }
    if let Some(row) = row.as_ref() {
        queue_affected_by_row_tx(connection, row)?;
    }
    // A newly sealed local claim needs its signature before its first verdict pass.
    // Thereafter this facade is updated only by the current-authority candidate fold.
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h3ef3e2963c54 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signatures(claim_id,signer,key,nonce,signature)
         VALUES (?1,?2,?3,?4,?5)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h3d338a6f6764 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![
            claim_id,
            signature.signer,
            signature.key,
            signature.nonce,
            text
        ])?;
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h974754085cbf owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_verdict_fresh(claim_id) VALUES (?1)")?
        .execute(/* sqlite-large-value: allow-for-now lv-h314ea5b7655f owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
    Ok(())
}

fn signature_candidates(connection: &Connection, claim_id: &str) -> Result<Vec<ClaimSignature>> {
    connection
        .prepare_cached(
            "SELECT signature FROM claim_signature_candidates WHERE claim_id=?1 ORDER BY signature",
        )?
        .query_map([claim_id], |row| row.get::<_, String>(0))?
        .map(|text| Ok(serde_json::from_str(&text?)?))
        .collect()
}

fn signature_fields(row: &ClaimRow) -> Value {
    crate::principal::facts::signature_fields(&row.kind, &row.body)
}

fn signature_matches(row: &ClaimRow, signature: &ClaimSignature) -> bool {
    crate::principal::facts::nonce_eligible(
        &row.id, &row.subject, &row.kind, row.actor.as_deref(), &row.body, signature,
    )
}

mod judging;
mod legacy;
mod migration;
pub(crate) use migration::SignatureEnvelopeWork;
pub(crate) use migration::initialize_schema;
pub use migration::{
    SignatureMigration, SignatureResume, SignatureResumeRequest, transient_storage_error,
};

/// Keep a signature a device made for the claim this node is about to write on `subject`, so the
/// write stores it in its own transaction.
pub fn attach_expected_signature_tx(
    transaction: &Transaction<'_>,
    claim_id: &str,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
) -> Result<()> {
    attach_expected_signature_inner_tx(transaction, claim_id, subject, kind, actor, None)
}

pub(crate) fn attach_expected_signature_from_writer_tx(
    transaction: &Transaction<'_>,
    claim_id: &str,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
    writer: &str,
) -> Result<()> {
    attach_expected_signature_inner_tx(transaction, claim_id, subject, kind, actor, Some(writer))
}

fn attach_expected_signature_inner_tx(
    transaction: &Transaction<'_>,
    claim_id: &str,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
    writer: Option<&str>,
) -> Result<()> {
    let Some(actor) = actor else {
        return Ok(());
    };
    let expected = transaction
        .prepare_cached(
            "SELECT signature FROM expected_claim_signatures
             WHERE subject=?1 AND kind=?2 AND actor=?3",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-hd18f6fae02cc owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![subject, kind, actor], |row| row.get::<_, String>(0))
        .optional()?;
    if let Some(text) = expected {
        transaction
            .prepare_cached(
                /* sqlite-large-value: allow-for-now lv-h72e4ba49a7d3 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM expected_claim_signatures WHERE subject=?1 AND kind=?2 AND actor=?3",
            )?
            .execute(/* sqlite-large-value: allow-for-now lv-h213133341fb4 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![subject, kind, actor])?;
        if let Ok(signature) = serde_json::from_str::<ClaimSignature>(&text) {
            // API validation precedes this writer loan. Recheck nonce use against the
            // serving model in the claim transaction, including an intervening cutover.
            let serving_legacy = legacy::active(transaction)?;
            if nonce_used_in_model(
                transaction,
                &signature.key,
                &signature.nonce,
                serving_legacy,
            )? {
                return Err(St3Error::new(
                    "device-signature-replayed",
                    "another live claim already carries this signature's nonce",
                )
                .into());
            }
            if let Some(writer) = writer {
                // This model choice remains valid inside this same writer transaction.
                store_candidate_tx(transaction, claim_id, &signature, writer, serving_legacy)?;
            } else {
                store_claim_signature_tx(transaction, claim_id, &signature)?;
            }
        }
    }
    Ok(())
}

struct ClaimRow {
    id: String,
    subject: String,
    kind: String,
    actor: Option<String>,
    body: Value,
}

fn claim_row(connection: &Connection, claim_id: &str) -> Result<Option<ClaimRow>> {
    Ok(connection
        .prepare_cached("SELECT id, subject, kind, actor, body FROM claims WHERE id=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h683d859bacef owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| {
            Ok(ClaimRow {
                id: row.get(0)?,
                subject: row.get(1)?,
                kind: row.get(2)?,
                actor: row.get(3)?,
                body: serde_json::from_str(&row.get::<_, String>(4)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
            })
        })
        .optional()?)
}

/// The member incarnations fleet membership admits, as `(host/NAME, key)`. A fold of every fleet
/// claim: hundreds of statements, and on a large store tens of milliseconds to seconds of the
/// writer each time.
fn member_roots(connection: &Connection) -> Result<BTreeSet<(String, String)>> {
    #[cfg(test)]
    MEMBER_ROOT_FOLDS.with(|folds| folds.set(folds.get() + 1));
    Ok(fleet_membership_tx(connection)?
        .incarnations()
        .map(|incarnation| {
            (
                format!("host/{}", incarnation.name),
                incarnation.member_key.clone(),
            )
        })
        .collect::<BTreeSet<_>>())
}

// Per thread, so tests running side by side cannot count each other's folds.
#[cfg(test)]
thread_local! {
    pub(crate) static MEMBER_ROOT_FOLDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Local inspection of retained candidate state. A threshold is an alert, never an eviction.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SignatureStateFault {
    pub code: String,
    pub subject: String,
    pub detail: String,
}

// The primary-key order otherwise lets SQLite choose a scan of every ordinary claim.
// Sort the sparse alert set after reading the existing threshold index instead.
fn signature_alerts_query(connection: &Connection) -> rusqlite::Result<rusqlite::CachedStatement<'_>> {
    connection.prepare_cached("SELECT claim_id,candidates FROM claim_signature_counts INDEXED BY claim_signature_counts_alert_index WHERE candidates>=64 ORDER BY claim_id")
}

impl Store {
    pub fn signature_candidate_count(&self, claim_id: &str) -> Result<u64> {
        Ok(self
            .readers
            .get()
            .prepare_cached("SELECT candidates FROM claim_signature_counts WHERE claim_id=?1")?
            .query_row(/* sqlite-large-value: allow-for-now lv-hab4cfb09b8f9 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0))
            .optional()?
            .unwrap_or(0))
    }
    pub fn signature_writer_candidate_count(&self, writer: &str) -> Result<u64> {
        Ok(self
            .readers
            .get()
            .prepare_cached("SELECT candidates FROM claim_signature_writer_counts WHERE writer=?1")?
            .query_row(/* sqlite-large-value: allow-for-now lv-h5fbe338e2956 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [writer], |row| row.get(0))
            .optional()?
            .unwrap_or(0))
    }
    pub fn signature_state_faults(&self) -> Result<Vec<SignatureStateFault>> {
        let mut faults = Vec::new();
        let migration = self.signature_migration()?;
        if migration.serving_legacy && migration.legacy_fault.is_none() {
            faults.push(SignatureStateFault { code:"signature-legacy-p0-exposed".into(), subject:format!("host/{}",self.origin), detail:format!("legacy first-carried authorization is active; candidate projection is {} at cursor {}, legacy through {}, envelopes through {}, envelope {} claim offset {}; corrected authority activates only after complete inventory and reconciliation",migration.phase,migration.cursor,migration.legacy_through,migration.envelopes_through,migration.envelope_row,migration.claim_offset) + "; " + &migration.progress_description() });
        }
        if migration.serving_legacy
            && migration.legacy_fault.is_none()
            && migration.fault.is_none()
            && now_ms().saturating_sub(migration.advanced_at_unix_ms) > 300_000
        {
            faults.push(SignatureStateFault { code:"signature-bootstrap-stalled".into(),subject:format!("host/{}",self.origin),detail:format!("no committed candidate bootstrap progress for at least 300s; inspect the writer queue and stopped-job logs; {}",migration.progress_description()) });
        }
        if let Some(detail) = self.lasting_signature_retry() {
            faults.push(SignatureStateFault {
                code: "signature-storage-retrying".into(),
                subject: format!("host/{}", self.origin),
                detail,
            });
        }
        if let Some(fault) = &migration.legacy_fault {
            faults.push(SignatureStateFault {code:"signature-state-corrupt".into(),subject:format!("host/{}",self.origin),detail:format!("legacy authorization is unavailable: {fault}; repair the named local defect before explicit resume")});
        }
        if let Some(reason) = migration.unavailable_reason() {
            let (code, detail) = reason.split_once(": ").unwrap_or((&reason, ""));
            faults.push(SignatureStateFault {
                code: code.into(),
                subject: format!("host/{}", self.origin),
                detail: detail.into(),
            });
        }
        let connection = self.readers.get();
        let pending:u64=connection.prepare_cached("SELECT COUNT(*) FROM claim_signature_judging INDEXED BY claim_signature_judging_pending WHERE complete=0 OR dirty=1 OR blocked=1")?.query_row(/* sqlite-large-value: allow-for-now lv-ha7a25e3cb8cb owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [],|row|row.get(0))?;
        if pending != 0 {
            let sample=connection.prepare_cached("SELECT claim_id FROM claim_signature_judging INDEXED BY claim_signature_judging_pending WHERE complete=0 OR dirty=1 OR blocked=1 ORDER BY claim_id LIMIT 20")?.query_map([],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            faults.push(SignatureStateFault {code:"signature-judgment-incomplete".into(),subject:format!("host/{}",self.origin),detail:format!("{pending} claims have pending bounded candidate judgment; first {} claim ids: {sample:?}; serving model is {}; candidate authority remains Held until affected pages reconcile",sample.len(),if migration.serving_legacy { "legacy (P0 exposed)" } else { "candidate" })});
        }
        let alerts=signature_alerts_query(&connection)?
            .query_map([],|row| Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (claim_id, count) in alerts {
            let writers=connection.prepare_cached("SELECT writer,COUNT(*) FROM claim_signature_sources WHERE claim_id=?1 GROUP BY writer ORDER BY writer")?
                .query_map([&claim_id],|row| Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            faults.push(SignatureStateFault { code:"signature-candidate-count".into(), subject:format!("claim/{claim_id}"), detail:format!("{count} distinct candidates retained; carrying writers: {writers:?}; threshold 64 is inspection only") });
        }
        Ok(faults)
    }
}

impl Store {
    /// The trust roots: `(host/NAME, key)` for every member incarnation fleet membership admits,
    /// and this node's own key. The membership part is folded again only when `fleet_generation`
    /// moved: it is a function of the fleet claims, their envelopes' signatures and the anchor,
    /// and triggers advance the generation whenever any of them changes.
    ///
    /// Call it before the transaction writes anything: the generation and the fold are then read
    /// from the same committed snapshot, so a cached pair is always consistent. A transaction that
    /// had already changed fleet claims could cache a membership its rollback would undo.
    pub(crate) fn trust_roots(
        &self,
        connection: &Connection,
        origin: &str,
    ) -> Result<BTreeSet<(String, String)>> {
        let generation: i64 =
            connection.query_row("SELECT value FROM fleet_generation WHERE id=1", [], |row| {
                row.get(0)
            })?;
        let cached = self
            .roots_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|(folded_at, _)| *folded_at == generation)
            .map(|(_, roots)| roots.clone());
        let mut roots = match cached {
            Some(roots) => roots,
            None => {
                let roots = member_roots(connection)?;
                *self
                    .roots_cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some((generation, roots.clone()));
                roots
            }
        };
        if let Some(own) = fleet_meta(connection, OWN_NODE_KEY)? {
            roots.insert((format!("host/{origin}"), own));
        }
        Ok(roots)
    }
}

/// Answers for one claim being judged: every question about order is about claims before it.
struct StoreFacts<'a> {
    connection: &'a Connection,
    roots: &'a BTreeSet<(String, String)>,
    position: Position,
    claim_id: String,
    depth: usize,
    /// Verdicts worked out during this pass for claims not yet cached.
    memo: &'a RefCell<BTreeMap<String, Verdict>>,
    /// What the verdict relied on.
    links: RefCell<BTreeSet<String>>,
    /// The first failed read, kept typed so a transient SQLite error stays retryable.
    fault: RefCell<Option<anyhow::Error>>,
    incomplete: RefCell<bool>,
}

impl StoreFacts<'_> {
    fn read<T>(&self, result: Result<T>, neutral: T) -> T {
        match result {
            Ok(value) => value,
            Err(error) => {
                self.fault.borrow_mut().get_or_insert(error);
                neutral
            }
        }
    }
    fn before(&self, other: &str) -> bool {
        self.read(position(self.connection, other), None)
            .is_some_and(|other| other.compare(&self.position) == CmpOrdering::Less)
    }

    fn verdict_of(&self, claim_id: &str) -> Verdict {
        if let Some(verdict) = self.memo.borrow().get(claim_id) {
            return verdict.clone();
        }
        if self.depth >= MAX_CHAIN_DEPTH {
            return Verdict::Invalid("the chain is too deep or loops".into());
        }
        let verdict = judge_claim(
            self.connection,
            self.roots,
            claim_id,
            self.depth + 1,
            self.memo,
        )
        .map(|(verdict, _, _)| verdict)
        .unwrap_or_else(|error| {
            let verdict = Verdict::Held(format!("signature-state-corrupt: {error:#}"));
            self.fault.borrow_mut().get_or_insert(error);
            verdict
        });
        self.memo
            .borrow_mut()
            .insert(claim_id.to_owned(), verdict.clone());
        verdict
    }
}

impl Facts for StoreFacts<'_> {
    fn delegation(&self, claim_id: &str) -> Option<Delegation> {
        self.links
            .borrow_mut()
            .insert(format!("delegation:{claim_id}"));
        let row = self.read(claim_row(self.connection, claim_id), None)?;
        if row.kind != KEY_GRANTED {
            return None;
        }
        let grant = KeyGrant::from_fields(row.body.get("fields")?)?;
        self.links
            .borrow_mut()
            .insert(format!("key:{}|{}", row.subject, grant.key));
        Some(Delegation {
            subject: row.subject,
            grant,
            verdict: self.verdict_of(claim_id),
        })
    }

    fn is_root(&self, subject: &str, key: &str) -> bool {
        self.links.borrow_mut().insert("root".into());
        crate::principal::facts::root_eligible(
            subject, key, crate::principal::facts::RootContext::Runtime(self.roots),
        )
    }

    fn revoked_before(&self, principal: &str, key: &str) -> bool {
        self.links
            .borrow_mut()
            .insert(format!("key:{principal}|{key}"));
        let revocations = self
            .connection
            .prepare_cached(
                "SELECT claims.id FROM claims WHERE claims.subject=?1 AND claims.kind=?2
                   AND json_extract(claims.body, '$.fields.key')=?3
                   AND EXISTS(SELECT 1 FROM claim_signature_candidates WHERE claim_id=claims.id)",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(params![principal, KEY_REVOKED, key], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(Into::into);
        let revocations = self.read(revocations, Vec::new());
        revocations.into_iter().any(|revocation| {
            if revocation == self.claim_id
                || !self.before(&revocation)
                || self.depth >= MAX_CHAIN_DEPTH
            {
                return false;
            }
            let (verdict, _, signature) = self.read(
                judge_claim(
                    self.connection,
                    self.roots,
                    &revocation,
                    self.depth + 1,
                    self.memo,
                ),
                (
                    Verdict::Held("signature-state-corrupt".into()),
                    BTreeSet::new(),
                    None,
                ),
            );
            if matches!(&verdict,Verdict::Held(reason) if reason.starts_with("signature-judgment-incomplete:")) { *self.incomplete.borrow_mut()=true; }
            verdict == Verdict::Verified
                && signature.is_some_and(|signature| {
                    signature.signer == principal
                        || Family::of(&signature.signer) == Some(Family::Node)
                })
        })
    }

    fn nonce_used_before(&self, key: &str, nonce: &str) -> bool {
        self.links
            .borrow_mut()
            .insert(format!("nonce:{key}|{nonce}"));
        let others = self
            .connection
            .prepare_cached(
                "SELECT claim_id FROM claim_signature_nonce_uses WHERE key=?1 AND nonce=?2 AND claim_id<>?3",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(params![key, nonce, self.claim_id], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(Into::into);
        let others = self.read(others, Vec::new());
        others.iter().any(|other| self.before(other))
    }
}

fn cached_verdict(connection: &Connection, claim_id: &str) -> Result<Option<Verdict>> {
    Ok(connection
        .prepare_cached("SELECT verdict, reason FROM claim_verdicts WHERE claim_id=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-hf0974ef66e69 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| {
            Ok(verdict_from(&row.get::<_, String>(0)?, row.get(1)?))
        })
        .optional()?)
}

fn verdict_from(name: &str, reason: Option<String>) -> Verdict {
    let reason = reason.unwrap_or_default();
    match name {
        "verified" => Verdict::Verified,
        "unsigned" => Verdict::Unsigned,
        "held" => Verdict::Held(reason),
        _ => Verdict::Invalid(reason),
    }
}

/// Judge one claim from the claims as they stand, with what it relied on.
fn judge_claim(
    connection: &Connection,
    roots: &BTreeSet<(String, String)>,
    claim_id: &str,
    depth: usize,
    memo: &RefCell<BTreeMap<String, Verdict>>,
) -> Result<(Verdict, BTreeSet<String>, Option<ClaimSignature>)> {
    if !migration::judging_available(connection)? {
        let reason = migration::unavailable(connection)?
            .unwrap_or_else(|| "signature-state-corrupt: missing inventory state".into());
        return Ok((Verdict::Held(reason), BTreeSet::new(), None));
    }
    let Some(row) = claim_row(connection, claim_id)? else {
        return Ok((
            Verdict::Held(format!("claim {claim_id} is not here")),
            BTreeSet::new(),
            None,
        ));
    };
    let count = judging::candidate_count(connection, claim_id)?;
    if count > CANDIDATE_PAGE as u64 {
        if judging::complete(connection, claim_id)? {
            judging::validate_cached(connection, &row)?;
            let verdict=cached_verdict(connection,claim_id)?.ok_or_else(|| anyhow::anyhow!("signature-state-corrupt: completed candidate judgment has no verdict for {claim_id}"))?;
            let signature=stored_signature(connection,claim_id)?.ok_or_else(|| anyhow::anyhow!("signature-state-corrupt: completed candidate judgment has no signature for {claim_id}"))?;
            return Ok((verdict, BTreeSet::new(), Some(signature)));
        }
        return Ok((
            Verdict::Held(format!("signature-judgment-incomplete: {claim_id}")),
            BTreeSet::new(),
            None,
        ));
    }
    judge_candidates(
        connection,
        roots,
        &row,
        signature_candidates(connection, claim_id)?,
        depth,
        memo,
    )
}

fn judge_candidates(
    connection: &Connection,
    roots: &BTreeSet<(String, String)>,
    row: &ClaimRow,
    signatures: Vec<ClaimSignature>,
    depth: usize,
    memo: &RefCell<BTreeMap<String, Verdict>>,
) -> Result<(Verdict, BTreeSet<String>, Option<ClaimSignature>)> {
    let claim_id = &row.id;
    let Some(position) = position(connection, claim_id)? else {
        return Ok((
            Verdict::Held(format!("claim {claim_id} is not here")),
            BTreeSet::new(),
            None,
        ));
    };
    let facts = StoreFacts {
        connection,
        roots,
        position,
        claim_id: row.id.clone(),
        depth,
        memo,
        links: RefCell::default(),
        fault: RefCell::default(),
        incomplete: RefCell::default(),
    };
    let fields = signature_fields(row);
    let judged = crate::principal::Judged {
        id: &row.id,
        subject: &row.subject,
        kind: &row.kind,
        actor: row.actor.as_deref(),
        content: content_digest(&row.subject, &row.kind, row.actor.as_deref(), &row.body),
        fields: &fields,
    };
    let mut best: Option<(Verdict, ClaimSignature)> = None;
    // Candidates already have normalized byte order. Equal verdict ranks keep the first.
    // Judge every candidate to preserve losing candidates' authority dependencies too.
    for signature in signatures {
        let verdict = judge(&judged, Some(&signature), &facts);
        if best
            .as_ref()
            .is_none_or(|(previous, _)| judging::rank(&verdict) < judging::rank(previous))
        {
            best = Some((verdict, signature));
        }
    }
    if let Some(error) = facts.fault.into_inner() {
        return Err(error.context("signature-state-corrupt"));
    }
    if facts.incomplete.into_inner() {
        return Ok((
            Verdict::Held(
                "signature-judgment-incomplete: an authority dependency is pending".into(),
            ),
            facts.links.into_inner(),
            best.map(|(_, signature)| signature),
        ));
    }
    let (verdict, signature) = best.map_or((Verdict::Unsigned, None), |(verdict, signature)| {
        (verdict, Some(signature))
    });
    Ok((verdict, facts.links.into_inner(), signature))
}

fn write_verdict_tx(
    connection: &Connection,
    claim_id: &str,
    signature: &ClaimSignature,
    verdict: &Verdict,
) -> Result<bool> {
    let previous = cached_verdict(connection, claim_id)?;
    let previous_signature = stored_signature(connection, claim_id)?;
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h5f8142f84899 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signatures SET signer=?1,key=?2,nonce=?3,signature=?4 WHERE claim_id=?5",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h63d55e8c184c owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![
            signature.signer,
            signature.key,
            signature.nonce,
            serde_json::to_string(signature)?,
            claim_id
        ])?;
    // Candidate pages have already committed their dependency links. Publishing the
    // verdict never reads, deletes or rewrites the full link set.
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h3af9bdc217e6 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO claim_verdicts(claim_id, verdict, reason, signer, on_behalf)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-he60e299d0182 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![
            claim_id,
            verdict.name(),
            verdict.reason(),
            signature.signer,
            signature.on_behalf
        ])?;
    let notify: bool = connection
        .prepare_cached("SELECT notified=0 FROM claim_signature_judging WHERE claim_id=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h7f915a30389b owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0))
        .optional()?
        .unwrap_or(false);
    let ready = !matches!(verdict,Verdict::Held(reason) if reason.starts_with("signature-judgment-incomplete:"));
    if ready {
        connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-hbe00773d6819 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_judging SET notified=1 WHERE claim_id=?1")?
            .execute(/* sqlite-large-value: allow-for-now lv-h4f2bc9087eab owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
    }
    Ok(previous.as_ref() != Some(verdict)
        || previous_signature.as_ref() != Some(signature)
        || (notify && ready))
}

/// Forget a claim's signature and every cached verdict row of it, when a checkpoint drops the
/// claim. None of these rows is synced or in a digest, and nothing reads them for a claim that is
/// gone: a nonce counts as used before only by a claim the store still holds, and a dropped claim
/// is never a delegation another verdict relies on (those kinds are never dropped).
pub(crate) fn forget_claim_tx(connection: &Connection, claim_id: &str) -> Result<usize> {
    // Absence of a formerly live nonce use can make a retained candidate valid again.
    // Queue its dependents before deleting the relation, in the deletion transaction.
    // The indexed join includes non-winning cryptographic uses, not just the facade.
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hd26d59e3827d owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_judging SET dirty=1,notified=0 WHERE claim_id IN (SELECT l.claim_id FROM claim_signature_nonce_uses n CROSS JOIN claim_verdict_links l ON l.link='nonce:'||n.key||'|'||n.nonce WHERE n.claim_id=?1)")?.execute(/* sqlite-large-value: allow-for-now lv-h4207bd94c16e owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h4ff355fec1f2 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_verdict_queue(claim_id) SELECT l.claim_id FROM claim_signature_nonce_uses n CROSS JOIN claim_verdict_links l ON l.link='nonce:'||n.key||'|'||n.nonce WHERE n.claim_id=?1")?.execute(/* sqlite-large-value: allow-for-now lv-h871e734931b2 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
    queue_affected_by_tx(connection, claim_id)?;
    if legacy::active(connection)? {
        legacy::queue_affected_by_tx(connection, claim_id)?;
    }
    let mut rows = 0;
    if legacy::present(connection)? {
        for table in legacy::TABLES {
            rows += connection
                .prepare_cached(&format!(/* sqlite-large-value: allow-for-now lv-h682b64691597 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {table} WHERE claim_id=?1"))?
                .execute(/* sqlite-large-value: allow-for-now lv-h1a31bc53b2c8 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
        }
    }
    for table in [
        "claim_signatures",
        "claim_signature_candidates",
        "claim_signature_nonce_uses",
        "claim_signature_judging",
        "claim_verdicts",
        "claim_verdict_links",
        "claim_verdict_queue",
        "claim_verdict_fresh",
    ] {
        rows += connection
            .prepare_cached(&format!(/* sqlite-large-value: allow-for-now lv-hf6676f3ba393 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {table} WHERE claim_id=?1"))?
            .execute(/* sqlite-large-value: allow-for-now lv-h8f30a91548fa owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
    }
    Ok(rows)
}

/// Queue the verdicts linked to `link`: the first page now, and the rest a page per later loan.
/// A change during an unfinished requeue starts it over; judging reads the current authority
/// either way.
fn queue_linked_tx(connection: &Connection, link: &str) -> Result<()> {
    requeue_page_tx(connection, link, "")?;
    Ok(())
}

/// The trust roots changed: queue what relied on them from the start, all in later loans.
fn start_root_requeue_tx(connection: &Connection, roots_digest: &str) -> Result<()> {
    set_meta_tx(connection, VERDICT_ROOTS, roots_digest)?;
    set_meta_tx(connection, &format!("{REQUEUE}root"), "")
}

/// Queue one page of the claims linked to `link` after `after`, in claim order, and keep or end
/// its cursor. Returns whether more remain.
fn requeue_page_tx(connection: &Connection, link: &str, after: &str) -> Result<bool> {
    let page = params![link, after, REQUEUE_PAGE as i64];
    let (last, count): (Option<String>, usize) = connection
        .prepare_cached("SELECT MAX(claim_id), COUNT(*) FROM (SELECT claim_id FROM claim_verdict_links WHERE link=?1 AND claim_id>?2 ORDER BY claim_id LIMIT ?3)")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h26d9a509862a owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ page, |row| Ok((row.get(0)?, row.get(1)?)))?;
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-he8dea43d15fe owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_judging SET dirty=1 WHERE claim_id IN (SELECT claim_id FROM claim_verdict_links WHERE link=?1 AND claim_id>?2 ORDER BY claim_id LIMIT ?3)")?.execute(/* sqlite-large-value: allow-for-now lv-h745c77576b29 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ page)?;
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hc142559507cb owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_verdict_queue(claim_id) SELECT claim_id FROM claim_verdict_links WHERE link=?1 AND claim_id>?2 ORDER BY claim_id LIMIT ?3")?.execute(/* sqlite-large-value: allow-for-now lv-h86a60c947619 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ page)?;
    let key = format!("{REQUEUE}{link}");
    match last.filter(|_| count == REQUEUE_PAGE) {
        Some(last) => {
            set_meta_tx(connection, &key, &last)?;
            Ok(true)
        }
        None => {
            connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-ha0cfaa1ec5cc owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1")?
                .execute(/* sqlite-large-value: allow-for-now lv-hdf404790377b owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [key])?;
            Ok(false)
        }
    }
}

/// Queue the next page of the first unfinished requeue. Returns whether any requeue remains.
fn advance_requeue_tx(connection: &Connection) -> Result<bool> {
    let cursor = connection
        .prepare_cached(
            "SELECT key, value FROM meta WHERE key>=?1 AND key<?2 ORDER BY key LIMIT 1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h582e5366bbee owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [REQUEUE, REQUEUE_END], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .optional()?;
    let Some((key, after)) = cursor else {
        return Ok(false);
    };
    if requeue_page_tx(connection, &key[REQUEUE.len()..], &after)? {
        return Ok(true);
    }
    requeue_pending(connection)
}

fn requeue_pending(connection: &Connection) -> Result<bool> {
    Ok(connection
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM meta WHERE key>=?1 AND key<?2)")?
        .query_row(/* sqlite-large-value: allow-for-now lv-hb771731d9c03 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [REQUEUE, REQUEUE_END], |row| row.get(0))?)
}

/// Where a claim relies on a link whose requeue cursor has not reached it yet, that link: the
/// claim's page is not queued, so its cached verdict can predate the change and reads as held,
/// as it did when one transaction queued every dependent.
pub(super) fn requeue_ahead_of(connection: &Connection, claim_id: &str) -> Result<Option<String>> {
    Ok(connection
        .prepare_cached(
            "SELECT l.link FROM meta m JOIN claim_verdict_links l
               ON l.link=substr(m.key, ?3) AND l.claim_id=?4
             WHERE m.key>=?1 AND m.key<?2 AND m.value<?4 ORDER BY m.key LIMIT 1",
        )?
        .query_row(
            /* sqlite-large-value: allow-for-now lv-hd85da347d250 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */
            params![REQUEUE, REQUEUE_END, REQUEUE.len() + 1, claim_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// The links a newly arrived signed claim can change: the claims waiting on it as a
/// delegation, the claims signed by a key it revokes, and the claims sharing its nonce.
fn queue_affected_by_tx(connection: &Connection, claim_id: &str) -> Result<()> {
    let Some(row) = claim_row(connection, claim_id)? else {
        return Ok(());
    };
    queue_affected_by_row_tx(connection, &row)
}
fn queue_affected_by_row_tx(connection: &Connection, row: &ClaimRow) -> Result<()> {
    let field_key = row
        .body
        .pointer("/fields/key")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match (row.kind.as_str(), field_key) {
        (KEY_GRANTED, _) => queue_linked_tx(connection, &format!("delegation:{}", row.id))?,
        (KEY_REVOKED, Some(key)) => {
            queue_linked_tx(connection, &format!("key:{}|{key}", row.subject))?
        }
        _ => {}
    }
    Ok(())
}

/// One claim this node is about to seal, as signing needs it.
pub struct Unsealed<'a> {
    pub subject: &'a str,
    pub kind: &'a str,
    pub actor: Option<&'a str>,
    pub body: &'a Value,
    pub accepted_at_unix_ms: u128,
}

/// Which complete authorization model supplies a local judgment. A legacy-serving member
/// is not qualified to supply candidate-model authority to another member or feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignatureModel {
    Legacy,
    Candidate,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClaimAuthorization {
    pub model: SignatureModel,
    pub verdict: Verdict,
    pub signature: Option<ClaimSignature>,
}

/// All `claim_signer` reads of a claim besides the keyring: the issuer and issuer key of the
/// grant it makes, when it is a key grant, and its actor. Claims with equal inputs are signed by
/// the same signer.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SignerInput {
    grant: Option<(String, String)>,
    actor: Option<String>,
}

impl SignerInput {
    pub(crate) fn of(claim: &Unsealed<'_>) -> Self {
        let grant = if claim.kind == KEY_GRANTED {
            claim
                .body
                .get("fields")
                .and_then(KeyGrant::from_fields)
                .map(|grant| (grant.issuer, grant.issuer_key))
        } else {
            None
        };
        Self {
            grant,
            actor: claim.actor.map(str::to_owned),
        }
    }
}

/// The key a claim is signed with, the principal it signs as, the principal it acts for and the
/// delegation chain from its key: what a claim signature records about its signer.
pub(crate) struct ClaimSigner {
    pub(crate) key: Arc<crate::fleet::MemberKey>,
    pub(crate) signer: String,
    pub(crate) on_behalf: Option<String>,
    pub(crate) chain: Vec<String>,
}

impl ClaimSigner {
    /// Whether `signature` names this signer: the same key, principal, principal acted for and
    /// chain. Two signatures by the same signer differ only in nonce, and so in signature bytes.
    pub(crate) fn signed(&self, signature: &ClaimSignature) -> bool {
        signature.key == self.key.public()
            && signature.signer == self.signer
            && signature.on_behalf == self.on_behalf
            && signature.chain == self.chain
    }
}

/// What a verdict recheck found.
#[derive(Clone, Debug, Serialize)]
pub struct VerdictMismatch {
    pub claim_id: String,
    pub cached: Option<String>,
    pub recomputed: String,
}

impl Store {
    /// Write `input` with a signature a device made for it, in the same transaction. The
    /// signature must already verify for `input`; an idempotent repeat keeps the first claim and
    /// its signature.
    pub fn append_signed_claim(
        &self,
        input: &ClaimInput,
        signature: &ClaimSignature,
    ) -> Result<(ClaimRecord, bool), St3Error> {
        let actor = input
            .actor
            .clone()
            .ok_or_else(|| St3Error::new("invalid-signature", "a signed claim names its actor"))?;
        let key = (input.subject.clone(), input.kind.clone(), actor);
        let text = serde_json::to_string(signature).map_err(internal)?;
        {
            let connection = self.connection.write();
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h29f3aefa1eb9 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO expected_claim_signatures(subject, kind, actor, signature)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![key.0, key.1, key.2, text],
                )
                .map_err(internal)?;
        }
        let appended = self.append_claim_outcome(input);
        // Nothing waits for a signature once the write has happened or failed.
        let connection = self.connection.write();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h57caa6e1f61e owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM expected_claim_signatures WHERE subject=?1 AND kind=?2 AND actor=?3",
                params![key.0, key.1, key.2],
            )
            .map_err(internal)?;
        appended
    }

    /// Whether a live claim already carries cryptographic signing use of this key/nonce.
    /// A partial legacy inventory cannot answer absence safely and refuses authorization.
    pub fn signature_nonce_used(&self, key: &str, nonce: &str) -> Result<bool> {
        self.read_snapshot(|_| nonce_used(&self.readers.get(), key, nonce))
    }

    /// Enrol a device of `person`: the person's root key, which this node holds, grants `key` as
    /// a device key. Returns the chain the device signs with: its grant, then the root's.
    pub fn enroll_device_key(
        &self,
        person: &str,
        key: &str,
        label: &str,
    ) -> Result<Vec<String>, St3Error> {
        if Family::of(person) != Some(Family::Person) {
            return Err(St3Error::new(
                "invalid-person",
                "only a person enrols devices",
            ));
        }
        self.ensure_principal_key(person)?;
        let root = self.held_root(person).ok_or_else(|| {
            St3Error::new(
                "no-person-key",
                "this node holds no root key for the person, so it cannot enrol a device",
            )
        })?;
        let (granted, _) = self.runtime.append_claim(
            self,
            &ClaimInput {
                subject: person.into(),
                kind: KEY_GRANTED.into(),
                actor: Some(person.into()),
                fields: KeyGrant {
                    key: key.into(),
                    role: Role::Device,
                    issuer: person.into(),
                    issuer_key: root.key.public().into(),
                    label: Some(label.into()),
                }
                .fields(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("device-key:{person}:{key}")),
            },
        )?;
        let mut chain = vec![granted.id];
        chain.extend(root.chain.iter().cloned());
        Ok(chain)
    }

    /// Public trust anchor for local pairing, from the exact root enrollment uses.
    /// Returns no key on legacy unsigned nodes. Never exposes private key material.
    pub fn person_root_grant(&self, person: &str) -> Result<Option<(String, String)>, St3Error> {
        if Family::of(person) != Some(Family::Person) {
            return Err(St3Error::new(
                "invalid-person",
                "only a person pairs devices",
            ));
        }
        self.ensure_principal_key(person)?;
        Ok(self.held_root(person).and_then(|root| {
            root.chain
                .first()
                .map(|grant| (root.key.public().to_owned(), grant.clone()))
        }))
    }

    /// Withdraw a device key of `person`, as the node.
    pub fn revoke_device_key(&self, person: &str, key: &str, reason: &str) -> Result<(), St3Error> {
        self.append_claim(&ClaimInput {
            subject: person.into(),
            kind: KEY_REVOKED.into(),
            actor: None,
            fields: BTreeMap::from([
                ("key".into(), Value::String(key.into())),
                ("reason".into(), Value::String(reason.into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("device-key-revoked:{person}:{key}")),
        })
        .map(|_| ())
    }

    /// Use `directory` for the private keys this node mints for people and agents, and load the
    /// keys it already holds.
    pub fn use_key_directory(&self, directory: &Path) -> Result<()> {
        self.keyring.set_directory(directory);
        let rows = {
            let connection = self.readers.get();
            let mut statement =
                connection.prepare("SELECT public_key, principal, role, chain FROM held_keys")?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (public, principal, role, chain) in rows {
            let Some(key) = self.keyring.load(&public)? else {
                continue;
            };
            let Ok(role) = serde_json::from_value::<Role>(Value::String(role)) else {
                continue;
            };
            self.keyring.insert(HeldKey {
                principal,
                role,
                key,
                chain: serde_json::from_str(&chain).unwrap_or_default(),
            });
        }
        Ok(())
    }

    /// The key that signs this node's own claims and vouches for the people and agents it
    /// speaks for. A fleet member's node key is its member key.
    pub fn set_node_key(&self, key: Arc<crate::fleet::MemberKey>) -> Result<()> {
        let public = key.public().to_owned();
        self.keyring.set_node(Some(key));
        let connection = self.connection.write();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h66ada43b6645 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![OWN_NODE_KEY, public],
        )?;
        Ok(())
    }

    fn node_subject(&self) -> String {
        format!("host/{}", self.origin)
    }

    /// Make sure this node holds a signing key for `actor` when it is a person or an agent,
    /// minting one and writing its delegation the first time. Without a node key nothing is
    /// minted, and claims stay unsigned as before.
    pub fn ensure_principal_key(&self, actor: &str) -> Result<(), St3Error> {
        let family = match Family::of(actor) {
            Some(family @ (Family::Person | Family::Agent)) => family,
            _ => return Ok(()),
        };
        if self.keyring.signing(actor).is_some() {
            return Ok(());
        }
        let Some(node) = self.keyring.node() else {
            return Ok(());
        };
        let _minting = self
            .principal_minting
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.keyring.signing(actor).is_some() {
            return Ok(());
        }
        let node_subject = self.node_subject();
        let grant = |subject: &str,
                     key: &crate::fleet::MemberKey,
                     role: Role,
                     issuer: &str,
                     issuer_key: &str,
                     label: Option<String>|
         -> Result<ClaimRecord, St3Error> {
            self.runtime
                .append_claim(
                    self,
                    &ClaimInput {
                        subject: subject.into(),
                        kind: KEY_GRANTED.into(),
                        actor: Some(issuer.into()),
                        fields: KeyGrant {
                            key: key.public().into(),
                            role,
                            issuer: issuer.into(),
                            issuer_key: issuer_key.into(),
                            label,
                        }
                        .fields(),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: None,
                    },
                )
                .map(|(claim, _)| claim)
        };
        let label = Some(format!("{} on {}", actor, self.origin));
        match family {
            Family::Agent => {
                let key = self.keyring.create().map_err(internal)?;
                let granted = grant(
                    actor,
                    &key,
                    Role::Agent,
                    &node_subject,
                    node.public(),
                    label,
                )?;
                self.hold_key(actor, Role::Agent, key, vec![granted.id])?;
            }
            Family::Person => {
                let root = match self.held_root(actor) {
                    Some(root) => root,
                    None => {
                        let key = self.keyring.create().map_err(internal)?;
                        let granted =
                            grant(actor, &key, Role::Root, &node_subject, node.public(), None)?;
                        self.hold_key(actor, Role::Root, key, vec![granted.id])?
                    }
                };
                let device = self.keyring.create().map_err(internal)?;
                let granted = grant(
                    actor,
                    &device,
                    Role::Device,
                    actor,
                    root.key.public(),
                    label,
                )?;
                let mut chain = vec![granted.id];
                chain.extend(root.chain.iter().cloned());
                self.hold_key(actor, Role::Device, device, chain)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn held_root(&self, principal: &str) -> Option<HeldKey> {
        let connection = self.readers.get();
        let public = connection
            .query_row(
                "SELECT public_key FROM held_keys WHERE principal=?1 AND role='root'",
                [principal],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()?;
        self.keyring.by_public(&public)
    }

    fn hold_key(
        &self,
        principal: &str,
        role: Role,
        key: Arc<crate::fleet::MemberKey>,
        chain: Vec<String>,
    ) -> Result<HeldKey, St3Error> {
        let role_name = serde_json::to_value(role)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        self.connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-hc16319dc28f4 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO held_keys(public_key, principal, role, chain)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    key.public(),
                    principal,
                    role_name,
                    serde_json::to_string(&chain).map_err(internal)?
                ],
            )
            .map_err(internal)?;
        let held = HeldKey {
            principal: principal.into(),
            role,
            key,
            chain,
        };
        self.keyring.insert(held.clone());
        Ok(held)
    }

    /// Sign one of this node's claims as it is sealed: a key grant by its issuer, a claim by a
    /// principal this node holds a key for with that key, anything else by the node on the
    /// writer's behalf. `None` without a node key.
    pub(crate) fn sign_unsealed(&self, claim: &Unsealed<'_>) -> Option<ClaimSignature> {
        let chosen = self.claim_signer(claim)?;
        let content = content_digest(claim.subject, claim.kind, claim.actor, claim.body);
        let signed_at = u64::try_from(claim.accepted_at_unix_ms).unwrap_or(u64::MAX);
        Some(ClaimSignature::sign(
            &chosen.key,
            &content,
            &chosen.signer,
            chosen.on_behalf.as_deref(),
            chosen.chain,
            signed_at,
        ))
    }

    /// Who `sign_unsealed` signs `claim` as, read from the keyring without signing. `None`
    /// without a node key.
    pub(crate) fn claim_signer(&self, claim: &Unsealed<'_>) -> Option<ClaimSigner> {
        self.claim_signer_for(&SignerInput::of(claim))
    }

    /// Who `sign_unsealed` signs a claim with these inputs as. `None` without a node key.
    pub(crate) fn claim_signer_for(&self, input: &SignerInput) -> Option<ClaimSigner> {
        let node = self.keyring.node()?;
        let node_subject = self.node_subject();
        if let Some((issuer, issuer_key)) = &input.grant {
            if *issuer == node_subject && issuer_key == node.public() {
                return Some(ClaimSigner {
                    key: node,
                    signer: node_subject,
                    on_behalf: None,
                    chain: Vec::new(),
                });
            }
            if let Some(held) = self.keyring.by_public(issuer_key)
                && held.principal == *issuer
            {
                return Some(ClaimSigner {
                    key: held.key,
                    signer: held.principal,
                    on_behalf: None,
                    chain: held.chain,
                });
            }
        }
        if let Some(actor) = input.actor.as_deref()
            && let Some(held) = self.keyring.signing(actor)
        {
            return Some(ClaimSigner {
                key: held.key,
                signer: actor.to_owned(),
                on_behalf: None,
                chain: held.chain,
            });
        }
        let on_behalf = input
            .actor
            .as_deref()
            .filter(|actor| *actor != node_subject)
            .map(str::to_owned);
        Some(ClaimSigner {
            key: node,
            signer: node_subject,
            on_behalf,
            chain: Vec::new(),
        })
    }

    /// Whether a pass has anything to judge: a newly signed claim, a queued one, or a link whose
    /// verdicts are still being queued. Three probes of small tables on a reader.
    pub fn verdicts_pending(&self) -> Result<bool> {
        let connection = self.readers.get();
        let judging: bool = connection
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM claim_verdict_fresh)
                     OR EXISTS(SELECT 1 FROM claim_verdict_queue)",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-ha76b0c3b2d8f owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?;
        Ok(judging || requeue_pending(&connection)?)
    }

    /// Judge every signed claim admitted since the last pass and every queued one. With
    /// `check_roots`, first queue every verdict that relied on the trust roots if fleet
    /// membership changed them. Each chunk takes the writer once; queued writes run between
    /// chunks. A pass with nothing to do reads one row.
    pub fn judge_claims(&self, check_roots: bool) -> Result<usize> {
        if legacy::active(&self.readers.get())? {
            // Ordinary API work maintains the serving model. The finite background job
            // alone owns the historical shadow queue until activation.
            return legacy::judge_claims(self, check_roots);
        }
        self.record_judgment_failure(self.judge_claims_inner(check_roots, None))
    }

    /// As [`Self::judge_claims`], bounded to [`VERDICT_SLICE_LOANS`] loans for the daemon's
    /// ordinary passes. Work left over keeps `verdicts_due` set.
    pub fn judge_claims_slice(&self, check_roots: bool) -> Result<usize> {
        if legacy::active(&self.readers.get())? {
            return legacy::judge_claims(self, check_roots);
        }
        let judged = self.record_judgment_failure(
            self.judge_claims_inner(check_roots, Some(VERDICT_SLICE_LOANS)),
        )?;
        if self.verdicts_pending()? {
            self.verdicts_due.store(true, Ordering::Release);
            // The paced signature job owns what a slice leaves; a faulted member waits for resume.
            if self.signature_migration()?.fault.is_none() {
                self.signature_job_wake.notify_one();
            }
        }
        Ok(judged)
    }

    /// One resumable judgment loan for the paced signature worker: the bootstrap's shadow
    /// judgment, and after it any backlog a resume or a trust-root change left.
    pub fn advance_signature_judgment(&self) -> Result<usize> {
        self.record_judgment_failure(self.judge_claims_inner(true, Some(1)))
    }

    fn record_judgment_failure(&self, result: Result<usize>) -> Result<usize> {
        self.note_signature_retry(result.as_ref().err());
        if let Err(error) = &result
            && !migration::transient_storage_error(error)
        {
            migration::record_fault(
                &self.connection.write(),
                &format!("verdict reconciliation: {error:#}"),
            )?;
        }
        result
    }
    fn judge_claims_inner(&self, check_roots: bool, max_loans: Option<usize>) -> Result<usize> {
        let migration = self.signature_migration()?;
        if migration.fault.is_some() || !matches!(migration.phase.as_str(), "done" | "judging") {
            return Ok(0);
        }
        let track_bootstrap = !migration.done();
        if check_roots {
            let mut connection = self.connection.write();
            let transaction = connection.transaction()?;
            let roots = self.trust_roots(&transaction, &self.origin)?;
            let roots_digest = crate::hash::canonical_hash(&roots)?;
            if fleet_meta(&transaction, VERDICT_ROOTS)?.as_deref() != Some(roots_digest.as_str()) {
                start_root_requeue_tx(&transaction, &roots_digest)?;
            }
            transaction.commit()?;
        }
        if !track_bootstrap && !self.verdicts_pending()? {
            return Ok(0);
        }
        let mut judged = 0;
        let mut loans = 0;
        loop {
            loans += 1;
            let mut connection = self.connection.write();
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let roots = self.trust_roots(&transaction, &self.origin)?;
            // A root can change between bounded loans. The final activation transaction
            // must reconcile that input too, rather than certify earlier cached roots.
            if check_roots || track_bootstrap {
                let roots_digest = crate::hash::canonical_hash(&roots)?;
                if fleet_meta(&transaction, VERDICT_ROOTS)?.as_deref()
                    != Some(roots_digest.as_str())
                {
                    start_root_requeue_tx(&transaction, &roots_digest)?;
                }
            }
            let requeuing = advance_requeue_tx(&transaction)?;
            let fresh = transaction
                .prepare_cached("SELECT claim_id FROM claim_verdict_fresh LIMIT ?1")?
                .query_map([JUDGE_CHUNK as i64], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for claim_id in &fresh {
                transaction
                    .prepare_cached(/* sqlite-large-value: allow-for-now lv-hfa4112c13a2b owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_verdict_fresh WHERE claim_id=?1")?
                    .execute(/* sqlite-large-value: allow-for-now lv-h28d625e3f51a owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
                // A new delegation, revocation or reused nonce changes what relied on it.
                queue_affected_by_tx(&transaction, claim_id)?;
                transaction
                    .prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-hdb329af360b2 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_verdict_queue(claim_id) VALUES (?1)",
                    )?
                    .execute(/* sqlite-large-value: allow-for-now lv-h9c7ea890a098 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
            }
            let queued = transaction
                .prepare_cached("SELECT claim_id FROM claim_verdict_queue LIMIT ?1")?
                .query_map([JUDGE_CHUNK as i64], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let memo = RefCell::new(BTreeMap::new());
            let mut candidates_judged = 0;
            for claim_id in &queued {
                if candidates_judged >= CANDIDATE_PAGE {
                    break;
                }
                transaction
                    .prepare_cached(/* sqlite-large-value: allow-for-now lv-h9b345496e02d owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_verdict_queue WHERE claim_id=?1")?
                    .execute(/* sqlite-large-value: allow-for-now lv-h29105e98503c owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [claim_id])?;
                // A signature can arrive for a claim a checkpoint has since dropped.
                let Some(row) = claim_row(&transaction, claim_id)? else {
                    continue;
                };
                let (result, used) = judging::advance(
                    &transaction,
                    &roots,
                    &row,
                    &memo,
                    CANDIDATE_PAGE - candidates_judged,
                )?;
                candidates_judged += used.max(1);
                let Some((verdict, signature)) = result else {
                    continue;
                };
                let Some(signature) = signature else {
                    continue;
                };
                if write_verdict_tx(&transaction, claim_id, &signature, &verdict)? {
                    // A changed delegation or revocation changes what relied on it.
                    queue_affected_by_row_tx(&transaction, &row)?;
                }
                judged += 1;
            }
            if track_bootstrap && candidates_judged != 0 {
                migration::note_work(&transaction, candidates_judged)?;
            }
            let drained = fresh.is_empty() && queued.is_empty() && !requeuing;
            if drained {
                migration::finish_judging(&transaction)?;
            }
            transaction.commit()?;
            if drained || max_loans.is_some_and(|max| loans >= max) {
                return Ok(judged);
            }
        }
    }

    /// Exercise a real bounded loan, then allow another real writer to change inputs.
    #[cfg(feature = "test-support")]
    pub fn judge_claim_page_for_test(&self, check_roots: bool) -> Result<usize> {
        self.judge_claims_inner(check_roots, Some(1))
    }

    pub fn signature_model(&self) -> Result<SignatureModel> {
        self.read_snapshot(|_| {
            Ok(if legacy::active(&self.readers.get())? {
                SignatureModel::Legacy
            } else {
                SignatureModel::Candidate
            })
        })
    }

    /// Pin the model, verdict and selected signature together for a relying reader.
    /// An incomplete candidate model gives a typed Held verdict without proof bytes.
    pub fn claim_authorization(&self, claim_id: &str) -> Result<ClaimAuthorization> {
        self.read_snapshot(|_| {
            let model=self.signature_model()?;
            let verdict=self.claim_verdict(claim_id)?;
            let signature=if matches!(&verdict,Verdict::Held(reason) if reason.starts_with("signature-migration-incomplete:") || reason.starts_with("signature-judgment-incomplete:") || reason.starts_with("signature-state-corrupt:")) { None } else { self.claim_signature(claim_id)? };
            Ok(ClaimAuthorization { model, verdict, signature })
        })
    }

    /// A claim's verdict as the cache holds it. A claim with no signature is `unsigned`.
    pub fn claim_verdict(&self, claim_id: &str) -> Result<Verdict> {
        self.read_snapshot(|_| {
            let connection = self.readers.get();
            if legacy::active(&connection)? {
                return Ok(
                    legacy::cached_verdict(&connection, claim_id)?.unwrap_or(Verdict::Unsigned)
                );
            }
            if let Some(reason) = migration::unavailable(&connection)? {
                return Ok(Verdict::Held(reason));
            }
            if let Some(reason) = judging::unavailable(&connection, claim_id)? {
                return Ok(Verdict::Held(reason));
            }
            Ok(cached_verdict(&connection, claim_id)?.unwrap_or(Verdict::Unsigned))
        })
    }

    /// A signature from the same serving model as its verdict.
    pub fn claim_signature(&self, claim_id: &str) -> Result<Option<ClaimSignature>> {
        self.read_snapshot(|_| serving_signature(&self.readers.get(), claim_id))
    }

    /// How many claims have each verdict, read only. This node's own claims are signed as their
    /// batches are sealed: claims in batches not sealed yet count as `unsealed`, and every other
    /// claim without a cached verdict is `unsigned`.
    pub fn claim_verdict_counts(&self) -> Result<BTreeMap<String, u64>> {
        self.read_snapshot(|_| self.claim_verdict_counts_snapshot())
    }
    fn claim_verdict_counts_snapshot(&self) -> Result<BTreeMap<String, u64>> {
        let connection = self.readers.get();
        let serving_legacy = legacy::active(&connection)?;
        if !serving_legacy && migration::unavailable(&connection)?.is_some() {
            let total: u64 =
                connection.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))?;
            return Ok(BTreeMap::from([("held".into(), total)]));
        }
        let unsealed: u64 = connection.query_row(
            "SELECT COUNT(*) FROM batches JOIN claims ON claims.batch_id=batches.id
             WHERE batches.rowid>?1",
            [self.seeded_batch_rowid.load(Ordering::Acquire)],
            |row| row.get(0),
        )?;
        let mut counts = if serving_legacy {
            connection
                .prepare("SELECT verdict, COUNT(*) FROM legacy_claim_verdicts GROUP BY verdict")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
                })?
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()?
        } else {
            // A claim an unfinished requeue has not reached counts as held, as it reads. The
            // per-row probe runs only while a requeue is pending.
            let requeuing = requeue_pending(&connection)?;
            connection
            .prepare("SELECT CASE WHEN EXISTS(SELECT 1 FROM claim_signature_judging p WHERE p.claim_id=v.claim_id AND (p.complete=0 OR p.dirty=1 OR p.blocked=1)) OR (c.candidates>32 AND NOT EXISTS(SELECT 1 FROM claim_signature_judging p WHERE p.claim_id=v.claim_id AND p.complete=1 AND p.dirty=0 AND p.candidate_count=c.candidates)) OR (?4 AND EXISTS(SELECT 1 FROM meta m JOIN claim_verdict_links l ON l.link=substr(m.key,?3) AND l.claim_id=v.claim_id WHERE m.key>=?1 AND m.key<?2 AND m.value<v.claim_id)) THEN 'held' ELSE verdict END, COUNT(*) FROM claim_verdicts v LEFT JOIN claim_signature_counts c USING(claim_id) GROUP BY 1")?
            .query_map(params![REQUEUE, REQUEUE_END, REQUEUE.len() + 1, requeuing], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?
        };
        let pending_uncached: u64 = if serving_legacy {
            0
        } else {
            connection.query_row("SELECT COUNT(*) FROM claim_signature_counts c JOIN claims ON claims.id=c.claim_id LEFT JOIN claim_verdicts v USING(claim_id) WHERE v.claim_id IS NULL AND c.candidates>32",[],|row|row.get(0))?
        };
        if pending_uncached != 0 {
            *counts.entry("held".into()).or_default() += pending_uncached;
        }
        let total: u64 =
            connection.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))?;
        let judged = counts.values().sum::<u64>();
        *counts.entry("unsigned".into()).or_default() +=
            total.saturating_sub(judged).saturating_sub(unsealed);
        if unsealed != 0 {
            counts.insert("unsealed".into(), unsealed);
        }
        Ok(counts)
    }

    /// Recompute every verdict from the claims alone and replace the cache with the result,
    /// reporting each claim whose cached verdict differed. A cache row edited, deleted or added
    /// by hand shows here and never survives.
    pub fn recheck_claim_verdicts(&self) -> Result<Vec<VerdictMismatch>> {
        if let Some(reason) = self.signature_migration()?.unavailable_reason() {
            anyhow::bail!("{reason}");
        }
        let cached = {
            let connection = self.readers.get();
            connection
                .prepare("SELECT claim_id, verdict FROM claim_verdicts")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()?
        };
        {
            let mut connection = self.connection.write();
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                /* sqlite-large-value: allow-for-now lv-h617a09575246 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_signature_judging; DELETE FROM claim_verdicts; DELETE FROM claim_verdict_links;
                 DELETE FROM claim_verdict_queue;
                 INSERT OR IGNORE INTO claim_verdict_fresh(claim_id) SELECT DISTINCT claim_id FROM claim_signature_candidates;",
            )?;
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h4e5e7d6674c0 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1 OR (key>=?2 AND key<?3)",
                params![VERDICT_ROOTS, REQUEUE, REQUEUE_END],
            )?;
            transaction.commit()?;
        }
        self.judge_claims(true)?;
        let connection = self.readers.get();
        let recomputed = connection
            .prepare("SELECT claim_id, verdict FROM claim_verdicts")?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
        let mut mismatches = Vec::new();
        for claim_id in cached
            .keys()
            .chain(recomputed.keys())
            .collect::<BTreeSet<_>>()
        {
            let before = cached.get(claim_id);
            let after = recomputed
                .get(claim_id)
                .cloned()
                .unwrap_or_else(|| "unsigned".into());
            if before.map(String::as_str).unwrap_or("unsigned") != after {
                mismatches.push(VerdictMismatch {
                    claim_id: claim_id.clone(),
                    cached: before.cloned(),
                    recomputed: after,
                });
            }
        }
        Ok(mismatches)
    }
}

fn set_meta_tx(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hfd8ac25bca92 owner=smallclaims::store::principals TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h02841b9e96c0 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![key, value])?;
    Ok(())
}

/// The rules as the graph holds them: for each `rule/NAME`, its latest [`crate::rules::RULE_SET`]
/// claim in canonical order.
pub fn current_rules_tx(connection: &Connection) -> Result<Vec<crate::rules::NamedRule>> {
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h6c81103a624c owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ &format!(
        "SELECT claims.subject, claims.id, claims.body FROM claims
         WHERE claims.kind=?1 ORDER BY {CANONICAL_ORDER}"
    ))?;
    let mut latest = BTreeMap::new();
    for row in statement.query_map([crate::rules::RULE_SET], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })? {
        let (subject, id, body) = row?;
        latest.insert(subject, (id, body));
    }
    Ok(latest
        .into_iter()
        .filter_map(|(subject, (claim_id, body))| {
            let body: Value = serde_json::from_str(&body).ok()?;
            Some(crate::rules::NamedRule {
                name: subject.strip_prefix("rule/")?.to_owned(),
                claim_id,
                rule: crate::rules::Rule::from_fields(body.get("fields")?)?,
            })
        })
        .collect())
}

/// Check one local write against the rules, inside the write's own transaction: refuse it with
/// `rule-denied` under a rule in enforce mode, and record each rule in audit mode that would have
/// refused it beside the write. Writes without an actor, and the audit records themselves, are
/// the node's own and pass; a person may always set a rule, so no rule can lock its owner out.
/// With no rules, one indexed probe.
pub fn rules_gate_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    actor: &str,
    kind: &str,
    subject: &str,
) -> Result<()> {
    if kind == crate::rules::RULE_SET {
        if Family::of(actor) == Some(Family::Person) {
            return Ok(());
        }
        return Err(anyhow::Error::new(St3Error::new(
            "rule-denied",
            format!("only a person sets rules, not {actor}"),
        )));
    }
    if kind == crate::rules::RULE_AUDITED {
        return Ok(());
    }
    let any: bool = transaction
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM claims WHERE kind=?1)")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h0b18afb3307a owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ [crate::rules::RULE_SET], |row| row.get(0))?;
    if !any {
        return Ok(());
    }
    let rules = current_rules_tx(transaction)?;
    let decision = crate::rules::decide(&rules, actor, kind, subject);
    if let Some(denied) = decision.denied {
        return Err(anyhow::Error::new(St3Error::new(
            "rule-denied",
            format!(
                "rule `{}` refuses {actor} writing {kind} on {subject}{}",
                denied.name,
                if denied.rule.description.is_empty() {
                    String::new()
                } else {
                    format!(": {}", denied.rule.description)
                }
            ),
        )));
    }
    for rule in decision.audited {
        append_claim_record_tx(
            transaction,
            origin,
            &format!("rule/{}", rule.name),
            crate::rules::RULE_AUDITED,
            None,
            &serde_json::json!({
                "fields": {
                    "rule": rule.claim_id,
                    "actor": actor,
                    "action": kind,
                    "target": subject,
                },
                "evidence": [],
            }),
            &[],
            None,
        )?;
    }
    Ok(())
}

impl Store {
    /// Set `rule/NAME` as `person` sets it.
    pub fn set_rule(
        &self,
        name: &str,
        rule: &crate::rules::Rule,
        person: &str,
    ) -> Result<ClaimRecord, St3Error> {
        if name.is_empty() || name.contains('/') {
            return Err(St3Error::new(
                "invalid-rule-name",
                "a rule name is one path segment",
            ));
        }
        self.append_claim(&ClaimInput {
            subject: format!("rule/{name}"),
            kind: crate::rules::RULE_SET.into(),
            actor: Some(person.into()),
            fields: rule.fields(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
    }

    /// The newest audit records, of one rule or of all, newest first.
    pub fn rule_audits(&self, rule: Option<&str>, limit: usize) -> Result<Vec<ClaimRecord>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h483f134075db owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ &format!(
            "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.kind=?1 AND (?2 IS NULL OR claims.subject=?2)
             ORDER BY claims.store_index DESC LIMIT ?3"
        ))?;
        let subject = rule.map(|name| format!("rule/{name}"));
        Ok(statement
            .query_map(
                params![crate::rules::RULE_AUDITED, subject, limit as i64],
                claim_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The rules, from memory; read again after any rule changes.
    pub fn current_rules(&self) -> Result<Arc<Vec<crate::rules::NamedRule>>> {
        if !self.rules_stale.load(Ordering::Acquire)
            && let Some(rules) = self
                .rules
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        {
            return Ok(rules);
        }
        self.rules_stale.store(false, Ordering::Release);
        let rules = Arc::new(current_rules_tx(&self.readers.get())?);
        *self.rules.write().unwrap_or_else(PoisonError::into_inner) = Some(rules.clone());
        Ok(rules)
    }
}

fn nonce_used(connection: &Connection, key: &str, nonce: &str) -> Result<bool> {
    nonce_used_in_model(connection, key, nonce, legacy::active(connection)?)
}
fn nonce_used_in_model(
    connection: &Connection,
    key: &str,
    nonce: &str,
    serving_legacy: bool,
) -> Result<bool> {
    let sql = if serving_legacy {
        "SELECT EXISTS(SELECT 1 FROM legacy_claim_signatures WHERE key=?1 AND nonce=?2)"
    } else {
        if let Some(reason) = migration::unavailable(connection)? {
            anyhow::bail!("{reason}");
        }
        "SELECT EXISTS(SELECT 1 FROM claim_signature_nonce_uses WHERE key=?1 AND nonce=?2)"
    };
    Ok(connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hf9d562de9ebc owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ sql)?
        .query_row(/* sqlite-large-value: allow-for-now lv-hc3493cbfe828 owner=smallclaims::store::principals TODO=trace-statement-and-value-representation */ params![key, nonce], |row| row.get(0))?)
}

pub(crate) fn serving_signature(
    connection: &Connection,
    claim_id: &str,
) -> Result<Option<ClaimSignature>> {
    if legacy::active(connection)? {
        return legacy::stored_signature(connection, claim_id);
    }
    if let Some(reason) = migration::unavailable(connection)? {
        anyhow::bail!("{reason}");
    }
    if let Some(reason) = judging::unavailable(connection, claim_id)? {
        anyhow::bail!("{reason}");
    }
    stored_signature(connection, claim_id)
}

pub(crate) fn envelope_signature(
    connection: &Connection,
    claim_id: &str,
) -> Result<Option<ClaimSignature>> {
    if legacy::active(connection)? {
        legacy::stored_signature(connection, claim_id)
    } else {
        stored_signature(connection, claim_id)
    }
}

#[cfg(test)]
mod signature_alert_tests {
    use super::*;
    use rusqlite::StatementStatus;

    #[test]
    fn signature_alerts_read_only_the_threshold_index_and_keep_claim_order() {
        let store = Store::open_memory("studio", Arc::new(runtime::Plain)).unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            // The common population must not be visited when finding two exceptional claims.
            for i in 0..10_000 {
                tx.execute("INSERT INTO claim_signature_counts(claim_id,candidates) VALUES (?1,63)",
                    [format!("ordinary/{i:05}")]).unwrap();
            }
            tx.execute("INSERT INTO claim_signature_counts(claim_id,candidates) VALUES ('a-alert',65),('z-alert',64)", []).unwrap();
            tx.commit().unwrap();
        }
        {
            let connection = store.readers.get();
            let mut statement = signature_alerts_query(&connection).unwrap();
            let alerts = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            assert_eq!(alerts, vec![("a-alert".into(),65),("z-alert".into(),64)]);
            assert_eq!(statement.get_status(StatementStatus::FullscanStep), 0);
            assert!(statement.get_status(StatementStatus::VmStep) < 100,
                "ordinary below-threshold rows must not contribute query work");
        }
        let faults = store.signature_state_faults().unwrap().into_iter()
            .filter(|fault| fault.code == "signature-candidate-count")
            .map(|fault| (fault.subject, fault.detail)).collect::<Vec<_>>();
        assert_eq!(faults.len(), 2);
        assert_eq!(faults[0].0, "claim/a-alert");
        assert!(faults[0].1.starts_with("65 distinct candidates retained;"));
        assert_eq!(faults[1].0, "claim/z-alert");
        assert!(faults[1].1.starts_with("64 distinct candidates retained;"));
    }
}

#[cfg(test)]
mod paced_verdict_tests {
    use super::*;

    /// A store whose roots changed under `count` verdicts that relied on them.
    fn root_linked(count: usize) -> Store {
        let store = Store::open_memory("studio", Arc::new(runtime::Plain)).unwrap();
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for i in 0..count {
            tx.execute(
                "INSERT INTO claim_verdict_links(link,claim_id) VALUES ('root',?1)",
                [format!("claim/{i:05}")],
            )
            .unwrap();
        }
        set_meta_tx(&tx, VERDICT_ROOTS, "roots-before-a-membership-change").unwrap();
        tx.commit().unwrap();
        drop(connection);
        store
    }

    fn count(store: &Store, sql: &str) -> u64 {
        store
            .readers
            .get()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    fn cursor(store: &Store, link: &str) -> Option<String> {
        fleet_meta(&store.readers.get(), &format!("{REQUEUE}{link}")).unwrap()
    }

    #[test]
    fn a_root_change_queues_its_dependents_a_page_per_loan() {
        let total = REQUEUE_PAGE * 2 + 100;
        let store = root_linked(total);
        store.advance_signature_judgment().unwrap();
        // One loan queued one page, not every root-linked verdict in one transaction.
        assert_eq!(cursor(&store, "root").as_deref(), Some("claim/01023"));
        assert!(count(&store, "SELECT COUNT(*) FROM claim_verdict_queue") <= REQUEUE_PAGE as u64);
        assert!(store.verdicts_pending().unwrap());
        let mut loans = 1;
        while store.verdicts_pending().unwrap() {
            store.advance_signature_judgment().unwrap();
            loans += 1;
            assert!(loans < 1_000, "the requeue and its queue drain");
        }
        assert!(loans >= total / JUDGE_CHUNK, "{loans}");
        assert_eq!(cursor(&store, "root"), None);
        assert_ne!(
            fleet_meta(&store.readers.get(), VERDICT_ROOTS)
                .unwrap()
                .as_deref(),
            Some("roots-before-a-membership-change")
        );
        assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_verdict_queue"), 0);
    }

    #[test]
    fn an_ordinary_pass_takes_a_slice_and_leaves_the_rest_due() {
        let store = root_linked(REQUEUE_PAGE * 2);
        store.judge_claims_slice(true).unwrap();
        assert!(store.verdicts_pending().unwrap());
        assert!(
            store.verdicts_due.load(Ordering::Acquire),
            "the next pass takes the next slice"
        );
        // A full pass, as tests and rechecks use, still drains everything.
        store.judge_claims(true).unwrap();
        assert!(!store.verdicts_pending().unwrap());
        assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_verdict_queue"), 0);
    }

    #[test]
    fn a_verdict_past_any_unfinished_requeue_cursor_reads_as_held() {
        for link in [
            "root",
            "key:person/ada|device-key",
            "delegation:claim/grant",
            "nonce:device-key|nonce-1",
        ] {
            let store = Store::open_memory("studio", Arc::new(runtime::Plain)).unwrap();
            {
                let mut connection = store.connection.write();
                let tx = connection.transaction().unwrap();
                for claim in ["claim/00001", "claim/00002", "claim/00003", "claim/00004"] {
                    tx.execute("INSERT INTO claim_verdicts(claim_id,verdict,signer) VALUES (?1,'verified','host/studio')", [claim]).unwrap();
                }
                for claim in ["claim/00001", "claim/00002", "claim/00003"] {
                    tx.execute(
                        "INSERT INTO claim_verdict_links(link,claim_id) VALUES (?1,?2)",
                        [link, claim],
                    )
                    .unwrap();
                }
                // claim/00004 relies on another link, whose requeue is not running.
                tx.execute("INSERT INTO claim_verdict_links(link,claim_id) VALUES ('delegation:claim/other','claim/00004')", []).unwrap();
                // The requeue of `link` has queued up to claim/00002.
                set_meta_tx(&tx, &format!("{REQUEUE}{link}"), "claim/00002").unwrap();
                tx.commit().unwrap();
            }
            let read = |claim: &str| store.claim_verdict(claim).unwrap();
            assert_eq!(read("claim/00001"), Verdict::Verified, "{link}");
            assert_eq!(read("claim/00002"), Verdict::Verified, "{link}");
            assert!(
                matches!(read("claim/00003"), Verdict::Held(reason) if reason.starts_with("signature-judgment-incomplete:") && reason.ends_with(link)),
                "{link}"
            );
            assert!(store.claim_signature("claim/00003").is_err(), "{link}");
            assert_eq!(read("claim/00004"), Verdict::Verified, "{link}");
            let counts = store.claim_verdict_counts().unwrap();
            assert_eq!(counts.get("verified"), Some(&3), "{link}");
            assert_eq!(counts.get("held"), Some(&1), "{link}");
            // A root change starts its requeue before any page: every dependent reads as held.
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            set_meta_tx(&tx, &format!("{REQUEUE}{link}"), "").unwrap();
            tx.commit().unwrap();
            drop(connection);
            for claim in ["claim/00001", "claim/00002", "claim/00003"] {
                assert!(matches!(read(claim), Verdict::Held(_)), "{link} {claim}");
            }
            assert_eq!(read("claim/00004"), Verdict::Verified, "{link}");
        }
    }

    #[test]
    fn a_changed_delegation_queues_its_dependents_a_page_now_and_the_rest_per_loan() {
        let store = Store::open_memory("studio", Arc::new(runtime::Plain)).unwrap();
        let total = REQUEUE_PAGE * 2 + 100;
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            for i in 0..total {
                tx.execute("INSERT INTO claim_verdict_links(link,claim_id) VALUES ('delegation:claim/grant',?1)", [format!("claim/{i:05}")]).unwrap();
            }
            queue_linked_tx(&tx, "delegation:claim/grant").unwrap();
            tx.commit().unwrap();
        }
        // The changed delegation queued one page in the judging transaction, not every dependent.
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM claim_verdict_queue"),
            REQUEUE_PAGE as u64
        );
        assert_eq!(
            cursor(&store, "delegation:claim/grant").as_deref(),
            Some("claim/01023")
        );
        assert!(store.verdicts_pending().unwrap());
        let mut loans = 0;
        while store.verdicts_pending().unwrap() {
            store.advance_signature_judgment().unwrap();
            loans += 1;
            assert!(loans < 1_000, "the requeue and its queue drain");
        }
        assert_eq!(cursor(&store, "delegation:claim/grant"), None);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_verdict_queue"), 0);
        // A small link is queued whole at once, as before, and leaves no cursor.
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute("INSERT INTO claim_verdict_links(link,claim_id) VALUES ('delegation:claim/small','claim/00001')", []).unwrap();
        queue_linked_tx(&tx, "delegation:claim/small").unwrap();
        assert!(
            fleet_meta(&tx, &format!("{REQUEUE}delegation:claim/small"))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            tx.query_row("SELECT COUNT(*) FROM claim_verdict_queue", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
}
