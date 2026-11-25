//! Local authority and its ordered home-ACK outbox share the session journal transaction.
//! No mission projection or second selected-home registry is consulted here.
use super::{ExecutionBinding, FractalError, internal};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_ACTIVE_EXECUTIONS: usize = 4096;
pub const MAX_RETAINED_AUTHORITIES: usize = 32768;
pub const MAX_CREDENTIALS: usize = 16384;
pub const MAX_PENDING_CHANGES: usize = MAX_ACTIVE_EXECUTIONS * 2;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityChange {
    pub sequence: u64,
    pub binding: ExecutionBinding,
    pub revoked: bool,
}

pub fn initialize_local_authority(connection: &Connection) -> Result<(), FractalError> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hfca990733a00 owner=fractal_light_node_core::authority::local TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_execution_authority_v1 (
             host TEXT NOT NULL, revision TEXT NOT NULL, binding_json TEXT NOT NULL,
             subject TEXT NOT NULL, runtime TEXT NOT NULL, epoch INTEGER NOT NULL,
             revoked INTEGER NOT NULL DEFAULT 0, admission_sequence INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY(host, revision)
         );
         CREATE INDEX IF NOT EXISTS local_execution_authority_subject_v1
             ON local_execution_authority_v1(host, subject) WHERE revoked=0;
         CREATE INDEX IF NOT EXISTS local_execution_authority_runtime_v1
             ON local_execution_authority_v1(host, runtime) WHERE revoked=0;
         CREATE TABLE IF NOT EXISTS local_execution_credentials_v1 (
             secret_hash TEXT PRIMARY KEY, host TEXT NOT NULL, revision TEXT NOT NULL,
             expires_at_unix_ms TEXT NOT NULL CHECK(
                 expires_at_unix_ms<>'' AND expires_at_unix_ms NOT GLOB '*[^0-9]*'
                 AND (expires_at_unix_ms='0' OR substr(expires_at_unix_ms,1,1)<>'0')),
             FOREIGN KEY(host, revision) REFERENCES local_execution_authority_v1(host, revision)
         );
         CREATE TABLE IF NOT EXISTS local_authority_epochs_v1 (
             epoch INTEGER PRIMARY KEY, last_sequence INTEGER NOT NULL DEFAULT 0,
             ack_through INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS local_authority_changes_v1 (
             epoch INTEGER NOT NULL, sequence INTEGER NOT NULL, binding_json TEXT NOT NULL,
             host TEXT NOT NULL, subject TEXT NOT NULL, runtime TEXT NOT NULL,
             revoked INTEGER NOT NULL, PRIMARY KEY(epoch, sequence)
         );
         CREATE INDEX IF NOT EXISTS local_authority_changes_conflict_v1
             ON local_authority_changes_v1(host, subject, runtime) WHERE revoked=1;
         CREATE TABLE IF NOT EXISTS local_authority_proposals_v1 (
             host TEXT NOT NULL, revision TEXT NOT NULL, binding_json TEXT NOT NULL,
             subject TEXT NOT NULL, runtime TEXT NOT NULL, epoch INTEGER NOT NULL,
             PRIMARY KEY(host, revision)
         );"
    ).map_err(internal)
}

