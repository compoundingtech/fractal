//! Explicitly provisioned native custody. Registry discovery never grants ownership.
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemberLifecycle {
    #[default]
    Service,
    AdoptOnly,
    /// Borrow a caller-owned shell's PTY; never launch or terminate that PTY.
    TerminalBound,
}

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
    pub lifecycle: MemberLifecycle,
    pub custody: SessionCustody,
    pub start_operation: Option<String>,
    pub invocation: Option<String>,
}

/// The additive proof column intentionally leaves pre-existing authoritative rows unproven.
pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS local_session_custody (
        subject TEXT NOT NULL, incarnation TEXT NOT NULL, body TEXT NOT NULL,
        active INTEGER NOT NULL DEFAULT 1, custody_settled INTEGER NOT NULL DEFAULT 0,
        detached_by TEXT, detach_receipted INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(subject,incarnation)
    ) WITHOUT ROWID;
    CREATE UNIQUE INDEX IF NOT EXISTS local_session_current ON local_session_custody(subject) WHERE active=1;")?;
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('local_session_custody') WHERE name='native_retirement_verified')",
        [], |row| row.get(0))?;
    if !installed {
        connection.execute("ALTER TABLE local_session_custody ADD COLUMN native_retirement_verified INTEGER NOT NULL DEFAULT 0", [])?;
    }
    Ok(())
}

pub fn validate(binding: &LocalSession) -> Result<()> {
    anyhow::ensure!(binding.subject.len() <= 1024 && binding.runtime_id.len() <= 1024
        && binding.incarnation.len() <= 1024 && binding.native_incarnation.len() <= 1024
        && binding.driver.as_ref().is_none_or(|v| v.len() <= 128)
        && binding.start_operation.as_ref().is_none_or(|v| !v.is_empty() && v.len() <= 1024)
        && binding.invocation.as_ref().is_none_or(|v| v.len() <= 1024), "session binding exceeds bounds");
    anyhow::ensure!(!binding.subject.is_empty() && !binding.runtime_id.is_empty()
        && !binding.native_incarnation.is_empty(), "session binding identity is empty");
    if binding.custody == SessionCustody::Borrowed {
        anyhow::ensure!(binding.terminal && binding.lifecycle == MemberLifecycle::TerminalBound
            && binding.invocation.as_deref() == Some(binding.incarnation.as_str())
            && binding.incarnation.strip_prefix(binding.native_incarnation.as_str())
                .is_some_and(|suffix| suffix.starts_with(":bound:") && suffix.len() > 7),
            "borrowed custody requires an exact terminal invocation fence");
    } else {
        anyhow::ensure!(binding.incarnation == binding.native_incarnation && binding.invocation.is_none(),
            "owned custody requires an exact native generation");
    }
    Ok(())
}

