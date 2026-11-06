use super::{Scope, St3Error, agent_review};
use crate::model::StepRunView;
use crate::store::{ReferenceGraph, identities, step_run_row_tx, step_timing_events};
use rusqlite::Connection;
use smallclaims::error::internal;
use std::collections::BTreeSet;

fn unresolved(subject: &str) -> St3Error {
    St3Error::new("reviewer-owner-unresolved", subject)
}

/// The caller supplies a raw step/assignment/pool view and requester from the same
/// caller-owned store snapshot. For reassignment, include the proposed assignee in that view.
/// Includes local responsibility and every carried source step.
/// Before production use, fractal-missions must add the run hierarchy, child runs,
/// loop rounds, enclosing parent step, and start/reassignment entry point.
/// Slice 5c (fractal-seats) must verify real person-work delegation.
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
    collect_step(connection, step, &mut include, &mut BTreeSet::new())?;
    let reviewer = agent_review::resolve_reviewer(scope, connection, reviewer)?;
    agent_review::independent_reviewer(&reviewer, &excluded)?;
    Ok(excluded)
}

// Reuse exactly the same local collection for the supplied step and each source.
fn collect_step(
    connection: &Connection,
    step: &StepRunView,
    include: &mut impl FnMut(&str) -> Result<(), St3Error>,
    visiting: &mut BTreeSet<String>,
) -> Result<(), St3Error> {
    if !visiting.insert(step.subject.clone()) {
        return Err(unresolved(&step.subject));
    }
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
    let events = step_timing_events(connection, &step.subject).map_err(internal)?;
    for (_, body, _) in &events {
        let fields = body.get("fields").unwrap_or(body);
        if let Some(actor) = fields.get("claimant").and_then(serde_json::Value::as_str) {
            include(actor)?;
        }
    }
    // Any agent writing work on this step contributed; over-exclusion is safer.
    let mut work = connection
        .prepare_cached(
            "SELECT actor FROM claims WHERE subject=?1 AND kind GLOB 'work.*'
         AND (actor GLOB 'agent/*' OR (actor IS NULL AND kind='work.submitted'))",
        )
        .map_err(internal)?;
    let actors = work
        .query_map([&step.subject], |row| row.get::<_, Option<String>>(0))
        .map_err(internal)?;
    for actor in actors {
        let actor = actor
            .map_err(internal)?
            .ok_or_else(|| unresolved(&step.subject))?;
        include(&actor)?;
    }
    for (kind, body, _) in &events {
        if kind != "step-run.carried" {
            continue;
        }
        let fields = body.get("fields").unwrap_or(body);
        let source = fields
            .get("source_step_run")
            .and_then(serde_json::Value::as_str)
            .filter(|source| source.starts_with("step-run/") && !source.contains("${"))
            .ok_or_else(|| unresolved(&step.subject))?;
        fractal_schema::registry()
            .validate_subject(source)
            .map_err(|_| unresolved(source))?;
        let source = step_run_row_tx(connection, source)
            .map_err(internal)?
            .ok_or_else(|| unresolved(source))?;
        collect_step(connection, &source, include, visiting)?;
    }
    visiting.remove(&step.subject);
    Ok(())
}

#[cfg(test)]
mod tests;
