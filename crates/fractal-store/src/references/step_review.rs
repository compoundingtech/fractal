//! Concrete step responsibility only; hierarchical boundaries are assembled by missions.
use super::{Scope, St3Error, agent_review};
use crate::model::StepRunView;
use crate::store::{ReferenceGraph, identities, step_timing_events};
use rusqlite::Connection;
use smallclaims::error::internal;
use std::collections::BTreeSet;

fn unresolved(subject: &str) -> St3Error {
    St3Error::new(
        "reviewer-owner-unresolved",
        format!("review responsibility `{subject}` is unresolved"),
    )
}

/// Internal cross-crate adapter for missions; no production gate dispatch calls this.
/// The caller supplies a raw step/assignment/pool view and requester from the same
/// caller-owned store snapshot. For reassignment, include the proposed assignee in that view.
/// This validates one step only, not a run, child tree, or enclosing parent boundary.
#[doc(hidden)]
pub fn validate_step_reviewer(
    scope: &Scope<'_>,
    connection: &Connection,
    step: &StepRunView,
    requester: &str,
    reviewer: &str,
) -> Result<BTreeSet<String>, St3Error> {
    let mut excluded = BTreeSet::new();
    let mut include = |actor: &str| -> Result<(), St3Error> {
        fractal_schema::registry()
            .validate_subject(actor)
            .map_err(|_| unresolved(actor))?;
        if actor.starts_with("person/") {
            return Ok(());
        }
        if !actor.starts_with("agent/") {
            return Err(unresolved(actor));
        }
        let identity = identities::resolve_reference_conn(connection, actor)?;
        if !scope.declared_subject(&ReferenceGraph { connection }, &identity)? {
            return Err(unresolved(actor));
        }
        excluded.insert(identity);
        Ok(())
    };
    include(requester)?;
    if !step.agentless
        && step.assigned_to.is_none()
        && step.claimant.is_none()
        && step.available_to.is_empty()
    {
        return Err(unresolved(&step.subject));
    }
    for actor in step
        .assigned_to
        .iter()
        .chain(&step.claimant)
        .chain(&step.carried_claimant)
    {
        include(actor)?;
    }
    if step.claimant.is_none() {
        for actor in &step.available_to {
            include(actor)?;
        }
    }
    // Read raw history: effective views and carried_claimants_tx intentionally drop
    // responsibility after lease expiry, terminal state, or a later claim.
    let mut pending = vec![step.subject.clone()];
    let mut visited = BTreeSet::new();
    while let Some(subject) = pending.pop() {
        if !visited.insert(subject.clone()) {
            continue;
        }
        let events = step_timing_events(connection, &subject).map_err(internal)?;
        if events.is_empty() && subject != step.subject {
            return Err(unresolved(&subject));
        }
        for (kind, body, _) in events {
            if let Some(actor) = body
                .pointer("/fields/claimant")
                .and_then(serde_json::Value::as_str)
            {
                include(actor)?;
            }
            if kind == "step-run.carried" {
                let source = body
                    .pointer("/fields/source_step_run")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| unresolved(&subject))?;
                fractal_schema::registry()
                    .validate_subject(source)
                    .map_err(|_| unresolved(source))?;
                if !source.starts_with("step-run/") {
                    return Err(unresolved(source));
                }
                pending.push(source.to_owned());
            }
        }
    }
    let reviewer = agent_review::resolve_reviewer(scope, connection, reviewer)?;
    agent_review::independent_reviewer(&reviewer, &excluded)?;
    Ok(excluded)
}

#[cfg(test)]
mod tests;
