//! Projection progress for bulk desired reads, sampled inside their pinned snapshot.
use super::*;

#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(super) struct DesiredProjectionProgress {
    // Both intent.desired and mission.published update their selected tables in the
    // graph projection transaction. Its frontier advances on each committed backlog
    // chunk without appending claims. Preserve missing health separately from index 0.
    graph: Option<(String, u64)>,
}

pub(super) fn desired_projection_progress(
    connection: &Connection,
) -> Result<DesiredProjectionProgress> {
    let graph = connection.query_row(
        "SELECT status, last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    Ok(DesiredProjectionProgress { graph })
}
