//! The part of `store::mission_list` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_delivery::store::mission_list::*;
pub use st3_delivery::store::mission_list::{MissionKey, MissionRows, RosterFleet};

/// What claims between two cuts changed in the missions list.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MissionChanges {
    /// Missions whose key or card any of those claims can change.
    pub missions: BTreeSet<String>,
    /// Whether one of them can change which runs wait on a person.
    pub attention: bool,
}

/// Claim kinds that move nothing the attention runs read: work activity and observations.
/// Any other kind, even an unknown one, rereads them, which costs one bounded read.
const ATTENTION_NEUTRAL_KINDS: &[&str] = &[
    "work.renewed",
    "work.progress",
    "work.extended",
    "work.claimed",
    "harness.observed",
    "harness.activity",
    "harness.usage",
    "harness.diagnostic",
    "runtime.observed",
    "transport.observed",
    "workspace.observed",
    "daemon.diagnostic",
    "message.sent",
    "message.staged",
    "message.delivered",
    "message.read",
    "message.closed",
];

/// Whether the SQL pattern `'__st3/%'` matches `id`: two characters of any kind, then `st3/`
/// in any ASCII case. The list hides these system missions.
fn system_mission(id: &str) -> bool {
    let mut characters = id.chars();
    characters.next().is_some()
        && characters.next().is_some()
        && characters.as_str().len() >= 4
        && characters.as_str().is_char_boundary(4)
        && characters.as_str()[..4].eq_ignore_ascii_case("st3/")
}

/// Each mission's key columns. With `?1` a JSON array of mission IDs, only those missions.
pub fn mission_keys_sql(selected: bool) -> String {
    let filter = if selected {
        "WHERE mission_id IN (SELECT value FROM json_each(?1))"
    } else {
        ""
    };
    format!(
        "WITH ids AS (
            SELECT mission_id FROM mission_definitions {filter}
            UNION SELECT mission_id FROM mission_runs {filter}
         ), run_states AS (
            SELECT mission_id, SUM(status='running') AS running, SUM(status='standing') AS standing
            FROM mission_runs {filter} GROUP BY mission_id
         ), latest AS (
            SELECT mission_id, status, updated_at_unix_ms,
                   ROW_NUMBER() OVER (PARTITION BY mission_id ORDER BY created_at_unix_ms DESC, id DESC) AS rank
            FROM mission_runs {filter}
         )
         SELECT ids.mission_id,
                COALESCE(latest.updated_at_unix_ms,published.accepted_at_unix_ms,'0'),
                CAST(COALESCE(latest.updated_at_unix_ms,published.accepted_at_unix_ms) AS INTEGER),
                CASE WHEN COALESCE(run_states.running,0)>0 THEN 'running'
                     WHEN COALESCE(run_states.standing,0)>0 THEN 'standing'
                     WHEN def.state='retired' THEN 'retired'
                     WHEN latest.status IS NOT NULL THEN latest.status
                     ELSE def.state END,
                latest.status, def.state,
                COALESCE(run_states.running,0)=0 AND COALESCE(run_states.standing,0)=0,
                CAST(latest.updated_at_unix_ms AS INTEGER)
         FROM ids
         LEFT JOIN mission_definitions def ON def.mission_id=ids.mission_id
         LEFT JOIN claims published ON published.id=def.claim_id
         LEFT JOIN run_states ON run_states.mission_id=ids.mission_id
         LEFT JOIN latest ON latest.mission_id=ids.mission_id AND latest.rank=1"
    )
}

pub trait MissionListStore {
    /// The keys of the selected missions, or of every mission, at the reader's cut, as the
    /// current list sees them at `now`. A selected mission the store no longer has gets none.
    fn mission_list_keys(
            &self,
            selected: Option<&BTreeSet<String>>,
            now: u128,
        ) -> Result<HashMap<String, MissionKey>>;
    /// Which missions the claims after `after` through `through` can change, read in the
    /// caller's snapshot. A card reads its mission's definition and runs, those runs' current
    /// steps and faults, the root runs those steps answer to, revision proposals for those
    /// runs, and the person asks that block them, which their requesters' declarations and
    /// runtime decide; and which runs wait on a person. A claim about any other subject
    /// changes no card.
    fn mission_list_changes(&self, after: u64, through: u64) -> Result<MissionChanges>;
    /// Every mission's `mission/` ID, defined or run, for a fold from nothing to read in chunks.
    fn mission_list_ids(&self) -> Result<Vec<String>>;
    /// The missions with a current step whose worker lease ended after `after` and by
    /// `through`: their cards show the step ready again with no new claim.
    fn missions_with_leases_ended(&self, after: u128, through: u128) -> Result<BTreeSet<String>>;
    /// When the first worker lease that has not ended by `now` ends.
    fn next_lease_end(&self, now: u128) -> Result<Option<u128>>;
    /// The missions that own each of `steps`.
    fn missions_of_steps<'a>(
            &self,
            steps: impl IntoIterator<Item = &'a String>,
        ) -> Result<BTreeSet<String>>;
    /// The missions that own each of `runs`, which may name runs with or without their
    /// `mission-run/` prefix.
    fn missions_of_runs<'a>(
            &self,
            runs: impl IntoIterator<Item = &'a String>,
        ) -> Result<BTreeSet<String>>;
    fn published_missions_list(&self) -> &published_list::PublishedList<MissionRows>;
}

