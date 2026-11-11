//! Inactive history projection. Neither rows nor replicated facts authorize execution.
//! Admission evidence remains sealed until the separately approved admission adapter lands.
use anyhow::{Context as _, Result, ensure};
use fractal_jobs::{Event, State as JobState, host_authority::JobHostBinding, transition};
use fractal_jobs::identity::{ExecutionRequest, IdempotencyKey};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};
use smallclaims::claim::ClaimRecord;

pub const KIND: &str = "job.recorded";
// Same subject-local repair ceiling as the operation-conflict contract (#6330).
const REFOLD_LIMIT: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum JobRecord {
    Submitted { binding: JobHostBinding, identity: IdempotencyKey, request: ExecutionRequest },
    Transition { binding: JobHostBinding, event: Event },
}

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS jobs (
        id TEXT PRIMARY KEY, requester TEXT NOT NULL, idempotency_key TEXT NOT NULL,
        request_digest TEXT NOT NULL, binding TEXT NOT NULL, execution_host TEXT NOT NULL,
        state TEXT NOT NULL, queued INTEGER NOT NULL, created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL, queue_order BLOB NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS jobs_queue ON jobs(execution_host, queued, queue_order, id);
    CREATE TABLE IF NOT EXISTS job_history (
        job_id TEXT NOT NULL, ordinal INTEGER NOT NULL, claim_id TEXT NOT NULL,
        event TEXT NOT NULL, state TEXT, refusal TEXT, at TEXT NOT NULL,
        PRIMARY KEY(job_id, ordinal)
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS job_receipts (
        requester TEXT NOT NULL, key TEXT NOT NULL, digest TEXT NOT NULL, job_id TEXT NOT NULL,
        PRIMARY KEY(requester, key)
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS job_pending (subject TEXT PRIMARY KEY) WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS jobs_claim_subject ON claims(subject) WHERE kind='job.recorded';
    CREATE TRIGGER IF NOT EXISTS jobs_pending_insert AFTER INSERT ON claims
    WHEN NEW.kind='job.recorded' BEGIN
        INSERT OR IGNORE INTO job_pending VALUES(NEW.subject);
    END;
    CREATE TRIGGER IF NOT EXISTS jobs_pending_repair_update AFTER UPDATE OF state,claim_id ON replica_records
    WHEN (OLD.state='repaired' OR NEW.state='repaired')
        AND (OLD.state IS NOT NEW.state OR OLD.claim_id IS NOT NEW.claim_id) BEGIN
        INSERT OR IGNORE INTO job_pending
        SELECT subject FROM claims WHERE kind='job.recorded' AND id IN (OLD.claim_id,NEW.claim_id);
    END;
    CREATE TRIGGER IF NOT EXISTS jobs_pending_repair_insert AFTER INSERT ON replica_records
    WHEN NEW.state='repaired' BEGIN
        INSERT OR IGNORE INTO job_pending
        SELECT subject FROM claims WHERE kind='job.recorded' AND id=NEW.claim_id;
    END;
    CREATE TRIGGER IF NOT EXISTS jobs_pending_repair_delete AFTER DELETE ON replica_records
    WHEN OLD.state='repaired' BEGIN
        INSERT OR IGNORE INTO job_pending
        SELECT subject FROM claims WHERE kind='job.recorded' AND id=OLD.claim_id;
    END;")?;
    Ok(())
}

pub fn open(tx: &Transaction<'_>) -> Result<()> { create_schema(tx) }

pub fn project(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    if claim.kind != KIND { return Ok(()); }
    refold(tx, &claim.subject)?;
    tx.execute("DELETE FROM job_pending WHERE subject=?1", [&claim.subject])?;
    Ok(())
}

/// Only subjects inserted by jobs claims are visited. No claims cursor exists here.
pub fn flush(tx: &Transaction<'_>) -> Result<()> {
    let mut statement = tx.prepare("SELECT subject FROM job_pending ORDER BY subject")?;
    let subjects = statement.query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for subject in subjects { refold(tx, &subject)?; }
    tx.execute("DELETE FROM job_pending", [])?;
    Ok(())
}

/// Partial index enumeration costs O(job records), including on a non-jobs log.
/// An empty job log performs no writes, including no projection clearing.
pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    let mut statement = tx.prepare("SELECT DISTINCT subject FROM claims INDEXED BY jobs_claim_subject WHERE kind='job.recorded'")?;
    let subjects = statement.query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for subject in subjects { refold(tx, &subject)?; }
    Ok(())
}

fn refold(tx: &Transaction<'_>, subject: &str) -> Result<()> {
    let sql = format!("SELECT id,body,accepted_at_unix_ms FROM claims INDEXED BY jobs_claim_subject
        WHERE kind='job.recorded' AND subject=?1
        AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
        ORDER BY {} LIMIT ?2", smallclaims::store::canonical::CANONICAL_ORDER);
    let mut statement = tx.prepare(&sql)?;
    let records = statement.query_map(params![subject, REFOLD_LIMIT + 1], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    // Refuse atomically before touching existing heads, never request whole-log replay.
    ensure!(records.len() <= REFOLD_LIMIT, "job-refold-limit: affected subject exceeds bounded refold");
    tx.execute("DELETE FROM jobs WHERE id=?1", [subject])?;
    tx.execute("DELETE FROM job_history WHERE job_id=?1", [subject])?;
    let mut current: Option<(JobHostBinding, JobState)> = None;
    for (ordinal, (claim, body, at)) in records.into_iter().enumerate() {
        let body: serde_json::Value = serde_json::from_str(&body)
            .unwrap_or_else(|_| serde_json::Value::String(body));
        let fields = body.get("fields").unwrap_or(&body);
        let raw = fields.get("record").unwrap_or(fields);
        let result = serde_json::from_value::<JobRecord>(raw.clone())
            .map_err(|error| anyhow::anyhow!("job-record-decode: {error}"))
            .and_then(|record| apply(tx, subject, &claim, &at, &record, &mut current));
        // Invalid edges/bindings are retained as refused history, never execution authority.
        let refusal = match result {
            Ok(()) => None,
            Err(error) if error.downcast_ref::<rusqlite::Error>().is_some() => return Err(error),
            Err(error) => Some(error.to_string()),
        };
        let state = current.as_ref().map(|(_, state)| serde_json::to_string(state)).transpose()?;
        tx.execute("INSERT INTO job_history VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![subject, ordinal, claim, serde_json::to_string(raw)?, state, refusal, at])?;
    }
    Ok(())
}

fn apply(
    tx: &Transaction<'_>, subject: &str, claim: &str, at: &str, record: &JobRecord,
    current: &mut Option<(JobHostBinding, JobState)>,
) -> Result<()> {
    match record {
        JobRecord::Submitted { binding, identity, request } => {
            ensure!(current.is_none(), "job-already-submitted");
            ensure!(binding.job_id == subject, "job-binding-subject-mismatch");
            ensure!(request.digest() == identity.request_digest, "job-request-digest-mismatch");
            request.resources.validate()?;
            request.command.validate()?;
            let digest = serde_json::to_string(&identity.request_digest)?;
            let queue_order = smallclaims::store::canonical::sortable_key(
                &smallclaims::store::canonical::claim_key(tx, claim)?,
            );
            tx.execute("INSERT INTO jobs VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,?8,?9)",
                params![subject, identity.requester, identity.key, digest,
                    serde_json::to_string(binding)?, binding.execution_host,
                    serde_json::to_string(&JobState::Queued)?, at, queue_order])?;
            *current = Some((binding.clone(), JobState::Queued));
        }
        JobRecord::Transition { binding, event } => {
            let (existing, state) = current.as_mut().context("job-not-submitted")?;
            ensure!(existing == binding, "job-binding-conflict");
            let next = transition(*state, *event)?;
            tx.execute("UPDATE jobs SET state=?2,queued=?3,updated_at=?4 WHERE id=?1",
                params![subject, serde_json::to_string(&next)?, matches!(next, JobState::Queued), at])?;
            *state = next;
        }
    }
    Ok(())
}


#[cfg(test)]
mod tests;