fn refused(reason: &str) -> FractalError {
    FractalError::new("local-execution-authority-refused", reason)
}
fn backpressure(reason: &str) -> FractalError {
    FractalError::new("local-authority-backpressure", reason)
}
fn decode(encoded: &str) -> Result<ExecutionBinding, FractalError> {
    let binding: ExecutionBinding = serde_json::from_str(encoded).map_err(internal)?;
    binding.validate()?;
    Ok(binding)
}
fn retained(connection: &Connection) -> Result<usize, FractalError> {
    connection.query_row(
        "SELECT (SELECT COUNT(*) FROM local_execution_authority_v1)
              + (SELECT COUNT(*) FROM local_authority_proposals_v1)", [], |row| row.get(0),
    ).map_err(internal)
}
fn previous(connection: &Connection, binding: &ExecutionBinding) -> Result<Option<(String, bool)>, FractalError> {
    connection.query_row(
        "SELECT binding_json, revoked FROM local_execution_authority_v1 WHERE host=?1 AND revision=?2",
        params![binding.host, binding.authorization_revision], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(internal)
}
fn awaiting_revocation(connection: &Connection, binding: &ExecutionBinding) -> Result<bool, FractalError> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_authority_changes_v1
         WHERE host=?1 AND revoked=1 AND (subject=?2 OR runtime=?3))",
        params![binding.host, binding.subject, binding.runtime], |row| row.get(0),
    ).map_err(internal)
}
fn append_change(tx: &Transaction<'_>, binding: &ExecutionBinding, revoked: bool) -> Result<u64, FractalError> {
    let pending: usize = tx.query_row("SELECT COUNT(*) FROM local_authority_changes_v1", [], |row| row.get(0)).map_err(internal)?;
    if pending >= MAX_PENDING_CHANGES { return Err(backpressure("the authority change outbox is full")); }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h846afd822e7f owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_authority_epochs_v1(epoch) VALUES (?1)", [binding.epoch]).map_err(internal)?;
    let previous: u64 = tx.query_row("SELECT last_sequence FROM local_authority_epochs_v1 WHERE epoch=?1", [binding.epoch], |row| row.get(0)).map_err(internal)?;
    let sequence = previous.checked_add(1).filter(|next| *next <= i64::MAX as u64)
        .ok_or_else(|| backpressure("the authority sequence space is exhausted"))?;
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h23bb01dbd333 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_authority_changes_v1(epoch, sequence, binding_json, host, subject, runtime, revoked)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![binding.epoch, sequence, serde_json::to_string(binding).map_err(internal)?, binding.host, binding.subject, binding.runtime, revoked],
    ).map_err(internal)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h7be9a2b9416c owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_authority_epochs_v1 SET last_sequence=?2 WHERE epoch=?1", params![binding.epoch, sequence]).map_err(internal)?;
    Ok(sequence)
}

/// Direct admission never replaces authority implicitly. Revocation must commit and be ACKed
/// by its home before any successor can mint a credential or enter the native backend.
pub fn admit_execution_tx(tx: &Transaction<'_>, binding: &ExecutionBinding) -> Result<(), FractalError> {
    binding.validate()?;
    let encoded = serde_json::to_string(binding).map_err(internal)?;
    if let Some((prior, revoked)) = previous(tx, binding)? {
        return if !revoked && prior == encoded { Ok(()) }
        else { Err(refused("the authorization revision is revoked or names a different execution")) };
    }
    let conflict: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_execution_authority_v1
         WHERE host=?1 AND revoked=0 AND (subject=?2 OR runtime=?3))",
        params![binding.host, binding.subject, binding.runtime], |row| row.get(0),
    ).map_err(internal)?;
    if conflict { return Err(refused("revoke the prior execution before admitting its successor")); }
    if awaiting_revocation(tx, binding)? { return Err(backpressure("the home has not acknowledged predecessor revocation")); }
    let active: usize = tx.query_row("SELECT COUNT(*) FROM local_execution_authority_v1 WHERE revoked=0", [], |row| row.get(0)).map_err(internal)?;
    let pending: usize = tx.query_row("SELECT COUNT(*) FROM local_authority_changes_v1", [], |row| row.get(0)).map_err(internal)?;
    // Reserve one outbox slot for every live execution's later revocation.
    if active >= MAX_ACTIVE_EXECUTIONS || pending + active + 2 > MAX_PENDING_CHANGES {
        return Err(backpressure("the active authority or reserved revocation outbox is full"));
    }
    let proposed: Option<String> = tx.query_row(
        "SELECT binding_json FROM local_authority_proposals_v1 WHERE host=?1 AND revision=?2",
        params![binding.host, binding.authorization_revision], |row| row.get(0),
    ).optional().map_err(internal)?;
    if proposed.as_ref().is_some_and(|prior| prior != &encoded) {
        return Err(refused("the proposed authorization revision changed identity"));
    }
    if proposed.is_none() && retained(tx)? >= MAX_RETAINED_AUTHORITIES {
        return Err(backpressure("the retained authority fence journal is full"));
    }
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h7ef13aff7482 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_execution_authority_v1(host, revision, binding_json, subject, runtime, epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![binding.host, binding.authorization_revision, encoded, binding.subject, binding.runtime, binding.epoch],
    ).map_err(internal)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-hd29ab23563f2 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_authority_proposals_v1 WHERE host=?1 AND revision=?2", params![binding.host, binding.authorization_revision]).map_err(internal)?;
    let sequence = append_change(tx, binding, false)?;
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h6c25c994f66a owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_execution_authority_v1 SET admission_sequence=?3 WHERE host=?1 AND revision=?2",
        params![binding.host, binding.authorization_revision, sequence],
    ).map_err(internal)?;
    Ok(())
}

