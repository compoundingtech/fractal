//! Native admission receipts and temporary migration allowances for client operations.
use anyhow::Result;
use rusqlite::{Connection, Transaction, OptionalExtension as _, params};
use serde_json::Value;
use crate::{config::LegacyPairingAllowance, store::Store};
use smallclaims::claim::{ClaimInput, ClaimRecord};

/// Authority granted by a historical person pairing challenge with no explicit scopes.
/// The API and atomic completion admission must use the same limited policy.
pub const LIMITED_PAIRING_SCOPES: &[&str] = &[
    "read.projections",
    "write.client-diagnostics",
    "read.glasses",
    "control.glasses",
    "read.arrangements",
    "control.arrangements",
    "terminal.read",
    "control.attention",
    "control.launches",
];

/// Every configured allowance ends no later than 2026-11-15T23:00:00Z.
pub const LEGACY_ALLOWANCE_CAP_UNIX_MS: u128 = 1_794_783_600_000;

/// Digest of the immutable authority fields captured by the operator.
pub fn record_hash(origin: &str, actor: Option<&str>, fields: &Value) -> Result<String> {
    smallclaims::hash::canonical_hash(&(origin, actor, fields))
}

pub const OPERATION_AUTHORIZED: &str = "client.operation-authorized";

/// Admission receipts use a separate subject so the operation's CAS head is unchanged.
pub fn append_operation_receipt_tx(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    anyhow::ensure!(claim.kind.starts_with("custom.client.pairing")
        || claim.kind.starts_with("custom.client.terminal"), "a client operation record is required");
    let hash = record_hash(&claim.origin, claim.actor.as_deref(), &claim.body["fields"])?;
    let mut fields = serde_json::json!({
        "record_id":claim.id,
        "record_hash":hash,
    });
    if claim.kind == "custom.client.pairing-revoked" {
        // An exact retained completion can be withdrawn without admitting its bearer.
        let completion = if let Some(id) = claim.body["fields"]["completion_id"].as_str() {
            tx.query_row(
                "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE id=?1 AND kind='custom.client.pairing-completed'",
                [id], smallclaims::store::claim_from_row,
            ).optional()?
        } else {
            latest_completion(tx, &claim.subject, Some(&claim.origin), smallclaims::store::now_ms())?
        }.ok_or_else(|| anyhow::anyhow!("a completion from this issuer is required"))?;
        anyhow::ensure!(completion.subject == claim.subject && completion.origin == claim.origin,
            "revocation must name a completion from this issuer");
        if !completion_is_admitted(tx, &completion.id, &completion.origin, completion.actor.as_deref(),
            &completion.body["fields"], smallclaims::store::now_ms())? {
            let person = completion.body["fields"]["person_id"].as_str();
            anyhow::ensure!(person.is_some_and(|person| person.starts_with("person/")
                && claim.actor.as_deref() == Some(person)),
                "a legacy completion can only be withdrawn by its issuing person");
        }
        fields["completion_id"] = serde_json::json!(completion.id);
    }
    let receipt = crate::store::append_claim_tx(tx, &claim.origin, &format!("custom/client/operation-{}", claim.id),
        OPERATION_AUTHORIZED, claim.actor.as_deref(), &serde_json::json!({"fields":fields}),
        std::slice::from_ref(&claim.id), Some(&claim.batch_id))?;
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-ha82423ec7c26 owner=fractal_store::pairing TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES (?1,?2)",
        params![format!("client_operation_local/{}", receipt.id), hash],
    )?;
    Ok(())
}

