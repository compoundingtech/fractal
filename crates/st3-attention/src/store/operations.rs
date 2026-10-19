//! The part of `store::operations` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::AttentionStore;
use super::*;
pub use st3_store::store::operations::*;

fn recently_ended_mission_valid_until(
    connection: &Connection,
    ids: &str,
    now: u128,
) -> Result<Option<u128>> {
    let mut statement = connection.prepare(
        /* sqlite-large-value: allow-for-now lv-h552ebda91731 owner=st3_attention::store::operations TODO=trace-statement-and-value-representation */ "WITH latest AS (
            SELECT mission_id,status,updated_at_unix_ms,ROW_NUMBER() OVER (
                PARTITION BY mission_id ORDER BY created_at_unix_ms DESC,id DESC
            ) AS position FROM mission_runs
            WHERE mission_id IN (SELECT CASE WHEN value LIKE 'mission/%'
                THEN substr(value,9) ELSE value END FROM json_each(?1))
         )
         SELECT latest.updated_at_unix_ms FROM latest
         LEFT JOIN mission_definitions def ON def.mission_id=latest.mission_id
         WHERE latest.position=1 AND latest.status IN ('failed','cancelled')
           AND COALESCE(def.state,'')<>'retired'
           AND NOT EXISTS (SELECT 1 FROM mission_runs active
               WHERE active.mission_id=latest.mission_id AND active.status IN ('running','standing'))",
    )?;
    let mut until = None;
    for updated in statement.query_map([ids], |row| row.get::<_, String>(0))? {
        // Membership includes equality with the 24-hour cutoff. It disappears one
        // millisecond later; an overflowing boundary cannot occur in a u128 cut.
        if let Some(boundary) = updated?.parse::<u128>()?.checked_add(RECENTLY_ENDED_MS)
            .and_then(|last_included| last_included.checked_add(1))
            && boundary > now
        {
            until = Some(until.map_or(boundary, |held: u128| held.min(boundary)));
        }
    }
    Ok(until)
}

pub trait OperationsStore {
    /// Exclusive clock fence for collection cards at `now`, with the graph held unchanged.
    ///
    /// Cards observe effective leases, future person asks and the recently-ended mission
    /// membership cutoff. They do not enrich steps with wakes or execution timing:
    /// `not_before`, mission deadlines and step timeout budgets only change these cards
    /// after a graph write. Selected IDs include the page's has_more lookahead; only the
    /// newest three runs and first twenty steps are rendered. IDs may have a `mission/` prefix.
    fn mission_collection_valid_until(
            &self,
            ids: &[String],
            now: u128,
        ) -> Result<Option<u128>>;
}

impl OperationsStore for Store {

