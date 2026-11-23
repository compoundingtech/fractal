//! Host-local disk capacity: the daemon's minute timer owns collection; readers do no I/O.
use super::*;
use sha2::{Digest, Sha256};

const CADENCE_MS: u64 = 60_000;
const MAX_AGE_MS: u64 = 2 * CADENCE_MS;

#[derive(Default)]
struct Samples(Mutex<BTreeMap<String, Reading>>);
struct Reading {
    at: u64,
    available: Option<u64>,
}

fn stream(objective: &Objective) -> String {
    let Indicator::DiskFree { host, path, floor_bytes } = &objective.indicator else { unreachable!() };
    // A path, host or floor change starts a fresh history; budget-only edits reuse measurements.
    let identity = serde_json::to_vec(&(&objective.name, host, path, floor_bytes)).expect("string tuple");
    format!("disk-free/{}", hex::encode(Sha256::digest(identity)))
}

/// Called once per UTC minute from the native accounting task, never a request handler.
pub(crate) fn sample(store: &Store) -> anyhow::Result<()> {
    let declarations = store.declared_objectives()?;
    let samples = store.fractal.extensions.get::<Samples>();
    let mut keep = BTreeSet::new();
    for declaration in declarations {
        let Ok(objective) = &declaration.objective else { continue; };
        let Indicator::DiskFree { host, path, .. } = &objective.indicator else { continue; };
        if host != store.origin() { continue; }
        let key = stream(objective);
        keep.insert(key.clone());
        // statvfs f_bavail * f_frsize is available to the daemon's user, including dataset quotas.
        let available = crate::disk::disk_space(std::path::Path::new(path)).map(|space| space.available);
        let at = smallclaims::windows::budgets::unix_ms();
        if let Err(error) = &available {
            tracing::warn!(objective = %objective.name, %error, "disk objective sample unavailable");
        }
        let available = available.ok();
        // Negation lets the existing strict bad-above integrator express free < authored floor.
        // Persist a NULL on failure to end the previous held value immediately.
        smallclaims::windows::budgets::record_gauge(&key, at, available.map(|bytes| -(bytes as f64)))?;
        samples.0.lock().unwrap_or_else(PoisonError::into_inner)
            .insert(key, Reading { at, available });
    }
    samples.0.lock().unwrap_or_else(PoisonError::into_inner).retain(|key, _| keep.contains(key));
    Ok(())
}