pub fn operation_is_trusted(
    connection: &Connection, id: &str, origin: &str, actor: Option<&str>, fields: &Value,
) -> Result<bool> {
    let expected = record_hash(origin, actor, fields)?;
    let mut statement = connection.prepare_cached(
        "SELECT id,json_extract(body,'$.fields.record_hash') FROM claims WHERE subject=?1 AND kind='client.operation-authorized' AND origin=?2 AND actor IS ?3 AND json_extract(body,'$.fields.record_id')=?4",
    )?;
    let mut rows = statement.query(params![format!("custom/client/operation-{id}"), origin, actor, id])?;
    let mut membership = None;
    while let Some(row) = rows.next()? {
        let receipt: String = row.get(0)?;
        if row.get::<_, String>(1)? != expected { continue; }
        let quarantined: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records AS records JOIN replica_restore_quarantine AS quarantine USING(writer,sequence,envelope_hash) WHERE records.claim_id=?1)",
            [&receipt], |row| row.get(0),
        )?;
        if quarantined { continue; }
        let local: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1 AND value=?2)",
            params![format!("client_operation_local/{receipt}"), expected], |row| row.get(0),
        )?;
        if local { return Ok(true); }
        let mut signatures = connection.prepare_cached(
            "SELECT records.writer,records.sequence,signatures.member_key FROM replica_records AS records JOIN replica_envelope_signatures AS signatures ON signatures.writer=records.writer AND signatures.sequence=records.sequence AND signatures.envelope_hash=records.envelope_hash WHERE records.claim_id=?1 AND records.state='valid' AND records.writer=?2",
        )?;
        let mut signers = signatures.query(params![receipt, origin])?;
        while let Some(signer) = signers.next()? {
            let writer: String = signer.get(0)?;
            let sequence: u64 = signer.get(1)?;
            let key: String = signer.get(2)?;
            if membership.is_none() {
                membership = Some(smallclaims::store::fleet_membership_tx(connection)?);
            }
            if matches!(membership.as_ref().unwrap().window(&writer, sequence),
                smallclaims::fleet::Window::Keyed(keys) if keys.contains(&key)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
/// Admission is independent of credential expiry, so an expired admitted grant remains revocable.
pub fn completion_is_admitted(
    connection: &Connection, id: &str, origin: &str, actor: Option<&str>,
    fields: &Value, now_unix_ms: u128,
) -> Result<bool> {
    Ok(operation_is_trusted(connection, id, origin, actor, fields)?
        || legacy_allowance_deadline(connection, id, origin, actor, fields, now_unix_ms)?.is_some())
}

/// Select the newest admitted operation without allowing an unadmitted retained row to shadow it.
pub fn latest_operation(
    connection: &Connection, subject: &str, kind: &str,
) -> Result<Option<ClaimRecord>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| smallclaims::store::canonical_sql(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind=?2 ORDER BY CANONICAL_DESC(claims)",
    ));
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h37e16ce1bdcb owner=fractal_store::pairing TODO=trace-statement-and-value-representation */ &QUERY)?;
    let mut rows = statement.query(params![subject, kind])?;
    while let Some(row) = rows.next()? {
        let claim = smallclaims::store::claim_from_row(row)?;
        if operation_is_trusted(connection, &claim.id, &claim.origin, claim.actor.as_deref(),
            &claim.body["fields"])? {
            return Ok(Some(claim));
        }
    }
    Ok(None)
}

/// Select admission before canonical ordering can choose a completion.
pub fn latest_completion(connection: &Connection, subject: &str, origin: Option<&str>, now_unix_ms: u128) -> Result<Option<ClaimRecord>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| smallclaims::store::canonical_sql(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='custom.client.pairing-completed' ORDER BY CANONICAL_DESC(claims)",
    ));
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hc6841fe7d058 owner=fractal_store::pairing TODO=trace-statement-and-value-representation */ &QUERY)?;
    let mut rows = statement.query([subject])?;
    while let Some(row) = rows.next()? {
        let claim = smallclaims::store::claim_from_row(row)?;
        if origin.is_some_and(|origin| origin != claim.origin) { continue; }
        if completion_is_admitted(connection, &claim.id, &claim.origin, claim.actor.as_deref(),
            &claim.body["fields"], now_unix_ms)? {
            return Ok(Some(claim));
        }
    }
    Ok(None)
}

/// Terminal authority is reduced only from admitted records of the original issuer and writer.
pub fn terminal_head(connection: &Connection, subject: &str, origin: &str, actor: Option<&str>) -> Result<Option<ClaimRecord>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| smallclaims::store::canonical_sql(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND origin=?2 AND actor IS ?3 AND kind IN ('custom.client.terminal-attached','custom.client.terminal-consumed','custom.client.terminal-detached') ORDER BY CANONICAL_DESC(claims)",
    ));
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hc9e762bbf083 owner=fractal_store::pairing TODO=trace-statement-and-value-representation */ &QUERY)?;
    let mut rows = statement.query(params![subject, origin, actor])?;
    while let Some(row) = rows.next()? {
        let claim = smallclaims::store::claim_from_row(row)?;
        if operation_is_trusted(connection, &claim.id, &claim.origin, claim.actor.as_deref(), &claim.body["fields"])? {
            return Ok(Some(claim));
        }
    }
    Ok(None)
}

