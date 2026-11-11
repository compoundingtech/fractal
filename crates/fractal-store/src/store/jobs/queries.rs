use super::*;
use rusqlite::OptionalExtension as _;

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
    pub record: JobRecord,
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

pub fn job_by_id(connection: &Connection, id: &str) -> Result<Option<JobRow>> {
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
    let mut statement = connection.prepare(
        "SELECT id,requester,idempotency_key,request_digest,binding,state,created_at,updated_at
         FROM jobs INDEXED BY jobs_queue
         WHERE execution_host=?1 AND queued=1 ORDER BY queue_order,id",
    )?;
    Ok(statement.query_map([host], job_row)?.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn history_by_job(connection: &Connection, id: &str) -> Result<Vec<HistoryRow>> {
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