pub(super) fn row(store: &Store, objective: &Objective, now: u64) -> Option<Value> {
    let Indicator::DiskFree { host, floor_bytes, .. } = &objective.indicator else { return None; };
    if host != store.origin() { return None; }
    let key = stream(objective);
    let samples = store.fractal.extensions.get::<Samples>();
    let guard = samples.0.lock().unwrap_or_else(PoisonError::into_inner);
    let reading = guard.get(&key);
    let fresh = reading.is_some_and(|reading| reading.at <= now && now - reading.at < MAX_AGE_MS);
    let available = reading.filter(|_| fresh).and_then(|reading| reading.available);
    let measured_at = reading.map(|reading| reading.at);
    drop(guard);
    let evidence = smallclaims::windows::budgets::gauge_report(&key, now, MAX_AGE_MS, -(*floor_bytes as f64))
        .unwrap_or_else(|error| json!({"available": false, "error": error.to_string(), "windows": {}}));
    let mut row = crate::objectives::native_budget(json!({
        "id": objective.name, "kind": "objective", "about": objective.about,
        "target": format!("available bytes ≥ {floor_bytes}"),
        "current": available.map_or_else(|| "unknown".to_owned(), |bytes| format!("{bytes} available bytes")),
        "available_bytes": available, "floor_bytes": floor_bytes, "host": host,
        "measured_at_unix_ms": measured_at,
        "source": {"kind": "native-disk-free", "cadence_ms": CADENCE_MS, "max_age_ms": MAX_AGE_MS},
    }), objective.budget.as_ref().expect("validated disk budget"), &evidence, CADENCE_MS);
    row["evidence"] = evidence;
    if available.is_none() {
        row["status"] = json!("no-data");
        row["remaining_percent"] = Value::Null;
        row["explain"] = json!(format!("{} Latest disk reading is unavailable, future-dated or at least two minutes old; missing sampling is unknown.",
            row["explain"].as_str().unwrap_or_default()));
    }
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::IntentInput;

    fn source(host: &str, path: &str) -> String {
        format!(r#"version 2
objective "disk/{host}/free" {{
  about "Available filesystem capacity"
  indicator "disk-free" {{
    host "{host}"
    path {path:?}
    floor-bytes 1000000000
  }}
  budget allowance=0.05 horizon="30d" min-coverage=0.9 min-samples=1 {{
    fast burn-rate=14.4 long="1h" short="5m"
    slow burn-rate=6 long="6h" short="30m"
  }}
}}"#)
    }

    fn store(source: &str) -> Store {
        let store = Store::open_memory("worker").unwrap();
        let intent = crate::graph::parse_intent(source, "worker").unwrap();
        let preview = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
        store.apply(&intent, &preview.subject_tokens, &intent.source_hash).unwrap();
        store
    }

    #[test]
    fn disk_free_samples_real_available_bytes_and_expires_without_reader_io() {
        let source = source("worker", "/");
        let objective = crate::objective::parse_document(&source).unwrap().remove(0);
        let store = store(&source);
        assert!(row(&store, &objective, smallclaims::windows::budgets::unix_ms()).unwrap()["available_bytes"].is_null());
        sample(&store).unwrap();
        let now = smallclaims::windows::budgets::unix_ms();
        let measured = row(&store, &objective, now).unwrap();
        assert!(measured["available_bytes"].as_u64().is_some());
        assert_eq!(measured["floor_bytes"], 1_000_000_000u64);
        let at = measured["measured_at_unix_ms"].as_u64().unwrap();
        let stale = row(&store, &objective, at + MAX_AGE_MS).unwrap();
        assert_eq!(stale["status"], "no-data");
        assert!(stale["available_bytes"].is_null());
        assert!(stale["remaining_percent"].is_null());
        assert_eq!(stale["measured_at_unix_ms"], at);
    }

    #[test]
    fn disk_free_only_samples_its_authored_host_and_failure_is_unknown() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let source = source("other", missing.to_str().unwrap());
        let objective = crate::objective::parse_document(&source).unwrap().remove(0);
        let store = store(&source);
        sample(&store).unwrap();
        assert!(row(&store, &objective, smallclaims::windows::budgets::unix_ms()).is_none());
        assert!(store.fractal.extensions.get::<Samples>().0.lock().unwrap().is_empty());
        let local_source = source.replace("other", "worker");
        let local = crate::objective::parse_document(&local_source).unwrap().remove(0);
        let local_store = self::store(&local_source);
        sample(&local_store).unwrap();
        let unknown = row(&local_store, &local, smallclaims::windows::budgets::unix_ms()).unwrap();
        assert_eq!(unknown["status"], "no-data");
        assert!(unknown["available_bytes"].is_null());
    }

    #[test]
    fn disk_free_path_floor_and_host_changes_never_reuse_history() {
        let objective = crate::objective::parse_document(&source("worker", "/")).unwrap().remove(0);
        let key = stream(&objective);
        for (host, path, floor_bytes) in [("other", "/", 1_000_000_000), ("worker", "/nix", 1_000_000_000), ("worker", "/", 2_000_000_000)] {
            let mut changed = objective.clone();
            changed.indicator = Indicator::DiskFree { host: host.into(), path: path.into(), floor_bytes };
            assert_ne!(stream(&changed), key);
        }
        let mut changed = objective.clone();
        changed.budget.as_mut().unwrap().allowance = 0.1;
        assert_eq!(stream(&changed), key);
    }
}
