//! Native admission receipts and temporary migration allowances for client operations.
use anyhow::Result;
use rusqlite::{Connection, Transaction, OptionalExtension as _, params};
use serde_json::Value;
use crate::{config::LegacyPairingAllowance, store::Store};
use smallclaims::claim::{ClaimInput, ClaimRecord};

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
    let mut fields = serde_json::json!({
        "record_id":claim.id,
        "record_hash":record_hash(&claim.origin, claim.actor.as_deref(), &claim.body["fields"])?,
    });
    if claim.kind == "custom.client.pairing-revoked" {
        let completion: String = tx.query_row(
            /* sqlite-large-value: allow-for-now lv-h137b7b4adce1 owner=fractal_store::pairing TODO=trace-statement-and-value-representation */ &smallclaims::store::canonical_sql("SELECT id FROM claims WHERE subject=?1 AND kind='custom.client.pairing-completed' AND origin=?2 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
            params![claim.subject, claim.origin], |row| row.get(0),
        )?;
        fields["completion_id"] = serde_json::json!(completion);
    }
    crate::store::append_claim_tx(tx, &claim.origin, &format!("custom/client/operation-{}", claim.id),
        OPERATION_AUTHORIZED, claim.actor.as_deref(), &serde_json::json!({"fields":fields}),
        std::slice::from_ref(&claim.id), Some(&claim.batch_id))?;
    Ok(())
}

pub fn operation_is_trusted(
    connection: &Connection, id: &str, origin: &str, actor: Option<&str>, fields: &Value,
) -> Result<bool> {
    let hash: Option<String> = connection.query_row(
        "SELECT json_extract(body,'$.fields.record_hash') FROM claims WHERE subject=?1 AND kind='client.operation-authorized' AND origin=?2 AND actor IS ?3 AND json_extract(body,'$.fields.record_id')=?4 LIMIT 1",
        params![format!("custom/client/operation-{id}"), origin, actor, id], |row| row.get(0),
    ).optional()?;
    let Some(hash) = hash else { return Ok(false); };
    Ok(hash == record_hash(origin, actor, fields)?)
}

/// Only the issuing operation can revoke its exact completion.
pub fn completion_is_revoked(
    connection: &Connection, subject: &str, completion_id: &str, issuer_origin: &str,
) -> Result<bool> {
    let writer: Option<String> = connection.query_row("SELECT actor FROM claims WHERE id=?1",
        [completion_id], |row| row.get(0))?;
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
        if origin == issuer_origin && actor == writer && linked
            && operation_is_trusted(connection, &id, &origin, actor.as_deref(), &body["fields"])? {
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
    let deadline = expiry.min(u128::from(until)).min(LEGACY_ALLOWANCE_CAP_UNIX_MS);
    Ok((now_unix_ms < deadline && hash == record_hash(origin, actor, fields)?).then_some(deadline))
}

impl Store {
    pub fn append_client_operation_claim(&self, input: &ClaimInput) -> Result<ClaimRecord, crate::FractalError> {
        if !input.kind.starts_with("custom.client.pairing") && !input.kind.starts_with("custom.client.terminal") {
            return Err(crate::FractalError::new("claim-write-forbidden", "a dedicated client operation record is required"));
        }
        crate::store::append_claim_with_commit_context(self, input, None, None, None, None,
            crate::store::ClaimCommitContext { client_operation:true, ..Default::default() })
            .map(|(claim, _)| claim)
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

    #[test]
    fn operation_receipts_bind_exact_records_and_historical_fields_confer_no_admission() {
        let store = Store::open_memory("member").unwrap();
        for kind in ["custom.client.pairing-begun", "custom.client.pairing-completed",
            "custom.client.terminal-attached"] {
            let input = ClaimInput {
                subject:format!("custom/client/record-{kind}"), kind:kind.into(),
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
