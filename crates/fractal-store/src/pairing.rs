//! Temporary migration allowance for existing pairings.
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::Value;
use crate::{config::LegacyPairingAllowance, store::Store};
use smallclaims::claim::ClaimRecord;

/// Every configured allowance ends no later than 2026-11-15T23:00:00Z.
pub const LEGACY_ALLOWANCE_CAP_UNIX_MS: u128 = 1_794_783_600_000;

/// Digest of the immutable authority fields captured by the operator.
pub fn record_hash(origin: &str, actor: Option<&str>, fields: &Value) -> Result<String> {
    smallclaims::hash::canonical_hash(&(origin, actor, fields))
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
    if crate::trusted_pairing_fields(actor, fields) {
        return Ok(true);
    }
    if now_unix_ms >= LEGACY_ALLOWANCE_CAP_UNIX_MS {
        return Ok(false);
    }
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_pairing_migration_allowances')",
        [], |row| row.get(0),
    )?;
    if !installed { return Ok(false); }
    let allowed = connection.query_row(
        "SELECT record_hash, until_unix_ms FROM local_pairing_migration_allowances WHERE completion_id=?1",
        [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
    ).optional()?;
    let Some((hash, until)) = allowed else { return Ok(false); };
    Ok(now_unix_ms < u128::from(until) && hash == record_hash(origin, actor, fields)?)
}

impl Store {
    /// Replace this member's operator-configured allowances before serving clients.
    /// Configuration can name only an existing exact completion record.
    pub fn configure_pairing_legacy_allowances(&self, allowances: &[LegacyPairingAllowance]) -> Result<()> {
        let mut connection = self.connection.write();
        let tx = connection.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS local_pairing_migration_allowances (completion_id TEXT PRIMARY KEY, record_hash TEXT NOT NULL, until_unix_ms INTEGER NOT NULL); DELETE FROM local_pairing_migration_allowances;")?;
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
            tx.execute(
                "INSERT INTO local_pairing_migration_allowances(completion_id,record_hash,until_unix_ms) VALUES (?1,?2,?3)",
                params![allowance.completion_id, allowance.record_hash,
                    allowance.until_unix_ms.min(LEGACY_ALLOWANCE_CAP_UNIX_MS as u64)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn pairing_completion_is_trusted(&self, claim: &ClaimRecord, now_unix_ms: u128) -> Result<bool> {
        if crate::trusted_pairing_fields(claim.actor.as_deref(), &claim.body["fields"]) {
            return Ok(true);
        }
        completion_is_trusted(&self.readers.get(), &claim.id, &claim.origin, claim.actor.as_deref(),
            &claim.body["fields"], now_unix_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallclaims::claim::ClaimInput;
    use serde_json::json;

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
