//! Offline index rollout probe. Input MUST be a checkpointed, quiescent, mode-0600 COPY.
//! Usage: cargo run --release --locked -p st3 --features test-support 
//!   --example first_open_index_cost -- COPY [N=1000] [K=20]
//! Never point this at a live store. Only private temporary copies are modified.

use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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

fn private_copy(source: &Path, target: &Path) -> Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = OpenOptions::new().write(true).create_new(true).mode(0o600).open(target)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
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
    let source = PathBuf::from(args.next().context("checkpointed offline COPY is required")?);
    let n = args.next().map(|v| v.to_string_lossy().parse::<usize>()).transpose()?.unwrap_or(1000);
    let k = args.next().map(|v| v.to_string_lossy().parse::<usize>()).transpose()?.unwrap_or(20);
    ensure!(args.next().is_none() && n > 0 && k > 0, "usage: COPY [N>0] [K>0]");
    let metadata = fs::symlink_metadata(&source)?;
    ensure!(metadata.is_file() && metadata.permissions().mode() & 0o777 == 0o600,
        "COPY must be a regular, non-symlink, mode-0600 offline file");
    ensure!(bytes(&sidecar(&source, "-wal"))? == 0 && !sidecar(&source, "-shm").exists(),
        "COPY must be checkpointed and closed, with no SHM or nonempty WAL; never use a live store");
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let path = directory.path().join("measurement.sqlite3");
    private_copy(&source, &path)?;
    let connection = configured(&path)?;
    let definitions = Store::first_open_index_definitions();
    ensure!(definitions.len() == 5, "expected the five owner-read indexes");
    for (name, _) in &definitions {
        connection.execute_batch(&format!("DROP INDEX IF EXISTS {name}"))?;
    }
    checkpoint(&connection, "TRUNCATE")?;
    // Identical pre-index image for timing the actual Store::open, not a sum of DDL times.
    let open_path = directory.path().join("open.sqlite3");
    private_copy(&path, &open_path)?;
    let through = smallclaims::store::current_index(&connection)?;
    let subjects = connection.prepare(
        "SELECT DISTINCT subject FROM claims WHERE subject>='agent/' AND subject<'agent0'
         AND store_index<=?1 ORDER BY subject LIMIT ?2",
    )?.query_map(params![through, i64::try_from(k)?], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(!subjects.is_empty(), "copy has no agent subjects");
    let before_owners = owners(&connection, &subjects, through, false)?;
    let before_inserts = inserts(&connection, n)?;
    checkpoint(&connection, "TRUNCATE")?;
    let page_size: u64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let autocheckpoint: u64 = connection.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))?;
    let mut builds = Vec::new();
    for (name, sql) in &definitions {
        let before = sizes(&path)?;
        let pages_before: u64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let stop = Arc::new(AtomicBool::new(false));
        let observer_stop = Arc::clone(&stop);
        let wal = sidecar(&path, "-wal");
        let observer = std::thread::spawn(move || -> Result<u64> {
            let mut peak = bytes(&wal)?;
            while !observer_stop.load(Ordering::Relaxed) {
                peak = peak.max(bytes(&wal)?);
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(peak.max(bytes(&wal)?))
        });
        let started = Instant::now();
        let result = connection.execute_batch(sql);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        stop.store(true, Ordering::Relaxed);
        let peak_wal_bytes = observer.join().map_err(|_| anyhow::anyhow!("WAL observer panicked"))??;
        result?;
        let after_create = sizes(&path)?;
        let pages_after: u64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let index_bytes = match connection.query_row(
            "SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name=?1", [name],
            |row| row.get::<_, u64>(0),
        ) {
            Ok(bytes) => json!({"method": "dbstat", "bytes": bytes}),
            Err(_) => json!({"method": "page_count_delta", "bytes": pages_after.saturating_sub(pages_before) * page_size,
                "caveat": "net database growth only; free-page reuse can undercount the index"}),
        };
        let checkpoint = checkpoint(&connection, "PASSIVE")?;
        builds.push(json!({"index": name, "create_ms": elapsed_ms, "index_bytes": index_bytes,
            "before": before, "after_create": after_create, "after_checkpoint": sizes(&path)?,
            "peak_wal_bytes_sampled": peak_wal_bytes, "checkpoint_passive": checkpoint}));
    }
    let after_inserts = inserts(&connection, n)?;
    let after_owners = owners(&connection, &subjects, through, true)?;
    let started = Instant::now();
    let store = Store::open(&open_path, "tester")?;
    let open_store_ms = started.elapsed().as_secs_f64() * 1000.0;
    drop(store);
    println!("{}", serde_json::to_string_pretty(&json!({
        "method": "offline private copies; deterministic subject order and claim selection; no source mutation; no subjects or bodies emitted",
        "source_bytes": metadata.len(), "n": n, "requested_k": k, "sample_k": subjects.len(),
        "page_size": page_size, "wal_autocheckpoint_pages": autocheckpoint,
        "builds": builds, "open_store_ms": open_store_ms,
        "claim_insert_before": before_inserts, "claim_insert_after": after_inserts,
        "owner_read_before": before_owners, "owner_read_after": after_owners,
        "caveats": [
            "Open-store is the synchronous Store::open startup-unavailability component on a separate pre-index copy; not full daemon restart/socket readiness, judge/replay, or other startup phases.",
            "Owner timings reuse the real first-page owner reader, not a rendered conversation page or main's old owner path; baseline removes only new INDEXED BY hints. EXPLAIN plans cover the identity and source queries actually taken, not auxiliary rival-source queries.",
            "Owner probe captures branch queries before timing, flushes statement cache but not SQLite/OS page caches; timings are warm data-cache, fresh statement-cache reads. Before/after order is fixed, not randomized. Probe-only SQL copying/hint transformation is included in owner time; production borrows the original SQL unchanged.",
            "Claims-table inserts use one rolled-back transaction and are not durable end-to-end claim ingestion. They exclude commit/fsync, signatures, envelopes, projection and replication; repeated same-subject runtime claims are a workload sample, not a fleet mix.",
            "Per-index CREATE is autocommit with production writer pragmas and default auto-checkpoint. Explicit PASSIVE checkpoints between builds differ from uninterrupted startup; use open_store_ms for the real combined startup installation.",
            "WAL peak is file length sampled every 2ms (a lower bound), not cumulative bytes written. Reused WAL allocation may exceed live frames. Checkpoint results are offline without concurrent writers/readers; they do not prove live checkpoint keep-up under daemon traffic.",
            "Existing index pages are dropped on the working copies without VACUUM; freelist reuse preserves production-copy layout but can reduce net file growth. dbstat is index allocation; page_count fallback is only net growth.",
            "Caller must supply a coherent, quiescent, checkpointed offline production-size copy. File mode/sidecar checks cannot prove that provenance. Temp copies are removed on exit."
        ]
    }))?);
    Ok(())
}