/// Provisioning may introduce an invocation, never rewrite who owns its physical generation.
/// Historical rows continue to fence escalation after logical detach or receipt settlement.
pub fn require_provisioning_compatible(connection: &Connection, binding: &LocalSession) -> Result<()> {
    validate(binding)?;
    let mut statement = connection.prepare("SELECT body FROM local_session_custody
        WHERE json_extract(body,'$.runtime_id')=?1 AND json_extract(body,'$.native_incarnation')=?2")?;
    let rows = statement.query_map(params![binding.runtime_id, binding.native_incarnation],
        |row| row.get::<_, String>(0))?;
    for body in rows {
        let prior: LocalSession = serde_json::from_str(&body?)?;
        anyhow::ensure!(prior.subject == binding.subject && prior.custody == binding.custody
            && prior.terminal == binding.terminal && prior.start_operation == binding.start_operation,
            "provisioned metadata would change retained native ownership or provenance");
    }
    Ok(())
}

/// Call under the journal's exclusive transaction. No graph commit finalizers participate.
pub fn bind(connection: &Connection, binding: &LocalSession) -> Result<()> {
    validate(binding)?;
    let prior: Option<Option<String>> = connection.query_row(
        "SELECT detached_by FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
        params![binding.subject, binding.incarnation], |row| row.get(0)).optional()?;
    if matches!(prior, Some(Some(_))) { return Ok(()); }
    if prior.is_none() {
        let count: u64 = connection.query_row("SELECT count(*) FROM local_session_custody", [], |row| row.get(0))?;
        anyhow::ensure!(count < 4096, "executor session custody capacity reached");
    }
    connection.execute("UPDATE local_session_custody SET active=0 WHERE subject=?1 AND incarnation<>?2 AND active=1",
        params![binding.subject, binding.incarnation])?;
    connection.execute("INSERT INTO local_session_custody(subject,incarnation,body) VALUES(?1,?2,?3)
        ON CONFLICT(subject,incarnation) DO UPDATE SET body=excluded.body,active=1,native_retirement_verified=0
        WHERE local_session_custody.detached_by IS NULL
          AND (local_session_custody.body<>excluded.body OR local_session_custody.active<>1)",
        params![binding.subject, binding.incarnation, serde_json::to_string(binding)?])?;
    Ok(())
}

pub fn local_session(connection: &Connection, subject: &str) -> Result<Option<LocalSession>> {
    let body: Option<String> = connection.query_row(
        "SELECT body FROM local_session_custody WHERE subject=?1 AND active=1 AND custody_settled=0 AND detached_by IS NULL",
        [subject], |row| row.get(0)).optional()?;
    body.map(|body| serde_json::from_str(&body).map_err(Into::into)).transpose()
}

/// Historical provenance grants no live control authority.
pub fn local_session_at(connection: &Connection, subject: &str, incarnation: &str) -> Result<Option<LocalSession>> {
    let body: Option<String> = connection.query_row(
        "SELECT body FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
        params![subject, incarnation], |row| row.get(0)).optional()?;
    body.map(|body| serde_json::from_str(&body).map_err(Into::into)).transpose()
}

pub fn detached(connection: &Connection, subject: &str, incarnation: &str) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM local_session_custody
        WHERE subject=?1 AND incarnation=?2 AND detached_by IS NOT NULL)",
        params![subject, incarnation], |row| row.get(0))?)
}

/// Durable logical detach only; never signal a person's shell.
pub fn detach(connection: &Connection, subject: &str, incarnation: &str, operation: &str) -> Result<()> {
    anyhow::ensure!(!operation.is_empty() && operation.len() <= 1024, "invalid detach operation");
    let changed = connection.execute("UPDATE local_session_custody
        SET detached_by=COALESCE(detached_by,?3),custody_settled=1 WHERE subject=?1 AND incarnation=?2
        AND json_extract(body,'$.custody')='Borrowed'",
        params![subject, incarnation, operation])?;
    anyhow::ensure!(changed == 1, "stale or non-borrowed session detach");
    Ok(())
}

pub fn verify_retirement(connection: &Connection, binding: &LocalSession) -> Result<()> {
    anyhow::ensure!(binding.custody == SessionCustody::Owned, "borrowed custody cannot retire its native runtime");
    let changed = connection.execute("UPDATE local_session_custody SET native_retirement_verified=1
        WHERE subject=?1 AND incarnation=?2 AND body=?3 AND active=1
          AND custody_settled=0 AND detached_by IS NULL",
        params![binding.subject, binding.incarnation, serde_json::to_string(binding)?])?;
    anyhow::ensure!(changed == 1, "native retirement custody changed before its proof was recorded");
    Ok(())
}

pub fn retirement_verified(connection: &Connection, binding: &LocalSession) -> Result<bool> {
    if binding.custody != SessionCustody::Owned { return Ok(false); }
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM local_session_custody
        WHERE subject=?1 AND incarnation=?2 AND body=?3 AND native_retirement_verified=1)",
        params![binding.subject, binding.incarnation, serde_json::to_string(binding)?], |row| row.get(0))?)
}

/// Requires exact positive physical proof; receipt remains blocked until this succeeds.
pub fn settle(connection: &Connection, binding: &LocalSession) -> Result<()> {
    anyhow::ensure!(retirement_verified(connection, binding)?, "native custody settlement lacks retirement proof");
    let changed = connection.execute("UPDATE local_session_custody SET custody_settled=1
        WHERE subject=?1 AND incarnation=?2 AND body=?3 AND native_retirement_verified=1",
        params![binding.subject, binding.incarnation, serde_json::to_string(binding)?])?;
    anyhow::ensure!(changed == 1, "native custody changed before local finalization");
    Ok(())
}
