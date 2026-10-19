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

fn family(name: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    *FAMILIES
        .iter()
        .find(|family| family.0 == name)
        .expect("a known stop-deferral family")
}

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

/// Claim ids of one family's claims for `subject`. The table is keyed by claim id because store
/// indexes are local and a checkpoint proof renumbers them. Claims older than the trigger come
/// from the subject/kind index until the backfill commits; after that the second arm is an
/// empty range.
pub fn family_sql(name: &str, subject: &str, reference: Option<&str>) -> String {
    let (name, kind, predicate, reference_sql) = family(name);
    let (copied, scanned) = match reference {
        Some(reference) => (
            format!(" AND reference={reference}"),
            format!(" AND {}={reference}", reference_sql.replace("body", "history.body")),
        ),
        None => (" AND reference=''".into(), String::new()),
    };
    format!(
        "SELECT claim_id FROM stop_deferral_claims
           WHERE subject={subject} AND family='{name}'{copied}
         UNION
         SELECT history.id FROM claims history INDEXED BY claims_subject_kind_index
           WHERE history.subject={subject} AND history.kind='{kind}'
             AND history.store_index<=(SELECT CASE WHEN complete THEN 0 ELSE ceiling END
               FROM local_stop_deferral_backfill)
             AND {}{scanned}",
        predicate.replace("body", "history.body"),
    )
}

impl Store {
    /// Current aged deferral faults for this page of seats at the caller's history cut. A
    /// stopped runtime, replaced declaration, cleared diagnostic or recovered fault hides the
    /// episode immediately, even before the next reconciler pass.
    pub fn stop_deferral_faults_for(
        &self,
        subjects: &[String],
        at_index: u64,
    ) -> Result<BTreeMap<String, String>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h8e14c329131f owner=st3_store::store::stop_deferral TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
            "WITH seats AS MATERIALIZED (
               SELECT desired.subject,desired.claim_id,
                 (SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.id IN ({}) AND +claims.store_index<=?2
                   ORDER BY CANONICAL_DESC(claims) LIMIT 1) AS diagnostic,
                 (SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.id IN ({}) AND +claims.store_index<=?2
                   ORDER BY CANONICAL_DESC(claims) LIMIT 1) AS failure,
                 (SELECT claims.body FROM claims INDEXED BY claims_subject_kind_accepted_index
                   JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.subject=desired.subject AND claims.kind='runtime.observed'
                     AND +claims.store_index<=?2
                     AND NOT EXISTS (SELECT 1 FROM replica_records repaired
                       WHERE repaired.claim_id=claims.id AND repaired.state='repaired')
                   ORDER BY CANONICAL_DESC(claims) LIMIT 1) AS runtime
               FROM desired JOIN mission_runs owner ON owner.id=substr(desired.owner_run,13)
               WHERE desired.subject IN (SELECT value FROM json_each(?1))
                 AND substr(desired.owner_run,1,12)='mission-run/'
                 AND (owner.phase='terminal' OR owner.status IN ('completed','failed','cancelled'))
             )
             SELECT seats.subject,json_extract(fault.body,'$.fields.reason')
             FROM seats JOIN claims diagnostic ON diagnostic.id=seats.diagnostic
               JOIN claims fault ON fault.id=seats.failure
             WHERE json_extract(diagnostic.body,'$.fields.code')='stop-deferred'
               AND json_extract(fault.body,'$.fields.episode')='stop-deferred:' || diagnostic.id
               AND json_extract(fault.body,'$.fields.source_revision')=seats.claim_id
               AND json_extract(diagnostic.body,'$.evidence[0]')=seats.claim_id
               AND json_extract(seats.runtime,'$.fields.status')='running'
               AND json_extract(diagnostic.body,'$.fields.incarnation_id') IS json_extract(seats.runtime,'$.fields.incarnation_id')
               AND json_extract(fault.body,'$.fields.incarnation') IS json_extract(seats.runtime,'$.fields.incarnation_id')
               AND NOT EXISTS (SELECT 1 FROM claims recovered
                 WHERE recovered.id IN ({}) AND recovered.store_index<=?2)
             ORDER BY seats.subject",
            family_sql("diagnostic", "desired.subject", None),
            family_sql("failure", "desired.subject", None),
            family_sql("recovered", "seats.subject", Some("fault.id")),
        )))?;
        let mut faults = BTreeMap::new();
        for chunk in subjects.chunks(256) {
            let rows = statement
                .query_map(params![serde_json::to_string(chunk)?, at_index], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            faults.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        Ok(faults)
    }

}
