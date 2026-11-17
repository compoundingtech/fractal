//! Host-local launch admission. The writer acknowledges only after every admitted spawn returns.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchFreeze {
    pub id: String,
    pub owner: String,
    pub reason: String,
    pub expires_at_unix_ms: u64,
}

pub(super) fn create_schema(connection: &rusqlite::Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS local_launch_freeze (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1), id TEXT NOT NULL,
        owner TEXT NOT NULL, reason TEXT NOT NULL, expires_at_unix_ms INTEGER NOT NULL)")?;
    Ok(())
}

fn now() -> Result<u64> {
    Ok(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis().try_into()?)
}

impl Store {
    pub fn launch_freeze(&self) -> Result<Option<LaunchFreeze>> {
        self.readers.get().query_row(
            "SELECT id,owner,reason,expires_at_unix_ms FROM local_launch_freeze WHERE singleton=1 AND expires_at_unix_ms>?1",
            [now()?], |row| Ok(LaunchFreeze { id: row.get(0)?, owner: row.get(1)?, reason: row.get(2)?, expires_at_unix_ms: row.get(3)? }))
            .optional().map_err(Into::into)
    }

    /// Keep this permit through physical spawn, including its final guard, not just dispatch.
    pub fn admit_launch(&self) -> Result<Option<tokio::sync::OwnedRwLockReadGuard<()>>> {
        let Ok(permit) = self.launch_admission.clone().try_read_owned() else { return Ok(None) };
        if self.launch_freeze()?.is_some() { return Ok(None); }
        Ok(Some(permit))
    }

    pub fn set_launch_freeze(&self, owner: &str, id: &str, reason: &str, duration_ms: u64) -> Result<LaunchFreeze> {
        anyhow::ensure!(!id.is_empty() && !reason.trim().is_empty(), "freeze id and reason are required");
        anyhow::ensure!((1..=1_800_000).contains(&duration_ms), "freeze duration must be 1..1800000 ms");
        let _exclusive = self.launch_admission.blocking_write();
        if let Some(current) = self.launch_freeze()? {
            anyhow::ensure!(current.owner == owner && current.id == id, "another launch freeze is active");
            return Ok(current);
        }
        let freeze = LaunchFreeze { id: id.into(), owner: owner.into(), reason: reason.into(), expires_at_unix_ms: now()?.checked_add(duration_ms).context("freeze expiry overflow")? };
        self.connection.write().execute(
            "INSERT OR REPLACE INTO local_launch_freeze(singleton,id,owner,reason,expires_at_unix_ms) VALUES(1,?1,?2,?3,?4)",
            rusqlite::params![freeze.id, freeze.owner, freeze.reason, freeze.expires_at_unix_ms])?;
        Ok(freeze)
    }

    pub fn clear_launch_freeze(&self, owner: &str, id: &str) -> Result<()> {
        let _exclusive = self.launch_admission.blocking_write();
        if let Some(current) = self.launch_freeze()? {
            anyhow::ensure!(current.owner == owner && current.id == id, "launch freeze owner or id does not match");
            self.connection.write().execute("DELETE FROM local_launch_freeze WHERE singleton=1 AND id=?1 AND owner=?2", rusqlite::params![id, owner])?;
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
        store.set_launch_freeze("person/operator", "activation-1", "replace daemon", 60_000).unwrap();
        assert!(store.admit_launch().unwrap().is_none());
        assert!(store.clear_launch_freeze("agent/other", "activation-1").is_err());
        store.clear_launch_freeze("person/operator", "activation-1").unwrap();
        assert!(store.admit_launch().unwrap().is_some());
    }

    #[test]
    fn expired_freeze_admits_without_clear() {
        let store = Store::open_memory("freeze").unwrap();
        store.set_launch_freeze("person/operator", "activation-1", "replace daemon", 60_000).unwrap();
        store.connection.write().execute("UPDATE local_launch_freeze SET expires_at_unix_ms=0", []).unwrap();
        assert!(store.launch_freeze().unwrap().is_none());
        assert!(store.admit_launch().unwrap().is_some());
    }

    #[test]
    fn freeze_survives_store_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("store.sqlite");
        let store = Store::open(&path, "freeze").unwrap();
        let freeze = store.set_launch_freeze("person/operator", "activation-1", "replace daemon", 60_000).unwrap();
        drop(store);
        let store = Store::open(&path, "freeze").unwrap();
        assert_eq!(store.launch_freeze().unwrap(), Some(freeze));
        assert!(store.admit_launch().unwrap().is_none());
    }
}