    fn mission_collection_valid_until(
            &self,
            ids: &[String],
            now: u128,
        ) -> Result<Option<u128>> {
        if ids.is_empty() {
            return Ok(None);
        }
        let ids = serde_json::to_string(ids)?;
        let connection = self.readers.get();
        let newest = "WITH newest AS (
            SELECT id, ROW_NUMBER() OVER (
                PARTITION BY mission_id ORDER BY created_at_unix_ms DESC,id DESC
            ) AS position FROM mission_runs
            WHERE mission_id IN (SELECT CASE WHEN value LIKE 'mission/%'
                THEN substr(value,9) ELSE value END FROM json_each(?1))
        )";
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hb5cdccd97167 owner=st3_attention::store::operations TODO=trace-statement-and-value-representation */ &format!(
            "{newest}, preview AS (
                SELECT s.*,ROW_NUMBER() OVER (
                    PARTITION BY s.run_id ORDER BY s.created_at_unix_ms,s.step_path
                ) AS step_position
                FROM step_runs s JOIN newest n ON n.id=s.run_id AND n.position<=3
                JOIN mission_runs r ON r.id=s.run_id AND r.current_generation_id=s.generation_id
            )
            SELECT subject,run_id,step_path,definition_hash,status,attempt,assignee,available_to,
                   agentless,title,goals,worker_reported,lease_owner,lease_incarnation,
                   lease_expires_at_unix_ms,blocked_reason,not_before_unix_ms,created_at_unix_ms,
                   updated_at_unix_ms,readiness_epoch,constraints
            FROM preview WHERE step_position<=20 AND (
                (lease_expires_at_unix_ms IS NOT NULL
                 AND status IN ('claimed','working','verifying','blocked'))
                OR (assignee LIKE 'person/%' AND status IN ('ready','pending'))
                OR status='waiting-person' OR (status='ready' AND blocked_reason IS NOT NULL)
            )"
        ))?;
        let steps = statement
            .query_map([&ids], step_run_from_row)?
            .map(|row| row.map(|step| (step.subject.clone(), step)))
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
        let changed = |step: &StepRunView, at: u128| -> Result<bool> {
            let mut before = step.clone();
            let mut after = step.clone();
            apply_effective_step_state(&connection, &mut before, now)?;
            apply_effective_step_state(&connection, &mut after, at)?;
            Ok(before.status != after.status
                || before.claimant != after.claimant
                || before.blocked_reason != after.blocked_reason
                || before.blockers != after.blockers)
        };
        let mut until = recently_ended_mission_valid_until(&connection, &ids, now)?;
        for step in steps.values() {
            if let Some(expiry) = step.claim_expires_at_unix_ms.filter(|expiry| *expiry > now)
                && until.is_none_or(|earliest| expiry < earliest)
                && changed(step, expiry)?
            {
                until = Some(expiry);
            }
        }

        // Decimal-text comparison preserves the full u128 clock range, unlike SQLite's
        // signed INTEGER cast. Only person_work::current consults acceptance time here.
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hc146a40666a3 owner=st3_attention::store::operations TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
            "{newest}
             SELECT claims.id,claims.store_index,claims.batch_id,claims.subject,claims.kind,
                    claims.origin,claims.actor,claims.body,claims.predecessors,claims.accepted_at_unix_ms
             FROM claims JOIN step_runs person ON person.subject=claims.subject
             JOIN newest n ON n.id=person.run_id AND n.position<=3
             WHERE claims.kind='work.person-asked'
               AND (length(claims.accepted_at_unix_ms)>length(?2)
                    OR (length(claims.accepted_at_unix_ms)=length(?2) AND claims.accepted_at_unix_ms>?2))
             ORDER BY CANONICAL_ASC(claims)"
        )))?;
        let asks = statement
            .query_map(params![ids, now.to_string()], claim_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if asks.is_empty() {
            return Ok(until);
        }
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hf0280e2bb481 owner=st3_attention::store::operations TODO=trace-statement-and-value-representation */ &format!(
            "{newest} SELECT id FROM newest WHERE position<=3"
        ))?;
        let runs = statement
            .query_map([&ids], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        let attention = self.human_attention_runs_at(now)?;
        for ask in asks {
            let at = ask.accepted_at_unix_ms;
            if until.is_some_and(|earliest| at >= earliest)
                || !person_work::current(&connection, &ask, at)?
            {
                continue;
            }
            // A future ask can change a preview's effective status or blocker list even
            // when another person item already keeps that run in human attention.
            let own = steps.get(&ask.subject);
            let origin = ask.body["fields"]["origin_step"]
                .as_str()
                .and_then(|subject| steps.get(subject));
            let preview_changes = own.map(|step| changed(step, at)).transpose()?.unwrap_or(false)
                || origin.map(|step| changed(step, at)).transpose()?.unwrap_or(false);
            let person = person_work::step(&connection, &ask.subject)?;
            let attention_changes = if let Some(person) = person {
                runs.contains(person.run.trim_start_matches("mission-run/"))
                    && person.assigned_to.as_deref().is_some_and(|id| id.starts_with("person/"))
                    && !attention.contains(&person.run)
                    && person_work::request(&connection, &ask.subject)?
                        .is_some_and(|first| first.id == ask.id)
            } else {
                false
            };
            if preview_changes || attention_changes {
                until = Some(at);
            }
        }
        Ok(until)
    }
}

