//! Read-only accounting of native person-step requests in retained local claims.
use super::*;

const WEEK_MS: u128 = 7 * 24 * 60 * 60 * 1000;
const MAX_SUBJECTS: usize = 10_000;

// Match the indexed decimal timestamp representation, including digit boundaries.
const SUBJECTS: &str = "SELECT DISTINCT subject FROM claims INDEXED BY claims_accepted_order_index
 WHERE kind='work.person-asked'
   AND length(accepted_at_unix_ms)=?1 AND accepted_at_unix_ms>?2 AND accepted_at_unix_ms<=?3
   AND json_extract(body, '$.fields.person')=?4
 LIMIT ?5";

// A schedule wrapper can be the root, so walk the actual parent-step links rather than
// testing only root_run_id. Refuse to cross a missing step or a different run tree.
const ANCESTRY: &str = "WITH RECURSIVE ancestry(id, mission_id, root_run_id, parent_step_run) AS (
 SELECT r.id, r.mission_id, r.root_run_id, r.parent_step_run
 FROM step_runs s JOIN mission_runs r ON r.id=s.run_id
 WHERE s.subject=?1 AND r.id=?2
 UNION
 SELECT r.id, r.mission_id, r.root_run_id, r.parent_step_run
 FROM ancestry a JOIN step_runs s ON s.subject=a.parent_step_run
 JOIN mission_runs r ON r.id=s.run_id AND r.root_run_id=a.root_run_id
)
SELECT EXISTS(SELECT 1 FROM ancestry WHERE mission_id=?3),
       EXISTS(SELECT 1 FROM ancestry WHERE id=root_run_id AND parent_step_run IS NULL)";

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct AttentionRequestCount {
    pub window_start_exclusive_unix_ms: u128,
    pub window_end_inclusive_unix_ms: u128,
    pub requests: usize,
    /// These recipient requests cannot be assigned to any mission with retained provenance.
    pub unattributed_for_person: usize,
    pub requests_by_originating_run: BTreeMap<String, usize>,
}

impl Store {
    /// Read a rolling week at one SQLite snapshot. A replica may lack claims or lineage
    /// already known on another member. The result does not certify fleet completeness.
    pub fn attention_request_count(
        &self,
        mission: &str,
        person: &str,
        at: u128,
    ) -> Result<AttentionRequestCount> {
        let connection = self.readers.get();
        let snapshot = connection.unchecked_transaction()?;
        let result = count(&snapshot, mission, person, at)?;
        snapshot.commit()?;
        Ok(result)
    }
}

fn count(
    connection: &Connection,
    mission: &str,
    person: &str,
    at: u128,
) -> Result<AttentionRequestCount> {
    let start = at.saturating_sub(WEEK_MS);
    let from = start.to_string();
    let to = at.to_string();
    let mut query = connection.prepare_cached(SUBJECTS)?;
    let mut subjects = BTreeSet::new();
    // The explicit width equality and scalar bounds seek the accepted-time index.
    for width in from.len()..=to.len() {
        let lower = if width == from.len() {
            from.clone()
        } else {
            "0".repeat(width)
        };
        let upper = if width == to.len() {
            to.clone()
        } else {
            "9".repeat(width)
        };
        for subject in query.query_map(
            params![width, lower, upper, person, MAX_SUBJECTS + 1],
            |row| row.get::<_, String>(0),
        )? {
            subjects.insert(subject?);
            anyhow::ensure!(
                subjects.len() <= MAX_SUBJECTS,
                "too many attention requests to count in one rolling week"
            );
        }
    }
    let mut ancestry = connection.prepare_cached(ANCESTRY)?;
    let mut report = AttentionRequestCount {
        window_start_exclusive_unix_ms: start,
        window_end_inclusive_unix_ms: at,
        requests: 0,
        unattributed_for_person: 0,
        requests_by_originating_run: BTreeMap::new(),
    };
    for subject in subjects {
        let Some(ask) = person_work::request(connection, &subject)? else {
            continue;
        };
        let fields = &ask.body["fields"];
        // Native updates share the claim kind. Legacy conversions are historical
        // attention migrations rather than new requests under this budget.
        if ask.accepted_at_unix_ms <= start
            || ask.accepted_at_unix_ms > at
            || fields["person"] != person
            || fields["request"]["type"] == "update"
            || !fields["legacy_request"].is_null()
        {
            continue;
        }
        let (Some(step), Some(run)) = (
            fields["origin_step"].as_str(),
            fields["run"]
                .as_str()
                .and_then(|value| value.strip_prefix("mission-run/")),
        ) else {
            report.unattributed_for_person += 1;
            continue;
        };
        let (matches, complete): (bool, bool) = ancestry
            .query_row(params![step, run, mission], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        if matches {
            report.requests += 1;
            *report
                .requests_by_originating_run
                .entry(format!("mission-run/{run}"))
                .or_default() += 1;
        } else if !complete {
            report.unattributed_for_person += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