pub fn revocation_is_trusted(connection: &Connection, revoked: &ClaimRecord, completed: &ClaimRecord) -> Result<bool> {
    if revoked.kind != "custom.client.pairing-revoked" || revoked.subject != completed.subject
        || revoked.origin != completed.origin {
        return Ok(false);
    }
    // Existing allowances never override an issuer's withdrawal, including retained history.
    if !operation_is_trusted(connection, &completed.id, &completed.origin, completed.actor.as_deref(), &completed.body["fields"])? {
        return Ok(true);
    }
    let linked: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='client.operation-authorized' AND origin=?2 AND actor IS ?3 AND json_extract(body,'$.fields.record_id')=?4 AND json_extract(body,'$.fields.completion_id')=?5)",
        params![format!("custom/client/operation-{}", revoked.id), revoked.origin, revoked.actor, revoked.id, completed.id],
        |row| row.get(0),
    )?;
    Ok(linked && operation_is_trusted(connection, &revoked.id, &revoked.origin, revoked.actor.as_deref(), &revoked.body["fields"])?)
}


/// Only the issuing operation can revoke its exact completion.
pub fn completion_is_revoked(
    connection: &Connection, subject: &str, completion_id: &str, issuer_origin: &str,
) -> Result<bool> {
    let (writer, completed): (Option<String>, String) = connection.query_row(
        "SELECT actor,body FROM claims WHERE id=?1", [completion_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let completed: Value = serde_json::from_str(&completed)?;
    let legacy = !operation_is_trusted(connection, completion_id, issuer_origin, writer.as_deref(), &completed["fields"])?;
    let mut statement = connection.prepare("SELECT id,origin,actor,body FROM claims WHERE subject=?1 AND kind='custom.client.pairing-revoked'")?;
    let mut rows = statement.query([subject])?;
    let mut revoked = false;
    let mut unadmitted = 0_u64;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let origin: String = row.get(1)?;
        let actor: Option<String> = row.get(2)?;
        let body: Value = serde_json::from_str(&row.get::<_, String>(3)?)?;
        let linked: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='client.operation-authorized' AND origin=?2 AND actor IS ?3 AND json_extract(body,'$.fields.record_id')=?4 AND json_extract(body,'$.fields.completion_id')=?5)",
            params![format!("custom/client/operation-{id}"), origin, actor, id, completion_id],
            |row| row.get(0),
        )?;
        if origin == issuer_origin && (legacy || (linked
            && operation_is_trusted(connection, &id, &origin, actor.as_deref(), &body["fields"])?)) {
            revoked = true;
        } else {
            unadmitted += 1;
        }
    }
    if unadmitted != 0 {
        tracing::warn!(count = unadmitted, "pairing revocation records require operation admission");
    }
    Ok(revoked)
}

/// The writer-transaction and bearer paths share the same admission rule.
pub fn completion_is_trusted(
    connection: &Connection,
    id: &str,
    origin: &str,
    actor: Option<&str>,
    fields: &Value,
    now_unix_ms: u128,
) -> Result<bool> {
    Ok(completion_deadline(connection, id, origin, actor, fields, now_unix_ms)?.is_some())
}

/// Effective authority deadline, including any temporary migration allowance.
pub fn completion_deadline(
    connection: &Connection, id: &str, origin: &str, actor: Option<&str>,
    fields: &Value, now_unix_ms: u128,
) -> Result<Option<u128>> {
    let expiry = fields.get("expires_at_unix_ms").and_then(Value::as_u64).map(u128::from).unwrap_or_default();
    if expiry <= now_unix_ms { return Ok(None); }
    if operation_is_trusted(connection, id, origin, actor, fields)? {
        return Ok(Some(expiry));
    }
    Ok(legacy_allowance_deadline(connection, id, origin, actor, fields, now_unix_ms)?.map(|until| expiry.min(until)))
}

