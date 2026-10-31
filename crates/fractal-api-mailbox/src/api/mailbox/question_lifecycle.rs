//! Owner-local native-question generation retirement. No wall-clock answer/expiry is inferred.
use super::*;
use crate::HarnessQuestionsStore;
use crate::store::harness_questions::QuestionGeneration;
use fractal_schema::harness_questions::Status;

const BATCH: usize = 128;

/// Runs even after observation is disabled: an already-published open still needs an end.
/// A periodic sweep is necessary when no successor ever reconnects after abrupt driver loss.
pub fn start(state: &AppState) -> tokio::task::JoinHandle<()> {
    let state = state.clone();
    tokio::spawn(crate::task_cpu::account("questions.lifecycle", async move {
        let mut recovery = Some(String::new());
        let mut recovery_failed = false;
        let mut cursor = String::new();
        loop {
            if let Some(after) = recovery.as_ref() {
                let store = state.store.clone();
                let origin = state.node.clone();
                let after = after.clone();
                match tokio::task::spawn_blocking(move || {
                    store.unleased_local_harness_questions(&origin, &after, BATCH)
                }).await {
                    Ok(Ok(batch)) => {
                        for question in &batch {
                            let result = retire_unleased(&state, question.clone()).await;
                            if let Err(error) = result {
                                recovery_failed = true;
                                eprintln!("fractal: question generation recovery: {error:#}");
                            }
                        }
                        recovery = if batch.len() == BATCH {
                            batch.last().map(|question| question.id.clone())
                        } else if std::mem::take(&mut recovery_failed) {
                            // Retry a whole pass after transient errors, without starving
                            // later keys behind an unavailable provider.
                            Some(String::new())
                        } else { None };
                    }
                    result => eprintln!("fractal: question generation recovery unavailable: {result:?}"),
                }
            }
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

async fn retire_unleased(state: &AppState, question: crate::api::HarnessQuestion) -> anyhow::Result<()> {
    let fence = Fence::new(&question.agent, &question.runtime_incarnation, "questions");
    let dir = authority::source_agent_dir(state, &fence);
    let admission = fractal_drivers::harness_state::record_lock_admission(&dir).await?;
    let checked = state.clone();
    let changed = tokio::task::spawn_blocking(move || {
        let _admission = admission;
        fractal_drivers::harness_state::with_record_lock(&dir, || {
            // A replay may have registered its still-live generation since the recovery scan.
            if checked.store.harness_question_generation(&question.id)?.is_some() {
                return Ok(false);
            }
            let observed = checked.store.latest_claim(
                &question.id, Some(fractal_schema::harness_questions::OBSERVED),
            )?;
            if !observed.is_some_and(|claim| claim.id == question.revision && claim.origin == checked.node) {
                return Ok(false);
            }
            let selected = checked.store.desired_subjects_named(std::slice::from_ref(&question.agent))?;
            if selected.first().and_then(|subject| subject.member.as_ref()).is_some_and(|member| {
                member.driver.as_deref().is_some_and(|driver| driver != "omp") || member.host != checked.node
            }) {
                return Ok(false);
            }
            // Pre-lease local OMP observations have no recoverable wrapper generation. The
            // upgrade revokes that observation authority, never inventing a native answer.
            checked.store.resolve_harness_question(
                &question.id, Status::Cancelled, &question.revision, &BTreeMap::new(), None,
            ).map(|(_, changed)| changed).map_err(Into::into)
        })
    }).await??;
    if changed { signal_local_change(state); }
    Ok(())
}
