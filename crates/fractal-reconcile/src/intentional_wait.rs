//! Intentional waits shared by diagnostics and stall reporting. Unknown evidence is not a wait.
use crate::{model::{GateSpec, StepRunView}, MissionsStore, Store};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentionalWait {
    HumanGate,
    Person,
    Answer,
    Blocked,
    Predicate,
    Backoff,
    Declared,
}

/// Classify only positive evidence, without evaluating gates or changing state.
pub fn classify(
    step: &StepRunView,
    now: u128,
    human_gate_pending: bool,
    predicate_pending: bool,
) -> Option<IntentionalWait> {
    if human_gate_pending {
        Some(IntentionalWait::HumanGate)
    } else if step.assigned_to.as_deref().is_some_and(|actor| actor.starts_with("person/")) {
        Some(IntentionalWait::Person)
    } else if step.status == "waiting-person" {
        Some(IntentionalWait::Answer)
    } else if step.status == "blocked" {
        Some(IntentionalWait::Blocked)
    } else if step.agentless && predicate_pending {
        Some(IntentionalWait::Predicate)
    } else if !step.agentless && step.not_before_unix_ms.is_some_and(|until| until > now) {
        Some(IntentionalWait::Backoff)
    } else if !step.agentless && matches!(step.status.as_str(), "claimed" | "working")
        && step.execution_paused_until_unix_ms.is_some_and(|until| until > now)
    {
        Some(IntentionalWait::Declared)
    } else {
        None
    }
}

/// Inspect at most four pinned gates using only fresh, already-warm observations.
/// An unknown earlier gate stops the search. The callback only renders a known false field.
pub fn predicate_wait<T>(
    store: &Store,
    step: &StepRunView,
    render: impl FnOnce(&str, &str, &str, &str, &Value, &Value) -> T,
) -> anyhow::Result<Option<T>> {
    let mut render = Some(render);
    for gate in store.waiting_on_field_gates(step)? {
        let GateSpec::Field { name, subject, path, operator, value } = &gate else {
            return Ok(None);
        };
        if subject.contains("${") || subject.starts_with("exec/") || subject.starts_with("effect/")
            || (subject.starts_with("resource/") && subject.contains('@'))
            || value.as_str().is_some_and(|text| text.contains("${"))
        {
            return Ok(None);
        }
        smallclaims::touched::note_read(|| subject.clone());
        let observation = store.inspect_cached_actual(subject, |actual| {
            crate::reconcile::inspect_field_gate(actual, subject, path, operator, value, |found, holds| {
                (holds, found.filter(|_| !holds).map(|found| {
                    render.take().expect("only one false gate is rendered")(name, subject, path, operator, value, found)
                }))
            })
        })?;
        match observation {
            Some((true, _)) => {}
            Some((false, Some(value))) => return Ok(Some(value)),
            _ => return Ok(None),
        }
    }
    Ok(None)
}
