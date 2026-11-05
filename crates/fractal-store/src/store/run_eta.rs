//! Selected-attempt wall-time boundaries and person-held duration accounting.
use super::*;

pub const SAMPLE_LIMIT: usize = 64;

pub fn open(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h1d1ee62dddd6 owner=fractal_store::store::run_eta TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS local_run_eta_samples (
 subject TEXT PRIMARY KEY, run_id TEXT NOT NULL, definition_hash TEXT NOT NULL,
 step_path TEXT NOT NULL, ended INTEGER NOT NULL, duration INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_run_eta_key ON local_run_eta_samples(definition_hash,step_path,ended DESC,subject DESC);
CREATE INDEX IF NOT EXISTS local_run_eta_run ON local_run_eta_samples(run_id);
CREATE TABLE IF NOT EXISTS local_run_eta_pending (run_id TEXT PRIMARY KEY) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS run_eta_run_insert AFTER INSERT ON mission_runs BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending VALUES(NEW.id); END;
CREATE TRIGGER IF NOT EXISTS run_eta_run_update AFTER UPDATE ON mission_runs BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending VALUES(NEW.id); END;
CREATE TRIGGER IF NOT EXISTS run_eta_run_delete AFTER DELETE ON mission_runs BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending VALUES(OLD.id); END;
CREATE TRIGGER IF NOT EXISTS run_eta_step_insert AFTER INSERT ON step_runs BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending VALUES(NEW.run_id); END;
CREATE TRIGGER IF NOT EXISTS run_eta_step_update AFTER UPDATE ON step_runs BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending VALUES(OLD.run_id);
 INSERT OR IGNORE INTO local_run_eta_pending VALUES(NEW.run_id); END;
CREATE TRIGGER IF NOT EXISTS run_eta_step_delete AFTER DELETE ON step_runs BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending VALUES(OLD.run_id); END;
CREATE TRIGGER IF NOT EXISTS run_eta_claim_insert AFTER INSERT ON claims
 WHEN NEW.kind IN ('work.claimed','work.failed','step-run.state','step-run.carried','step-run.retried','work.person-asked','work.person-done','work.person-cancelled') BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending SELECT run_id FROM step_runs WHERE subject=NEW.subject
 OR subject IN (SELECT json_extract(body,'$.fields.origin_step') FROM claims WHERE subject=NEW.subject AND kind='work.person-asked'); END;
CREATE TRIGGER IF NOT EXISTS run_eta_claim_update AFTER UPDATE OF accepted_at_unix_ms,body ON claims BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending SELECT run_id FROM step_runs WHERE subject=NEW.subject
 OR subject IN (SELECT json_extract(body,'$.fields.origin_step') FROM claims WHERE subject=NEW.subject AND kind='work.person-asked'); END;
CREATE TRIGGER IF NOT EXISTS run_eta_claim_delete AFTER DELETE ON claims BEGIN
 INSERT OR IGNORE INTO local_run_eta_pending SELECT run_id FROM step_runs WHERE subject=OLD.subject
 OR subject=json_extract(OLD.body,'$.fields.origin_step')
 OR subject IN (SELECT json_extract(body,'$.fields.origin_step') FROM claims WHERE subject=OLD.subject AND kind='work.person-asked'); END;
INSERT OR IGNORE INTO local_run_eta_pending SELECT id FROM mission_runs
 WHERE NOT EXISTS(SELECT 1 FROM meta WHERE key='run_eta_samples_v1');
INSERT OR IGNORE INTO meta VALUES('run_eta_samples_v1','1');
"#)?;
    flush(tx)
}

pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-h35597d583326 owner=fractal_store::store::run_eta TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_run_eta_samples", [])?;
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h7a59c8a62c5a owner=fractal_store::store::run_eta TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_run_eta_pending SELECT id FROM mission_runs",
        [],
    )?;
    flush(tx)
}

