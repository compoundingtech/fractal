//! Offline index rollout probe. Input MUST be a checkpointed, quiescent, mode-0600 COPY.
//! Usage: cargo run --release --locked -p st3 --features test-support
//!   --example first_open_index_cost -- --in-place-scratch COPY [N=1000] [K=20]
//! Never point this at a live store. Explicitly mutates the supplied disposable scratch copy.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use st3::store::Store;

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}

fn bytes(path: &Path) -> Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn sizes(path: &Path) -> Result<Value> {
    Ok(json!({"main_bytes": bytes(path)?, "wal_bytes": bytes(&sidecar(path, "-wal"))?}))
}

fn with_wal_peak<T>(path: &Path, work: impl FnOnce() -> Result<T>) -> Result<(T, f64, u64)> {
    let stop = Arc::new(AtomicBool::new(false));
    let observer_stop = Arc::clone(&stop);
    let wal = sidecar(path, "-wal");
    let initial = bytes(&wal)?;
    let observer = std::thread::spawn(move || -> Result<u64> {
        let mut peak = initial;
        while !observer_stop.load(Ordering::Relaxed) {
            peak = peak.max(bytes(&wal)?);
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(peak.max(bytes(&wal)?))
    });
    let started = Instant::now();
    let result = work();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    stop.store(true, Ordering::Relaxed);
    let peak = observer.join().map_err(|_| anyhow::anyhow!("WAL observer panicked"))??;
    Ok((result?, elapsed_ms, peak))
}

fn configured(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        PRAGMA foreign_keys=ON; PRAGMA cache_size=-32768; PRAGMA busy_timeout=5000;")?;
    smallclaims::store::configure_projection_writer(&connection)?;
    Ok(connection)
}

fn checkpoint(connection: &Connection, mode: &str) -> Result<Value> {
    let started = Instant::now();
    let (busy, log, checkpointed) = connection.query_row(
        &format!("PRAGMA wal_checkpoint({mode})"), [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
    )?;
    Ok(json!({"busy": busy, "log_frames": log, "checkpointed_frames": checkpointed,
        "remaining_frames": (log - checkpointed).max(0),
        "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0}))
}

// No commits, replica envelopes, folding, or projection work: isolate claims-table/index writes.
// Rollback restores the data and sequence. The caller checkpoints before later build measurements.
fn inserts(connection: &Connection, n: usize) -> Result<Value> {
    let representative: i64 = connection.query_row(
        "SELECT store_index FROM claims WHERE kind='runtime.observed'
         AND typeof(json_extract(body, CASE WHEN json_type(body,'$.fields') IS NULL
             THEN '$.runtime_id' ELSE '$.fields.runtime_id' END))='text'
         AND typeof(json_extract(body, CASE WHEN json_type(body,'$.fields') IS NULL
             THEN '$.incarnation_id' ELSE '$.fields.incarnation_id' END))='text'
         ORDER BY store_index LIMIT 1", [], |row| row.get(0),
    ).context("copy needs a runtime.observed claim with runtime_id and incarnation_id")?;
    let transaction = connection.unchecked_transaction()?;
    let elapsed_ms = {
        let mut insert = transaction.prepare(
            "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             SELECT ?1,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
             FROM claims WHERE store_index=?2",
        )?;
        let started = Instant::now();
        for i in 0..n {
            ensure!(insert.execute(params![format!("first-open-cost-ada-{i}"), representative])? == 1,
                "representative claim disappeared");
        }
        started.elapsed().as_secs_f64() * 1000.0
    };
    transaction.rollback()?;
    Ok(json!({"inserts": n, "elapsed_ms": elapsed_ms, "us_per_insert": elapsed_ms * 1000.0 / n as f64,
        "method": "one rolled-back transaction; runtime.observed with both identity fields; same existing subject/batch/body; unique synthetic IDs; no commit/fsync or application projection cost"}))
}

fn owners(connection: &Connection, subjects: &[String], through: u64, indexed: bool) -> Result<Value> {
    let samples = subjects.iter().enumerate().map(|(ordinal, subject)| -> Result<Value> {
        let mut sample = Store::first_open_owner_cost(connection, subject, through, indexed)?;
        sample["sample_ordinal"] = json!(ordinal);
        Ok(sample)
    }).collect::<Result<Vec<_>>>()?;
    let total_ms: f64 = samples.iter().filter_map(|sample| sample["elapsed_ms"].as_f64()).sum();
    Ok(json!({"label": if indexed { "same owner query with the new indexes" }
        else { "same owner query without the new indexes" },
        "subjects": samples.len(), "total_ms": total_ms, "samples": samples}))
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    ensure!(args.next().as_deref() == Some(std::ffi::OsStr::new("--in-place-scratch")),
        "explicit --in-place-scratch consent is required; never use a live store");
    let path = PathBuf::from(args.next().context("checkpointed offline scratch COPY is required")?);
    let n = args.next().map(|v| v.to_string_lossy().parse::<usize>()).transpose()?.unwrap_or(1000);
    let k = args.next().map(|v| v.to_string_lossy().parse::<usize>()).transpose()?.unwrap_or(20);
    ensure!(args.next().is_none() && n > 0 && k > 0,
        "usage: --in-place-scratch COPY [N>0] [K>0]");
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(metadata.is_file() && metadata.permissions().mode() & 0o777 == 0o600,
        "COPY must be a regular, non-symlink, mode-0600 offline file");
    ensure!(bytes(&sidecar(&path, "-wal"))? == 0 && !sidecar(&path, "-shm").exists(),
        "COPY must be checkpointed and closed, with no SHM or nonempty WAL; never use a live store");
    let definitions = Store::first_open_index_definitions();
    ensure!(definitions.len() == 5, "expected the five owner-read indexes");
    // Refuse before configuring a writer or modifying schema. Never silently normalize an
    // already-indexed input: actual Store::open must install all five indexes for this run.
    let preflight = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    for (name, _) in &definitions {
        let exists: bool = preflight.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name=?1)",
            [name], |row| row.get(0),
        )?;
        ensure!(!exists, "scratch already contains {name}; supply a fresh pre-index copy");
    }
    drop(preflight);
    let connection = configured(&path)?;
    let through = smallclaims::store::current_index(&connection)?;
    let subjects = connection.prepare(
        "SELECT DISTINCT subject FROM claims WHERE subject>='agent/' AND subject<'agent0'
         AND store_index<=?1 ORDER BY subject LIMIT ?2",
    )?.query_map(params![through, i64::try_from(k)?], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(!subjects.is_empty(), "copy has no agent subjects");
    // 1. Baseline, with all five indexes absent and no persistent synthetic claims.
    let before_owners = owners(&connection, &subjects, through, false)?;
    let before_inserts = inserts(&connection, n)?;
    checkpoint(&connection, "TRUNCATE")?;
    drop(connection);
    // 2. The real combined installation in Store::open; no per-index checkpoints inserted.
    let open_before = sizes(&path)?;
    let (store, open_store_ms, open_peak_wal_bytes) =
        with_wal_peak(&path, || Store::open(&path, "tester"))?;
    let open_after = sizes(&path)?;
    let connection = configured(&path)?;
    let open_checkpoint = checkpoint(&connection, "PASSIVE")?;
    let open_after_checkpoint = sizes(&path)?;
    drop(store);
    // 3. Indexed owner/read-insert comparisons before any index is dropped.
    let after_owners = owners(&connection, &subjects, through, true)?;
    let after_inserts = inserts(&connection, n)?;
    checkpoint(&connection, "TRUNCATE")?;
    let page_size: u64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let autocheckpoint: u64 = connection.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))?;
    // 4. Allocation of each index from the actual initial Store::open installation.
    let mut allocations = definitions.iter().map(|(name, _)| {
        let allocation = connection.query_row(
            "SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name=?1", [name],
            |row| row.get::<_, u64>(0),
        ).ok();
        json!({"index": name, "method": if allocation.is_some() { "dbstat" }
            else { "page_count_delta_on_rebuild" }, "bytes": allocation})
    }).collect::<Vec<_>>();
    // 5. Rebuild ONE index at a time. This is not first-install timing: DROP frees pages
    // for reuse and other four indexes remain present. Do not VACUUM or make more DB copies.
    let mut builds = Vec::new();
    for (ordinal, (name, sql)) in definitions.iter().enumerate() {
        connection.flush_prepared_statement_cache();
        connection.execute_batch(&format!("DROP INDEX {name}"))?;
        checkpoint(&connection, "TRUNCATE")?;
        let before = sizes(&path)?;
        let pages_before: u64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let (_, elapsed_ms, peak_wal_bytes) =
            with_wal_peak(&path, || { connection.execute_batch(sql)?; Ok(()) })?;
        let after_create = sizes(&path)?;
        let pages_after: u64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        if allocations[ordinal]["bytes"].is_null() {
            allocations[ordinal]["bytes"] = json!(pages_after.saturating_sub(pages_before) * page_size);
            allocations[ordinal]["caveat"] = json!("net database growth on rebuild only; free-page reuse can undercount or yield zero");
        }
        let checkpoint = checkpoint(&connection, "PASSIVE")?;
        builds.push(json!({"index": name, "rebuild_create_ms": elapsed_ms,
            "before": before, "after_create": after_create, "after_checkpoint": sizes(&path)?,
            "peak_wal_bytes_sampled": peak_wal_bytes, "checkpoint_passive": checkpoint}));
    }
    println!("{}", serde_json::to_string_pretty(&json!({
        "method": "in-place disposable offline scratch; no additional database copies; deterministic subject order and claim selection; no subjects or bodies emitted",
        "source_bytes": metadata.len(), "n": n, "requested_k": k, "sample_k": subjects.len(),
        "page_size": page_size, "wal_autocheckpoint_pages": autocheckpoint,
        "open_store_ms": open_store_ms, "open_before": open_before, "open_after": open_after,
        "open_peak_wal_bytes_sampled": open_peak_wal_bytes,
        "open_checkpoint_passive": open_checkpoint, "open_after_checkpoint": open_after_checkpoint,
        "index_allocations": allocations, "per_index_rebuilds": builds,
        "claim_insert_before": before_inserts, "claim_insert_after": after_inserts,
        "owner_read_before": before_owners, "owner_read_after": after_owners,
        "caveats": [
            "Input is modified in place and kept on exit with the five indexes installed. Reruns refuse this indexed input. Caller must supply a coherent, quiescent, checkpointed, disposable offline production-size copy; checks cannot prove provenance.",
            "Open-store is the actual synchronous Store::open startup-unavailability component, not full daemon restart/socket readiness, judge/replay, or other startup phases. Baseline probes precede it and warm caches.",
            "Owner timings reuse the real first-page owner reader, not a rendered conversation page or main's old owner path; baseline removes only new INDEXED BY hints. EXPLAIN plans cover identity/source queries actually taken, not auxiliary rival-source queries.",
            "Owner probe captures branch queries before timing, flushes statement cache but not SQLite/OS page caches; timings are warm data-cache, fresh statement-cache reads. Before/after order is fixed. Probe SQL copying/hint transformation is included; production borrows SQL unchanged.",
            "Claims-table inserts use one rolled-back transaction, not durable end-to-end ingestion: no commit/fsync, signatures, envelopes, projection or replication cost. Repeated same-subject runtime claims are a sample, not a fleet mix.",
            "Per-index times are DROP/CREATE rebuilds after the actual combined install. Other four indexes remain present; DROP frees pages for reuse, and no VACUUM is run. These times are not original first-install per-index costs. dbstat allocations are captured before any DROP; fallback page_count deltas reflect rebuild net growth only and may be zero.",
            "Per-index rebuilds use production writer pragmas, default auto-checkpoint and explicit PASSIVE checkpoints between builds. Use open_store_ms and its WAL measurements for the uninterrupted initial installation.",
            "WAL peaks are file lengths sampled every 2ms (lower bounds), not cumulative write volume. WAL allocation can exceed live frames. Offline checkpoints without concurrent traffic cannot prove live checkpoint keep-up."
        ]
    }))?);
    Ok(())
}
