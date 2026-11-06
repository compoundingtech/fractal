//! A seat whose declaration is a stop takes no new work. When the run that stopped it ended while
//! the seat still held work in another run, the seat may finish that work and only that: work in
//! a run this host recorded before the stop. Anything that arrives later neither holds the seat
//! nor can be claimed by it, so a stopping seat always drains.
use super::*;

/// The stop declared for `seat`, if its declaration is one: the run whose end declared it
/// (`None` for a person's or an agent's own stop) and the stop claim's local store index.
pub fn stop_cut(connection: &Connection, seat: &str) -> Result<Option<(Option<String>, u64)>> {
    Ok(connection
        .query_row(
            "SELECT desired.owner_run, claims.store_index
             FROM desired JOIN claims ON claims.id=desired.claim_id
             WHERE desired.subject=?1 AND desired.kind='stop'",
            [seat],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)? as u64,
                ))
            },
        )
        .optional()?)
}

/// Live work `seat` holds or is assigned in a run outside the root of `owner_run`, at most three
/// steps, or only `step` when one is named. With `before`, only work in runs this host recorded
/// before that store index. A run whose creation claim this host no longer has counts as earlier:
/// a missing record never ends a hold early.
pub fn held_work(
    connection: &Connection,
    seat: &str,
    owner_run: &str,
    before: Option<u64>,
    step: Option<&str>,
) -> Result<Vec<String>> {
    let owner = owner_run.strip_prefix("mission-run/").unwrap_or(owner_run);
    let mut statement = connection.prepare_cached(
        "SELECT step_runs.subject
         FROM step_runs JOIN mission_runs ON mission_runs.id=step_runs.run_id
         WHERE (step_runs.assignee=?1 OR step_runs.lease_owner=?1)
           AND (?4 IS NULL OR step_runs.subject=?4)
           AND step_runs.status IN ('ready','claimed','working','verifying')
           AND step_runs.generation_id=mission_runs.current_generation_id
           AND mission_runs.status IN ('running','standing','blocked')
           AND mission_runs.phase NOT LIKE 'cleanup-%'
           AND mission_runs.phase<>'terminal'
           AND mission_runs.root_run_id<>COALESCE(
                 (SELECT root_run_id FROM mission_runs WHERE id=?2), ?2)
           AND (?3 IS NULL OR COALESCE((SELECT MIN(claims.store_index) FROM claims
                 WHERE claims.subject='mission-run/' || mission_runs.id
                   AND claims.kind='mission-run.created'), -1)<?3)
         ORDER BY step_runs.subject LIMIT 3",
    )?;
    let subjects = statement
        .query_map(
            params![seat, owner, before.map(|index| index as i64), step],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(subjects)
}

/// The refusal every new-work path gives a seat whose declaration is a stop.
pub fn stopping(seat: &str) -> FractalError {
    FractalError::new(
        "seat-stopping",
        format!(
            "`{seat}` is stopping: it takes no new work, and finishes only the work it held when the stop was declared"
        ),
    )
}

/// Refuse `actor`'s claim of `step` while its declaration is a stop, unless the step is work the
/// stop found it holding in another run.
pub fn admit_claim(tx: &Connection, actor: &str, step: &str) -> Result<(), FractalError> {
    let Some((owner_run, cut)) = stop_cut(tx, actor).map_err(internal)? else {
        return Ok(());
    };
    let held = match owner_run {
        Some(owner) => !held_work(tx, actor, &owner, Some(cut), Some(step))
            .map_err(internal)?
            .is_empty(),
        None => false,
    };
    if held { Ok(()) } else { Err(stopping(actor)) }
}
