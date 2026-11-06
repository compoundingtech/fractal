use super::*;
use crate::model::LoopSpec;
use fractal_store::references::{Scope, validate_step_reviewer};
use fractal_store::store::{
    flatten_mission_step_specs, pinned_generation_mission_tx, run_generation_ids_tx,
    step_run_row_tx,
};

/// Caller-owned snapshot of every generation, descendant and starting parent step, not siblings.
/// Includes strict loop-round history; public parsers still reject agent-review.
/// Loops with keep/discard branches or nested loops sharing a loop path currently fail closed
/// with `reviewer-owner-unresolved` until a later slice.
#[doc(hidden)]
pub fn validate_run_reviewer(
    scope: &Scope<'_>,
    connection: &Connection,
    run: &str,
    requester: &str,
    reviewer: &str,
) -> Result<BTreeSet<String>, FractalError> {
    let unresolved = |subject: &str| FractalError::new("reviewer-owner-unresolved", subject);
    let mut pending = BTreeSet::from([run.to_owned()]);
    let mut visited = BTreeSet::new();
    let mut excluded = BTreeSet::new();
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
            for step in &view.steps {
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
            for step in &specs {
                if let Some(spec) = step.loop_spec.as_deref() {
                    validate_loop_history(connection, &generation, &step.path, spec, &children)?;
                }
            }
            pending.extend(children);
        }
    }
    Ok(excluded)
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
        required = required.max(round);
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
    if !seen
        || rounds.range(..=required).count() != required as usize
        || (required == 0 && spec.for_each.is_none())
    {
        return Err(unresolved());
    }
    Ok(())
}

/// Validate an existing run plus the concrete requester and proposed assignment, without writing.
/// Loops with keep/discard branches or nested loops sharing a loop path currently fail closed
/// with `reviewer-owner-unresolved` until a later slice.
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
