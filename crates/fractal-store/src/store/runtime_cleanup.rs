//! Node-local, exact-incarnation proof that historical exec cleanup is complete.
//! Installation creates an empty table: no history scan or large-table rewrite.
use super::*;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-haba524422e8c owner=fractal_store::store::runtime_cleanup TODO=chunk-or-reference-variable-columns */
        "CREATE TABLE IF NOT EXISTS local_runtime_cleanup_settled (
            runtime_id TEXT NOT NULL,
            incarnation TEXT NOT NULL,
            subject TEXT NOT NULL,
            PRIMARY KEY(runtime_id, incarnation)
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS local_runtime_cleanup_subject ON local_runtime_cleanup_settled(subject);",
    )?;
    Ok(())
}

/// Permanent subject removal, after any projection reconstruction has finished.
pub fn forget_subject(connection: &Connection, subject: &str) -> Result<()> {
    // Base projection reconstruction can run before daemon-local schema installation.
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='local_runtime_cleanup_settled' AND type='table')",
        [], |row| row.get(0),
    )?;
    if installed {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h3a3c620624e1 owner=fractal_store::store::runtime_cleanup TODO=bound-or-reference-values-and-reclamation */
            "DELETE FROM local_runtime_cleanup_settled WHERE subject=?1",
            [subject],
        )?;
    }
    Ok(())
}

/// Only a fenced, newly applied start result can supersede this subject's generation.
/// Another retained desired subject may still reconcile an older historical incarnation.
pub fn apply_generation(
    connection: &Connection, subject: &str, runtime_id: &str, incarnation: &str,
) -> Result<()> {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h355a9421263e owner=fractal_store::store::runtime_cleanup TODO=bound-or-reference-values-and-reclamation */
        "DELETE FROM local_runtime_cleanup_settled
         WHERE runtime_id=?1 AND incarnation<>?2
           AND (subject=?3 OR NOT EXISTS(SELECT 1 FROM desired
             WHERE desired.subject=local_runtime_cleanup_settled.subject))",
        params![runtime_id, incarnation, subject],
    )?;
    Ok(())
}

impl Store {
    pub fn runtime_cleanup_settled(&self, runtime_id: &str, incarnation: &str) -> Result<bool> {
        Ok(self.readers.get().prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM local_runtime_cleanup_settled WHERE runtime_id=?1 AND incarnation=?2)",
        )?.query_row(/* sqlite-large-value: allow-for-now lv-hf597e11eb8de owner=fractal_store::store::runtime_cleanup TODO=trace-statement-and-value-representation */ params![runtime_id, incarnation], |row| row.get(0))?)
    }

    /// Call only after native exact-generation whole-scope retirement has been proven.
    pub fn settle_runtime_cleanup(&self, subject: &str, runtime_id: &str, incarnation: &str) -> Result<()> {
        self.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-he32bd26469c5 owner=fractal_store::store::runtime_cleanup TODO=bound-or-reference-values-and-reclamation */
            "INSERT INTO local_runtime_cleanup_settled(runtime_id,incarnation,subject) VALUES(?1,?2,?3)
             ON CONFLICT(runtime_id,incarnation) DO UPDATE SET subject=excluded.subject",
            params![runtime_id, incarnation, subject],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_receipts_survive_late_observations_and_reconstruction() {
        let store = Store::open_memory("node").unwrap();
        let subject = "agent/example/history";
        runtime_starts_tests_shared_seats::publish(&store,
            "version 2\nagent \"example/history\" { command \"true\" }\n", "history");
        store.settle_runtime_cleanup(subject, "work", "current").unwrap();
        let observed = ClaimInput {
            subject: subject.into(), kind: "runtime.observed".into(), actor: None,
            fields: BTreeMap::from([
                ("runtime_id".into(), json!("work")),
                ("incarnation_id".into(), json!("old")),
                ("status".into(), json!("exited")),
                ("terminal".into(), json!(false)),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        };
        // Both local observations and ordinary driver exit claims may arrive late.
        store.append_latest_observation(&observed, now_ms()).unwrap();
        store.append_claim(&observed).unwrap();
        assert!(store.runtime_cleanup_settled("work", "current").unwrap());
        {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            rebuild_base_aggregate_tx(&transaction, &Aggregate::Desired(subject.into())).unwrap();
            transaction.commit().unwrap();
        }
        assert!(store.desired_subjects().unwrap().iter().any(|desired| desired.subject == subject));
        assert!(store.runtime_cleanup_settled("work", "current").unwrap());
    }
}
