//! The part of `store::revision_seats` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_missions::store::revision_seats::*;
pub use fractal_missions::store::revision_seats::{carry_completed_agents_tx};

pub trait RevisionSeatsStore {
    /// Check completed declaration carry before the API publishes the candidate revision.
    /// Cutover rechecks under its writer transaction, so this preview grants no authority.
    fn validate_revision_seat_carry(
            &self,
            current: &MissionRunView,
            next: &MissionSpec,
        ) -> Result<(), St3Error>;
}

impl RevisionSeatsStore for Store {

    fn validate_revision_seat_carry(
            &self,
            current: &MissionRunView,
            next: &MissionSpec,
        ) -> Result<(), St3Error> {
        let old = self
            .mission_spec(
                current.mission.trim_start_matches("mission/"),
                Some(&current.revision),
            )
            .map_err(internal)?
            .ok_or_else(|| {
                St3Error::new(
                    "missing-mission-revision",
                    "the current mission revision is unavailable",
                )
            })?;
        let old = crate::mission::run_mission(old, current.after.as_deref())?;
        let next = crate::mission::run_mission(next.clone(), current.after.as_deref())?;
        let compatible = compatible_step_paths(&old, &next);
        let mut compatible = carried_revision_step_paths(&old, &next, &current.steps, compatible);
        if is_failed_terminal(&current.status, &current.phase) {
            retain_reopened_step_paths(&next, &current.steps, &mut compatible);
        }
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction().map_err(internal)?;
        plan_completed_agents_tx(
            &transaction,
            current,
            &next,
            &compatible,
            "run-generation/00000000000000000000000000000000",
        )?;
        Ok(())
    }
}

