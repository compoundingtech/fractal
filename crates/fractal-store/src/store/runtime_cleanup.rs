//! Node-local, exact-incarnation proof that historical exec cleanup is complete.
//! Installation creates an empty table: no history scan or large-table rewrite.
use super::*;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS local_runtime_cleanup_settled (
            runtime_id TEXT NOT NULL,
            incarnation TEXT NOT NULL,
            PRIMARY KEY(runtime_id, incarnation)
        ) WITHOUT ROWID;",
    )?;
    let has_subject: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('local_runtime_cleanup_settled') WHERE name='subject')",
        [], |row| row.get(0),
    )?;
    if !has_subject {
        // Nullable ADD COLUMN changes only schema metadata, not existing receipt rows.
        connection.execute_batch("ALTER TABLE local_runtime_cleanup_settled ADD COLUMN subject TEXT;")?;
    }
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS local_runtime_cleanup_subject ON local_runtime_cleanup_settled(subject);
         CREATE TRIGGER IF NOT EXISTS local_runtime_cleanup_incarnation
         AFTER INSERT ON local_observations
         WHEN NEW.kind='runtime.observed'
          AND json_type(NEW.body,'$.fields.runtime_id')='text'
          AND json_type(NEW.body,'$.fields.incarnation_id')='text'
         BEGIN
           DELETE FROM local_runtime_cleanup_settled
           WHERE runtime_id=json_extract(NEW.body,'$.fields.runtime_id')
             AND incarnation<>json_extract(NEW.body,'$.fields.incarnation_id');
           UPDATE local_runtime_cleanup_settled SET subject=NEW.subject
           WHERE runtime_id=json_extract(NEW.body,'$.fields.runtime_id')
             AND incarnation=json_extract(NEW.body,'$.fields.incarnation_id')
             AND subject IS NOT NEW.subject;
         END;",
    )?;
    Ok(())
}

/// Permanent subject removal, after any projection reconstruction has finished.
/// The observation lookup also reclaims receipts written before subject ownership was added.
pub fn forget_subject(connection: &Connection, subject: &str) -> Result<()> {
    // Base projection reconstruction can run before daemon-local schema installation.
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='local_runtime_cleanup_settled' AND type='table')",
        [], |row| row.get(0),
    )?;
    if installed {
        connection.execute(
            "DELETE FROM local_runtime_cleanup_settled WHERE subject=?1
             OR (subject IS NULL AND runtime_id IN (
               SELECT json_extract(body,'$.fields.runtime_id') FROM local_observations
               WHERE subject=?1 AND kind='runtime.observed'))",
            [subject],
        )?;
    }
    Ok(())
}

impl Store {
    pub fn runtime_cleanup_settled(&self, runtime_id: &str, incarnation: &str) -> Result<bool> {
        Ok(self.readers.get().prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM local_runtime_cleanup_settled WHERE runtime_id=?1 AND incarnation=?2)",
        )?.query_row(params![runtime_id, incarnation], |row| row.get(0))?)
    }

    /// Call only after native exact-generation whole-scope retirement has been proven.
    pub fn settle_runtime_cleanup(&self, subject: &str, runtime_id: &str, incarnation: &str) -> Result<()> {
        self.connection.write().execute(
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
    fn settlement_receipts_follow_incarnation_events_not_history_age() {
        let store = Store::open_memory("node").unwrap();
        store.settle_runtime_cleanup("exec/work", "work", "old").unwrap();
        store.settle_runtime_cleanup("exec/history", "history", "historical").unwrap();
        let observed = |incarnation: &str, status: &str| ClaimInput {
            subject: "exec/work".into(), kind: "runtime.observed".into(), actor: None,
            fields: BTreeMap::from([
                ("runtime_id".into(), json!("work")),
                ("incarnation_id".into(), json!(incarnation)),
                ("status".into(), json!(status)),
                ("terminal".into(), json!(false)),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        };
        // A rolled-back recording cannot reclaim the old generation.
        {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            insert_local_observation_tx(&transaction, "node", &observed("new", "running"), now_ms()).unwrap();
            transaction.rollback().unwrap();
        }
        assert!(store.runtime_cleanup_settled("work", "old").unwrap());
        store.append_latest_observation(&observed("new", "running"), now_ms()).unwrap();
        assert!(!store.runtime_cleanup_settled("work", "old").unwrap());
        store.settle_runtime_cleanup("exec/work", "work", "new").unwrap();
        store.append_latest_observation(&observed("new", "exited"), now_ms()).unwrap();
        assert!(store.runtime_cleanup_settled("work", "new").unwrap());
        assert!(store.runtime_cleanup_settled("history", "historical").unwrap());
    }
}
