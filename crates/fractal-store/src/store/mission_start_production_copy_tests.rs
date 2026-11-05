//! How long starting one scheduled mission run holds the writer on a production-size store copy
//! (#4810). Ignored: set `ST3_MISSION_START_PRODUCTION_COPY` to a disposable, checkpointed copy
//! of a store. The test writes to it. It starts a new occurrence of each of the missions the copy
//! started most often in its last day, twice each, with the requester, workspace, inputs and
//! parent (for a child round) of that mission's latest run, and prints one JSON receipt with each
//! start's writer-lend stages.

use super::*;
use std::time::Instant;

fn lend_receipt(took: std::time::Duration) -> Value {
    // The writer thread closes the lend after the connection comes back to it.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let (stages, _) = smallclaims::windows::holds::recent_staged()
        .into_iter()
        .rev()
        .find(|(stages, _)| stages.iter().any(|(stage, _)| *stage == "existing-occurrence"))
        .unwrap_or_default();
    json!({
        "wall_ms": took.as_millis(),
        "held_ms": stages.iter().map(|(_, spent)| spent.as_micros()).sum::<u128>() as f64 / 1000.0,
        "stages": stages.iter().filter(|(_, spent)| spent.as_micros() >= 100)
            .map(|(stage, spent)| json!([stage, spent.as_micros() as f64 / 1000.0])).collect::<Vec<_>>(),
    })
}

/// The presentation enrichment of each of a run's steps, part by part, timed on a reader.
fn view_parts(store: &Store, run: &MissionRunView) -> Value {
    let connection = store.readers.get();
    let now = now_ms();
    let mut parts = BTreeMap::<&str, f64>::new();
    let mut definitions = PinnedDefinitions::default();
    for step in &run.steps {
        let mut step = step.clone();
        let mut time = |part: &'static str, work: &mut dyn FnMut(&mut StepRunView) -> rusqlite::Result<()>| {
            let started = Instant::now();
            work(&mut step).unwrap();
            *parts.entry(part).or_default() += started.elapsed().as_micros() as f64 / 1000.0;
        };
        time("effective-state", &mut |step| apply_effective_step_state(&connection, step, now));
        time("execution-clock", &mut |step| {
            step_execution_clock_at(&connection, &step.subject, step.attempt, now, false).map(|_| ())
        });
        time("timeout-extension", &mut |step| {
            step_timeout_extension_at(&connection, &step.subject, step.attempt, now).map(|_| ())
        });
        time("summaries", &mut |step| enrich_step_summaries_at(&connection, step, now));
        time("nudge", &mut |step| enrich_step_nudge_at(&connection, step, now));
        time("wake", &mut |step| enrich_step_wake_at(&connection, step, now));
        time("definition", &mut |step| enrich_step_definition_cached(&connection, step, &mut definitions));
        time("handoff", &mut |step| adhoc_work::enrich_handoff(&connection, step, now));
        time("responses", &mut |step| person_work::enrich_responses(&connection, step));
    }
    let id = run.id.trim_start_matches("mission-run/");
    let mut timed = |part: &'static str, work: &mut dyn FnMut()| {
        let started = Instant::now();
        work();
        parts.insert(part, started.elapsed().as_micros() as f64 / 1000.0);
    };
    timed("run:provenance", &mut || { crate::provenance::read(&connection, &run.mission, &run.revision).unwrap(); });
    timed("run:loops", &mut || { loop_run_views_tx(&connection, run).unwrap(); });
    timed("run:outcome", &mut || { mission_run_outcome_tx(&connection, run).unwrap(); });
    timed("run:scheduler-fault", &mut || { first_readiness_fault_tx(&connection, &run.subject).unwrap(); });
    timed("run:header", &mut || { mission_run_header_tx(&connection, id).unwrap(); });
    timed("whole:created-view", &mut || { created_mission_run_view_tx(&connection, id).unwrap(); });
    timed("whole:presentation-view", &mut || { mission_run_view_tx(&connection, id).unwrap(); });
    json!({"parts_ms": parts})
}

