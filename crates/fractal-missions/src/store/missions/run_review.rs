use super::*;
use crate::model::LoopSpec;
use fractal_store::references::{Scope, validate_step_reviewer};
use fractal_store::store::{
    flatten_mission_step_specs, pinned_generation_mission_tx, run_generation_ids_tx,
    step_run_row_tx,
};

pub use fractal_store::store::agent_review_transaction::ReviewOwnerSnapshot;

/// The only responsibility loader for the dormant agent-answer transaction helper.
/// FINAL must use this loader before enabling the wire; callers cannot supply an owner set.
#[doc(hidden)]
pub fn review_owner_snapshot(
    scope: &Scope<'_>,
    transaction: &rusqlite::Transaction<'_>,
    owner: &str,
    reviewer: &str,
) -> Result<ReviewOwnerSnapshot, FractalError> {
    let unresolved = || FractalError::new("reviewer-owner-unresolved", owner);
    let step = if owner.starts_with("step-run/") {
        Some(
            step_run_row_tx(transaction, owner)
                .map_err(internal)?
                .ok_or_else(unresolved)?,
        )
    } else if let Some(path) = owner.strip_prefix("loop-run/") {
        Some(
            step_run_row_tx(transaction, &format!("step-run/{path}"))
                .map_err(internal)?
                .ok_or_else(unresolved)?,
        )
    } else if owner.starts_with("mission-run/") {
        None
    } else {
        return Err(unresolved());
    };
    let run = step.as_ref().map_or(owner, |step| step.run.as_str());
    let header = mission_run_header_tx(transaction, run.trim_start_matches("mission-run/"))
        .map_err(internal)?;
    if step
        .as_ref()
        .is_some_and(|step| step.generation != header.generation)
    {
        return Err(unresolved());
    }
    if let Some(step) = step.as_ref().filter(|_| owner.starts_with("loop-run/")) {
        let definition =
            pinned_generation_mission_tx(transaction, generation_id_from_subject(&step.generation))
                .map_err(internal)?
                .ok_or_else(unresolved)?;
        if crate::mission::find_step(&definition, &step.step)
            .is_none_or(|spec| spec.loop_spec.is_none())
        {
            return Err(unresolved());
        }
    }
    validate_reviewer_boundary(
        scope,
        transaction,
        run,
        &header.requester,
        reviewer,
        step.as_ref().map(|step| step.step.as_str()),
    )
}

/// Caller-owned snapshot of every generation, descendant and starting parent step, not siblings.
/// Includes strict loop-round history; public parsers still reject agent-review.
/// Revisions published before the nested-loop owning-path fix can share a loop path;
/// those revisions still fail closed with `reviewer-owner-unresolved`.
#[doc(hidden)]
pub fn validate_run_reviewer(
    scope: &Scope<'_>,
    connection: &Connection,
    run: &str,
    requester: &str,
    reviewer: &str,
) -> Result<BTreeSet<String>, FractalError> {
    validate_reviewer_boundary(scope, connection, run, requester, reviewer, None)
        .map(|snapshot| snapshot.responsible_agents)
}

