//! Transitional gate-slot lifecycle ingress. Receipts remain authoritative in the producer spool.
use std::{collections::BTreeSet, path::{Path, PathBuf}, sync::Arc};
use anyhow::{Context as _, Result, ensure};
use notify::Watcher as _;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use crate::store::{Store, append_claim_tx, latest_claim_of_kind_tx};

pub const KIND: &str = "gate.admission.observed";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    id: String,
    host: String,
    class: String,
    at_ms: u64,
    onset_ms: u64,
    epoch: String,
    coverage: String,
    outcome: String,
    reason: String,
}

fn import(store: &Store, bytes: &[u8]) -> Result<bool> {
    let receipt: Receipt = serde_json::from_slice(bytes).context("decode gate admission receipt")?;
    ensure!(receipt.schema == "gate-slot.admission/v1", "unsupported gate admission schema");
    ensure!(receipt.host == store.origin(), "gate admission receipt is not from this host");
    ensure!(receipt.coverage == "logical", "gate admission coverage must remain logical");
    ensure!(matches!(receipt.class.as_str(), "gate" | "heavy" | "eval"), "unsupported gate admission class");
    ensure!(matches!(receipt.outcome.as_str(), "accepted" | "granted" | "failed" | "excluded"), "unsupported gate admission outcome");
    ensure!(receipt.onset_ms > 0 && receipt.at_ms >= receipt.onset_ms, "non-monotonic gate admission timestamp");
    ensure!(!receipt.id.is_empty() && !receipt.epoch.is_empty() && receipt.id.len() <= 256 && receipt.epoch.len() <= 256, "invalid gate admission identity");
    ensure!(receipt.id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.')), "invalid gate admission id");
    let subject = format!("gate-admission/{}/{}/{}", receipt.host, receipt.id, receipt.outcome);
    let body = serde_json::json!({"fields": receipt, "evidence": []});
    let mut connection = store.connection.write();
    let tx = connection.transaction()?;
    if let Some(previous) = latest_claim_of_kind_tx(&tx, &subject, KIND)? {
        ensure!(previous.body.get("fields") == body.get("fields"), "conflicting immutable gate admission receipt");
        return Ok(false);
    }
    append_claim_tx(&tx, store.origin(), &subject, KIND, None, &body, &[], None)?;
    tx.commit()?;
    Ok(true)
}

fn ingest(store: &Store, directory: &Path, seen: &mut BTreeSet<PathBuf>) -> Result<usize> {
    let mut imported = 0;
    for entry in std::fs::read_dir(directory).context("read gate admission spool")? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || path.extension().is_none_or(|ext| ext != "json") || seen.contains(&path) { continue; }
        let bytes = std::fs::read(&path).context("read gate admission receipt")?;
        imported += usize::from(import(store, &bytes)?);
        seen.insert(path);
    }
    Ok(imported)
}

/// Watches before replaying, closing the scan/watch race. No polling or silent fallback.
/// Import failure stops this source loudly; unconsumed authoritative files remain for recovery.
pub async fn run(store: Arc<Store>, directory: PathBuf, notify: Arc<Notify>, events: watch::Sender<u64>) -> Result<()> {
    let directory = directory.canonicalize().context("resolve gate admission spool")?;
    let (changed, mut changes) = watch::channel(Ok::<(), String>(()));
    let watched = directory.clone();
    let mut watcher = notify::RecommendedWatcher::new(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = &event {
            if matches!(event.kind, notify::EventKind::Access(_)) { return; }
            if !event.need_rescan() && !event.paths.iter().any(|path| path != &watched && path.extension().is_some_and(|ext| ext == "json")) { return; }
        }
        changed.send_if_modified(|state| {
            // Once a watcher error occurs, do not overwrite it with a later event.
            if state.is_err() { return false; }
            *state = event.map(|_| ()).map_err(|error| error.to_string());
            true
        });
    }, notify::Config::default().with_follow_symlinks(false))?;
    watcher.watch(&directory, notify::RecursiveMode::NonRecursive)?;
    let mut seen = BTreeSet::new();
    loop {
        let store = store.clone();
        let directory = directory.clone();
        let (next_seen, imported) = tokio::task::spawn_blocking(move || {
            let result = ingest(&store, &directory, &mut seen);
            (seen, result)
        }).await?;
        seen = next_seen;
        if imported? > 0 {
            notify.notify_one();
            events.send_modify(|generation| *generation = generation.saturating_add(1));
        }
        changes.changed().await.context("gate admission watch closed")?;
        changes.borrow_and_update().clone().map_err(anyhow::Error::msg)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn receipt(outcome: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"schema":"gate-slot.admission/v1", "id":"epoch-1-2", "host":"example-host", "class":"heavy", "onset_ms":100, "at_ms":200, "epoch":"epoch", "coverage":"logical", "outcome":outcome, "reason":"test"})).unwrap()
    }
    #[test]
    fn durable_import_is_idempotent_and_rejects_changed_receipts() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("graph.db"), "example-host").unwrap();
        assert!(import(&store, &receipt("accepted")).unwrap());
        assert!(!import(&store, &receipt("accepted")).unwrap());
        let mut changed: serde_json::Value = serde_json::from_slice(&receipt("accepted")).unwrap();
        changed["at_ms"] = 300.into();
        assert!(import(&store, &serde_json::to_vec(&changed).unwrap()).is_err());
        assert!(import(&store, &receipt("failed")).unwrap());
    }
    #[test]
    fn only_own_host_logical_admission_evidence_is_admitted() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("graph.db"), "other-host").unwrap();
        assert!(import(&store, &receipt("granted")).is_err());
    }

    #[test]
    fn spool_replay_survives_restart_without_deleting_authority() {
        let root = tempfile::tempdir().unwrap();
        let spool = root.path().join("spool");
        std::fs::create_dir(&spool).unwrap();
        std::fs::write(spool.join("one.accepted.json"), receipt("accepted")).unwrap();
        std::fs::write(spool.join("one.failed.json"), receipt("failed")).unwrap();
        std::fs::write(spool.join("incomplete.tmp"), b"{").unwrap();
        let path = root.path().join("graph.db");
        {
            let store = Store::open(&path, "example-host").unwrap();
            assert_eq!(ingest(&store, &spool, &mut BTreeSet::new()).unwrap(), 2);
        }
        let store = Store::open(&path, "example-host").unwrap();
        assert_eq!(ingest(&store, &spool, &mut BTreeSet::new()).unwrap(), 0);
        assert!(spool.join("one.accepted.json").is_file());
        assert_eq!(store.claims_by_kinds_after(&[KIND], None, 0, 10).unwrap().claims.len(), 2);
    }
}
