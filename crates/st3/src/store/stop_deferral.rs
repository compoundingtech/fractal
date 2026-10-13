//! Retained stops remain runtime work even when their owner is terminal. Read only the named
//! seats' diagnostic/fault histories; unrelated harness diagnostics do not end a deferral.
use super::*;

impl Store {
    pub(crate) fn latest_stop_deferral(&self, subject: &str) -> Result<Option<ClaimRecord>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        connection.prepare_cached(&canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS} FROM claims INDEXED BY claims_stop_deferral_diagnostic_index
             JOIN batches ON batches.id=claims.batch_id
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
                "SELECT {CLAIM_COLUMNS} FROM claims INDEXED BY claims_stop_deferral_failure_index
             JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind='operational.failure'
               AND json_extract(claims.body,'$.fields.condition')='stop-deferred'
               AND json_extract(claims.body,'$.fields.episode')=?2
               AND NOT EXISTS (SELECT 1 FROM claims recovered INDEXED BY claims_stop_deferral_recovery_index
                 WHERE recovered.subject=?1 AND recovered.kind='operational.recovered'
                   AND json_extract(recovered.body,'$.fields.failure')=+claims.id)
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
                 (SELECT claims.id FROM claims INDEXED BY claims_stop_deferral_diagnostic_index
                   JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.subject=desired.subject AND claims.kind='harness.diagnostic'
                     AND +claims.store_index<=?2
                     AND json_extract(claims.body,'$.fields.code') IN ('stop-deferred','stop-deferral-cleared')
                   ORDER BY CANONICAL_DESC(claims) LIMIT 1) AS diagnostic,
                 (SELECT claims.id FROM claims INDEXED BY claims_stop_deferral_failure_index
                   JOIN batches ON batches.id=claims.batch_id
                   WHERE claims.subject=desired.subject AND claims.kind='operational.failure'
                     AND +claims.store_index<=?2 AND json_extract(claims.body,'$.fields.condition')='stop-deferred'
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
               AND NOT EXISTS (SELECT 1 FROM claims recovered INDEXED BY claims_stop_deferral_recovery_index
                 WHERE recovered.subject=seats.subject AND recovered.kind='operational.recovered'
                   AND recovered.store_index<=?2 AND json_extract(recovered.body,'$.fields.failure')=+fault.id)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_deferral_reads_do_not_scan_unrelated_diagnostic_history() {
        let store = Store::open_memory("node").unwrap();
        let sqlite_version: String = store
            .connection
            .write()
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        eprintln!("SQLite {sqlite_version}");
        let subjects = (0..256)
            .map(|i| format!("agent/orchid/worker-{i:03}"))
            .collect::<Vec<_>>();
        {
            let connection = store.connection.write();
            connection.execute("INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES ('batch/fixture','fixture',1,'fixture','1000000000000')", []).unwrap();
            connection.execute("INSERT INTO mission_runs VALUES ('orchid','orchid','revision','generation','revision','orchid',NULL,'/tmp','person/operator','{}','run','completed','terminal','1000000000000','1000000000000')", []).unwrap();
            for (i, subject) in subjects.iter().enumerate() {
                let id = format!("{i:064x}");
                connection.execute("INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,'batch/fixture',?2,'intent.desired','fixture','{}','[]','1000000000000')", params![id,subject]).unwrap();
                connection.execute("INSERT INTO desired(subject,kind,revision,claim_id,body,owner_run) VALUES (?1,'stop','revision',?2,'{}','mission-run/orchid')", params![subject,id]).unwrap();
            }
        }
        let mut costs = Vec::new();
        let mut previous = 0;
        for count in [1_000, 10_000] {
            {
                let connection = store.connection.write();
                for i in previous..count {
                    connection.execute("INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,'batch/fixture',?2,'harness.diagnostic','fixture',?3,'[]',?4)", params![
                        format!("diagnostic-{i}"), subjects[0],
                        r#"{"fields":{"code":"fixture-unrelated","reason":"fixture"},"evidence":[]}"#,
                        (1_000_000_001_000_u64+i).to_string(),
                    ]).unwrap();
                }
            }
            previous = count;
            // Warm statement preparation before counting exactly the reads under review.
            assert!(store.latest_stop_deferral(&subjects[0]).unwrap().is_none());
            assert!(
                store
                    .stop_deferral_faults_for(&subjects, i64::MAX as u64)
                    .unwrap()
                    .is_empty()
            );
            let before = smallclaims::sqlite::work::total();
            let started = std::time::Instant::now();
            assert!(store.latest_stop_deferral(&subjects[0]).unwrap().is_none());
            assert!(
                store
                    .stop_deferral_faults_for(&subjects, i64::MAX as u64)
                    .unwrap()
                    .is_empty()
            );
            let cost = smallclaims::sqlite::work::total() - before;
            eprintln!(
                "unrelated diagnostics={count}, terminal-owner page=256: {cost:?}, wall={:?}",
                started.elapsed()
            );
            assert!(cost.vm_steps <= 80_000, "bounded page read: {cost:?}");
            costs.push(cost);
        }
        assert!(
            costs[1].vm_steps <= costs[0].vm_steps + 256,
            "history must not increase SQL work: {costs:?}"
        );
    }
}
