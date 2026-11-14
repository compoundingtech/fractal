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

pub(super) fn initialize_write_witness(connection: &Connection) -> Result<()> {
    // Complete-store startup must stay read-only even while another connection
    // holds the writer. INSERT OR IGNORE alone would still acquire that lock.
    let initialized: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_desired_write_witness WHERE id=1)",
        [], |row| row.get(0),
    )?;
    if !initialized {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h5650ece9d11e owner=fractal_store::store::desired_progress TODO=bound-or-reference-values-and-reclamation */
            "INSERT OR IGNORE INTO local_desired_write_witness(id,generation) VALUES(1,0)",
            [],
        )?;
    }
    Ok(())
}
