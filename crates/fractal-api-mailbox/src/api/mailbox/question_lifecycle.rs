//! Owner-local native-question generation retirement. No wall-clock answer/expiry is inferred.
use super::*;
use crate::HarnessQuestionsStore;
use crate::store::harness_questions::QuestionGeneration;
use fractal_schema::harness_questions::Status;

const BATCH: usize = 128;

/// Runs even after observation is disabled: an already-published open still needs an end.
/// A periodic sweep is necessary when no successor ever reconnects after abrupt driver loss.
/// A missing lease is indeterminate, not evidence of death: older observations
/// do not carry the exact wrapper/provider generation needed for retirement.
pub fn start(state: &AppState) -> tokio::task::JoinHandle<()> {
    let state = state.clone();
    tokio::spawn(crate::task_cpu::account("questions.lifecycle", async move {
        let mut cursor = String::new();
        loop {
            let store = state.store.clone();
            let after = cursor.clone();
            match tokio::task::spawn_blocking(move || store.harness_question_generations(&after, BATCH)).await {
                Ok(Ok(batch)) => {
                    for generation in &batch {
                        let fence = Fence::new(&generation.agent, &generation.runtime_incarnation, "questions");
                        let dir = authority::source_agent_dir(&state, &fence);
                        let admission = fractal_drivers::harness_state::record_lock_admission(&dir).await;
                        let result = match admission {
                            Ok(admission) => {
                                let checked = state.clone();
                                let generation = generation.clone();
                                tokio::task::spawn_blocking(move || {
                                    let _admission = admission;
                                    fractal_drivers::harness_state::with_record_lock(&dir, || {
                                        reconcile_generation(&checked.store, &generation, || {
                                            authority::question_generation_current(&checked, &generation)
                                        })
                                    })
                                }).await.map_err(anyhow::Error::from).and_then(|result| result)
                            }
                            Err(error) => Err(error),
                        };
                        match result {
                            Ok(true) => signal_local_change(&state),
                            Ok(false) => {}
                            Err(error) => eprintln!("fractal: question generation reconciliation: {error:#}"),
                        }
                    }
                    if batch.len() == BATCH {
                        cursor = batch.last().expect("nonempty batch").question.clone();
                        continue;
                    }
                    cursor.clear();
                }
                result => eprintln!("fractal: question generation scan unavailable: {result:?}"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }))
}

/// Caller holds the provider record lock, serializing with ingestion and takeover.
/// Successful terminal/timeout closures retain their first resolution in every race.
pub(super) fn reconcile_generation(
    store: &Store,
    generation: &QuestionGeneration,
    still_current: impl FnOnce() -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    if store.harness_question_generation(&generation.question)?.as_ref() != Some(generation) {
        return Ok(false);
    }
    let current = store.harness_question_held(&generation.question)?;
    let Some(current) = current.filter(|question| question.resolution.is_none()) else {
        store.forget_harness_question_generation(generation)?;
        return Ok(false);
    };
    anyhow::ensure!(current.agent == generation.agent
        && current.runtime_incarnation == generation.runtime_incarnation
        && current.session == generation.session, "question generation binding changed");
    if still_current()? { return Ok(false); }
    let (_, changed) = store.resolve_harness_question(
        &current.id, Status::Cancelled, &current.revision, &BTreeMap::new(), None,
    )?;
    // Delete only after the immutable resolution is durable; a crash just replays retirement.
    store.forget_harness_question_generation(generation)?;
    Ok(changed)
}

