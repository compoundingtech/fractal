use super::*;
use rusqlite::OptionalExtension as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobsProjectionStale {
    pub subject: String,
}

impl std::fmt::Display for JobsProjectionStale {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "jobs-projection-stale: {}", self.subject)
    }
}

impl std::error::Error for JobsProjectionStale {}

// Until 4c-2 adds the runtime repair dispatch, repair-page commits may leave
// pending subjects. Only existing flush/open/rebuild hooks drain them; these
// read-only queries report typed staleness and never acquire a writer to drain.
// Reads never take the writer lock to repair projections. The enclosing read
// transaction makes this fence and the following row query one consistent cut.
pub(super) fn ensure_current(connection: &Transaction<'_>, subject: &str) -> Result<()> {
    let pending: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM job_pending WHERE subject=?1)",
        [subject], |row| row.get(0),
    )?;
    if pending {
        return Err(JobsProjectionStale { subject: subject.to_owned() }.into());
    }
    Ok(())
}

/// History only: no conversion to executable jobs or host-authority evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRow {
    pub id: String,
    pub requester: String,
    pub idempotency_key: String,
    pub request_digest: fractal_jobs::identity::RequestDigest,
    pub binding: JobHostBinding,
    pub state: JobState,
    pub created_at: u128,
    pub updated_at: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryRow {
    pub claim_id: String,
    /// Raw record JSON is preserved even when decoding was refused.
    pub record: serde_json::Value,
    pub state: Option<JobState>,
    pub refusal: Option<String>,
    pub at: u128,
}

fn decode<T: serde::de::DeserializeOwned>(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: row.get(0)?,
        requester: row.get(1)?,
        idempotency_key: row.get(2)?,
        request_digest: decode(row, 3)?,
        binding: decode(row, 4)?,
        state: decode(row, 5)?,
        created_at: decode(row, 6)?,
        updated_at: decode(row, 7)?,
    })
}

pub fn job_by_id(connection: &Transaction<'_>, id: &str) -> Result<Option<JobRow>> {
    ensure_current(connection, id)?;
    Ok(connection.query_row(
        "SELECT id,requester,idempotency_key,request_digest,binding,state,created_at,updated_at
         FROM jobs WHERE id=?1",
        [id],
        job_row,
    ).optional()?)
}

/// The index carries the shared canonical key, not receiver arrival order.
/// An explicit transaction ensures all rows belong to one consistent snapshot.
pub fn queue_for_host(connection: &Transaction<'_>, host: &str) -> Result<Vec<JobRow>> {
    let pending: Option<String> = connection.query_row(
        "SELECT pending.subject FROM job_pending pending
         LEFT JOIN jobs ON jobs.id=pending.subject
         WHERE jobs.execution_host=?1 OR jobs.id IS NULL OR EXISTS(
             SELECT 1 FROM claims INDEXED BY jobs_claim_subject
             WHERE kind='job.recorded' AND subject=pending.subject
             AND CASE WHEN json_valid(body) THEN COALESCE(
                 json_extract(body,'$.fields.record.binding.execution_host'),
                 json_extract(body,'$.record.binding.execution_host')) END=?1)
         ORDER BY pending.subject LIMIT 1",
        [host], |row| row.get(0),
    ).optional()?;
    if let Some(subject) = pending {
        return Err(JobsProjectionStale { subject }.into());
    }
    let mut statement = connection.prepare(
        "SELECT id,requester,idempotency_key,request_digest,binding,state,created_at,updated_at
         FROM jobs INDEXED BY jobs_queue
         WHERE execution_host=?1 AND queued=1 ORDER BY queue_order,id",
    )?;
    Ok(statement.query_map([host], job_row)?.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Requester history uses canonical submission order, not receiver arrival time.
pub fn jobs_for_requester(connection: &Transaction<'_>, requester: &str) -> Result<Vec<JobRow>> {
    // A pending submission can introduce a requester absent from the current rows.
    let pending: Option<String> = connection.query_row(
        "SELECT subject FROM job_pending ORDER BY subject LIMIT 1", [],
        |row| row.get(0),
    ).optional()?;
    if let Some(subject) = pending {
        return Err(JobsProjectionStale { subject }.into());
    }
    let mut statement = connection.prepare(
        "SELECT id,requester,idempotency_key,request_digest,binding,state,created_at,updated_at
         FROM jobs INDEXED BY jobs_requester WHERE requester=?1 ORDER BY queue_order,id",
    )?;
    Ok(statement.query_map([requester], job_row)?.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn history_by_job(connection: &Transaction<'_>, id: &str) -> Result<Vec<HistoryRow>> {
    ensure_current(connection, id)?;
    let mut statement = connection.prepare(
        "SELECT claim_id,event,state,refusal,at FROM job_history WHERE job_id=?1 ORDER BY ordinal",
    )?;
    let rows = statement.query_map([id], |row| {
        let state: Option<String> = row.get(2)?;
        let state = state.map(|state| serde_json::from_str(&state)).transpose().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
        })?;
        Ok(HistoryRow {
            claim_id: row.get(0)?,
            record: decode(row, 1)?,
            state,
            refusal: row.get(3)?,
            at: decode(row, 4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
