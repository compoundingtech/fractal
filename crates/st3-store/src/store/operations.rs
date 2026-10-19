//! Bounded operational reads. These never hydrate a run's steps or replay work history.
use super::*;

fn short(value: &str) -> String {
    value.chars().take(2_000).collect()
}

fn recently_ended_mission_valid_until(
    connection: &Connection,
    ids: &str,
    now: u128,
) -> Result<Option<u128>> {
    let mut statement = connection.prepare(
        /* sqlite-large-value: allow-for-now lv-hf0fc4fe0767b owner=st3_store::store::operations TODO=trace-statement-and-value-representation */ "WITH latest AS (
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

impl Store {
    /// Counts all runs; previews only the newest runs and the newest failures.
    pub fn mission_overview(&self, mission: &str, limit: usize) -> Result<Value> {
        let mission = mission.strip_prefix("mission/").unwrap_or(mission);
        let connection = self.readers.get();
        let mut counts = BTreeMap::<String, u64>::new();
        let mut statement = connection.prepare(
            "SELECT status, COUNT(*) FROM mission_runs WHERE mission_id=?1 GROUP BY status",
        )?;
        for row in statement.query_map([mission], |row| Ok((row.get(0)?, row.get(1)?)))? {
            let (state, count) = row?;
            counts.insert(state, count);
        }
        if counts.is_empty()
            && !connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM mission_definitions WHERE mission_id=?1)",
                [mission],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(St3Error::new(
                "not-found",
                format!("mission `mission/{mission}` does not exist"),
            )
            .into());
        }
        let mut previews = Vec::new();
        for failed in [false, true] {
            let mut statement = connection.prepare(
                "SELECT r.id, r.current_generation_id, r.requester, r.status, r.phase, r.created_at_unix_ms,
                        r.updated_at_unix_ms,COALESCE(g.revision,r.initial_revision) FROM mission_runs r
                 LEFT JOIN run_generations g ON g.id=r.current_generation_id
                 WHERE r.mission_id=?1 AND (NOT ?2 OR r.status='failed')
                 ORDER BY r.created_at_unix_ms DESC, r.id DESC LIMIT ?3",
            )?;
            let mut items = Vec::new();
            for row in statement.query_map(params![mission, failed, limit.clamp(1, 20)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })? {
                let (id, generation, requester, status, phase, created, updated, revision) = row?;
                let subject = format!("mission-run/{id}");
                let reason: Option<String> = connection
                    .query_row(
                        /* sqlite-large-value: allow-for-now lv-hb7f1fd7bd939 owner=st3_store::store::operations TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT COALESCE(json_extract(body,'$.fields.reason'),
                                     json_extract(body,'$.fields.summary'))
                     FROM claims WHERE subject=?1 AND kind='mission-run.state'
                       AND COALESCE(json_extract(body,'$.fields.reason'),json_extract(body,'$.fields.summary')) IS NOT NULL
                       AND (json_extract(body,'$.fields.status')=?2 OR json_extract(body,'$.fields.phase') LIKE '%-' || ?2)
                     ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
                        params![subject,status],
                        |row| row.get(0),
                    )
                    .optional()?
                    .flatten();
                items.push(
                    json!({"id": subject, "generation_id": format!("run-generation/{generation}"),
                    "requester": short(&requester), "status": status, "phase": phase,
                    "revision":revision,
                    "created_at_unix_ms": created.parse::<u128>()?,
                    "updated_at_unix_ms": updated.parse::<u128>()?,
                    "reason": reason.as_deref().map(short)}),
                );
            }
            previews.push(items);
        }
        let revision: Option<String> = connection
            .query_row(
                "SELECT revision FROM mission_definitions WHERE mission_id=?1",
                [mission],
                |row| row.get(0),
            )
            .optional()?;
        let provenance = revision
            .as_deref()
            .map(|revision| crate::provenance::read(&connection, mission, revision))
            .transpose()?
            .flatten();
        let mut overview = json!({"mission": format!("mission/{mission}"),
            "total_runs": counts.values().sum::<u64>(), "counts": counts,
            "newest": previews[0], "failed": previews[1], "preview_limit": limit.clamp(1,20)});
        if let Some(revision) = revision {
            overview["revision"] = json!(revision);
        }
        if let Some(provenance) = provenance {
            overview["provenance"] = serde_json::to_value(provenance)?;
        }
        Ok(overview)
    }

    /// A run card's progress counts cover every step; only its first twenty steps are hydrated.
    pub fn mission_step_preview(&self, run: &str) -> Result<(u64, u64, Vec<StepRunView>)> {
        self.mission_step_preview_at(run, now_ms())
    }

    pub fn mission_step_preview_at(
        &self,
        run: &str,
        at_unix_ms: u128,
    ) -> Result<(u64, u64, Vec<StepRunView>)> {
        let run = run.trim_start_matches("mission-run/");
        let connection = self.readers.get();
        let (total, done) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(s.status='completed'),0)
             FROM step_runs s JOIN mission_runs r ON r.id=s.run_id
             WHERE r.id=?1 AND s.generation_id=r.current_generation_id",
            [run],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
        )?;
        let mut statement=connection.prepare(
            "SELECT subject,run_id,step_path,definition_hash,status,attempt,assignee,available_to,agentless,title,goals,worker_reported,
                    lease_owner,lease_incarnation,lease_expires_at_unix_ms,blocked_reason,not_before_unix_ms,created_at_unix_ms,updated_at_unix_ms,readiness_epoch,constraints
             FROM step_runs WHERE run_id=?1 AND generation_id=(SELECT current_generation_id FROM mission_runs WHERE id=?1)
             ORDER BY created_at_unix_ms,step_path LIMIT 20",
        )?;
        let mut steps = statement
            .query_map([run], step_run_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for step in &mut steps {
            apply_effective_step_state(&connection, step, at_unix_ms)?;
        }
        Ok((total, done, steps))
    }

    /// Exclusive clock fence for collection cards at `now`, with the graph held unchanged.
    ///
    /// Cards observe effective leases, future person asks and the recently-ended mission
    /// membership cutoff. They do not enrich steps with wakes or execution timing:
    /// `not_before`, mission deadlines and step timeout budgets only change these cards
    /// after a graph write. Selected IDs include the page's has_more lookahead; only the
    /// newest three runs and first twenty steps are rendered. IDs may have a `mission/` prefix.
    pub fn mission_collection_valid_until(
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
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hd95aac5f4b8b owner=st3_store::store::operations TODO=trace-statement-and-value-representation */ &format!(
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
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hfd8255e31968 owner=st3_store::store::operations TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
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
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h8c5f7a3ac315 owner=st3_store::store::operations TODO=trace-statement-and-value-representation */ &format!(
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

    /// Terminal transitions in a stable claim-index window. A retry does not erase a failure.
    pub fn outcome_history(
        &self,
        collection: &str,
        since: u128,
        until: u128,
        status: Option<&str>,
        actor: Option<&str>,
        before: u64,
        limit: usize,
    ) -> Result<Value> {
        anyhow::ensure!(
            matches!(collection, "missions" | "work"),
            "unknown outcome collection"
        );
        anyhow::ensure!(
            status.is_none_or(|s| matches!(s, "failed" | "cancelled" | "timed-out" | "completed")),
            "status must be failed, cancelled, timed-out, or completed"
        );
        let connection = self.readers.get();
        let kinds = if collection == "missions" {
            "c.kind='mission-run.state'"
        } else {
            "c.kind IN ('step-run.state','work.failed')"
        };
        let prior = canonical::after_sql("c", "p");
        let reason_order = canonical::order_sql("p", true);
        let reason = format!("COALESCE(json_extract(c.body,'$.fields.reason'), json_extract(c.body,'$.fields.summary'),
            (SELECT COALESCE(json_extract(p.body,'$.fields.reason'),json_extract(p.body,'$.fields.summary'))
             FROM claims p WHERE p.subject=c.subject AND p.kind=c.kind AND {prior}
               AND COALESCE(json_extract(p.body,'$.fields.reason'),json_extract(p.body,'$.fields.summary')) IS NOT NULL
               AND (json_extract(p.body,'$.fields.status')=json_extract(c.body,'$.fields.status')
                    OR json_extract(p.body,'$.fields.phase') LIKE '%-' || json_extract(c.body,'$.fields.status'))
             ORDER BY {reason_order} LIMIT 1))");
        // The kind index bounds this to lifecycle claims even on graphs dominated by heartbeats.
        let sql = format!(
            "SELECT c.store_index, c.id, c.subject, c.actor, c.body, c.accepted_at_unix_ms,
                    COALESCE(r.mission_id,sr.mission_id), s.run_id, {reason}
             FROM claims c LEFT JOIN step_runs s ON s.subject=c.subject
             LEFT JOIN mission_runs sr ON sr.id=s.run_id
             LEFT JOIN mission_runs r ON r.id=substr(c.subject,13) AND c.subject LIKE 'mission-run/%'
             WHERE c.kind IN ('mission-run.state','step-run.state','work.failed')
               AND {kinds} AND c.store_index < ?1
               AND CAST(c.accepted_at_unix_ms AS INTEGER)>=?2
               AND CAST(c.accepted_at_unix_ms AS INTEGER)<=?3
               AND json_extract(c.body,'$.fields.status') IN ('failed','cancelled','completed')
               AND (?4 IS NULL OR json_extract(c.body,'$.fields.status')=?4
                    OR (?4='timed-out' AND json_extract(c.body,'$.fields.status')='failed'
                        AND (lower(COALESCE({reason},'')) LIKE '%timeout%'
                          OR lower(COALESCE({reason},'')) LIKE '%timed out%')))
               AND (?5 IS NULL OR c.actor=?5 OR s.assignee=?5 OR s.lease_owner=?5)
             ORDER BY c.store_index DESC LIMIT ?6"
        );
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h839ea7f2c066 owner=st3_store::store::operations TODO=trace-statement-and-value-representation */ &sql)?;
        let rows = statement.query_map(
            params![
                before.min(i64::MAX as u64),
                since as i64,
                until.min(i64::MAX as u128) as i64,
                status,
                actor,
                limit.clamp(1, 200) + 1
            ],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        let mut items = Vec::new();
        for row in rows {
            let (index, id, subject, actor, body, at, mission, run, reason) = row?;
            let body: Value = serde_json::from_str(&body)?;
            let fields = &body["fields"];
            items.push(
                json!({"index":index, "id":id, "subject":subject, "actor":actor,
                "status":fields["status"], "reason":reason.as_deref().map(short),
                "at_unix_ms":at.parse::<u128>()?, "mission":mission.map(|m| format!("mission/{m}")),
                "run":run.map(|r| format!("mission-run/{r}"))}),
            );
        }
        let has_more = items.len() > limit.clamp(1, 200);
        items.truncate(limit.clamp(1, 200));
        Ok(
            json!({"collection":collection, "since_unix_ms":since, "until_unix_ms":until,
            "status":status, "items":items, "has_more":has_more,
            "next_before":if has_more { items.last().and_then(|v| v["index"].as_u64()) } else {None}}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_reason_recovery_uses_canonical_order_despite_reversed_arrival() {
        let store = Store::open_memory("fixture").unwrap();
        {
            let connection = store.connection.lock().unwrap();
            connection.execute("INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES ('reason-batch','fixture',1,'reason-hash','1')", []).unwrap();
            // Cleanup arrived before the two explanatory claims. The older explanation
            // arrived last; neither its arrival nor the cleanup's cursor chooses the reason.
            for (id, at, reason) in [
                ("cleanup", "30", None),
                ("newer", "20", Some("newer timeout reason")),
                ("older", "10", Some("older reason")),
            ] {
                connection.execute("INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,'reason-batch','mission-run/example/reason','mission-run.state','fixture',?2,'[]',?3)",
                    params![id,json!({"fields":{"status":"failed","phase":"terminal","reason":reason}}).to_string(),at]).unwrap();
            }
        }
        let history = store
            .outcome_history("missions", 0, 100, None, None, u64::MAX, 10)
            .unwrap();
        let cleanup = history["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "cleanup")
            .unwrap();
        assert_eq!(cleanup["reason"], "newer timeout reason");
        let timed = store
            .outcome_history("missions", 0, 100, Some("timed-out"), None, u64::MAX, 10)
            .unwrap();
        assert!(
            timed["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == "cleanup")
        );
    }
    #[test]
    fn mission_overview_missing_mission_is_not_found() {
        let store = Store::open_memory("fixture").unwrap();
        for mission in ["mission/example/missing", "example/missing"] {
            let error = store.mission_overview(mission, 10).unwrap_err();
            let error = error.downcast_ref::<St3Error>().unwrap();
            assert_eq!(error.code, "not-found");
            assert_eq!(
                error.message,
                "mission `mission/example/missing` does not exist"
            );
        }
    }

}
