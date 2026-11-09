//! Offline-only, opt-in measurement. Copy this file and its module declaration unchanged
//! to the comparison tree. Opening uses the daemon API and can commit migrations/backfills;
//! the measured rebuilds themselves always roll back. No explicit projection drain/replay.
use super::*;
use std::time::Instant;

// One test-only hashing contract for both crates; copy this helper along with this file.
#[path = "../../../fractal-missions/src/store/tests/huge_part_digest.rs"]
mod huge_part_digest;

pub(super) fn open_real_part_store() -> Store {
    let supplied = std::env::var_os("FRACTAL_REAL_PART_DB")
        .expect("set FRACTAL_REAL_PART_DB to an offline scratch database file");
    let path = std::path::PathBuf::from(supplied);
    assert!(path.to_string_lossy().contains("/scratch-"), "refusing non-scratch path");
    let path = path.canonicalize().expect("scratch database must already exist");
    assert!(path.to_string_lossy().contains("/scratch-"), "refusing non-scratch canonical path");
    assert!(path.is_file(), "FRACTAL_REAL_PART_DB must name a database file");
    let before = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open scratch preflight read-only");
    print_open_state(&before, "before_open");
    // Restored backups require their assigned fresh writer, not an invented bench origin.
    let origin: String = before.query_row(
        "SELECT value FROM meta WHERE key='backup_restore_writer'", [], |row| row.get(0),
    ).optional().unwrap().unwrap_or_else(|| "real-part-bench".into());
    drop(before);
    let started = Instant::now();
    let store = Store::open(&path, origin).expect("daemon-style scratch Store::open");
    println!("real_part_open open_ms={} startup_projection=daemon_open_projections open_may_commit=true explicit_drain=false explicit_replay=false", started.elapsed().as_millis());
    print_open_state(&store.readers.get(), "after_open");
    let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
    println!("real_part_storage db_bytes={} wal_bytes={}",
        std::fs::metadata(&path).unwrap().len(),
        std::fs::metadata(wal).map_or(0, |metadata| metadata.len()));
    store
}

fn print_open_state(connection: &Connection, phase: &str) {
    let target = smallclaims::store::current_index(connection).unwrap();
    let mut statement = connection.prepare(
        "SELECT aggregate,status,last_good_store_index,error_code FROM projection_health WHERE aggregate='graph'",
    ).unwrap();
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?,
        row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, Option<String>>(3)?))).unwrap() {
        let (aggregate, status, frontier, error) = row.unwrap();
        println!("real_part_health phase={phase} target={target} aggregate={aggregate:?} status={status} frontier={frontier} error={error:?}");
    }
    let mut summary = connection.prepare(
        "SELECT status,error_code,COUNT(*) FROM projection_health GROUP BY status,error_code ORDER BY status,error_code",
    ).unwrap();
    for row in summary.query_map([], |row| Ok((row.get::<_, String>(0)?,
        row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?))).unwrap() {
        let (status, error, count) = row.unwrap();
        println!("real_part_health_summary phase={phase} status={status} error={error:?} rows={count}");
    }
    let pending: i64 = connection.query_row(
        "SELECT COUNT(*) FROM meta WHERE key GLOB 'projection-run-rebuild:*'", [], |row| row.get(0),
    ).unwrap();
    println!("real_part_frontier phase={phase} target={target} pending_parts={pending}");
}

pub(super) struct PartShape {
    pub(super) top: String,
    pub(super) runs: BTreeSet<String>,
    pub(super) generations: usize,
    pub(super) steps: usize,
    pub(super) asks: usize,
    pub(super) claims: i64,
    projected: BTreeMap<&'static str, i64>,
    subjects: BTreeSet<String>,
}

