//! Pending projections are rebuilt in separate writer loans, never in a batched action.
use super::*;
use std::time::{Duration, Instant};

pub(super) const API_TURN_BUDGET: Duration = Duration::from_secs(8);

enum Attempt<T> {
    NeedsDrain,
    Complete(T),
}

pub(super) fn execute<T: Send, E: From<FractalError> + Send>(
    graph: &GraphStore,
    fence: Option<RunMutationFence<'_>>,
    budget: Option<Duration>,
    action: impl FnOnce(&Transaction<'_>) -> std::result::Result<T, E> + Send,
) -> std::result::Result<T, E> {
    execute_with_turn_hook(graph, fence, budget, action, || {})
}

fn execute_with_turn_hook<T: Send, E: From<FractalError> + Send>(
    graph: &GraphStore,
    fence: Option<RunMutationFence<'_>>,
    budget: Option<Duration>,
    action: impl FnOnce(&Transaction<'_>) -> std::result::Result<T, E> + Send,
    mut after_turn: impl FnMut(),
) -> std::result::Result<T, E> {
    let deadline = budget.map(|budget| Instant::now() + budget);
    let mut action = Some(action);
    // The savepoint is gone, and the batched writer has returned, before we take
    // the projection gate. No callback or persisted request preparation runs on NeedsDrain.
    let attempt = graph.connection.batched(|transaction| {
        if pending(transaction, fence).map_err(E::from)?.is_some() {
            return Ok(Attempt::NeedsDrain);
        }
        #[cfg(any(test, feature = "test-support"))]
        let _role = rebuild_observer::role("action");
        action.take().expect("fence action runs once")(transaction).map(Attempt::Complete)
    }).map_err(|error| E::from(internal(error)))??;
    if let Attempt::Complete(value) = attempt {
        return Ok(value);
    }

    let _projection = if let Some(deadline) = deadline {
        graph.projection.lock_until(deadline)
            .ok_or_else(|| E::from(not_ready(graph)))?
    } else {
        graph.projection.lock()
    };
    loop {
        // This is an admission budget, not cancellation of a running transaction.
        // In particular an action is never queued to run after a not-ready response.
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(E::from(not_ready(graph)));
        }
        let mut writer = graph.connection.write();
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            drop(writer);
            return Err(E::from(not_ready(graph)));
        }
        let transaction = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| E::from(internal(error)))?;
        // Re-resolve the run/root on every turn, including the final action turn.
        if let Some(aggregate) = pending(&transaction, fence).map_err(E::from)? {
            #[cfg(any(test, feature = "test-support"))]
            let _role = rebuild_observer::role("fence-drain");
            rebuild_pending_run_tx(&transaction, &aggregate).map_err(E::from)?;
            let caught_up = !projection_work_pending_tx(&transaction).map_err(E::from)?
                && !graph.replication_projection_deferred();
            transaction.commit().map_err(|error| E::from(internal(error)))?;
            // Returning the loan invokes the commit/cache observer. Publish only after it.
            drop(writer);
            graph.runtime.projection_advanced(caught_up);
            after_turn();
        } else {
            #[cfg(any(test, feature = "test-support"))]
            let _role = rebuild_observer::role("action");
            let value = action.take().expect("fence action runs once")(&transaction)?;
            transaction.commit().map_err(|error| E::from(internal(error)))?;
            drop(writer);
            // The projection guard outlives both the action COMMIT and writer return.
            return Ok(value);
        }
    }
}

fn pending(
    transaction: &Transaction<'_>,
    fence: Option<RunMutationFence<'_>>,
) -> Result<Option<Aggregate>, FractalError> {
    match fence {
        None => Ok(None),
        Some(RunMutationFence::All) => {
            let value: Option<String> = transaction.query_row(
                "SELECT value FROM meta WHERE key GLOB ?1 ORDER BY key LIMIT 1",
                [PENDING_RUN_REBUILD], |row| row.get(0),
            ).optional().map_err(internal)?;
            value.map(|value| serde_json::from_str(&value).map_err(internal)).transpose()
        }
        Some(RunMutationFence::Run(subject)) => {
            if !projection_work_pending_tx(transaction)? {
                return Ok(None);
            }
            if let Some(root) = run_tree_of_tx(transaction, subject)?
                && let Some(aggregate) = pending_run_for_subject_tx(transaction, &format!("mission-run/{root}"))?
            {
                return Ok(Some(aggregate));
            }
            pending_run_for_subject_tx(transaction, subject)
        }
    }
}

fn not_ready(graph: &GraphStore) -> FractalError {
    // Committed markers describe the projected frontier, not admitted envelopes.
    let count = graph.readers.get().query_row(
        "SELECT COUNT(*) FROM meta WHERE key GLOB ?1",
        [PENDING_RUN_REBUILD], |row| row.get::<_, u64>(0),
    );
    match count {
        Ok(count) => FractalError::new("database-busy", "run projections are still catching up; retry the mutation")
            .with_detail("reason", "run-projection-drain")
            .with_detail("pending_parts", count),
        Err(error) => internal(error),
    }
}

#[cfg(test)]
mod tests;
