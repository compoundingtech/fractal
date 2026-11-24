//! Executor-local custody. These rows are not graph projections and never authorize a launch.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionCustody { Owned, Borrowed }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSession {
    pub subject: String,
    pub runtime_id: String,
    /// Logical seat incarnation, including the invocation fence for borrowed terminals.
    pub incarnation: String,
    /// Exact native process/PTY generation, never a PID alone.
    pub native_incarnation: String,
    pub terminal: bool,
    pub driver: Option<String>,
    pub lifecycle: crate::model::MemberLifecycle,
    pub custody: SessionCustody,
    pub start_operation: Option<String>,
    pub invocation: Option<String>,
}

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS local_session_custody (
        subject TEXT NOT NULL, incarnation TEXT NOT NULL, body TEXT NOT NULL,
        active INTEGER NOT NULL DEFAULT 1, custody_settled INTEGER NOT NULL DEFAULT 0,
        detached_by TEXT, detach_receipted INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(subject,incarnation)
    ) WITHOUT ROWID;
    CREATE UNIQUE INDEX IF NOT EXISTS local_session_current ON local_session_custody(subject) WHERE active=1;")?;
    Ok(())
}

/// Existing checkpoint removal may reclaim only closed custody with settled local receipts.
pub fn forget_subject(connection: &Connection, subject: &str) -> Result<()> {
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='local_session_custody')", [], |row| row.get(0))?;
    if installed {
        connection.execute("DELETE FROM local_session_custody WHERE subject=?1 AND custody_settled=1
            AND (detached_by IS NULL OR detach_receipted=1)
            AND NOT EXISTS(SELECT 1 FROM local_runtime_starts WHERE subject=?1 AND
                (finished=0 OR applied=0 OR receipted=0 OR (revoked_by IS NOT NULL AND cleanup_verified=0)))",
            [subject])?;
    }
    Ok(())
}

impl Store {
    /// Admission is bounded per executor. Previous generations retain custody and receipt fences.
    pub fn bind_local_session(&self, binding: &LocalSession) -> Result<()> {
        anyhow::ensure!(binding.subject.len() <= 1024 && binding.runtime_id.len() <= 1024
            && binding.incarnation.len() <= 1024 && binding.native_incarnation.len() <= 1024
            && binding.driver.as_ref().is_none_or(|v| v.len() <= 128)
            && binding.start_operation.as_ref().is_none_or(|v| v.len() <= 1024)
            && binding.invocation.as_ref().is_none_or(|v| v.len() <= 1024), "session binding exceeds bounds");
        anyhow::ensure!(!binding.subject.is_empty() && !binding.runtime_id.is_empty()
            && !binding.native_incarnation.is_empty(), "session binding identity is empty");
        if binding.custody == SessionCustody::Borrowed {
            anyhow::ensure!(binding.terminal && binding.lifecycle == crate::model::MemberLifecycle::TerminalBound
                && binding.invocation.as_deref() == Some(binding.incarnation.as_str())
                && binding.incarnation.strip_prefix(binding.native_incarnation.as_str())
                    .is_some_and(|suffix| suffix.starts_with(":bound:") && suffix.len() > 7),
                "borrowed custody requires an exact terminal invocation fence");
        } else {
            anyhow::ensure!(binding.incarnation == binding.native_incarnation && binding.invocation.is_none(),
                "owned custody requires an exact native generation");
        }
        let mut connection = self.connection.write();
        let tx = connection.transaction()?;
        let prior: Option<Option<String>> = tx.query_row(
            "SELECT detached_by FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
            params![binding.subject, binding.incarnation], |row| row.get(0)).optional()?;
        if matches!(prior, Some(Some(_))) { return Ok(()); }
        if prior.is_none() {
            let count: u64 = tx.query_row("SELECT count(*) FROM local_session_custody", [], |row| row.get(0))?;
            anyhow::ensure!(count < 4096, "executor session custody capacity reached");
        }
        tx.execute("UPDATE local_session_custody SET active=0 WHERE subject=?1 AND incarnation<>?2 AND active=1",
            params![binding.subject, binding.incarnation])?;
        tx.execute("INSERT INTO local_session_custody(subject,incarnation,body) VALUES(?1,?2,?3)
            ON CONFLICT(subject,incarnation) DO UPDATE SET body=excluded.body,active=1
            WHERE local_session_custody.detached_by IS NULL
              AND (local_session_custody.body<>excluded.body OR local_session_custody.active<>1)",
            params![binding.subject, binding.incarnation, serde_json::to_string(binding)?])?;
        tx.commit()?;
        Ok(())
    }

    pub fn local_session_known(&self, subject: &str) -> Result<bool> {
        Ok(self.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM local_session_custody WHERE subject=?1)",
            [subject], |row| row.get(0))?)
    }

    pub fn local_session(&self, subject: &str) -> Result<Option<LocalSession>> {
        let body: Option<String> = self.readers.get().query_row(
            "SELECT body FROM local_session_custody WHERE subject=?1 AND active=1 AND detached_by IS NULL",
            [subject], |row| row.get(0)).optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into)).transpose()
    }

    /// Retained provenance for recovery; unlike `local_session`, this grants no live control.
    pub fn local_session_at(&self, subject: &str, incarnation: &str) -> Result<Option<LocalSession>> {
        let body: Option<String> = self.readers.get().query_row(
            "SELECT body FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
            params![subject, incarnation], |row| row.get(0)).optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into)).transpose()
    }

    pub fn local_session_detached(&self, subject: &str, incarnation: &str) -> Result<bool> {
        Ok(self.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM local_session_custody
            WHERE subject=?1 AND incarnation=?2 AND detached_by IS NOT NULL)",
            params![subject, incarnation], |row| row.get(0))?)
    }

    /// Durable logical detach only: never signal a person's shell or synthesize process exit.
    pub fn detach_local_session(&self, subject: &str, incarnation: &str, operation: &str) -> Result<()> {
        anyhow::ensure!(!operation.is_empty() && operation.len() <= 1024, "invalid detach operation");
        let changed = self.connection.write().execute("UPDATE local_session_custody
            SET detached_by=COALESCE(detached_by,?3),custody_settled=1 WHERE subject=?1 AND incarnation=?2
            AND json_extract(body,'$.custody')='Borrowed'",
            params![subject, incarnation, operation])?;
        anyhow::ensure!(changed == 1, "stale or non-borrowed session detach");
        Ok(())
    }

    pub fn pending_local_session_detaches(&self) -> Result<Vec<(LocalSession, String)>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body,detached_by FROM local_session_custody
            WHERE detached_by IS NOT NULL AND detach_receipted=0 ORDER BY subject")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|(body, operation)| Ok((serde_json::from_str(&body)?, operation))).collect()
    }

    pub fn ack_local_session_detach(&self, subject: &str, incarnation: &str, operation: &str) -> Result<()> {
        self.connection.write().execute("UPDATE local_session_custody SET detach_receipted=1
            WHERE subject=?1 AND incarnation=?2 AND detached_by=?3", params![subject, incarnation, operation])?;
        Ok(())
    }

    /// Call only after exact native closure and the corresponding observation receipt.
    pub fn settle_local_session_custody(&self, subject: &str, native_incarnation: &str) -> Result<()> {
        self.connection.write().execute("UPDATE local_session_custody SET custody_settled=1
            WHERE subject=?1 AND json_extract(body,'$.native_incarnation')=?2",
            params![subject, native_incarnation])?;
        Ok(())
    }
}
