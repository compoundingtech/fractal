//! Invocation-local validation of the existing message-cache keys. No key survives this call.
use super::*;

pub const CHUNK: usize = 128;

#[derive(Debug, PartialEq)]
pub struct Version {
    pub latest_claim_index: u64,
    pub desired_claim_id: Option<String>,
    /// Which kinds of local mark the message has, as a bitmask (message_marks.rs): a mark changes
    /// the view as a claim does, and unlike a rowid the mask cannot repeat a trimmed mark's key.
    pub latest_mark: i64,
}

// Each supplied subject drives the same two existing indexed lookups as the point reader.
// Keep duplicate subjects and their positions; no graph scan or membership filtering.
const QUERY: &str = "SELECT requested.key,
    (SELECT COALESCE(MAX(store_index), 0) FROM claims WHERE subject=requested.value),
    (SELECT claim_id FROM desired WHERE subject=requested.value),
    (SELECT COALESCE(SUM(CASE kind WHEN 'message.delivered' THEN 1 WHEN 'message.read' THEN 2
        WHEN 'message.closed' THEN 4 ELSE 0 END), 0) FROM local_message_marks WHERE message=requested.value)
    FROM json_each(?1) AS requested ORDER BY requested.key";

pub fn read(connection: &Connection, subjects: &[&str]) -> Result<Vec<Version>> {
    anyhow::ensure!(
        subjects.len() <= CHUNK,
        "message version batch exceeds 128 subjects"
    );
    if subjects.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h773c483c8e52 owner=fractal_store::store::message_versions TODO=trace-statement-and-value-representation */ QUERY)?;
    let mut rows = statement.query([serde_json::to_string(subjects)?])?;
    let mut versions = Vec::with_capacity(subjects.len());
    for ordinal in 0..subjects.len() {
        let row = rows.next()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        anyhow::ensure!(
            row.get::<_, usize>(0)? == ordinal,
            "message version batch changed input order"
        );
        versions.push(Version {
            latest_claim_index: row.get(1)?,
            desired_claim_id: row.get(2)?,
            latest_mark: row.get(3)?,
        });
    }
    anyhow::ensure!(
        rows.next()?.is_none(),
        "message version batch returned extra rows"
    );
    Ok(versions)
}

#[cfg(test)]
mod tests;
