use super::*;
use fractal_store::references::{Scope, validate_step_reviewer};
use fractal_store::store::{
    flatten_mission_step_specs, pinned_generation_mission_tx, run_generation_ids_tx,
    step_run_row_tx,
};

/// Caller-owned snapshot of every generation, descendant and starting parent step, not siblings.
/// Loops fail closed; start/reassignment is unavailable and public parsers still reject agent-review.
#[doc(hidden)]
pub fn validate_run_reviewer(
    scope: &Scope<'_>,
    connection: &Connection,
    run: &str,
    requester: &str,
    reviewer: &str,
) -> Result<BTreeSet<String>, St3Error> {
    let unresolved = |subject: &str| St3Error::new("reviewer-owner-unresolved", subject);
    let mut pending = vec![run.to_owned()];
    let mut visited = BTreeSet::new();
    let mut excluded = BTreeSet::new();
    while let Some(subject) = pending.pop() {
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
            if specs.iter().any(|step| step.loop_spec.is_some()) {
                return Err(St3Error::new("reviewer-history-unsupported", &subject));
            }
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
            pending.extend(
                children
                    .query_map(params![start, end], |row| row.get::<_, String>(0))
                    .map_err(internal)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(internal)?,
            );
        }
    }
    Ok(excluded)
}

#[cfg(test)]
mod tests;