impl MissionListStore for Store {

    fn mission_list_keys(
            &self,
            selected: Option<&BTreeSet<String>>,
            now: u128,
        ) -> Result<HashMap<String, MissionKey>> {
        let ended_since = now.saturating_sub(RECENTLY_ENDED_MS);
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7ab511c79858 owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ &mission_keys_sql(selected.is_some()))?;
        let read = |row: &rusqlite::Row<'_>| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        };
        let rows = match selected {
            Some(selected) => statement
                .query_map(
                    [serde_json::to_string(
                        &selected.iter().map(|id| id.trim_start_matches("mission/")).collect::<Vec<_>>(),
                    )?],
                    read,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            None => statement.query_map([], read)?.collect::<rusqlite::Result<Vec<_>>>()?,
        };
        let mut keys = HashMap::with_capacity(rows.len());
        for (id, page_time, order, state, latest, definition, idle, latest_updated) in rows {
            let open = state
                .as_deref()
                .is_some_and(|state| !matches!(state, "completed" | "failed" | "cancelled" | "retired"));
            // A run that failed or was cancelled keeps its mission in view for a while, unless
            // the mission was retired.
            let ended = matches!(latest.as_deref(), Some("failed" | "cancelled"))
                && definition.as_deref() != Some("retired")
                && idle;
            let ended_until = latest_updated
                .filter(|_| ended)
                .map(|updated| (updated.max(0) as u128).saturating_add(RECENTLY_ENDED_MS));
            let recently_ended = ended && latest_updated.is_some_and(|updated| updated as i128 >= ended_since as i128);
            let shown = !system_mission(&id) && (open || recently_ended);
            keys.insert(
                format!("mission/{id}"),
                MissionKey {
                    order,
                    page_time: page_time.parse().unwrap_or(0),
                    visible: shown,
                    visible_until: ended_until.filter(|_| shown && !open),
                },
            );
        }
        Ok(keys)
    }