fn validate_reviewer_boundary(
    scope: &Scope<'_>,
    connection: &Connection,
    run: &str,
    requester: &str,
    reviewer: &str,
    root_step: Option<&str>,
) -> Result<ReviewOwnerSnapshot, FractalError> {
    let unresolved = |subject: &str| FractalError::new("reviewer-owner-unresolved", subject);
    let mut pending = BTreeSet::from([run.to_owned()]);
    let mut visited = BTreeSet::new();
    let mut excluded = BTreeSet::new();
    let mut agentless = true;
    while let Some(subject) = pending.pop_first() {
        if !visited.insert(subject.clone()) {
            return Err(unresolved(&subject));
        }
        let id = subject
            .strip_prefix("mission-run/")
            .ok_or_else(|| unresolved(&subject))?;
        let header = mission_run_header_tx(connection, id).map_err(internal)?;
        if let Some(parent) = &header.parent_step_run {
            let step = step_run_row_tx(connection, parent)
                .map_err(internal)?
                .ok_or_else(|| unresolved(parent))?;
            let owner =
                mission_run_header_tx(connection, step.run.trim_start_matches("mission-run/"))
                    .map_err(internal)?;
            for requester in [requester, owner.requester.as_str()] {
                excluded.extend(validate_step_reviewer(
                    scope, connection, &step, requester, reviewer,
                )?);
            }
        }
        let generations = run_generation_ids_tx(connection, id)?;
        if generations.is_empty()
            || !generations.contains(generation_id_from_subject(&header.generation))
        {
            return Err(unresolved(&subject));
        }
        for generation in generations {
            let definition = pinned_generation_mission_tx(connection, &generation)
                .map_err(internal)?
                .ok_or_else(|| unresolved(&generation))?;
            let definition = crate::mission::run_mission(definition, header.after.as_deref())?;
            let specs = flatten_mission_step_specs(&definition);
            let view = run_generation_rows_tx(connection, &generation).map_err(internal)?;
            let expected: BTreeSet<_> = specs.iter().map(|s| s.path.as_str()).collect();
            let actual: BTreeSet<_> = view.steps.iter().map(|s| s.step.as_str()).collect();
            if view.run != subject || expected.is_empty() || actual != expected {
                return Err(unresolved(&generation));
            }
            let boundary = root_step.filter(|_| subject == run);
            let selected = |path: &str| {
                boundary.is_none_or(|root| {
                    path == root
                        || path
                            .strip_prefix(root)
                            .is_some_and(|tail| tail.starts_with('/'))
                })
            };
            if boundary.is_some_and(|root| !expected.contains(root)) {
                return Err(unresolved(&generation));
            }
            if let Some(root) = boundary {
                let mut path = root;
                while let Some(parent) = crate::mission::parent_step_path(&definition, path) {
                    let step = view
                        .steps
                        .iter()
                        .find(|step| step.step == parent)
                        .ok_or_else(|| unresolved(parent))?;
                    excluded.extend(validate_step_reviewer(
                        scope,
                        connection,
                        step,
                        &header.requester,
                        reviewer,
                    )?);
                    path = parent;
                }
            }
            for step in view.steps.iter().filter(|step| selected(&step.step)) {
                for requester in [requester, header.requester.as_str()] {
                    excluded.extend(validate_step_reviewer(
                        scope, connection, step, requester, reviewer,
                    )?);
                }
            }
            let start = format!("step-run/{generation}/");
            let end = format!("step-run/{generation}0");
            const CHILDREN_SQL: &str =
                "SELECT 'mission-run/' || id FROM mission_runs WHERE parent_step_run>=?1 AND parent_step_run<?2
                 UNION SELECT subject FROM claims WHERE kind='mission-run.created'
                 AND COALESCE(json_extract(body, '$.fields.parent_step_run'), json_extract(body, '$.parent_step_run'))>=?1
                 AND COALESCE(json_extract(body, '$.fields.parent_step_run'), json_extract(body, '$.parent_step_run'))<?2";
            let mut children = connection.prepare_cached(
                /* sqlite-large-value: allow-for-now lv-hed575cdba946 owner=fractal_missions::store::missions::run_review TODO=trace-statement-and-value-representation */ CHILDREN_SQL,
            ).map_err(internal)?;
            let children = children
                .query_map(params![start, end], |row| row.get::<_, String>(0))
                .map_err(internal)?
                .collect::<rusqlite::Result<BTreeSet<_>>>()
                .map_err(internal)?;
            let children = if boundary.is_some() {
                let mut selected_children = BTreeSet::new();
                for child in children {
                    let header =
                        mission_run_header_tx(connection, child.trim_start_matches("mission-run/"))
                            .map_err(internal)?;
                    let parent = header
                        .parent_step_run
                        .as_deref()
                        .ok_or_else(|| unresolved(&child))?;
                    let parent = view
                        .steps
                        .iter()
                        .find(|step| step.subject == parent)
                        .ok_or_else(|| unresolved(&child))?;
                    if selected(&parent.step) {
                        selected_children.insert(child);
                    }
                }
                selected_children
            } else {
                children
            };
            for step in specs.iter().filter(|step| selected(&step.path)) {
                if let Some(spec) = step.loop_spec.as_deref() {
                    validate_loop_history(connection, &generation, &step.path, spec, &children)?;
                } else {
                    agentless &= view
                        .steps
                        .iter()
                        .any(|row| row.step == step.path && row.agentless);
                }
            }
            pending.extend(children);
        }
    }
    Ok(ReviewOwnerSnapshot {
        responsible_agents: excluded,
        reviewer: resolve_reference_conn(connection, reviewer)?,
        agentless,
    })
}