pub fn flush(tx: &Transaction<'_>) -> Result<()> {
    let runs = tx
        .prepare_cached("SELECT run_id FROM local_run_eta_pending ORDER BY run_id")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for run in runs {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h080c6702874c owner=fractal_store::store::run_eta TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_run_eta_samples WHERE run_id=?1", [&run])?;
        let steps = tx.prepare_cached("SELECT s.subject,s.definition_hash,s.step_path,s.attempt
            FROM step_runs s JOIN mission_runs r ON r.id=s.run_id AND r.current_generation_id=s.generation_id
            WHERE r.id=?1 AND r.phase='terminal' AND r.status='completed' AND s.status='completed'")?
            .query_map([&run], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, u32>(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (subject, hash, path, attempt) in steps {
            let events = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h43c8328fcd04 owner=fractal_store::store::run_eta TODO=trace-statement-and-value-representation */ &format!("SELECT kind,body,claims.accepted_at_unix_ms FROM claims JOIN batches ON batches.id=claims.batch_id
                WHERE subject=?1 ORDER BY {CANONICAL_ORDER}"))?
                .query_map([&subject], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?)))?
                .map(|row| { let (kind,body,at)=row?; Ok((kind,serde_json::from_str(&body)?,at.parse()?)) })
                .collect::<Result<Vec<_>>>()?;
            let (Some(start), Some(end)) = step_timestamps(&events, attempt) else {
                continue;
            };
            let asks = tx.prepare_cached("SELECT subject,body,accepted_at_unix_ms FROM claims
                WHERE kind='work.person-asked' AND (subject=?1 OR json_extract(body,'$.fields.origin_step')=?1)")?
                .query_map([&subject], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut holds = Vec::new();
            for (ask, body, at) in asks {
                let body: Value = serde_json::from_str(&body)?;
                let fields = &body["fields"];
                if fields["request"]["type"] == "update"
                    || fields[if ask == subject {
                        "attempt"
                    } else {
                        "origin_attempt"
                    }]
                    .as_u64()
                    .unwrap_or(1)
                        != u64::from(attempt)
                {
                    continue;
                }
                let close = tx.query_row(/* sqlite-large-value: allow-for-now lv-h7fa46ab76bbc owner=fractal_store::store::run_eta TODO=trace-statement-and-value-representation */ &format!("SELECT claims.accepted_at_unix_ms FROM claims JOIN batches ON batches.id=claims.batch_id
                    WHERE subject=?1 AND kind IN ('work.person-done','work.person-cancelled') ORDER BY {CANONICAL_ORDER} LIMIT 1"),
                    [&ask], |row| row.get::<_, String>(0)).optional()?.map(|at| at.parse::<u128>()).transpose()?.unwrap_or(end);
                holds.push((at.parse::<u128>()?, close));
            }
            let duration = person_adjusted_duration(start, end, &mut holds);
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-hbd1849a668b1 owner=fractal_store::store::run_eta TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_run_eta_samples VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    subject,
                    run,
                    hash,
                    path,
                    i64::try_from(end)?,
                    i64::try_from(duration)?
                ],
            )?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h6f55c65253e5 owner=fractal_store::store::run_eta TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_run_eta_samples WHERE definition_hash=?1 AND step_path=?2 AND subject NOT IN (
                SELECT subject FROM local_run_eta_samples WHERE definition_hash=?1 AND step_path=?2 ORDER BY ended DESC,subject DESC LIMIT ?3)",
                params![hash,path,SAMPLE_LIMIT])?;
        }
        tx.execute(/* sqlite-large-value: allow-for-now lv-h62f0aab7c5d9 owner=fractal_store::store::run_eta TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_run_eta_pending WHERE run_id=?1", [&run])?;
    }
    Ok(())
}

/// Overlapping person holds count once; queueing, released intervals and agent waits remain.
pub fn person_adjusted_duration(start: u128, end: u128, holds: &mut [(u128, u128)]) -> u128 {
    holds.sort_unstable();
    let mut through = start;
    let mut held = 0;
    for &(low, high) in holds.iter() {
        let low = low.max(start).max(through);
        let high = high.min(end);
        held += high.saturating_sub(low);
        through = through.max(high);
    }
    end.saturating_sub(start).saturating_sub(held)
}

/// First execution opening and authoritative terminal edge, independent of lease intervals.
pub fn step_timestamps(
    events: &[(String, Value, u128)],
    attempt: u32,
) -> (Option<u128>, Option<u128>) {
    let (mut started, mut completed) = (None, None);
    let mut current_attempt = 1;
    for (kind, body, at) in events {
        let fields = body.get("fields").unwrap_or(body);
        if matches!(
            kind.as_str(),
            "step-run.state" | "step-run.carried" | "step-run.retried"
        ) && let Some(value) = fields["attempt"].as_u64()
        {
            current_attempt = value;
        }
        if fields["attempt"].as_u64().unwrap_or(current_attempt) != u64::from(attempt) {
            continue;
        }
        let status = fields["status"].as_str().unwrap_or_default();
        if kind == "work.claimed"
            || (matches!(kind.as_str(), "step-run.state" | "step-run.carried")
                && matches!(status, "claimed" | "working"))
        {
            started.get_or_insert(*at);
        }
        if matches!(
            kind.as_str(),
            "step-run.state" | "work.failed" | "work.person-done" | "work.person-cancelled"
        ) && matches!(status, "completed" | "failed" | "cancelled" | "skipped")
        {
            completed.get_or_insert(*at);
        }
    }
    (started, completed)
}

pub fn read(connection: &Connection, run: &str, at: u128) -> Result<Option<Value>> {
    let run = run.trim_start_matches("mission-run/");
    let body = connection
        .query_row(
            "SELECT d.body FROM mission_runs r
        JOIN run_generations g ON g.id=r.current_generation_id
        JOIN mission_revisions d ON d.mission_id=r.mission_id AND d.revision=g.revision
        WHERE r.id=?1 AND r.phase<>'terminal'",
            [run],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(body) = body else { return Ok(None) };
    let mission: MissionSpec = serde_json::from_str(&body)?;
    let specs = flatten_mission_step_specs(&mission);
    let critical = specs.iter().any(|step| !step.dependencies.is_empty());
    let steps = connection.prepare_cached("SELECT step_path,definition_hash,status FROM step_runs
        WHERE run_id=?1 AND generation_id=(SELECT current_generation_id FROM mission_runs WHERE id=?1)")?
        .query_map([run], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut weights = BTreeMap::new();
    let mut samples = SAMPLE_LIMIT;
    for (path, hash, status) in steps {
        if matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "skipped"
        ) {
            continue;
        }
        // Runtime person requests are holds, not extra declared work requiring samples.
        if !specs.iter().any(|step| step.path == path) {
            continue;
        }
        let durations = connection
            .prepare_cached(
                "SELECT duration FROM local_run_eta_samples
            WHERE definition_hash=?1 AND step_path=?2 ORDER BY duration",
            )?
            .query_map(params![hash, path], |row| row.get::<_, u64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if durations.is_empty() {
            return Ok(None);
        }
        samples = samples.min(durations.len());
        // Nearest-rank empirical quartiles remain integer durations, including n=1.
        let n = durations.len();
        weights.insert(path, (durations[n.div_ceil(4) - 1], durations[(3 * n).div_ceil(4) - 1]));
    }
    let edges = flattened_dependencies(&mission);
    let mut totals = BTreeMap::<String, (u64, u64)>::new();
    while totals.len() < weights.len() {
        let before = totals.len();
        for (path, &(low, high)) in &weights {
            if totals.contains_key(path) {
                continue;
            }
            let dependencies = edges
                .get(path)
                .into_iter()
                .flatten()
                .filter(|step| weights.contains_key(*step));
            if critical && dependencies.clone().any(|step| !totals.contains_key(step)) {
                continue;
            }
            let prefix = if critical {
                dependencies
                    .filter_map(|step| totals.get(step))
                    .fold((0, 0), |(a, b), &(c, d)| (a.max(c), b.max(d)))
            } else {
                (0, 0)
            };
            totals.insert(
                path.clone(),
                (prefix.0.saturating_add(low), prefix.1.saturating_add(high)),
            );
        }
        anyhow::ensure!(totals.len() > before, "cyclic ETA step dependencies");
    }
    if weights.is_empty() {
        return Ok(Some(
            json!({"remaining_low_ms":0,"remaining_high_ms":0,"basis":{"method":if critical {"critical-path"} else {"per-step"},"samples":0},"as_of":at}),
        ));
    }
    let (low, high) = totals.values().fold((0_u64, 0_u64), |(a, b), &(c, d)| {
        if critical {
            (a.max(c), b.max(d))
        } else {
            (a.saturating_add(c), b.saturating_add(d))
        }
    });
    Ok(Some(
        json!({"remaining_low_ms":low,"remaining_high_ms":high,"basis":{"method":if critical {"critical-path"} else {"per-step"},"samples":samples},"as_of":at}),
    ))
}
