//! Bounded prefix reads for automation; no mission, agent or roster fold.
use super::*;

macro_rules! run_query {
    ($end:literal, $active:literal) => {
        concat!(
            "SELECT r.id,r.mission_id,g.revision,r.status,r.phase,r.created_at_unix_ms,r.updated_at_unix_ms,r.current_generation_id FROM mission_runs r JOIN run_generations g ON g.id=r.current_generation_id WHERE r.mission_id>=?1 AND r.mission_id>='' ",
            $end, $active,
            " ORDER BY r.mission_id,r.created_at_unix_ms DESC,r.id DESC LIMIT ?3"
        )
    };
}
const MAX_STEPS: usize = 500;

#[derive(Debug, Serialize)]
pub struct MissionRunsPage {
    pub runs: Vec<MissionRunSummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct MissionRunSummary {
    pub run_id: String,
    pub mission_id: String,
    pub revision: String,
    pub status: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    pub steps: Vec<MissionRunStepSummary>,
    pub steps_truncated: bool,
    #[serde(skip)]
    generation: String,
}

#[derive(Debug, Serialize)]
pub struct MissionRunStepSummary {
    pub step_id: String,
    pub state: String,
}

// SQLite's BINARY text ordering agrees with Unicode scalar ordering. Increment the
// last incrementable scalar, dropping its suffix, to form an exclusive prefix end.
fn prefix_end(prefix: &str) -> Option<String> {
    for (offset, ch) in prefix.char_indices().rev() {
        let next = u32::from(ch) + 1;
        let next = if next == 0xd800 { 0xe000 } else { next };
        if let Some(next) = char::from_u32(next) {
            let mut end = prefix[..offset].to_owned();
            end.push(next);
            return Some(end);
        }
    }
    None
}

impl Store {
    /// Read current-generation headers and step states only. Limits apply before
    /// hydration, including a separate bound on each run's steps.
    pub fn mission_runs_by_prefix(&self, prefix: &str, active: bool, limit: usize) -> Result<MissionRunsPage> {
        anyhow::ensure!((1..=500).contains(&limit), "run limit must be 1 through 500");
        let prefix = prefix.strip_prefix("mission/").unwrap_or(prefix);
        let end = prefix_end(prefix);
        let connection = self.readers.get();
        let sql = match (end.is_some(), active) {
            (true, false) => run_query!("AND r.mission_id<?2", ""),
            (true, true) => run_query!("AND r.mission_id<?2", " AND r.status NOT IN ('completed','failed','cancelled')"),
            (false, false) => run_query!("AND ?2 IS NULL", ""),
            (false, true) => run_query!("AND ?2 IS NULL", " AND r.status NOT IN ('completed','failed','cancelled')"),
        };
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hbc959d871d00 owner=fractal_store::store::mission_run_query TODO=trace-statement-and-value-representation */ sql)?;
        let mut runs = statement.query_map(params![prefix, end, limit + 1], |row| Ok(MissionRunSummary {
            run_id: format!("mission-run/{}", row.get::<_, String>(0)?),
            mission_id: format!("mission/{}", row.get::<_, String>(1)?),
            revision: row.get(2)?, status: row.get(3)?, phase: row.get(4)?,
            started_at: row.get(5)?, updated_at: row.get(6)?, generation: row.get(7)?,
            host_id: None, steps: Vec::new(), steps_truncated: false,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let truncated = runs.len() > limit;
        runs.truncate(limit);
        let snapshot_unix_ms = now_ms();
        let mut steps = connection.prepare(
            "SELECT subject,run_id,step_path,definition_hash,status,attempt,assignee,
                    available_to,agentless,title,goals,worker_reported,lease_owner,
                    lease_incarnation,lease_expires_at_unix_ms,blocked_reason,
                    not_before_unix_ms,created_at_unix_ms,updated_at_unix_ms,
                    readiness_epoch,constraints
             FROM step_runs WHERE run_id=?1 AND generation_id=?2
             ORDER BY step_path LIMIT ?3",
        )?;
        let mut owners = connection.prepare(
            "SELECT r.status,r.phase,r.current_generation_id,g.status,root.status,root.phase
             FROM mission_runs r JOIN run_generations g
               ON g.id=r.current_generation_id AND g.run_id=r.id
             JOIN mission_runs root ON root.id=r.root_run_id WHERE r.id=?1",
        )?;
        static HOST_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| canonical_sql(
            "SELECT origin FROM claims WHERE subject=?1 AND kind='mission-run.created' AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired') ORDER BY CANONICAL_ASC(claims) LIMIT 1",
        ));
        let mut host = connection.prepare(/* sqlite-large-value: allow-for-now lv-h890a38462624 owner=fractal_store::store::mission_run_query TODO=trace-statement-and-value-representation */ &HOST_SQL)?;
        for run in &mut runs {
            let id = run.run_id.strip_prefix("mission-run/").expect("run subject prefix");
            let mut selected = steps.query_map(
                params![id, run.generation, MAX_STEPS + 1], step_run_from_row,
            )?.collect::<rusqlite::Result<Vec<_>>>()?;
            run.steps_truncated = selected.len() > MAX_STEPS;
            selected.truncate(MAX_STEPS);
            let owner: Option<EffectiveStepOwner> = owners.query_row(/* sqlite-large-value: allow-for-now lv-hd7513b5c921e owner=fractal_store::store::mission_run_query TODO=trace-statement-and-value-representation */ [id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?,
                    row.get(3)?, row.get(4)?, row.get(5)?))
            }).optional()?;
            for mut step in selected {
                step.generation = format!("run-generation/{}", run.generation);
                apply_effective_step_state_with_owner(
                    &connection, &mut step, snapshot_unix_ms, owner.clone(),
                )?;
                run.steps.push(MissionRunStepSummary {
                    step_id: step.subject, state: step.status,
                });
            }
            run.host_id = host.query_row(/* sqlite-large-value: allow-for-now lv-hf58566f1b601 owner=fractal_store::store::mission_run_query TODO=trace-statement-and-value-representation */ [&run.run_id], |row| row.get(0)).optional()?;
        }
        Ok(MissionRunsPage { runs, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallclaims::sqlite::work::SqliteWorkScope;

    fn insert_run(connection: &Connection, id: &str, mission: &str, status: &str) {
        connection.execute(
            "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
             VALUES(?1,?2,'revision',?1,'revision',?1,'/work/example','person/operator','{}','eval',?3,'steps','100','200')",
            params![id, mission, status],
        ).unwrap();
        connection.execute(
            "INSERT INTO run_generations(id,run_id,revision,status,actor,reason,created_at_unix_ms,updated_at_unix_ms)
             VALUES(?1,?1,'revision','active','person/operator','fixture','100','200')", [id],
        ).unwrap();
    }

    #[test]
    fn prefix_query_is_bounded_and_literal() {
        let store = Store::open_memory("node-a").unwrap();
        {
            let connection = store.connection.write();
            for (id, mission, status) in [
                ("a", "roll_%/one", "running"),
                ("b", "roll_%/two", "standing"),
                ("c", "roll_%/done", "completed"),
                ("d", "roll-ab/unrelated", "running"),
            ] {
                insert_run(&connection, id, mission, status);
            }
            for index in 0..502 {
                connection.execute(
                    "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,attempt,goals,created_at_unix_ms,updated_at_unix_ms)
                     VALUES(?1,'a','a',?1,'hash','pending',0,'[]','100','200')",
                    [format!("step-run/{index:04}")],
                ).unwrap();
            }
        }
        let page = store.mission_runs_by_prefix("mission/roll_%/", true, 1).unwrap();
        assert!(page.truncated);
        assert_eq!(page.runs[0].run_id, "mission-run/a");
        assert_eq!(page.runs[0].steps.len(), MAX_STEPS);
        assert!(page.runs[0].steps_truncated);
        assert_eq!(page.runs[0].steps[0].state, "pending");
        assert!(serde_json::to_value(&page.runs[0]).unwrap().get("host_id").is_none());
        let page = store.mission_runs_by_prefix("roll_%/", true, 100).unwrap();
        assert_eq!(page.runs.len(), 2);
        assert!(!page.truncated);
        assert_eq!(store.mission_runs_by_prefix("roll_%/", false, 100).unwrap().runs.len(), 3);
        assert!(store.mission_runs_by_prefix("missing", true, 100).unwrap().runs.is_empty());
        assert!(store.mission_runs_by_prefix("", false, 0).is_err());
        assert!(store.mission_runs_by_prefix("", false, 501).is_err());
        assert_eq!(prefix_end("roll/"), Some("roll0".into()));
        assert_eq!(prefix_end("x\u{10ffff}"), Some("y".into()));
    }

    fn insert_claimed_step(connection: &Connection, id: &str) {
        connection.execute(
            "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,attempt,goals,lease_expires_at_unix_ms,created_at_unix_ms,updated_at_unix_ms)
             VALUES(?1,?2,?2,'work','hash','working',0,'[]','1','100','200')",
            params![format!("step-run/{id}/work"), id],
        ).unwrap();
    }

    #[test]
    fn prefix_query_uses_effective_lease_effect_and_terminal_ownership_states() {
        let store = Store::open_memory("node-a").unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            for id in ["expired", "effect", "terminal", "generation", "root"] {
                insert_run(&tx, id, &format!("roll/{id}"), "running");
                insert_claimed_step(&tx, id);
            }
            tx.execute("UPDATE mission_runs SET status='completed' WHERE id='terminal'", []).unwrap();
            tx.execute("UPDATE run_generations SET status='cancelled' WHERE id='generation'", []).unwrap();
            // A terminal root alone does not cancel an active owner or generation.
            // Descendant cancellation must be persisted by the cancellation operation.
            tx.execute("UPDATE mission_runs SET root_run_id='terminal' WHERE id='root'", []).unwrap();
            append_claim_record_tx(
                &tx, "node-a", "effect/unresolved", "effect.unresolved",
                Some("person/operator"),
                &json!({"fields": {"bound_step_run": "step-run/effect/work"}}), &[], None,
            ).unwrap();
            tx.commit().unwrap();
        }
        for (id, expected) in [
            ("expired", "ready"), ("effect", "blocked"),
            ("terminal", "cancelled"), ("generation", "cancelled"), ("root", "ready"),
        ] {
            let page = store.mission_runs_by_prefix(&format!("roll/{id}"), false, 100).unwrap();
            assert_eq!(page.runs[0].steps[0].state, expected, "{id}");
        }
    }

    #[test]
    fn prefix_query_cost_does_not_grow_with_unrelated_runs_or_agents() {
        let store = Store::open_memory("node-a").unwrap();
        insert_run(&store.connection.write(), "target", "roll/target", "running");
        insert_claimed_step(&store.connection.write(), "target");
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            append_claim_record_tx(
                &tx, "node-origin", "mission-run/target", "mission-run.created",
                Some("person/operator"), &json!({"fields": {}}), &[], None,
            ).unwrap();
            tx.commit().unwrap();
        }
        let measure = |active| {
            let scope = SqliteWorkScope::start();
            let page = store.mission_runs_by_prefix("roll/", active, 100).unwrap();
            assert_eq!(page.runs.len(), 1);
            assert_eq!(page.runs[0].host_id.as_deref(), Some("node-origin"));
            assert_eq!(page.runs[0].steps[0].state, "ready");
            let cost = scope.finish();
            assert!(cost.vm_steps > 0, "{cost:?}");
            assert_eq!(cost.fullscan_steps, 0, "{cost:?}");
            cost
        };
        let small = [measure(false), measure(true)];
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            for index in 0..10_000 {
                insert_run(&tx, &format!("unrelated-{index}"), &format!("unrelated/{index}"), "running");
                tx.execute(
                    "INSERT INTO desired(subject,kind,revision,claim_id,body,member) VALUES(?1,'agent','revision',(SELECT id FROM claims WHERE subject='mission-run/target' LIMIT 1),'{}','{\"host\":\"node-a\"}')",
                    [format!("agent/unrelated-{index}")],
                ).unwrap();
            }
            tx.commit().unwrap();
        }
        for (baseline, active) in small.into_iter().zip([false, true]) {
            let large = measure(active);
            println!("mission run prefix active={active}: small={baseline:?}, large={large:?}");
            assert!(large.vm_steps <= baseline.vm_steps + 100, "active={active}, small={baseline:?}, large={large:?}");
            assert_eq!(large.statements, baseline.statements);
        }
    }
}
