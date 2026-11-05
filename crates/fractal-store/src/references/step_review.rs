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
/// LOCAL step only: not carried source steps, a run, child tree, or parent boundary.
/// Slice 5b must add those boundaries before any production caller uses this helper.
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
        if actor.contains("${") {
            return Err(unresolved(actor));
        }
        fractal_schema::registry()
            .validate_subject(actor)
            .map_err(|_| unresolved(actor))?;
        if actor.starts_with("person/") {
            return Ok(());
        }
        if !actor.starts_with("agent/") {
            return Err(unresolved(actor));
        }
        let identity = identities::resolve_reference_conn(connection, actor).map_err(|error| {
            if error.code == "lifecycle-unavailable" {
                unresolved(actor)
            } else {
                error
            }
        })?;
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
    for (_, body, _) in step_timing_events(connection, &step.subject).map_err(internal)? {
        let fields = body.get("fields").unwrap_or(&body);
        if let Some(actor) = fields.get("claimant").and_then(serde_json::Value::as_str) {
            include(actor)?;
        }
    }
    let reviewer = agent_review::resolve_reviewer(scope, connection, reviewer)?;
    agent_review::independent_reviewer(&reviewer, &excluded)?;
    Ok(excluded)
}

#[cfg(test)]
mod tests;
