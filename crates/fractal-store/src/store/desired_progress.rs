//! Projection progress for bulk desired reads, sampled inside their pinned snapshot.
use super::*;

#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(super) struct DesiredProjectionProgress {
    // Both intent.desired and mission.published update their selected tables in the
    // graph projection transaction. Its frontier advances on each committed backlog
    // chunk without appending claims. Preserve missing health separately from index 0.
    graph: Option<(String, u64)>,
    // Durable row triggers advance this even when repair preserves graph health.
    desired_write_generation: u64,
}

pub(super) fn desired_projection_progress(
    connection: &Connection,
) -> Result<DesiredProjectionProgress> {
    let graph = connection.query_row(
        "SELECT status, last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let desired_write_generation = connection.query_row(
        "SELECT generation FROM local_desired_write_witness WHERE id=1",
        [], |row| row.get(0),
    )?;
    Ok(DesiredProjectionProgress { graph, desired_write_generation })
}
