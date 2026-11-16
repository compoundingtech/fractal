//! A mission that runs a program publishes only with its bundle's bytes in this store
//! (docs/st3/program-engine.md §4). The step definition names the bytes by `bundle_hash`, so the
//! `mission.published` claim promotes the staged blob and replicates it; a claim naming a blob
//! this member never held would be refused by every replica as `missing-replicated-blob`.
use super::*;

pub(super) fn validate_publication(connection: &Connection, intent: &NormalizedIntent) -> Result<(), FractalError> {
    for mission in intent.missions.values() {
        for (step, program) in mission.programs() {
            let held: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM blobs WHERE hash=?1) OR EXISTS(SELECT 1 FROM local_blobs WHERE hash=?1)",
                    [&program.bundle_hash],
                    |row| row.get(0),
                )
                .map_err(internal)?;
            if !held {
                return Err(FractalError::new(
                    "missing-program-bundle",
                    format!(
                        "mission `{}` step `{step}` runs program `{}` with sha256 {}, whose bytes this member does not hold; publish it with `fractal apply`, which uploads the bundle",
                        mission.id, program.bundle, program.bundle_hash
                    ),
                ));
            }
        }
    }
    Ok(())
}
