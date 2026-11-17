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
    Ok(())
}

impl Store {
    pub fn runtime_cleanup_settled(&self, runtime_id: &str, incarnation: &str) -> Result<bool> {
        Ok(self.readers.get().prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM local_runtime_cleanup_settled WHERE runtime_id=?1 AND incarnation=?2)",
        )?.query_row(params![runtime_id, incarnation], |row| row.get(0))?)
    }

    /// Call only after native exact-generation whole-scope retirement has been proven.
    pub fn settle_runtime_cleanup(&self, runtime_id: &str, incarnation: &str) -> Result<()> {
        self.connection.write().execute(
            "INSERT OR IGNORE INTO local_runtime_cleanup_settled(runtime_id,incarnation) VALUES(?1,?2)",
            params![runtime_id, incarnation],
        )?;
        Ok(())
    }
}
