//! Retained stops remain runtime work even when their owner is terminal. Read only the named
//! seats' diagnostic/fault histories; unrelated harness diagnostics do not end a deferral.
//!
//! A trigger copies each family's claims into `stop_deferral_claims` as they are inserted, so
//! opening a store never scans claims to build an index. Claims older than the trigger
//! (at or below the recorded ceiling) are copied once after the API serves, by a paced scan on
//! a reader. Until that scan commits, reads also look those claims up through the subject/kind
//! index, so they return the same rows either way.
use super::*;

/// (family, claim kind, predicate on `body`, recovered failure reference)
pub const FAMILIES: [(&str, &str, &str, &str); 3] = [
    (
        "diagnostic",
        "harness.diagnostic",
        "json_extract(body,'$.fields.code') IN ('stop-deferred','stop-deferral-cleared')",
        "''",
    ),
    (
        "failure",
        "operational.failure",
        "json_extract(body,'$.fields.condition')='stop-deferred'",
        "''",
    ),
    (
        "recovered",
        "operational.recovered",
        "1",
        "COALESCE(json_extract(body,'$.fields.failure'),'')",
    ),
];

pub fn create_schema(connection: &Connection) -> Result<()> {
    let arms = |column: usize| {
        FAMILIES
            .iter()
            .map(|family| {
                let (name, kind, predicate, reference) = *family;
                let value = [name, reference][column];
                format!(
                    "WHEN NEW.kind='{kind}' AND {} THEN {}",
                    predicate.replace("body", "NEW.body"),
                    if column == 0 { format!("'{value}'") } else { value.replace("body", "NEW.body") }
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let kinds = FAMILIES
        .iter()
        .map(|family| format!("'{}'", family.1))
        .collect::<Vec<_>>()
        .join(",");
    // The trigger and the ceiling commit together: every claim above the ceiling was copied
    // by the trigger, and every claim at or below it is the backfill's.
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-hd82a4f1dd9c8 owner=st3_store::store::stop_deferral TODO=chunk-or-reference-variable-columns */ "SAVEPOINT stop_deferral_schema;
         DROP INDEX IF EXISTS claims_stop_deferral_diagnostic_index;
         DROP INDEX IF EXISTS claims_stop_deferral_failure_index;
         DROP INDEX IF EXISTS claims_stop_deferral_recovery_index;
         CREATE TABLE IF NOT EXISTS stop_deferral_claims (
             subject TEXT NOT NULL,
             family TEXT NOT NULL,
             reference TEXT NOT NULL,
             claim_id TEXT NOT NULL,
             PRIMARY KEY(subject,family,reference,claim_id)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS local_stop_deferral_backfill (
             singleton INTEGER PRIMARY KEY CHECK(singleton=1),
             ceiling INTEGER NOT NULL,
             complete INTEGER NOT NULL
         );
         INSERT OR IGNORE INTO local_stop_deferral_backfill
             SELECT 1,COALESCE(MAX(store_index),0),MAX(store_index) IS NULL FROM claims;
         CREATE TRIGGER IF NOT EXISTS stop_deferral_claims_copy AFTER INSERT ON claims
         WHEN NEW.kind IN ({kinds})
         BEGIN
             INSERT OR IGNORE INTO stop_deferral_claims(subject,family,reference,claim_id)
             SELECT NEW.subject,family,reference,NEW.id FROM (SELECT
                 CASE {} END AS family, CASE {} END AS reference)
             WHERE family IS NOT NULL;
         END;
         RELEASE stop_deferral_schema;",
        arms(0),
        arms(1),
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    

}