/// Offer a freshly checked native/program identity. `false` is a durable pending proposal,
/// not an error: commit it and its predecessor revocations before returning backpressure.
/// A later home ACK never auto-admits: the native caller must revalidate and offer again.
pub fn offer_execution_tx(tx: &Transaction<'_>, binding: &ExecutionBinding) -> Result<bool, FractalError> {
    binding.validate()?;
    let encoded = serde_json::to_string(binding).map_err(internal)?;
    if let Some((prior, revoked)) = previous(tx, binding)? {
        return if !revoked && prior == encoded { Ok(true) }
        else { Err(refused("the offered revision was revoked or changed identity")) };
    }
    let proposal: Option<String> = tx.query_row(
        "SELECT binding_json FROM local_authority_proposals_v1 WHERE host=?1 AND revision=?2",
        params![binding.host, binding.authorization_revision], |row| row.get(0),
    ).optional().map_err(internal)?;
    if proposal.as_ref().is_some_and(|prior| prior != &encoded) {
        return Err(refused("the proposed revision changed identity"));
    }
    if proposal.is_none() && retained(tx)? >= MAX_RETAINED_AUTHORITIES {
        return Err(backpressure("the retained authority fence journal is full"));
    }
    let mut statement = tx.prepare(
        "SELECT binding_json FROM local_execution_authority_v1
         WHERE host=?1 AND revoked=0 AND (subject=?2 OR runtime=?3) ORDER BY revision",
    ).map_err(internal)?;
    let conflicts = statement.query_map(params![binding.host, binding.subject, binding.runtime], |row| row.get::<_, String>(0))
        .map_err(internal)?.collect::<Result<Vec<_>, _>>().map_err(internal)?;
    drop(statement);
    for prior in conflicts {
        let prior = decode(&prior)?;
        revoke_execution_tx(tx, &prior.host, &prior.authorization_revision)?;
    }
    // Superseded proposals never had authority, but retain their revision as a tombstone so
    // a stale adoption event cannot resurrect them after a newer native incarnation.
    let mut statement = tx.prepare(
        "SELECT binding_json FROM local_authority_proposals_v1
         WHERE host=?1 AND revision<>?4 AND (subject=?2 OR runtime=?3)",
    ).map_err(internal)?;
    let superseded = statement.query_map(params![binding.host, binding.subject, binding.runtime, binding.authorization_revision], |row| row.get::<_, String>(0))
        .map_err(internal)?.collect::<Result<Vec<_>, _>>().map_err(internal)?;
    drop(statement);
    for prior in superseded {
        let prior = decode(&prior)?;
        tombstone_proposal(tx, &prior)?;
    }
    if proposal.is_none() {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h84750ef1c986 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_authority_proposals_v1(host, revision, binding_json, subject, runtime, epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![binding.host, binding.authorization_revision, encoded, binding.subject, binding.runtime, binding.epoch],
        ).map_err(internal)?;
    }
    if awaiting_revocation(tx, binding)? { return Ok(false); }
    admit_execution_tx(tx, binding)?;
    Ok(true)
}

fn tombstone_proposal(tx: &Transaction<'_>, binding: &ExecutionBinding) -> Result<(), FractalError> {
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-hb63a6f1d5a4d owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_execution_authority_v1(host, revision, binding_json, subject, runtime, epoch, revoked) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
        params![binding.host, binding.authorization_revision, serde_json::to_string(binding).map_err(internal)?, binding.subject, binding.runtime, binding.epoch],
    ).map_err(internal)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-hdaa80fa55663 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_authority_proposals_v1 WHERE host=?1 AND revision=?2", params![binding.host, binding.authorization_revision]).map_err(internal)?;
    Ok(())
}