fn validate_loop_history(
    connection: &Connection,
    generation: &str,
    step_path: &str,
    spec: &LoopSpec,
    children: &BTreeSet<String>,
) -> Result<(), FractalError> {
    let subject = format!("loop-run/{generation}/{}", spec.path);
    // Inline nesting rewrites the owning step path, not the loop history path.
    let parent = format!("step-run/{generation}/{step_path}");
    let unresolved = || FractalError::new("reviewer-owner-unresolved", &subject);
    let mut rounds = BTreeSet::new();
    let mut linked = BTreeMap::new();
    for child in children {
        let header = mission_run_header_tx(connection, child.trim_start_matches("mission-run/"))
            .map_err(internal)?;
        if header.parent_step_run.as_deref() != Some(parent.as_str()) {
            continue;
        }
        let round = header
            .inputs
            .get(crate::mission::LOOP_ROUND_INPUT)
            .and_then(|input| input.value.parse::<u32>().ok())
            .filter(|round| *round > 0)
            .ok_or_else(unresolved)?;
        if header.mission != format!("mission/{}", spec.round.id) {
            if spec
                .on_keep
                .iter()
                .chain(&spec.on_discard)
                .any(|branch| header.mission.strip_prefix("mission/") == Some(branch.id.as_str()))
            {
                continue;
            }
            return Err(unresolved());
        }
        rounds.insert(round);
        linked.insert(child.as_str(), round);
    }
    const HISTORY_SQL: &str = "SELECT kind, body FROM claims WHERE subject=?1 AND kind IN ('loop.state','loop.round-result','loop.round-dispatch')";
    let mut history = connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-hde8dcd507000 owner=fractal_missions::store::missions::run_review TODO=trace-statement-and-value-representation */ HISTORY_SQL
    ).map_err(internal)?;
    let mut rows = history.query([&subject]).map_err(internal)?;
    let mut required = 0;
    let mut seen = false;
    let mut item_count = None;
    while let Some(row) = rows.next().map_err(internal)? {
        let kind: String = row.get(0).map_err(internal)?;
        seen |= kind == "loop.state";
        let body: String = row.get(1).map_err(internal)?;
        let body: Value = serde_json::from_str(&body).map_err(|_| unresolved())?;
        let fields: BTreeMap<String, Value> =
            serde_json::from_value(body.get("fields").unwrap_or(&body).clone())
                .map_err(|_| unresolved())?;
        fractal_schema::registry()
            .validate_claim(&subject, &kind, &fields)
            .map_err(|_| unresolved())?;
        let round = fields
            .get("round")
            .and_then(Value::as_u64)
            .and_then(|round| u32::try_from(round).ok())
            .ok_or_else(unresolved)?;
        if let Some(items) = fields.get("items").and_then(Value::as_array) {
            let count = u32::try_from(items.len()).map_err(|_| unresolved())?;
            if item_count.replace(count).is_some_and(|old| old != count) {
                return Err(unresolved());
            }
        }
        // The for-each evaluator enters attempt 1 before it snapshots even an empty list.
        let entry = spec.for_each.is_some()
            && kind == "loop.state"
            && round == 1
            && fields.get("status").and_then(Value::as_str) == Some("running")
            && !fields.contains_key("items");
        if !entry {
            required = required.max(round);
        }
        if kind != "loop.state" {
            let child = fields
                .get("mission_run")
                .and_then(Value::as_str)
                .ok_or_else(unresolved)?;
            if round == 0 || linked.get(child) != Some(&round) {
                return Err(unresolved());
            }
        }
    }
    if spec.for_each.is_some() {
        required = required.max(item_count.ok_or_else(unresolved)?);
    }
    if !seen
        || rounds.range(..=required).count() != required as usize
        || (required == 0 && spec.for_each.is_none())
    {
        return Err(unresolved());
    }
    Ok(())
}

/// Validate an existing run plus the concrete requester and proposed assignment, without writing.
/// Revisions published before the nested-loop owning-path fix can share a loop path;
/// those revisions still fail closed with `reviewer-owner-unresolved`.
#[doc(hidden)]
pub fn validate_run_reviewer_assignment(
    scope: &Scope<'_>,
    connection: &Connection,
    run: &str,
    requester: &str,
    proposed_assignee: &str,
    reviewer: &str,
) -> Result<BTreeSet<String>, FractalError> {
    let mut excluded = validate_run_reviewer(scope, connection, run, requester, reviewer)?;
    let header = mission_run_header_tx(connection, run.trim_start_matches("mission-run/"))
        .map_err(internal)?;
    let mut step =
        run_generation_rows_tx(connection, generation_id_from_subject(&header.generation))
            .map_err(internal)?
            .steps
            .into_iter()
            .next()
            .ok_or_else(|| FractalError::new("reviewer-owner-unresolved", run))?;
    step.assigned_to = Some(proposed_assignee.to_owned());
    excluded.extend(validate_step_reviewer(
        scope, connection, &step, requester, reviewer,
    )?);
    Ok(excluded)
}

#[cfg(test)]
mod tests;
