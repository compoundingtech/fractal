//! Host-local launch admission. A successful freeze acknowledges physical spawn quiescence.
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

pub const DEFAULT_WAIT_MS: u64 = 5_000;
pub const MAX_WAIT_MS: u64 = 10_000;
pub const MAX_DURATION_MS: u64 = 1_800_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchFreeze {
    pub id: String,
    pub owner: String,
    pub reason: String,
    pub expires_at_unix_ms: u64,
}

#[derive(Default)]
struct AdmissionState {
    active: usize,
    setting: bool,
    generation: u64,
}

#[derive(Default)]
pub struct Admission {
    state: StdMutex<AdmissionState>,
    changed: Condvar,
}

/// Owned across the launch worker thread, including the backend's final physical guard.
pub struct LaunchPermit(Arc<Admission>);
impl Drop for LaunchPermit {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

struct Exclusive(Arc<Admission>);
impl Drop for Exclusive {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.setting = false;
        state.generation = state.generation.wrapping_add(1);
        self.0.changed.notify_all();
    }
}

impl Admission {
    fn exclusive(self: &Arc<Self>, wait_ms: u64) -> Result<Exclusive, FractalError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.setting {
            return Err(FractalError::new("launch-freeze-busy", "another launch freeze mutation is in progress"));
        }
        state.setting = true;
        let (mut state, _) = self.changed.wait_timeout_while(state, Duration::from_millis(wait_ms), |state| state.active != 0)
            .unwrap_or_else(PoisonError::into_inner);
        if state.active != 0 {
            state.setting = false;
            self.changed.notify_all();
            return Err(FractalError::new("launch-freeze-timeout", "earlier launches did not quiesce within the freeze wait bound; no lease was armed"));
        }
        drop(state);
        Ok(Exclusive(self.clone()))
    }
}

pub(super) fn create_schema(connection: &rusqlite::Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h323e0b4f2aa0 owner=fractal_store::store::launch_freeze TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_launch_freeze (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1), id TEXT NOT NULL,
        owner TEXT NOT NULL, reason TEXT NOT NULL, expires_at_unix_ms INTEGER NOT NULL)")?;
    Ok(())
}

fn now() -> Result<u64, FractalError> {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(internal)?.as_millis().try_into().map_err(internal)
}

fn dependency(origin: &str) -> String { format!("launch-freeze/{origin}") }

fn changed_tx(transaction: &Transaction<'_>, origin: &str, freeze: &LaunchFreeze, decision: &str) -> Result<(), FractalError> {
    insert_local_observation_tx(transaction, origin, &ClaimInput {
        subject: dependency(origin), kind: "runtime.reconcile-decision".into(),
        actor: Some(freeze.owner.clone()),
        fields: BTreeMap::from([
            ("key".into(), json!(freeze.id)),
            ("decision".into(), json!(decision)),
            ("reason".into(), json!(freeze.reason)),
        ]),
        evidence: Vec::new(), idempotency_key: None, expected_subject: None,
    }, now_ms())?;
    Ok(())
}

impl Store {
    pub fn launch_freeze(&self) -> Result<Option<LaunchFreeze>, FractalError> {
        smallclaims::touched::note_read(|| dependency(self.origin()));
        self.readers.get().query_row(
            "SELECT id,owner,reason,expires_at_unix_ms FROM local_launch_freeze WHERE singleton=1 AND expires_at_unix_ms>?1",
            [now()?], |row| Ok(LaunchFreeze { id: row.get(0)?, owner: row.get(1)?, reason: row.get(2)?, expires_at_unix_ms: row.get(3)? }))
            .optional().map_err(internal)
    }

    /// The only launch admission predicate; do not release until physical spawn has returned.
    pub fn admit_launch(&self) -> Result<Option<LaunchPermit>, FractalError> {
        // Even the in-memory drain is a dependency: its completion wakes parked effects.
        smallclaims::touched::note_read(|| dependency(self.origin()));
        let generation = {
            let state = self.launch_admission.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.setting { return Ok(None); }
            state.generation
        };
        // Never hold the admission mutex during a reader-pool or SQLite wait.
        let frozen = self.launch_freeze()?.is_some();
        let mut state = self.launch_admission.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.setting || state.generation != generation || frozen { return Ok(None); }
        state.active += 1;
        Ok(Some(LaunchPermit(self.launch_admission.clone())))
    }