fn legacy_allowance_deadline(
    connection: &Connection, id: &str, origin: &str, actor: Option<&str>,
    fields: &Value, now_unix_ms: u128,
) -> Result<Option<u128>> {
    if now_unix_ms >= LEGACY_ALLOWANCE_CAP_UNIX_MS {
        return Ok(None);
    }
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_pairing_migration_allowances')",
        [], |row| row.get(0),
    )?;
    if !installed { return Ok(None); }
    let allowed = connection.query_row(
        "SELECT record_hash, until_unix_ms FROM local_pairing_migration_allowances WHERE completion_id=?1",
        [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
    ).optional()?;
    let Some((hash, until)) = allowed else { return Ok(None); };
    let deadline = u128::from(until).min(LEGACY_ALLOWANCE_CAP_UNIX_MS);
    Ok((now_unix_ms < deadline && hash == record_hash(origin, actor, fields)?).then_some(deadline))
}

impl Store {
    /// Consume the exact admitted challenge once, ignoring unadmitted retained subject rows.
    pub fn complete_client_pairing(
        &self, input: &ClaimInput, begun_id: &str,
    ) -> Result<ClaimRecord, crate::FractalError> {
        let refused = || crate::FractalError::new("stale-subject", "the pairing challenge changed or was already consumed");
        if input.kind != "custom.client.pairing-completed"
            || input.expected_subject.is_some()
            || input.idempotency_key.is_some()
            || input.evidence != [begun_id]
        {
            return Err(refused());
        }
        let admission = |connection: &Connection| {
            let now = smallclaims::store::now_ms();
            let begun = latest_operation(connection, &input.subject, "custom.client.pairing-begun")
                .map_err(|error| crate::FractalError::new("internal", error.to_string()))?.ok_or_else(refused)?;
            let fields = &begun.body["fields"];
            // Historical challenges omitted scopes; their established meaning is this
            // limited policy. Explicit scopes, including malformed values, stay exact.
            let scopes = fields.get("scopes").cloned()
                .unwrap_or_else(|| serde_json::json!(LIMITED_PAIRING_SCOPES));
            if begun.id != begun_id || begun.origin != self.origin()
                || input.actor != begun.actor
                || fields["person_id"] != input.fields.get("person_id").cloned().unwrap_or_default()
                || fields["pairing_id"] != input.fields.get("pairing_id").cloned().unwrap_or_default()
                || scopes != input.fields.get("scopes").cloned().unwrap_or_default()
                || fields["expires_at_unix_ms"].as_u64().is_none_or(|expiry| u128::from(expiry) <= now)
                || latest_completion(connection, &input.subject, None, now)
                    .map_err(|error| crate::FractalError::new("internal", error.to_string()))?.is_some()
            {
                return Err(refused());
            }
            Ok(())
        };
        crate::store::append_claim_with_commit_context(self, input, None, None, None, None,
            crate::store::ClaimCommitContext {
                client_operation: true, admission: Some(&admission), ..Default::default()
            }).map(|(claim, _)| claim)
    }

    pub fn append_client_operation_claim(&self, input: &ClaimInput) -> Result<ClaimRecord, crate::FractalError> {
        if !input.kind.starts_with("custom.client.pairing") && !input.kind.starts_with("custom.client.terminal") {
            return Err(crate::FractalError::new("claim-write-forbidden", "a dedicated client operation record is required"));
        }
        crate::store::append_claim_with_commit_context(self, input, None, None, None, None,
            crate::store::ClaimCommitContext { client_operation:true, ..Default::default() })
            .map(|(claim, _)| claim)
    }

    pub fn newest_admitted_client_operation(&self, subject: &str, kind: &str) -> Result<Option<ClaimRecord>> {
        latest_operation(&self.readers.get(), subject, kind)
    }

    pub fn client_operation_is_trusted(&self, claim: &ClaimRecord) -> Result<bool> {
        operation_is_trusted(&self.readers.get(), &claim.id, &claim.origin, claim.actor.as_deref(), &claim.body["fields"])
    }

