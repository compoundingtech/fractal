//! Read-only observability. Pin freshness comes from the existing retention search, never a
//! second walk or a startup index build. Before the first successful pass it is unavailable.
use std::sync::{Mutex, OnceLock};
use serde::Serialize;

#[derive(Default)]
pub struct State {
    pub checkpoint_enabled: OnceLock<bool>,
    observations: Mutex<Observations>,
}

#[derive(Clone, Default, Serialize)]
pub struct Observations {
    pub passes_started: u64,
    pub passes_completed: u64,
    pub deleted_since_start: u64,
    pub scanned_since_start: u64,
    pub pin_sample: Option<PinSample>,
}

#[derive(Clone, Serialize)]
pub struct PinSample {
    pub sampled_at_unix_ms: u128,
    pub retention_cutoff_unix_ms: u128,
    pub protected_observations: u64,
    pub oldest_pin: Option<Pin>,
}

#[derive(Clone, Serialize)]
pub struct Pin {
    pub local_id: i64,
    pub observed_at_unix_ms: i64,
}

impl State {
    pub fn checkpoint_enabled(&self) -> Option<bool> { self.checkpoint_enabled.get().copied() }
    pub fn snapshot(&self) -> Observations {
        self.observations.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }
    pub(super) fn start(&self) {
        self.observations.lock().unwrap_or_else(|error| error.into_inner()).passes_started += 1;
    }
    pub(super) fn deleted(&self, rows: usize) {
        self.observations.lock().unwrap_or_else(|error| error.into_inner()).deleted_since_start += rows as u64;
    }
    pub(super) fn scanned(&self, rows: usize) {
        self.observations.lock().unwrap_or_else(|error| error.into_inner()).scanned_since_start += rows as u64;
    }
    pub(super) fn completed(&self, sample: PinSample) {
        let mut observations = self.observations.lock().unwrap_or_else(|error| error.into_inner());
        observations.passes_completed += 1;
        observations.pin_sample = Some(sample);
    }
}

pub(super) fn install(connection: &rusqlite::Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        "CREATE TEMP TABLE storage_local_appends (
             kind TEXT PRIMARY KEY, claims INTEGER NOT NULL, body_bytes INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE TEMP TRIGGER storage_local_append AFTER INSERT ON main.local_observations BEGIN
             INSERT INTO storage_local_appends VALUES (NEW.kind, 1, length(CAST(NEW.body AS BLOB)))
             ON CONFLICT(kind) DO UPDATE SET claims=claims+1, body_bytes=body_bytes+excluded.body_bytes;
         END;"
    )?;
    Ok(())
}

pub fn append_counters(store: &super::Store) -> anyhow::Result<Vec<smallclaims::store::append_census::AppendCounter>> {
    let connection = store.connection.write();
    let mut statement = connection.prepare_cached(
        "SELECT kind, claims, body_bytes FROM temp.storage_local_appends ORDER BY kind"
    )?;
    Ok(statement.query_map([], |row| Ok(smallclaims::store::append_census::AppendCounter {
        kind: row.get(0)?, claims: row.get(1)?, body_bytes: row.get(2)?,
    }))?.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_census_pins_are_unavailable_until_trim_then_sampled_without_new_searches() {
        let store = super::super::Store::open_memory("test").unwrap();
        assert!(store.storage_census.snapshot().pin_sample.is_none());
        store.connection.write().execute_batch(
            "INSERT INTO local_observations(id, after_store_index, subject, kind, body, observed_at_unix_ms) VALUES
                (1,0,'agent/test.launcher','agent.launcher-bound','{}',1),
                (2,0,'agent/test.pending','custom.test','{}',2),
                (3,0,'agent/test.evidence','custom.test','{}',3),
                (4,0,'agent/test.history','custom.test','{}',4),
                (5,0,'agent/test.history','custom.test','{}',20);
             INSERT INTO local_latest_slots VALUES ('agent/test.pending','custom.test','slot',0,'{}',2);
             INSERT INTO local_runtime_start_evidence VALUES ('test-operation',3);"
        ).unwrap();
        let counters = append_counters(&store).unwrap();
        assert_eq!(counters.iter().map(|counter| counter.claims).sum::<u64>(), 5);
        assert_eq!(counters.iter().map(|counter| counter.body_bytes).sum::<u64>(), 10);
        let trimmed = store.trim_local_observations_pass(10, 10, 10).unwrap();
        assert_eq!(trimmed.deleted, 1);
        let snapshot = store.storage_census.snapshot();
        assert_eq!(snapshot.passes_started, 1);
        assert_eq!(snapshot.passes_completed, 1);
        assert_eq!(snapshot.deleted_since_start, 1);
        let sample = snapshot.pin_sample.unwrap();
        assert_eq!(sample.retention_cutoff_unix_ms, 10);
        assert_eq!(sample.protected_observations, 3);
        assert_eq!(sample.oldest_pin.unwrap().local_id, 1);
        assert!(sample.sampled_at_unix_ms > 0);
        assert_eq!(append_counters(&store).unwrap().iter().map(|counter| counter.claims).sum::<u64>(), 5);
        let work = smallclaims::sqlite::work::SqliteWorkScope::start();
        assert!(store.storage_census.snapshot().pin_sample.is_some());
        assert_eq!(work.finish().statements, 0, "reading the pin sample must execute no SQL");
    }
}