pub fn revoke_execution_tx(tx: &Transaction<'_>, host: &str, revision: &str) -> Result<(), FractalError> {
    let prior: Option<String> = tx.query_row(
        "SELECT binding_json FROM local_execution_authority_v1 WHERE host=?1 AND revision=?2 AND revoked=0",
        params![host, revision], |row| row.get(0),
    ).optional().map_err(internal)?;
    if let Some(prior) = prior {
        let binding = decode(&prior)?;
        append_change(tx, &binding, true)?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h3ae46b6e28a0 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_execution_authority_v1 SET revoked=1 WHERE host=?1 AND revision=?2", params![host, revision]).map_err(internal)?;
    }
    let proposal: Option<String> = tx.query_row(
        "SELECT binding_json FROM local_authority_proposals_v1 WHERE host=?1 AND revision=?2",
        params![host, revision], |row| row.get(0),
    ).optional().map_err(internal)?;
    if let Some(proposal) = proposal { tombstone_proposal(tx, &decode(&proposal)?)?; }
    Ok(())
}

/// Includes active and proposed executions even when their command receipts were compacted.
pub fn revoke_epoch_tx(tx: &Transaction<'_>, host: &str, epoch: u64) -> Result<(), FractalError> {
    let mut statement = tx.prepare(
        "SELECT revision FROM local_execution_authority_v1 WHERE host=?1 AND epoch=?2 AND revoked=0
         UNION SELECT revision FROM local_authority_proposals_v1 WHERE host=?1 AND epoch=?2",
    ).map_err(internal)?;
    let revisions = statement.query_map(params![host, epoch], |row| row.get::<_, String>(0))
        .map_err(internal)?.collect::<Result<Vec<_>, _>>().map_err(internal)?;
    drop(statement);
    for revision in revisions { revoke_execution_tx(tx, host, &revision)?; }
    Ok(())
}

pub fn pending_changes(connection: &Connection, epoch: u64) -> Result<Vec<AuthorityChange>, FractalError> {
    let mut statement = connection.prepare(
        "SELECT sequence, binding_json, revoked FROM local_authority_changes_v1 WHERE epoch=?1 ORDER BY sequence LIMIT ?2",
    ).map_err(internal)?;
    let rows = statement.query_map(params![epoch, MAX_PENDING_CHANGES + 1], |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))).map_err(internal)?;
    let mut changes = Vec::new();
    for row in rows {
        if changes.len() == MAX_PENDING_CHANGES { return Err(backpressure("the authority outbox exceeds its bound")); }
        let (sequence, encoded, revoked) = row.map_err(internal)?;
        let binding = decode(&encoded)?;
        if binding.epoch != epoch { return Err(refused("the authority change epoch is corrupt")); }
        changes.push(AuthorityChange { sequence, binding, revoked });
    }
    Ok(changes)
}

pub fn acknowledged_change_high_water(connection: &Connection, epoch: u64) -> Result<u64, FractalError> {
    Ok(connection.query_row("SELECT ack_through FROM local_authority_epochs_v1 WHERE epoch=?1", [epoch], |row| row.get(0))
        .optional().map_err(internal)?.unwrap_or(0))
}

pub fn acknowledge_changes_tx(tx: &Transaction<'_>, epoch: u64, through: u64) -> Result<(), FractalError> {
    let (last, floor): (u64, u64) = tx.query_row(
        "SELECT last_sequence, ack_through FROM local_authority_epochs_v1 WHERE epoch=?1", [epoch], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(internal)?.unwrap_or((0, 0));
    if through <= floor { return Ok(()); }
    if through > last { return Err(refused("the authority ACK is beyond the issued sequence")); }
    let count: u64 = tx.query_row("SELECT COUNT(*) FROM local_authority_changes_v1 WHERE epoch=?1 AND sequence<=?2", params![epoch, through], |row| row.get(0)).map_err(internal)?;
    if count != through - floor { return Err(refused("the authority ACK crosses a missing change")); }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h541d7b3c8525 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_authority_changes_v1 WHERE epoch=?1 AND sequence<=?2", params![epoch, through]).map_err(internal)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h7e8bdf58715c owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_authority_epochs_v1 SET ack_through=?2 WHERE epoch=?1", params![epoch, through]).map_err(internal)?;
    Ok(())
}

pub fn require_execution(connection: &Connection, binding: &ExecutionBinding) -> Result<(), FractalError> {
    binding.validate()?;
    let encoded = serde_json::to_string(binding).map_err(internal)?;
    if let Some((prior, revoked)) = previous(connection, binding)? {
        if !revoked && prior == encoded { return Ok(()); }
        return Err(refused("the execution is revoked or its admitted identity differs"));
    }
    let proposed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_authority_proposals_v1 WHERE host=?1 AND revision=?2 AND binding_json=?3)",
        params![binding.host, binding.authorization_revision, encoded], |row| row.get(0),
    ).map_err(internal)?;
    if proposed { Err(backpressure("the execution awaits a fresh offer after home revocation ACK")) }
    else { Err(refused("the execution is not currently admitted")) }
}

