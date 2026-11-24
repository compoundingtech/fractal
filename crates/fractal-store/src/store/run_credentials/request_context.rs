//! Preserve authenticated run scope across async routing and blocking graph writer admission.
//! This is an additional commit-time fence, not a replacement for the endpoint's ordinary gates.
use super::{FractalError, RunBinding, internal};
use rusqlite::{Transaction, params};
use std::cell::RefCell;

tokio::task_local! { static REQUEST: RunBinding; }
thread_local! { static BLOCKING: RefCell<Option<RunBinding>> = const { RefCell::new(None) }; }

pub async fn scope<F: Future>(binding: RunBinding, future: F) -> F::Output {
    REQUEST.scope(binding, future).await
}

pub fn capture() -> Option<RunBinding> {
    REQUEST.try_with(Clone::clone).ok().or_else(|| BLOCKING.with(|slot| slot.borrow().clone()))
}

/// Install only for the callback's lifetime, including unwinding and nested blocking callbacks.
pub fn with<T>(binding: Option<RunBinding>, callback: impl FnOnce() -> T) -> T {
    struct Restore(Option<RunBinding>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BLOCKING.with(|slot| { slot.replace(self.0.take()); });
        }
    }
    let _restore = Restore(BLOCKING.with(|slot| slot.replace(binding)));
    callback()
}

pub(crate) fn admit_tx(tx: &Transaction<'_>) -> Result<(), FractalError> {
    if let Ok(result) = REQUEST.try_with(|binding| check_tx(tx, binding)) { return result; }
    BLOCKING.with(|slot| {
        slot.borrow().as_ref().map_or(Ok(()), |binding| check_tx(tx, binding))
    })
}

fn check_tx(tx: &Transaction<'_>, binding: &RunBinding) -> Result<(), FractalError> {
    let current: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM step_runs step JOIN mission_runs mission ON mission.id=step.run_id
         WHERE step.subject=?1 AND step.attempt=?2 AND step.run_id=?3
           AND step.status IN ('ready','working','blocked','waiting-person')
           AND step.generation_id=mission.current_generation_id AND mission.phase<>'terminal')",
        params![binding.step_run, binding.attempt, binding.run.trim_start_matches("mission-run/")],
        |row| row.get(0),
    ).map_err(internal)?;
    if current { Ok(()) } else {
        Err(FractalError::new("run-credential-ended", "the authenticated run attempt ended or was replaced before effect admission"))
    }
}
