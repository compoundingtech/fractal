use super::*;
use crate::pairing::{append_operation_receipt_tx, operation_is_trusted};

impl Store {
    /// Commit an approved renewal and its replacement in one writer transaction.
    pub fn commit_device_renewal(
        &self, stage: &ClaimRecord, source: &ClaimRecord, challenge: &ClaimRecord,
        consumed: &ClaimInput, replacement: &ClaimInput,
    ) -> Result<ClaimRecord> {
        let mut connection = self.connection.write();
        let tx = connection.transaction()?;
        let fields = &stage.body["fields"];
        let historical = &source.body["fields"];
        anyhow::ensure!(stage.origin == self.origin()
            && stage.kind == "custom.client.pairing-renewal-staged"
            && operation_is_trusted(&tx, &stage.id, &stage.origin, stage.actor.as_deref(), fields)?
            && smallclaims::store::latest_claim_id_tx(&tx, &stage.subject)?.as_deref() == Some(stage.id.as_str()),
            "the renewal is unavailable or already completed");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
        anyhow::ensure!(fields["expires_at_unix_ms"].as_u64().is_some_and(|expiry| u128::from(expiry) > now),
            "the renewal expired");
        anyhow::ensure!(challenge.kind == "custom.client.pairing-renewal-challenge"
            && challenge.origin == self.origin()
            && operation_is_trusted(&tx, &challenge.id, &challenge.origin, challenge.actor.as_deref(), &challenge.body["fields"])?
            && challenge.body["fields"]["renewal_id"] == fields["renewal_id"]
            && challenge.body["fields"]["expires_at_unix_ms"].as_u64().is_some_and(|expiry| u128::from(expiry) > now),
            "the renewal challenge expired or is unavailable");
        anyhow::ensure!(fields["source_grant_id"].as_str() == Some(source.id.as_str())
            && fields["old_public_key"] == historical["device_public_key"]
            && fields["person_id"] == historical["person_id"]
            && source.origin == self.origin() && source.kind == "custom.client.pairing-completed",
            "the renewal source does not match");
        let person = fields["person_id"].as_str().context("renewal person missing")?;
        let old_key = historical["device_public_key"].as_str().context("source key missing")?;
        let device = historical["device_id"].as_str().context("source device missing")?;
        let candidate = replacement.fields.get("device_public_key").and_then(Value::as_str).context("candidate key missing")?;
        let raw_withdrawal: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='custom.client.pairing-revoked' AND (subject=?1 OR json_extract(body,'$.fields.device_id')=?2))",
            params![source.subject, device], |row| row.get(0),
        )?;
        let key_withdrawal: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='principal.key-revoked' AND subject=?1 AND json_extract(body,'$.fields.key') IN (?2,?3))",
            params![person, old_key, candidate], |row| row.get(0),
        )?;
        anyhow::ensure!(!raw_withdrawal && !key_withdrawal, FractalError::new("forbidden", "a renewal key or source was withdrawn"));
        anyhow::ensure!(consumed.kind == "custom.client.pairing-renewal-consumed"
            && consumed.subject == stage.subject && consumed.actor.as_deref() == Some(person)
            && replacement.kind == "custom.client.pairing-completed" && replacement.actor.as_deref() == Some(person)
            && replacement.fields.get("person_id") == fields.get("person_id")
            && replacement.fields.get("scopes") == fields.get("scopes")
            && replacement.fields.get("delegated_scopes") == fields.get("scopes"),
            "the replacement does not match the approved renewal");
        let grant_expiry = fields["grant_expires_at_unix_ms"].as_u64().context("approved grant expiry missing")?;
        anyhow::ensure!(u128::from(grant_expiry) > now && u128::from(grant_expiry) <= now + 30 * 24 * 60 * 60 * 1000
            && replacement.fields.get("expires_at_unix_ms") == Some(&fields["grant_expires_at_unix_ms"]),
            "the replacement expiry does not match the approved renewal");
        let previous: Vec<String> = serde_json::from_value(historical["scopes"].clone())?;
        let scopes: Vec<String> = serde_json::from_value(fields["scopes"].clone())?;
        anyhow::ensure!(!scopes.is_empty() && scopes.iter().all(|scope| previous.contains(scope)), "renewal scopes exceed the source grant");
        anyhow::ensure!(crate::pairing::latest_completion(&tx, &source.subject, Some(self.origin()), now)?
            .is_some_and(|current| current.id == source.id), "the renewal source is no longer the current admitted completion");
        let completed = append_claim_tx(&tx, self.origin(), &replacement.subject, &replacement.kind,
            replacement.actor.as_deref(), &json!({"fields":replacement.fields}), &replacement.evidence, None)?;
        append_operation_receipt_tx(&tx, &completed)?;
        let retired = append_claim_tx(&tx, self.origin(), &source.subject, "custom.client.pairing-revoked",
            Some(person), &json!({"fields":BTreeMap::from([("person_id",json!(person)),("device_id",json!(device)),("completion_id",json!(source.id))])}),
            std::slice::from_ref(&source.id), Some(&completed.batch_id))?;
        append_operation_receipt_tx(&tx, &retired)?;
        let receipt = append_claim_tx(&tx, self.origin(), &consumed.subject, &consumed.kind,
            consumed.actor.as_deref(), &json!({"fields":consumed.fields}), std::slice::from_ref(&stage.id), Some(&completed.batch_id))?;
        append_operation_receipt_tx(&tx, &receipt)?;
        tx.commit()?;
        Ok(completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(subject: &str, kind: &str, fields: Value) -> ClaimInput {
        ClaimInput { subject:subject.into(), kind:kind.into(), actor:Some("person/example".into()),
            fields:serde_json::from_value(fields).unwrap(), evidence:vec![],expected_subject:None,idempotency_key:None }
    }
    fn fixture() -> (Store, ClaimRecord, ClaimRecord, ClaimRecord, ClaimInput, ClaimInput) {
        fixture_with_expiry(u64::MAX,u64::MAX)
    }
    fn fixture_with_expiry(stage_expiry: u64, challenge_expiry: u64) -> (Store, ClaimRecord, ClaimRecord, ClaimRecord, ClaimInput, ClaimInput) {
        let store=Store::open_memory("member").unwrap();
        let grant_expiry = smallclaims::store::now_ms() + 29 * 24 * 60 * 60 * 1000;
        let source=store.append_client_operation_claim(&input("custom/client/pairing-history","custom.client.pairing-completed",
            json!({"person_id":"person/example","device_id":"device/old","device_public_key":"old-key","scopes":["read.projections"],"expires_at_unix_ms":1}))).unwrap();
        let stage=store.append_client_operation_claim(&input("custom/client/pairing-renewal-stage","custom.client.pairing-renewal-staged",
            json!({"person_id":"person/example","renewal_id":"renewal/test","source_grant_id":source.id,"old_public_key":"old-key","scopes":["read.projections"],"expires_at_unix_ms":stage_expiry,"grant_expires_at_unix_ms":grant_expiry}))).unwrap();
        let challenge=store.append_client_operation_claim(&input("custom/client/pairing-renewal-challenge","custom.client.pairing-renewal-challenge",
            json!({"person_id":"person/example","renewal_id":"renewal/test","expires_at_unix_ms":challenge_expiry}))).unwrap();
        let consumed=input(&stage.subject,"custom.client.pairing-renewal-consumed",json!({"person_id":"person/example","renewal_id":"renewal/test"}));
        let replacement=input("custom/client/pairing-replacement","custom.client.pairing-completed",
            json!({"person_id":"person/example","device_public_key":"candidate-key","scopes":["read.projections"],"delegated_scopes":["read.projections"],"expires_at_unix_ms":grant_expiry}));
        (store,stage,source,challenge,consumed,replacement)
    }
    fn withdraw(store: &Store, source: &ClaimRecord) {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let withdrawal = append_claim_tx(&tx, "member", &source.subject, "custom.client.pairing-revoked",
            Some("person/example"), &json!({"fields":{"person_id":"person/example","device_id":"device/old","completion_id":source.id}}), &[], None).unwrap();
        append_operation_receipt_tx(&tx, &withdrawal).unwrap();
        tx.commit().unwrap();
        drop(connection);
        let reloaded = store.claim_by_id(&withdrawal.id).unwrap().unwrap();
        assert_eq!(serde_json::to_string(&withdrawal.body).unwrap(), serde_json::to_string(&reloaded.body).unwrap());
        assert!(store.client_operation_is_trusted(&reloaded).unwrap());
        assert!(store.pairing_completion_is_revoked(source).unwrap());
    }
    #[test]
    fn device_renewal_commits_receipts_and_replacement_once() {
        let (store,stage,source,challenge,consumed,replacement)=fixture();
        let result=store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).unwrap();
        assert!(store.client_operation_is_trusted(&result).unwrap());
        assert_eq!(result.body["fields"]["expires_at_unix_ms"], stage.body["fields"]["grant_expires_at_unix_ms"]);
        let retired=store.claims_for_subject_kind_at(&source.subject,"custom.client.pairing-revoked",None,true,1).unwrap().claims.remove(0);
        assert!(store.client_operation_is_trusted(&retired).unwrap());
        assert!(store.pairing_completion_is_revoked(&source).unwrap());
        assert!(crate::pairing::revocation_is_trusted(&store.readers.get(), &retired, &source).unwrap());
        assert_eq!(retired.batch_id,result.batch_id);
        let receipt=store.claims_for_subject_kind_at(&stage.subject,"custom.client.pairing-renewal-consumed",None,true,1).unwrap().claims.remove(0);
        assert!(store.client_operation_is_trusted(&receipt).unwrap());
        assert_eq!(receipt.batch_id,result.batch_id);
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_err());
    }
    #[test]
    fn device_renewal_refuses_a_changed_replacement_expiry() {
        let (store,stage,source,challenge,consumed,mut replacement)=fixture();
        replacement.fields.insert("expires_at_unix_ms".into(), json!(smallclaims::store::now_ms() + 30 * 24 * 60 * 60 * 1000));
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_err());
    }
    #[test]
    fn device_renewal_write_failure_keeps_stage_available() {
        let (store,stage,source,challenge,consumed,replacement)=fixture();
        store.connection.write().execute_batch("CREATE TRIGGER renewal_fixture_failure BEFORE INSERT ON claims WHEN NEW.kind='custom.client.pairing-renewal-consumed' BEGIN SELECT RAISE(ABORT,'fixture write refusal'); END;").unwrap();
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_err());
        assert!(store.claims_for_subject_kind_at(&stage.subject,"custom.client.pairing-renewal-consumed",None,true,1).unwrap().claims.is_empty());
        assert!(store.claims_for_subject_kind_at(&replacement.subject,"custom.client.pairing-completed",None,true,1).unwrap().claims.is_empty());
        store.connection.write().execute_batch("DROP TRIGGER renewal_fixture_failure").unwrap();
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_ok());
    }
    #[test]
    fn device_renewal_rechecks_withdrawal_after_preparation() {
        let (store,stage,source,challenge,consumed,replacement)=fixture();
        withdraw(&store, &source);
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_err());
        assert!(store.claims_for_subject_kind_at(&stage.subject,"custom.client.pairing-renewal-consumed",None,true,1).unwrap().claims.is_empty());
    }
    #[test]
    fn device_renewal_refuses_stage_and_challenge_expiry_at_commit() {
        for (stage_expiry,challenge_expiry) in [(1,u64::MAX),(u64::MAX,1)] {
            let (store,stage,source,challenge,consumed,replacement)=fixture_with_expiry(stage_expiry,challenge_expiry);
            assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_err());
        }
    }
    #[test]
    fn device_renewal_refused_candidate_does_not_consume_stage() {
        let (store,stage,source,challenge,consumed,mut replacement)=fixture();
        store.revoke_device_key("person/example","candidate-key","fixture withdrawal").unwrap();
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_err());
        replacement.fields.insert("device_public_key".into(),json!("fresh-candidate-key"));
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_ok());
    }
    #[test]
    fn device_renewal_withdrawal_is_not_limited_by_unrelated_history() {
        let (store,stage,source,challenge,consumed,replacement)=fixture();
        withdraw(&store, &source);
        {
            let mut connection=store.connection.write();
            let tx=connection.transaction().unwrap();
            for index in 0..10_001 {
                append_claim_tx(&tx,"member",&format!("custom/client/pairing-unrelated-{index}"),"custom.client.pairing-revoked",
                    Some("person/example"),&json!({"fields":{"person_id":"person/example","device_id":format!("device/unrelated-{index}")}}),&[],None).unwrap();
            }
            tx.commit().unwrap();
        }
        assert!(store.commit_device_renewal(&stage,&source,&challenge,&consumed,&replacement).is_err());
    }
}
