//! The part of `store::seat_status` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_store::store::seat_status::*;
pub use st3_store::store::seat_status::{MAX_TRANSITIONS, WINDOW_MS, enrich_harness, history_at, history_at_with_sources, history_source, observation_time, permission_blocked, state_run_since, transition_positions};

pub trait SeatStatusAttentionStore {
    /// Client status describes harness evidence. Work progress can prove delivery readiness
    /// internally, but never substitutes for an observation in this contract.
    fn observed_harness_at(
            &self,
            subject: &str,
            index: u64,
        ) -> Result<Option<crate::model::CurrentHarnessView>>;
}

impl SeatStatusAttentionStore for Store {

    fn observed_harness_at(
            &self,
            subject: &str,
            index: u64,
        ) -> Result<Option<crate::model::CurrentHarnessView>> {
        let connection = self.readers.get();
        let mut harness = current_harness_fold_at(&connection, subject, Some(index), false, false)?;
        if let Some(view) = harness.as_mut() {
            enrich_harness(&connection, subject, Some(index), view)?;
        }
        Ok(harness)
    }
}


#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::seat_status_tests_shared_seats::*;
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::seat_status_tests_shared_seats::{checked_harness, input, observe, runtime};
    use super::*;

    #[test]
    fn status_upgrade_preserves_since_from_legacy_observations() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms() - 1_000;
        let first = observe(&store, "one", "idle", at, "ready");
        store.connection.lock().unwrap().execute(
            "UPDATE claims SET body=json_remove(body, '$.fields.observed_since_ms', '$.fields.status_transition') WHERE id=?1",
            [first.id],
        ).unwrap();
        let before = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        observe(&store, "one", "idle", at + 1, "waiting");
        let after = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(before.since_unix_ms, at);
        assert_eq!(after.since_unix_ms, before.since_unix_ms);
    }
}