// Keep the discovery identical to rebuild_run_part_tx: projected links plus creating
// claims, root-owned asks only in the root part, recursively all descendants otherwise.
fn part_runs(transaction: &Transaction<'_>, top: &str) -> BTreeSet<String> {
    let root = run_tree_of_tx(transaction, &format!("mission-run/{top}")).unwrap()
        .expect("part must have a known root");
    let mut statement = transaction.prepare_cached(
        "SELECT subject,json_extract(body,'$.fields.parent_step_run') FROM claims
         WHERE kind='mission-run.created' AND json_extract(body,'$.fields.root_mission_run')=?1",
    ).unwrap();
    let created = statement.query_map([format!("mission-run/{root}")], |row| {
        Ok((row.get::<_, String>(0)?.trim_start_matches("mission-run/").to_owned(),
            row.get::<_, Option<String>>(1)?))
    }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let asks_of = |run: &str| {
        let mut asks = string_column_tx(transaction,
            "SELECT id FROM mission_runs WHERE root_run_id=?1 AND id<>?1 AND parent_step_run IS NULL", run).unwrap();
        asks.extend(string_column_tx(transaction,
            "SELECT json_extract(body,'$.fields.run') FROM claims WHERE kind='work.person-asked'
             AND json_extract(body,'$.fields.owner_run')=?1", &format!("mission-run/{run}")).unwrap()
            .into_iter().map(|run| run.trim_start_matches("mission-run/").to_owned()));
        if run == root {
            asks.extend(created.iter().filter(|(child, parent)| parent.is_none() && *child != root)
                .map(|(child, _)| child.clone()));
        }
        asks
    };
    let mut runs = BTreeSet::from([top.to_owned()]);
    let mut pending = if top == root {
        let asks = asks_of(top);
        runs.extend(asks.iter().cloned());
        asks
    } else { vec![top.to_owned()] };
    while let Some(run) = pending.pop() {
        let mut found = asks_of(&run);
        if run != root {
            found.extend(string_column_tx(transaction,
                "SELECT id FROM mission_runs WHERE root_run_id=?1 AND id<>?1", &run).unwrap());
        }
        for generation in run_generation_ids_tx(transaction, &run).unwrap() {
            let start = format!("step-run/{generation}/");
            let end = format!("step-run/{generation}0");
            let mut statement = transaction.prepare_cached(
                "SELECT id FROM mission_runs WHERE parent_step_run>=?1 AND parent_step_run<?2",
            ).unwrap();
            found.extend(statement.query_map(params![start, end], |row| row.get::<_, String>(0))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap());
            found.extend(created.iter().filter(|(_, parent)|
                parent.as_deref().is_some_and(|parent| parent.starts_with(&start)))
                .map(|(child, _)| child.clone()));
        }
        for run in found {
            if runs.insert(run.clone()) { pending.push(run); }
        }
    }
    runs
}

fn part_shape(transaction: &Transaction<'_>, top: String) -> PartShape {
    let runs = part_runs(transaction, &top);
    let mut generations = BTreeSet::new();
    let mut proposals = BTreeSet::new();
    let mut asks = BTreeSet::new();
    let mut projected = BTreeMap::new();
    for run in &runs {
        generations.extend(run_generation_ids_tx(transaction, run).unwrap());
        proposals.extend(string_column_tx(transaction,
            "SELECT id FROM revision_proposals WHERE run_id=?1", run).unwrap());
        proposals.extend(string_column_tx(transaction,
            "SELECT subject FROM claims WHERE kind='revision-proposal.created'
             AND json_extract(body,'$.fields.run')=?1", &format!("mission-run/{run}")).unwrap()
            .into_iter().map(|id| id.trim_start_matches("revision-proposal/").to_owned()));
        asks.extend(string_column_tx(transaction,
            "SELECT json_extract(body,'$.fields.run') FROM claims WHERE kind='work.person-asked'
             AND json_extract(body,'$.fields.owner_run')=?1", &format!("mission-run/{run}")).unwrap()
            .into_iter().map(|id| id.trim_start_matches("mission-run/").to_owned()));
        asks.extend(string_column_tx(transaction,
            "SELECT id FROM mission_runs WHERE root_run_id=?1 AND id<>?1 AND parent_step_run IS NULL", run).unwrap());
        asks.extend(string_column_tx(transaction,
            "SELECT subject FROM claims WHERE kind='mission-run.created'
             AND json_extract(body,'$.fields.root_mission_run')=?1 AND subject<>?1
             AND json_extract(body,'$.fields.parent_step_run') IS NULL",
            &format!("mission-run/{run}")).unwrap().into_iter()
            .map(|id| id.trim_start_matches("mission-run/").to_owned()));
        for table in ["mission_runs", "run_generations", "step_runs", "revision_proposals", "mission_run_deadlines", "mission_run_after"] {
            let column = if table == "mission_runs" { "id" } else { "run_id" };
            let count: i64 = transaction.query_row(/* sqlite-large-value: allow-for-now lv-hce2a04018181 owner=fractal_store::store::real_part_rebuild_bench TODO=trace-statement-and-value-representation */ &format!("SELECT COUNT(*) FROM {table} WHERE {column}=?1"),
                [run], |row| row.get(0)).unwrap();
            *projected.entry(table).or_insert(0) += count;
        }
    }
    let subjects = runs.iter().map(|id| format!("mission-run/{id}"))
        .chain(generations.iter().map(|id| format!("run-generation/{id}")))
        .chain(proposals.iter().map(|id| format!("revision-proposal/{id}")))
        .collect::<BTreeSet<_>>();
    // Same claim membership as claims_in_replay_order_tx, without fetching bodies or
    // sorting all claims before the cold rebuild. Subject and step ranges are disjoint.
    let suffix = "AND NOT EXISTS(SELECT 1 FROM replica_records WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')";
    let mut by_subject = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-h38b527d3f169 owner=fractal_store::store::real_part_rebuild_bench TODO=trace-statement-and-value-representation */ &format!(
        "SELECT COUNT(*) FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.subject=?1 {suffix}")).unwrap();
    let mut claims = 0_i64;
    for subject in &subjects {
        claims += by_subject.query_row(/* sqlite-large-value: allow-for-now lv-h488b50c88484 owner=fractal_store::store::real_part_rebuild_bench TODO=trace-statement-and-value-representation */ [subject], |row| row.get::<_, i64>(0)).unwrap();
    }
    let mut steps = BTreeSet::new();
    for generation in &generations {
        let start = format!("step-run/{generation}/");
        let end = format!("step-run/{generation}0");
        claims += transaction.query_row(/* sqlite-large-value: allow-for-now lv-h05f33f795c89 owner=fractal_store::store::real_part_rebuild_bench TODO=trace-statement-and-value-representation */ &format!(
            "SELECT COUNT(*) FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject>=?1 AND claims.subject<?2 {suffix}"), params![start, end],
            |row| row.get::<_, i64>(0)).unwrap();
        let mut statement = transaction.prepare_cached(
            "SELECT DISTINCT subject FROM claims WHERE subject>=?1 AND subject<?2",
        ).unwrap();
        steps.extend(statement.query_map(params![start, end], |row| row.get::<_, String>(0))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap());
    }
    // Include projected steps even when their creating claims have not arrived.
    for run in &runs {
        steps.extend(string_column_tx(transaction, "SELECT subject FROM step_runs WHERE run_id=?1", run).unwrap());
    }
    let asks = asks.intersection(&runs).count();
    PartShape { top, runs, generations: generations.len(), steps: steps.len(),
        asks, claims, projected, subjects }
}

pub(super) fn select_real_part(transaction: &Transaction<'_>) -> PartShape {
    if let Ok(subject) = std::env::var("FRACTAL_REAL_PART_RUN") {
        assert!(subject.starts_with("mission-run/"), "FRACTAL_REAL_PART_RUN must be a mission-run/... subject");
        let Some(Aggregate::RunPart(top)) = run_part_of_tx(transaction, &subject).unwrap() else {
            panic!("subject must resolve to a part under a never-ended root");
        };
        return part_shape(transaction, top);
    }
    let mut statement = transaction.prepare_cached(
        "SELECT id FROM mission_runs WHERE root_run_id=id AND parent_step_run IS NULL
         AND status NOT IN ('completed','failed','cancelled') AND phase<>'terminal' ORDER BY id",
    ).unwrap();
    let roots = statement.query_map([], |row| row.get::<_, String>(0)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    roots.into_iter().filter(|root| !run_ever_ended_tx(transaction, root).unwrap())
        .map(|root| part_shape(transaction, root)).max_by(|a, b| a.claims.cmp(&b.claims).then_with(|| b.top.cmp(&a.top)))
        .expect("no live never-ended root part found")
}

pub(super) fn print_shape(shape: &PartShape) {
    println!("real_part_shape subject=mission-run/{} runs={} generations={} steps={} asks={} claims={} projected={:?}",
        shape.top, shape.runs.len(), shape.generations, shape.steps, shape.asks, shape.claims, shape.projected);
}

#[test]
#[ignore = "requires an offline /scratch- database; daemon-style open can mutate it"]
fn measure_real_part_rebuild() {
    let (digest_after_open, digest_after_rebuild) =
        match std::env::var("FRACTAL_REAL_PART_WHOLE_DIGEST") {
            Ok(mode) => match mode.as_str() {
                "1" | "after_rebuild" => (false, true),
                "both" => (true, true),
                _ => panic!("FRACTAL_REAL_PART_WHOLE_DIGEST must be 1, after_rebuild, or both; unset it to disable (got {mode:?})"),
            },
            Err(std::env::VarError::NotPresent) => (false, false),
            Err(error) => panic!("invalid FRACTAL_REAL_PART_WHOLE_DIGEST: {error}; expected 1, after_rebuild, or both"),
        };
    let store = open_real_part_store();
    if digest_after_open {
        let mut connection = store.connection.write();
        // No rebuild writes: pin the committed post-open state for the entire scan.
        let transaction = connection.transaction().unwrap();
        huge_part_digest::print_whole_store_digest(&transaction, "after_open");
        transaction.rollback().expect("post-open read snapshot must roll back");
    }
    let shape = {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let shape = select_real_part(&transaction);
        transaction.rollback().unwrap();
        shape
    };
    print_shape(&shape);
    println!("real_part_cache rep_1=first_rebuild_not_os_cold selection_reads_warm_indexes=true cache_flush=false");
    for rep in 1..=3 {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let started = Instant::now();
        let result = rebuild_run_part_tx(&transaction, &shape.top);
        let elapsed = started.elapsed();
        if result.is_ok() {
            println!("real_part_digest_rep rep={rep}");
            huge_part_digest::print_part_digest(&transaction, &shape.runs, &shape.subjects);
            if rep == 1 && digest_after_rebuild {
                huge_part_digest::print_whole_store_digest(&transaction, "after_rebuild");
            }
        }
        transaction.rollback().expect("rebuild transaction must roll back");
        result.expect("rebuild failed (transaction rolled back)");
        println!("real_part claims={} runs={} steps={} rebuild_ms={} rep={rep} gate_15s={}",
            shape.claims, shape.runs.len(), shape.steps, elapsed.as_millis(),
            if elapsed < std::time::Duration::from_secs(15) { "pass" } else { "fail" });
    }
}
