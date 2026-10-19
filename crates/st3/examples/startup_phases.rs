//! Time the daemon's startup store phases on a private, previously copied store.
//! Usage: cargo run --release -p st3 --example startup_phases -- CLONE NODE [FLEET_ID]
//! Runs what `st up` runs before it binds its sockets, in the same order, then what it starts at
//! bind, and prints each phase's wall time and the statements that cost the most. It starts no
//! runtime, seat or listener. Never supply a live store: this opens and writes CLONE.

use st3::MissionsStore;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use st3::store::Store;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().context("private clone path is required")?);
    let node = args
        .next()
        .context("the node name is required")?
        .into_string()
        .map_err(|_| anyhow::anyhow!("the node must be UTF-8"))?;
    let fleet = args.next().and_then(|fleet| fleet.into_string().ok());
    ensure!(!path.is_symlink(), "clone must not be a symlink");
    ensure!(
        path.metadata()?.permissions().mode() & 0o777 == 0o600,
        "clone must be mode 0600"
    );
    let mut phases = Vec::new();
    let mut phase = |name: &'static str, work: &mut dyn FnMut() -> Result<Value>| -> Result<()> {
        let started = Instant::now();
        let detail = st3::performance::task(name, &mut *work)?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("{name}: {elapsed_ms:.0} ms");
        phases.push(json!({"phase": name, "elapsed_ms": elapsed_ms, "detail": detail}));
        Ok(())
    };
    let mut store = None;
    phase("open-store", &mut || {
        let opened = Store::open(&path, &node)?;
        if let Some(fleet) = &fleet {
            opened.bind_fleet(fleet)?;
        }
        store = Some(opened);
        Ok(Value::Null)
    })?;
    let store = store.expect("the store opened");
    phase("judge-claims", &mut || Ok(json!(store.judge_claims(true)?)))?;
    phase("validate-replication-backlog", &mut || {
        let admission = store.validate_replication_backlog()?;
        Ok(json!({"unknown": admission.unknown, "invalid": admission.invalid}))
    })?;
    phase("apply-replication-repairs", &mut || {
        Ok(json!(store.apply_replication_repairs()?))
    })?;
    phase("settle-runs", &mut || {
        Ok(json!(store.settle_runs_for_canonical_replay()?))
    })?;
    phase("project-replication-backlog", &mut || {
        Ok(json!(store.project_replication_backlog_with_progress(
            "startup/project-replication-backlog",
            |_| {}
        )?))
    })?;
    phase("complete-replication-activation", &mut || {
        Ok(json!(store.complete_replication_activation()?))
    })?;
    let report = st3::performance::snapshot();
    let queries = report["queries"]
        .as_array()
        .map(|queries| queries.iter().take(25).cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"phases": phases, "queries": queries}))?
    );
    Ok(())
}
