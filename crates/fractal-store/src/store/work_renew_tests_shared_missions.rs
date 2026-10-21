#![allow(unused_imports)]
use crate::store::*;

pub struct Renewal {
    pub _directory: tempfile::TempDir,
    pub store: Store,
    pub subject: String,
}

pub fn request(key: &str, summary: Option<&str>) -> WorkRequest {
    WorkRequest {
        wait_ms: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: summary.map(Into::into),
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    }
}

pub fn writer_work(key: &str) -> smallclaims::sqlite::work::SqliteWork {
    WORK_ACTION_WRITER_WORK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .copied()
        .unwrap_or_else(|| panic!("the work action `{key}` ran no writer job"))
}

/// What the writer answered for a work action before a renewal's view moved to a reader: the
/// committed step row enriched inside a writer job.
pub fn view_enriched_in_writer(store: &Store, subject: &str) -> StepRunView {
    store
        .connection
        .batched(|transaction| -> rusqlite::Result<StepRunView> {
            let mut view = transaction.query_row(
                "SELECT subject, run_id, step_path, definition_hash, status, attempt, assignee, available_to, agentless, title, goals, worker_reported,
                        lease_owner, lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms, created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints
                 FROM step_runs WHERE subject=?1",
                [subject],
                step_run_from_row,
            )?;
            enrich_step_queue(transaction, &mut view)?;
            Ok(view)
        })
        .unwrap()
        .unwrap()
}

/// A view without the one field that moves with the clock while a claim is active.
pub fn timeless(view: &StepRunView) -> Value {
    let mut value = serde_json::to_value(view).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("execution_elapsed_ms");
    value
}