    fn mission_list_changes(&self, after: u64, through: u64) -> Result<MissionChanges> {
        let connection = self.readers.get();
        let mut changes = MissionChanges::default();
        let mut runs = BTreeSet::new();
        let mut steps = BTreeSet::new();
        let mut requesters = BTreeSet::new();
        let mut every_ask = false;
        let mut tree_roots = BTreeSet::new();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h533b21797a0b owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ published_list::CLAIMS_SINCE)?;
        let claims = statement
            .query_map(params![after, through], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (subject, kind, origin) in claims {
            changes.attention |= !ATTENTION_NEUTRAL_KINDS.contains(&kind.as_str());
            if subject.starts_with("mission/") {
                changes.missions.insert(subject.clone());
            } else if let Some(run) = subject.strip_prefix("mission-run/") {
                runs.insert(run.to_owned());
                // A run's state ends the steps of the runs under it, and decides whether asks
                // in them are current: refold the runs under it too.
                if matches!(kind.as_str(), "mission-run.state" | "mission-run.created") {
                    tree_roots.insert(run.to_owned());
                }
            } else if let Some(generation) = subject.strip_prefix("run-generation/") {
                let mut owner = connection
                    .prepare_cached("SELECT run_id FROM run_generations WHERE id=?1")?;
                if let Some(run) = owner.query_row(/* sqlite-large-value: allow-for-now lv-h673df99fbbfc owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ [generation], |row| row.get::<_, String>(0)).optional()? {
                    runs.insert(run);
                }
            } else if let Some(proposal) = subject.strip_prefix("revision-proposal/") {
                // A proposal drains, applies or releases its run with no claim on the run.
                runs.extend(self.revision_proposal_run(proposal)?);
            } else if subject.starts_with("step-run/") {
                // A person's answer or a cancelled ask unblocks the step that asked, and ends
                // a run the ask started; answers and cancellations name neither, the ask does.
                if kind.starts_with("work.person-") {
                    let (asked_from, started) = self.ask_origin_and_run(&subject)?;
                    steps.extend(origin.or(asked_from));
                    runs.extend(started.map(|run| run.trim_start_matches("mission-run/").to_owned()));
                }
                steps.insert(subject);
            } else if subject.starts_with("subscription/") || subject.starts_with("schedule/") {
                // An ask's currency walks the subscription or schedule that delivered its run.
                every_ask = true;
            } else if subject.starts_with("agent/") && published_list::decides_asks(&kind) {
                requesters.insert(subject);
            }
        }
        runs.extend(self.runs_under(&tree_roots)?);
        // A step that asked decides whether its asks are current, as when it leaves waiting.
        steps.extend(self.asks_from_steps(&steps)?);
        // A requester's declaration and runtime decide whether its asks still block steps, and
        // whether its runs wait on a person.
        let mut asks = self.asks_by_requesters(&requesters)?;
        if every_ask {
            asks = self.every_ask()?;
        }
        changes.attention |= !asks.is_empty();
        for (ask, origin) in asks {
            steps.insert(ask);
            steps.extend(origin);
        }
        if !steps.is_empty() {
            let mut owner = connection.prepare_cached("SELECT run_id FROM step_runs WHERE subject=?1")?;
            for step in &steps {
                if let Some(run) = owner.query_row(/* sqlite-large-value: allow-for-now lv-h5fd9c311de0c owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ [step], |row| row.get::<_, String>(0)).optional()? {
                    runs.insert(run);
                }
            }
        }
        if !runs.is_empty() {
            let mut owner = connection.prepare_cached("SELECT mission_id FROM mission_runs WHERE id=?1")?;
            for run in &runs {
                if let Some(mission) = owner.query_row(/* sqlite-large-value: allow-for-now lv-h593faf904dab owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ [run], |row| row.get::<_, String>(0)).optional()? {
                    changes.missions.insert(format!("mission/{mission}"));
                }
            }
        }
        Ok(changes)
    }

    fn mission_list_ids(&self) -> Result<Vec<String>> {
        let connection = self.readers.get();
        let ids = connection
            .prepare_cached(
                "SELECT 'mission/' || mission_id FROM mission_definitions
                 UNION SELECT 'mission/' || mission_id FROM mission_runs",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    fn missions_with_leases_ended(&self, after: u128, through: u128) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT DISTINCT 'mission/' || r.mission_id FROM step_runs s INDEXED BY step_runs_lease_index
             JOIN mission_runs r ON r.id=s.run_id
             WHERE s.lease_owner IS NOT NULL
               AND CAST(s.lease_expires_at_unix_ms AS INTEGER)>?1
               AND CAST(s.lease_expires_at_unix_ms AS INTEGER)<=?2",
        )?;
        let missions = statement
            .query_map(
                params![after.min(i64::MAX as u128) as i64, through.min(i64::MAX as u128) as i64],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        Ok(missions)
    }

    fn next_lease_end(&self, now: u128) -> Result<Option<u128>> {
        let connection = self.readers.get();
        let end = connection
            .prepare_cached(
                "SELECT MIN(CAST(lease_expires_at_unix_ms AS INTEGER)) FROM step_runs INDEXED BY step_runs_lease_index
                 WHERE lease_owner IS NOT NULL AND CAST(lease_expires_at_unix_ms AS INTEGER)>?1",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h426bfc004f77 owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ [now.min(i64::MAX as u128) as i64], |row| row.get::<_, Option<i64>>(0))?;
        Ok(end.map(|end| end.max(0) as u128))
    }

    fn missions_of_steps<'a>(
            &self,
            steps: impl IntoIterator<Item = &'a String>,
        ) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let mut owner = connection.prepare_cached(
            "SELECT r.mission_id FROM step_runs s JOIN mission_runs r ON r.id=s.run_id WHERE s.subject=?1",
        )?;
        let mut missions = BTreeSet::new();
        for step in steps {
            if let Some(mission) = owner.query_row(/* sqlite-large-value: allow-for-now lv-hc1f0dd5188ac owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ [step], |row| row.get::<_, String>(0)).optional()? {
                missions.insert(format!("mission/{mission}"));
            }
        }
        Ok(missions)
    }

    fn missions_of_runs<'a>(
            &self,
            runs: impl IntoIterator<Item = &'a String>,
        ) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let mut owner = connection.prepare_cached("SELECT mission_id FROM mission_runs WHERE id=?1")?;
        let mut missions = BTreeSet::new();
        for run in runs {
            let run = run.strip_prefix("mission-run/").unwrap_or(run);
            if let Some(mission) = owner.query_row(/* sqlite-large-value: allow-for-now lv-ha9088152655f owner=st3_missions::store::mission_list TODO=trace-statement-and-value-representation */ [run], |row| row.get::<_, String>(0)).optional()? {
                missions.insert(format!("mission/{mission}"));
            }
        }
        Ok(missions)
    }

    fn published_missions_list(&self) -> &published_list::PublishedList<MissionRows> {
        &self.smalltalk.published_missions
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_missions_match_the_sql_pattern_exactly() {
        for hidden in ["__st3/loop", "abst3/x", "__ST3/round", "é_st3/x"] {
            assert!(system_mission(hidden), "{hidden}");
        }
        for shown in ["st3/x", "_st3/x", "__st4/x", "fleet/st3/x", "__st3"] {
            assert!(!system_mission(shown), "{shown}");
        }
    }
}