/// Native entry and credential issuance share the exact durable home-ACK fence.
pub fn require_acknowledged_execution(connection: &Connection, binding: &ExecutionBinding) -> Result<(), FractalError> {
    require_execution(connection, binding)?;
    let acknowledged: bool = connection.query_row(
        "SELECT authority.admission_sequence>0 AND authority.admission_sequence<=epoch.ack_through
         FROM local_execution_authority_v1 authority
         JOIN local_authority_epochs_v1 epoch ON epoch.epoch=authority.epoch
         WHERE authority.host=?1 AND authority.revision=?2",
        params![binding.host, binding.authorization_revision], |row| row.get(0),
    ).optional().map_err(internal)?.unwrap_or(false);
    if !acknowledged {
        return Err(backpressure("the home has not acknowledged this execution admission"));
    }
    Ok(())
}

pub fn issue_local_credential_tx(tx: &Transaction<'_>, binding: &ExecutionBinding, lifetime_ms: u64, now_ms: u64) -> Result<String, FractalError> {
    require_acknowledged_execution(tx, binding)?;
    // Match existing store capabilities' u128 expiry: program attempts deliberately request
    // u64::MAX lifetime and end by ordered revocation, not a 292-million-year SQLite i64 cap.
    let now = now_ms.to_string();
    let expires = ((now_ms as u128) + (lifetime_ms as u128)).to_string();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h7082b3f480c9 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_execution_credentials_v1 WHERE length(expires_at_unix_ms)<length(?1)
         OR (length(expires_at_unix_ms)=length(?1) AND expires_at_unix_ms<=?1)",
        [&now],
    ).map_err(internal)?;
    let count: usize = tx.query_row("SELECT COUNT(*) FROM local_execution_credentials_v1", [], |row| row.get(0)).map_err(internal)?;
    if count >= MAX_CREDENTIALS { return Err(backpressure("the unexpired credential hash journal is full")); }
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| refused(&error.to_string()))?;
    let secret = hex::encode(bytes);
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-hda5c0f67ec00 owner=fractal_light_node_core::authority::local TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_execution_credentials_v1(secret_hash, host, revision, expires_at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
        params![hex::encode(Sha256::digest(secret.as_bytes())), binding.host, binding.authorization_revision, expires],
    ).map_err(internal)?;
    Ok(secret)
}

/// Resolve offline after restart from hashed local admission, never from a mission lookup.
pub fn local_binding(connection: &Connection, secret: &str, now_ms: u64) -> Result<ExecutionBinding, FractalError> {
    let (encoded, expires): (String, String) = connection.query_row(
        "SELECT authority.binding_json, credential.expires_at_unix_ms FROM local_execution_credentials_v1 credential
         JOIN local_execution_authority_v1 authority ON authority.host=credential.host AND authority.revision=credential.revision
         WHERE credential.secret_hash=?1 AND authority.revoked=0",
        [hex::encode(Sha256::digest(secret.as_bytes()))], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(internal)?.ok_or_else(|| refused("the local credential is unknown or revoked"))?;
    if expires.parse::<u128>().map_err(internal)? <= now_ms as u128 {
        return Err(refused("the local credential is expired"));
    }
    decode(&encoded)
}

pub fn active_bindings(connection: &Connection) -> Result<Vec<ExecutionBinding>, FractalError> {
    let mut statement = connection.prepare(
        "SELECT binding_json FROM local_execution_authority_v1 WHERE revoked=0 ORDER BY host, runtime, revision LIMIT ?1",
    ).map_err(internal)?;
    let rows = statement.query_map([MAX_ACTIVE_EXECUTIONS + 1], |row| row.get::<_, String>(0)).map_err(internal)?;
    let mut bindings = Vec::new();
    for row in rows {
        if bindings.len() == MAX_ACTIVE_EXECUTIONS { return Err(backpressure("the active authority report exceeds its bound")); }
        bindings.push(decode(&row.map_err(internal)?)?);
    }
    Ok(bindings)
}

#[cfg(test)]
mod tests;