    pub fn set_launch_freeze(&self, owner: &str, id: &str, reason: &str, duration_ms: u64, wait_ms: u64) -> Result<LaunchFreeze, FractalError> {
        let result = self.set_launch_freeze_inner(owner, id, reason, duration_ms, wait_ms);
        self.launch_admission_changed(owner)?;
        result
    }

    fn set_launch_freeze_inner(&self, owner: &str, id: &str, reason: &str, duration_ms: u64, wait_ms: u64) -> Result<LaunchFreeze, FractalError> {
        if id.is_empty() || id.len() > 256 || reason.trim().is_empty() || reason.len() > 4096
            || !(1..=MAX_DURATION_MS).contains(&duration_ms) || wait_ms > MAX_WAIT_MS {
            return Err(FractalError::new("launch-freeze-invalid", "id (1..256 bytes), reason (1..4096 bytes), duration (1..1800000 ms) and wait (0..10000 ms) are required"));
        }
        let _exclusive = self.launch_admission.exclusive(wait_ms)?;
        if let Some(current) = self.launch_freeze()? {
            if current.owner != owner || current.id != id {
                return Err(FractalError::new("launch-freeze-conflict", "another launch freeze is active"));
            }
            return Ok(current);
        }
        let producer_open: bool = self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM local_runtime_starts WHERE dispatched=1 AND cleanup_verified=0
             AND (finished=0 OR (COALESCE(json_extract(result,'$.verification'),'') NOT IN
             ('operation-incarnation','operation-ended','operation-retired')
             AND COALESCE(json_extract(result,'$.producer_closed'),0)<>1)))",
            [], |row| row.get(0)).map_err(internal)?;
        if producer_open {
            return Err(FractalError::new("launch-freeze-producer-uncertain", "a detached launch producer remains unverified; no lease was armed"));
        }
        let freeze = LaunchFreeze { id: id.into(), owner: owner.into(), reason: reason.into(), expires_at_unix_ms: now()?.saturating_add(duration_ms) };
        let mut writer = self.connection.write();
        let transaction = writer.transaction().map_err(internal)?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h4a2e34db3d3e owner=fractal_store::store::launch_freeze TODO=bound-or-reference-values-and-reclamation */
            "INSERT OR REPLACE INTO local_launch_freeze(singleton,id,owner,reason,expires_at_unix_ms) VALUES(1,?1,?2,?3,?4)",
            rusqlite::params![freeze.id, freeze.owner, freeze.reason, freeze.expires_at_unix_ms]).map_err(internal)?;
        changed_tx(&transaction, self.origin(), &freeze, "launch-freeze-set")?;
        transaction.commit().map_err(internal)?;
        Ok(freeze)
    }

    pub fn clear_launch_freeze(&self, owner: &str, id: &str) -> Result<(), FractalError> {
        let result = self.clear_launch_freeze_inner(owner, id);
        self.launch_admission_changed(owner)?;
        result
    }

    /// Called after the exclusion guard drops, including on every bounded refusal.
    /// HTTP cancellation cannot stop the synchronous blocking producer midway through this.
    fn launch_admission_changed(&self, owner: &str) -> Result<(), FractalError> {
        let mut writer = self.connection.write();
        let transaction = writer.transaction().map_err(internal)?;
        insert_local_observation_tx(&transaction, self.origin(), &ClaimInput {
            subject: dependency(self.origin()), kind: "runtime.reconcile-decision".into(),
            actor: Some(owner.into()),
            fields: BTreeMap::from([("decision".into(), json!("launch-admission-drain-ended"))]),
            evidence: Vec::new(), idempotency_key: None, expected_subject: None,
        }, now_ms())?;
        transaction.commit().map_err(internal)?;
        Ok(())
    }

    fn clear_launch_freeze_inner(&self, owner: &str, id: &str) -> Result<(), FractalError> {
        let _exclusive = self.launch_admission.exclusive(DEFAULT_WAIT_MS)?;
        if let Some(current) = self.launch_freeze()? {
            if current.owner != owner || current.id != id {
                return Err(FractalError::new("launch-freeze-owner", "launch freeze owner or id does not match"));
            }
            let mut writer = self.connection.write();
            let transaction = writer.transaction().map_err(internal)?;
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h24dc70bc2f33 owner=fractal_store::store::launch_freeze TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_launch_freeze WHERE singleton=1 AND id=?1 AND owner=?2", rusqlite::params![id, owner]).map_err(internal)?;
            changed_tx(&transaction, self.origin(), &current, "launch-freeze-cleared")?;
            transaction.commit().map_err(internal)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freeze_defers_until_clear_and_rejects_foreign_clear() {
        let store = Store::open_memory("freeze").unwrap();
        store.set_launch_freeze("person/operator", "activation-1", "replace daemon", 60_000, DEFAULT_WAIT_MS).unwrap();
        assert!(store.admit_launch().unwrap().is_none());
        assert!(store.clear_launch_freeze("agent/other", "activation-1").is_err());
        store.clear_launch_freeze("person/operator", "activation-1").unwrap();
        assert!(store.admit_launch().unwrap().is_some());
    }

    #[test]
    fn expired_freeze_admits_without_clear() {
        let store = Store::open_memory("freeze").unwrap();
        store.set_launch_freeze("person/operator", "activation-1", "replace daemon", 60_000, DEFAULT_WAIT_MS).unwrap();
        store.connection.write().execute("UPDATE local_launch_freeze SET expires_at_unix_ms=0", []).unwrap();
        assert!(store.launch_freeze().unwrap().is_none());
        assert!(store.admit_launch().unwrap().is_some());
    }

    #[test]
    fn freeze_survives_store_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("store.sqlite");
        let store = Store::open(&path, "freeze").unwrap();
        let freeze = store.set_launch_freeze("person/operator", "activation-1", "replace daemon", 60_000, DEFAULT_WAIT_MS).unwrap();
        drop(store);
        let store = Store::open(&path, "freeze").unwrap();
        assert_eq!(store.launch_freeze().unwrap(), Some(freeze));
        assert!(store.admit_launch().unwrap().is_none());
    }

    #[test]
    fn timeout_releases_admission_and_never_arms_a_lease() {
        let store = Store::open_memory("freeze").unwrap();
        let permit = store.admit_launch().unwrap().unwrap();
        let error = store.set_launch_freeze("person/operator", "activation", "replace daemon", 60_000, 0).unwrap_err();
        assert_eq!(error.code, "launch-freeze-timeout");
        assert!(store.launch_freeze().unwrap().is_none());
        drop(permit);
        assert!(store.admit_launch().unwrap().is_some());
    }

    #[test]
    fn set_waits_for_prior_spawn_before_acknowledging() {
        let store = Arc::new(Store::open_memory("freeze").unwrap());
        let permit = store.admit_launch().unwrap().unwrap();
        let setter = store.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            tx.send(()).unwrap();
            setter.set_launch_freeze("person/operator", "activation", "replace daemon", 60_000, DEFAULT_WAIT_MS)
        });
        rx.recv().unwrap();
        drop(permit);
        thread.join().unwrap().unwrap();
        assert!(store.admit_launch().unwrap().is_none());
    }

    #[test]
    fn uncertain_detached_producer_refuses_freeze_without_leaving_admission_closed() {
        let store = Store::open_memory("freeze").unwrap();
        store.connection.write().execute(
            "INSERT INTO local_runtime_starts(id,subject,owner,body,dispatched,finished,result)
             VALUES('uncertain','agent/example/worker','previous-daemon','{}',1,1,?1)",
            [r#"{"error":"producer timed out","producer_closed":false}"#]).unwrap();
        let error = store.set_launch_freeze("person/operator", "activation", "replace daemon", 60_000, 0).unwrap_err();
        assert_eq!(error.code, "launch-freeze-producer-uncertain");
        assert!(store.launch_freeze().unwrap().is_none());
        assert!(store.admit_launch().unwrap().is_some());
    }
}
