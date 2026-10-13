//! Retained stops remain runtime work even when their owner is terminal. Read only the named
//! seats' diagnostic/fault histories; unrelated harness diagnostics do not end a deferral.
use super::*;

impl Store {
    pub(crate) fn latest_stop_deferral(&self, subject: &str) -> Result<Option<ClaimRecord>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        connection.prepare_cached(&canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind='harness.diagnostic'
               AND json_extract(claims.body,'$.fields.code') IN ('stop-deferred','stop-deferral-cleared')
             ORDER BY CANONICAL_DESC(claims) LIMIT 1"
        )))?.query_row([subject], claim_from_row).optional().map_err(Into::into)
    }

    pub(crate) fn pending_stop_deferral_failure(
        &self,
        diagnostic: &ClaimRecord,
    ) -> Result<Option<ClaimRecord>> {
        smallclaims::touched::note_read(|| diagnostic.subject.clone());
        let connection = self.readers.get();
        connection
            .prepare_cached(&canonical_sql(&format!(
                "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind='operational.failure'
               AND json_extract(claims.body,'$.fields.episode')=?2
               AND NOT EXISTS (SELECT 1 FROM claims recovered
                 WHERE recovered.subject=?1 AND recovered.kind='operational.recovered'
                   AND json_extract(recovered.body,'$.fields.failure')=claims.id)
             ORDER BY CANONICAL_DESC(claims) LIMIT 1"
            )))?
            .query_row(
                params![
                    diagnostic.subject,
                    format!("stop-deferred:{}", diagnostic.id)
                ],
                claim_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Current aged deferral faults for this page of seats at the caller's history cut. A
    /// stopped runtime, replaced declaration, cleared diagnostic or recovered fault hides the
    /// episode immediately, even before the next reconciler pass.
    pub(crate) fn stop_deferral_faults_for(
        &self,
        subjects: &[String],
        at_index: u64,
    ) -> Result<BTreeMap<String, String>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(&canonical_sql(
            "WITH seats AS MATERIALIZED (
               SELECT desired.subject,desired.claim_id,
                 (SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.subject=desired.subject AND claims.kind='harness.diagnostic'
                     AND claims.store_index<=?2
                     AND json_extract(claims.body,'$.fields.code') IN ('stop-deferred','stop-deferral-cleared')
                   ORDER BY CANONICAL_DESC(claims) LIMIT 1) AS diagnostic,
                 (SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.subject=desired.subject AND claims.kind='operational.failure'
                     AND claims.store_index<=?2 AND json_extract(claims.body,'$.fields.condition')='stop-deferred'
                   ORDER BY CANONICAL_DESC(claims) LIMIT 1) AS failure,
                 (SELECT claims.body FROM claims JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.subject=desired.subject AND claims.kind='runtime.observed'
                     AND claims.store_index<=?2
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
                 WHERE recovered.subject=seats.subject AND recovered.kind='operational.recovered'
                   AND recovered.store_index<=?2 AND json_extract(recovered.body,'$.fields.failure')=fault.id)
             ORDER BY seats.subject"
        ))?;
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
