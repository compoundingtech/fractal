//! Local, read-only report of native person asks by mission × person over a rolling 7 days.
//! It measures the attention budget window (docs/st3/priority.md); it is a report, not admission or enforcement, and is not fleet-complete.
use super::*;

const WEEK_MS: u128 = 7 * 24 * 60 * 60 * 1000;
const MAX_SUBJECTS: usize = 10_000;
const MAX_CANONICAL_CLAIMS: usize = 128;

mod report;
pub use report::{AttentionBudgetReport, AttentionDeclaration, AttentionRequestReference};

// The existing partial recipient index seeks only this person's native ask rows.
// Its implicit rowid suffix supports newest local admission first without a sort.
// Bound lifetime candidates before applying the rolling window; never truncate.
const SUBJECTS: &str = "SELECT subject, accepted_at_unix_ms
 FROM claims INDEXED BY claims_person_ask_recipient_index
 WHERE kind='work.person-asked' AND json_extract(body, '$.fields.person')=?1
 ORDER BY store_index DESC LIMIT ?2";

// A schedule wrapper can be the root, so walk the actual parent-step links rather than
// testing only root_run_id. Refuse to cross a missing step or a different run tree.
const ANCESTRY: &str = "WITH RECURSIVE ancestry(id, mission_id, root_run_id, parent_step_run, depth) AS (
 SELECT r.id, r.mission_id, r.root_run_id, r.parent_step_run, 0
 FROM step_runs s JOIN mission_runs r ON r.id=s.run_id
 WHERE s.subject=?1 AND r.id=?2
 UNION ALL
 SELECT r.id, r.mission_id, r.root_run_id, r.parent_step_run, a.depth+1
 FROM ancestry a JOIN step_runs s ON s.subject=a.parent_step_run
 JOIN mission_runs r ON r.id=s.run_id AND r.root_run_id=a.root_run_id
 WHERE a.depth<63
)
SELECT EXISTS(SELECT 1 FROM ancestry WHERE mission_id=?3),
       EXISTS(SELECT 1 FROM ancestry WHERE id=root_run_id AND parent_step_run IS NULL),
       EXISTS(SELECT 1 FROM ancestry a JOIN step_runs s ON s.subject=a.parent_step_run
              JOIN mission_runs r ON r.id=s.run_id AND r.root_run_id=a.root_run_id
              WHERE a.depth=63)";

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
    /// Read a rolling week at one SQLite snapshot, reusing a caller's pinned snapshot.
    /// A replica may lack claims or lineage; the result does not certify fleet completeness.
    pub fn attention_request_count(
        &self,
        mission: &str,
        person: &str,
        at: u128,
    ) -> Result<AttentionRequestCount> {
        self.read_snapshot_unindexed(|| count(&self.readers.get(), mission, person, at))
    }
}

fn count(
    connection: &Connection,
    mission: &str,
    person: &str,
    at: u128,
) -> Result<AttentionRequestCount> {
    count_with_refs(connection, mission, person, at, |_, _, _| {})
}

fn count_with_refs(
    connection: &Connection,
    mission: &str,
    person: &str,
    at: u128,
    mut reference: impl FnMut(&ClaimRecord, &str, &str),
) -> Result<AttentionRequestCount> {
    let start = at.saturating_sub(WEEK_MS);
    // MAX_SUBJECTS is a retained-lifetime recipient claim bound, not a weekly quota.
    let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h5e9f5342baeb owner=fractal_store::store::attention_budget TODO=trace-statement-and-value-representation */ SUBJECTS)?;
    let mut subjects = BTreeSet::new();
    for (index, candidate) in query
        .query_map(params![person, MAX_SUBJECTS + 1], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .enumerate()
    {
        anyhow::ensure!(
            index < MAX_SUBJECTS,
            "recipient lifetime request-claim limit of {MAX_SUBJECTS} exceeded"
        );
        let (subject, accepted) = candidate?;
        let accepted: u128 = accepted.parse()?;
        if accepted > start && accepted <= at {
            subjects.insert(subject);
        }
    }
    let mut ancestry = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h003a82076c71 owner=fractal_store::store::attention_budget TODO=trace-statement-and-value-representation */ ANCESTRY)?;
    let mut report = AttentionRequestCount {
        window_start_exclusive_unix_ms: start,
        window_end_inclusive_unix_ms: at,
        requests: 0,
        unattributed_for_person: 0,
        requests_by_originating_run: BTreeMap::new(),
    };
    for subject in subjects {
        let canonical_claims: usize = connection.query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
             WHERE subject=?1 AND kind='work.person-asked' LIMIT ?2)",
            params![subject, MAX_CANONICAL_CLAIMS + 1],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            canonical_claims <= MAX_CANONICAL_CLAIMS,
            "native request canonical-history limit of {MAX_CANONICAL_CLAIMS} exceeded"
        );
        let Some(ask) = person_work::request(connection, &subject)? else {
            continue;
        };
        let fields = &ask.body["fields"];
        // Native updates share the claim kind. Legacy conversions are historical
        // attention migrations rather than new requests in this report.
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
        let (matches, complete, overflow): (bool, bool, bool) = ancestry
            .query_row(/* sqlite-large-value: allow-for-now lv-h97369a95acd3 owner=fractal_store::store::attention_budget TODO=trace-statement-and-value-representation */ params![step, run, mission], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        anyhow::ensure!(!overflow, "attention ancestry limit of 64 vertices exceeded");
        if matches {
            report.requests += 1;
            reference(&ask, run, step);
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