#[test]
#[ignore = "needs ST3_MISSION_START_PRODUCTION_COPY, a disposable production-size store copy"]
fn scheduled_mission_start_writer_hold_on_production_copy() {
    let path = std::env::var_os("ST3_MISSION_START_PRODUCTION_COPY")
        .expect("ST3_MISSION_START_PRODUCTION_COPY names a disposable store copy");
    let store = Store::open(Path::new(&path), "mission-start-copy").unwrap();
    store.project_replication_backlog().unwrap();
    let missions = {
        let connection = store.readers.get();
        let mut statement = connection.prepare(
            "SELECT latest.mission_id, latest.requester, latest.workspace, latest.inputs, latest.root_revision,
                    latest.root_run_id, latest.parent_step_run FROM mission_runs latest
             JOIN (SELECT mission_id, MAX(created_at_unix_ms) AS created, COUNT(*) AS runs FROM mission_runs
                   WHERE CAST(created_at_unix_ms AS INTEGER) > (SELECT MAX(CAST(created_at_unix_ms AS INTEGER)) FROM mission_runs) - 86400000
                   GROUP BY mission_id) recent
               ON recent.mission_id=latest.mission_id AND recent.created=latest.created_at_unix_ms
             ORDER BY recent.runs DESC, latest.mission_id LIMIT 5",
        ).unwrap();
        statement
            .query_map([], |row| {
                let inputs = serde_json::from_str::<BTreeMap<String, MissionRunInput>>(&row.get::<_, String>(3)?)
                    .unwrap()
                    .into_iter()
                    .map(|(name, input)| (name, input.value))
                    .collect::<BTreeMap<_, _>>();
                let child = row.get::<_, Option<String>>(6)?.map(|parent_step_run| -> rusqlite::Result<_> {
                    Ok((row.get::<_, String>(4)?, row.get::<_, String>(5)?, parent_step_run))
                }).transpose()?;
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, inputs, child))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let probe = std::env::var("ST3_MISSION_START_PROBE").unwrap_or_else(|_| "production-copy-probe".into());
    let mut starts = Vec::new();
    for (index, (mission, requester, workspace, inputs, child)) in missions.iter().enumerate() {
        for attempt in 0..2 {
            let key = format!("{probe}:{index}:{attempt}");
            let subject = format!("mission-run/{}", &hex::encode(Sha256::digest(key.as_bytes()))[..32]);
            let request = MissionRunRequest {
                mission: mission.clone(),
                revision: None,
                workspace: workspace.clone(),
                requester: Some(requester.clone()),
                mode: None,
                inputs: inputs.clone(),
                idempotency_key: key,
            };
            let started = Instant::now();
            let context = child.as_ref().map(|(root_revision, root_run_id, parent_step_run)| ChildMissionContext {
                root_revision: root_revision.clone(),
                root_run_id: root_run_id.clone(),
                parent_step_run: parent_step_run.clone(),
                default_selector: None,
            });
            let result = store.create_mission_run_inner(&request, context, Some(&subject), None);
            let mut receipt = lend_receipt(started.elapsed());
            receipt["mission"] = json!(index);
            receipt["child"] = json!(child.is_some());
            receipt["steps"] = json!(result.as_ref().map(|run| run.steps.len()).ok());
            if let Ok(run) = &result {
                receipt["view_parts"] = view_parts(&store, run);
            }
            receipt["error"] = json!(result.err().map(|error| error.to_string()));
            starts.push(receipt);
        }
    }
    let count = |sql: &str| -> i64 {
        store.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-hff68dd108f7a owner=fractal_store::store::mission_start_production_copy_tests TODO=trace-statement-and-value-representation */ sql, [], |row| row.get(0)).unwrap()
    };
    println!(
        "{}",
        json!({"claims": count("SELECT COUNT(*) FROM claims"), "mission_runs": count("SELECT COUNT(*) FROM mission_runs"),
            "step_runs": count("SELECT COUNT(*) FROM step_runs"), "starts": starts})
    );
}