    /// Replace this member's operator-configured allowances before serving clients.
    /// Configuration can name only an existing exact completion record.
    pub fn configure_pairing_legacy_allowances(&self, allowances: &[LegacyPairingAllowance]) -> Result<()> {
        let mut connection = self.connection.write();
        let tx = connection.transaction()?;
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-he7e4b4d5ad69 owner=fractal_store::pairing TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_pairing_migration_allowances (completion_id TEXT PRIMARY KEY, record_hash TEXT NOT NULL, until_unix_ms INTEGER NOT NULL); DELETE FROM local_pairing_migration_allowances;")?;
        for allowance in allowances {
            let stored = tx.query_row(
                "SELECT origin, actor, body FROM claims WHERE id=?1 AND kind='custom.client.pairing-completed'",
                [&allowance.completion_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)),
            ).optional()?;
            let Some((origin, actor, body)) = stored else {
                anyhow::bail!("a configured pairing migration allowance has no existing completion record");
            };
            let body: Value = serde_json::from_str(&body)?;
            let person = body["fields"].get("person_id").and_then(Value::as_str);
            anyhow::ensure!(person.is_some_and(|person| person.starts_with("person/") && actor.as_deref() == Some(person)),
                "a configured pairing migration allowance must be written by its recorded person");
            anyhow::ensure!(record_hash(&origin, actor.as_deref(), &body["fields"])? == allowance.record_hash,
                "a configured pairing migration allowance does not match the completion record");
            let previous_revocation: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=(SELECT subject FROM claims WHERE id=?1) AND kind='custom.client.pairing-revoked')",
                [&allowance.completion_id], |row| row.get(0),
            )?;
            anyhow::ensure!(!previous_revocation, "a configured pairing migration allowance has prior revocation records; pair again");
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h45575741d81f owner=fractal_store::pairing TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_pairing_migration_allowances(completion_id,record_hash,until_unix_ms) VALUES (?1,?2,?3)",
                params![allowance.completion_id, allowance.record_hash,
                    allowance.until_unix_ms.min(LEGACY_ALLOWANCE_CAP_UNIX_MS as u64)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn pairing_completion_is_admitted(&self, claim: &ClaimRecord, now_unix_ms: u128) -> Result<bool> {
        completion_is_admitted(&self.readers.get(), &claim.id, &claim.origin, claim.actor.as_deref(),
            &claim.body["fields"], now_unix_ms)
    }

    pub fn newest_admitted_pairing_completion(&self, subject: &str, now_unix_ms: u128) -> Result<Option<ClaimRecord>> {
        latest_completion(&self.readers.get(), subject, None, now_unix_ms)
    }

    pub fn pairing_revocation_is_trusted(&self, revoked: &ClaimRecord, completed: &ClaimRecord) -> Result<bool> {
        revocation_is_trusted(&self.readers.get(), revoked, completed)
    }

    pub fn pairing_completion_is_trusted(&self, claim: &ClaimRecord, now_unix_ms: u128) -> Result<bool> {
        completion_is_trusted(&self.readers.get(), &claim.id, &claim.origin, claim.actor.as_deref(),
            &claim.body["fields"], now_unix_ms)
    }

    pub fn pairing_completion_deadline(&self, claim: &ClaimRecord, now_unix_ms: u128) -> Result<Option<u128>> {
        completion_deadline(&self.readers.get(), &claim.id, &claim.origin, claim.actor.as_deref(),
            &claim.body["fields"], now_unix_ms)
    }

    pub fn pairing_completion_is_revoked(&self, claim: &ClaimRecord) -> Result<bool> {
        completion_is_revoked(&self.readers.get(), &claim.subject, &claim.id, &claim.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallclaims::claim::ClaimInput;
    use serde_json::json;

    fn completion_input(actor: Option<&str>) -> ClaimInput {
        ClaimInput {
            subject:"custom/client/pairing-admission".into(),kind:"custom.client.pairing-completed".into(),
            actor:actor.map(str::to_owned), fields:serde_json::from_value(json!({
                "person_id":"person/example","session_actor":"person/example/session/phone",
                "credential_hash":"test-digest","expires_at_unix_ms":u64::MAX,"scopes":["read.projections"],
            })).unwrap(),
            evidence:vec![],expected_subject:None,idempotency_key:None,
        }
    }

    #[test]
    fn pairing_operation_receipts_require_local_or_keyed_envelope_admission() {
        let source = Store::open_memory("issuer").unwrap();
        let completed = source.append_client_operation_claim(&completion_input(Some("person/example"))).unwrap();
        let exchange = source.export_replication_exchange("receipt-fleet", &Default::default()).unwrap();
        for restore in [false, true] {
            let target = Store::open_memory("receiver").unwrap();
            target.receive_replication_exchange("issuer", "receipt-fleet", &exchange).unwrap();
            let admission = if restore { target.admit_restored_backlog() } else { target.validate_replication_backlog() }.unwrap();
            assert!(admission.held > 0);
            assert!(!target.client_operation_is_trusted(&completed).unwrap());
        }
        let source = Store::open_memory("issuer").unwrap();
        source.bind_fleet("receipt-fleet").unwrap();
        let key = std::sync::Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
        source.pin_fleet_anchor(key.public()).unwrap();
        source.set_member_key(Some(key.clone())).unwrap();
        source.admit_fleet_anchor("receipt-fleet", key.public(), "listening").unwrap();
        let completed = source.append_client_operation_claim(&completion_input(Some("person/example"))).unwrap();
        let exchange = source.export_replication_exchange("receipt-fleet", &Default::default()).unwrap();
        for restore in [false, true] {
            let target = Store::open_memory("receiver").unwrap();
            target.pin_fleet_anchor(key.public()).unwrap();
            target.receive_replication_exchange("issuer", "receipt-fleet", &exchange).unwrap();
            if restore { target.admit_restored_backlog() } else { target.validate_replication_backlog() }.unwrap();
            target.project_replication_backlog().unwrap();
            assert!(target.client_operation_is_trusted(&completed).unwrap());
        }
        let retained = Store::open_memory("retained").unwrap();
        let completed = retained.append_claim(&completion_input(Some("person/example"))).unwrap();
        retained.append_claim(&ClaimInput {
            subject:format!("custom/client/operation-{}", completed.id),kind:OPERATION_AUTHORIZED.into(),
            actor:completed.actor.clone(),fields:serde_json::from_value(json!({
                "record_id":completed.id,"record_hash":record_hash(&completed.origin, completed.actor.as_deref(), &completed.body["fields"]).unwrap(),
            })).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None,
        }).unwrap();
        assert!(!retained.client_operation_is_trusted(&completed).unwrap());
    }

    #[test]
    fn pairing_migration_allowances_require_the_person_writer_and_honor_issuer_withdrawal() {
        for actor in [None, Some("person/other"), Some("person/example")] {
            let store = Store::open_memory("issuer").unwrap();
            let completed = store.append_claim(&completion_input(actor)).unwrap();
            let configured = store.configure_pairing_legacy_allowances(&[LegacyPairingAllowance {
                completion_id:completed.id.clone(),
                record_hash:record_hash(&completed.origin, completed.actor.as_deref(), &completed.body["fields"]).unwrap(),
                until_unix_ms:LEGACY_ALLOWANCE_CAP_UNIX_MS as u64,
            }]);
            if actor != Some("person/example") {
                assert!(configured.is_err());
                continue;
            }
            configured.unwrap();
            assert!(!store.pairing_completion_is_revoked(&completed).unwrap());
            let mut withdrawal = completion_input(None);
            withdrawal.kind = "custom.client.pairing-revoked".into();
            withdrawal.fields.clear();
            store.append_claim(&withdrawal).unwrap();
            assert!(store.pairing_completion_is_revoked(&completed).unwrap());
        }
    }

    #[test]
    fn operation_receipts_bind_exact_records_and_historical_fields_confer_no_admission() {
        let store = Store::open_memory("member").unwrap();
        for kind in ["custom.client.pairing-begun", "custom.client.pairing-completed",
            "custom.client.terminal-attached"] {
            let input = ClaimInput {
                subject:format!("custom/client/record-{}", kind.rsplit('.').next().unwrap()), kind:kind.into(),
                actor:Some("person/example".into()),
                fields:serde_json::from_value(json!({
                    "person_id":"person/example","pairing_operation_version":1,
                    "terminal_operation_version":1,"expires_at_unix_ms":u64::MAX
                })).unwrap(),
                evidence:vec![],expected_subject:None,idempotency_key:None,
            };
            let historical = store.append_claim(&input).unwrap();
            assert!(!store.client_operation_is_trusted(&historical).unwrap());
            assert!(!store.pairing_completion_is_trusted(&historical, 1).unwrap());
            let admitted = store.append_client_operation_claim(&input).unwrap();
            assert!(store.client_operation_is_trusted(&admitted).unwrap());
            let receipts = store.claims_for_subject_kind_at(
                &format!("custom/client/operation-{}", admitted.id), OPERATION_AUTHORIZED,
                None, true, 10,
            ).unwrap().claims;
            assert_eq!(receipts.len(), 1);
            assert_eq!(receipts[0].batch_id, admitted.batch_id);
            assert!(matches!(
                crate::store::classify_replicated_claim_with_registry(
                    &receipts[0], fractal_schema::registry(),
                ).unwrap(),
                smallclaims::store::ReplicatedClaimAdmission::Valid,
            ));
            let mut previous = fractal_schema::registry().clone();
            previous.claims.remove(OPERATION_AUTHORIZED);
            assert!(matches!(
                crate::store::classify_replicated_claim_with_registry(&receipts[0], &previous).unwrap(),
                smallclaims::store::ReplicatedClaimAdmission::UnknownKind,
            ));
            let mut changed = admitted.clone();
            changed.body["fields"]["person_id"] = json!("person/other");
            assert!(!store.client_operation_is_trusted(&changed).unwrap());
            let mut changed = admitted.clone();
            changed.origin = "other-member".into();
            assert!(!store.client_operation_is_trusted(&changed).unwrap());
            let mut changed = admitted;
            changed.id = historical.id;
            assert!(!store.client_operation_is_trusted(&changed).unwrap());
        }
    }

    #[test]
    fn temporary_pairing_allowance_binds_existing_content_and_deadline() {
        let store = Store::open_memory("member").unwrap();
        let claim = store.append_claim(&ClaimInput {
            subject:"custom/client/pairing-existing".into(),
            kind:"custom.client.pairing-completed".into(), actor:Some("person/example".into()),
            fields:serde_json::from_value(json!({
                "person_id":"person/example","session_actor":"person/example/session/phone",
                "credential_hash":"test-credential-digest","scopes":["read.projections"],
                "expires_at_unix_ms":u64::MAX
            })).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
        let before = LEGACY_ALLOWANCE_CAP_UNIX_MS - 1;
        assert!(!store.pairing_completion_is_trusted(&claim, before).unwrap());
        let mut allowance = LegacyPairingAllowance {
            completion_id:claim.id.clone(),
            record_hash:record_hash(&claim.origin, claim.actor.as_deref(), &claim.body["fields"]).unwrap(),
            until_unix_ms:(LEGACY_ALLOWANCE_CAP_UNIX_MS + 86_400_000) as u64,
        };
        store.configure_pairing_legacy_allowances(&[allowance.clone()]).unwrap();
        assert!(store.pairing_completion_is_trusted(&claim, before).unwrap());
        for now in [LEGACY_ALLOWANCE_CAP_UNIX_MS, LEGACY_ALLOWANCE_CAP_UNIX_MS + 1] {
            assert!(!store.pairing_completion_is_trusted(&claim, now).unwrap());
        }
        let mut different = claim.clone();
        different.id = "another-completion".into();
        assert!(!store.pairing_completion_is_trusted(&different, before).unwrap());
        let mut different = claim.clone();
        different.body["fields"]["scopes"] = json!(["terminal.control"]);
        assert!(!store.pairing_completion_is_trusted(&different, before).unwrap());
        let mut different = claim.clone();
        different.origin = "another-member".into();
        assert!(!store.pairing_completion_is_trusted(&different, before).unwrap());
        allowance.until_unix_ms = (before - 100) as u64;
        store.configure_pairing_legacy_allowances(&[allowance.clone()]).unwrap();
        assert!(!store.pairing_completion_is_trusted(&claim, before).unwrap());
        allowance.record_hash = "different-content".into();
        assert!(store.configure_pairing_legacy_allowances(&[allowance]).is_err());
        store.configure_pairing_legacy_allowances(&[]).unwrap();
        assert!(!store.pairing_completion_is_trusted(&claim, before).unwrap());
    }
}
