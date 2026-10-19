//! The part of `store::stop_deferral` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_store::store::stop_deferral::*;
pub use st3_store::store::stop_deferral::{FAMILIES, create_schema};

fn family(name: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    *FAMILIES
        .iter()
        .find(|family| family.0 == name)
        .expect("a known stop-deferral family")
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

pub trait StopDeferralAttentionStore {
    /// Current aged deferral faults for this page of seats at the caller's history cut. A
    /// stopped runtime, replaced declaration, cleared diagnostic or recovered fault hides the
    /// episode immediately, even before the next reconciler pass.
    fn stop_deferral_faults_for(
            &self,
            subjects: &[String],
            at_index: u64,
        ) -> Result<BTreeMap<String, String>>;
}

impl StopDeferralAttentionStore for Store {

    fn stop_deferral_faults_for(
            &self,
            subjects: &[String],
            at_index: u64,
        ) -> Result<BTreeMap<String, String>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h5e430fe482e9 owner=st3_attention::store::stop_deferral TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
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

