//! Daemon-authored, owner-local observations: deliberately NOT registered claim
//! kinds. The public claim/observation validators cannot mint reboot evidence.
use super::*;

const KIND: &str = "runtime.start.reboot-witness";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RebootWitnessRecord {
    pub observation_id: String,
    pub digest: String,
    pub manifest: Value,
}

fn checked_record(origin: &str, id: i64, body: Value) -> Result<RebootWitnessRecord> {
    let manifest = body["fields"]["manifest"].clone();
    let digest = body["fields"]["digest"].as_str().context("missing reboot witness digest")?;
    anyhow::ensure!(manifest["host"].as_str() == Some(origin), "reboot witness belongs to another host");
    anyhow::ensure!(canonical_hash(&manifest)? == digest, "reboot witness digest mismatch");
    Ok(RebootWitnessRecord { observation_id: format!("{LOCAL_OBSERVATION_ID_PREFIX}{origin}/{id}"),
        digest: digest.into(), manifest })
}

impl Store {
    /// Only the daemon's fenced capture handler calls this. No request-supplied
    /// manifest or generic claim kind is accepted by that handler.
    pub fn retain_reboot_witness(&self, operation_id: &str, manifest: &Value, requester: &str) -> Result<RebootWitnessRecord> {
        self.connection.batched(|tx| -> Result<RebootWitnessRecord> {
            let operation = tx.query_row("SELECT * FROM local_runtime_starts WHERE id=?1", [operation_id], runtime_starts::read)?;
            anyhow::ensure!(operation.revoked_by.is_some(), "reboot witness operation is not revoked");
            anyhow::ensure!(manifest["version"] == 1 && manifest["operation_id"].as_str() == Some(operation_id)
                && manifest["host"].as_str() == Some(self.origin.as_str())
                && manifest["runtime_id"] == operation.body["member"]["runtime_id"]
                && operation.body["member"]["host"].as_str() == Some(self.origin.as_str())
                && operation.body["member"]["terminal"] == true, "reboot witness operation binding mismatch");
            let digest = canonical_hash(manifest)?;
            let body = json!({"fields": {"manifest": manifest, "digest": digest, "requester": requester}, "evidence": []});
            let now = now_ms();
            // sqlite-large-value: allow-for-now lv-h90797e284665 owner=fractal_store::store::reboot_witness TODO=bound-or-reference-values-and-reclamation
            tx.execute("INSERT INTO local_observations(after_store_index,subject,kind,actor,body,observed_at_unix_ms) VALUES (?1,?2,?3,'daemon/runtime',?4,?5)",
                params![current_index(tx)?, operation.subject, KIND, canonical_json_text(&body)?, now.to_string()])?;
            let id = tx.last_insert_rowid();
            // Retain the authentic witness until this operation is closed. The
            // existing observation trimmer respects these operation evidence pins.
            // sqlite-large-value: allow-for-now lv-he41e21dfd24d owner=fractal_store::store::reboot_witness TODO=bound-or-reference-values-and-reclamation
            tx.execute("INSERT INTO local_runtime_start_evidence(operation_id,observation_id) VALUES (?1,?2)", params![operation_id,id])?;
            checked_record(&self.origin, id, body)
        }).map_err(|error| anyhow::anyhow!("{error}"))?
    }

    pub fn reboot_witness_for(&self, subject: &str, operation_id: &str) -> Result<Option<RebootWitnessRecord>> {
        let connection = self.readers.get();
        let row: Option<(i64, String)> = connection.query_row(
            "SELECT id,body FROM local_observations WHERE subject=?1 AND kind=?2 AND json_extract(body,'$.fields.manifest.operation_id')=?3 AND actor='daemon/runtime' ORDER BY id DESC LIMIT 1",
            params![subject,KIND,operation_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        row.map(|(id,body)| checked_record(&self.origin,id,serde_json::from_str(&body)?)).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reboot_witness_rejects_tampered_digest_and_foreign_host() {
        let manifest = json!({"host":"owner","operation_id":"op"});
        let body = json!({"fields":{"manifest":manifest,"digest":canonical_hash(&manifest).unwrap()}});
        assert!(checked_record("owner",1,body.clone()).is_ok());
        assert!(checked_record("foreign",1,body.clone()).is_err());
        let mut tampered = body; tampered["fields"]["manifest"]["operation_id"] = json!("other");
        assert!(checked_record("owner",1,tampered).is_err());
    }
    #[test]
    fn reboot_witness_cannot_be_forged_as_a_public_observation() {
        let input = ClaimInput { subject:"agent/owner/seat".into(), kind:KIND.into(), actor:Some("daemon/runtime".into()),
            fields:BTreeMap::new(),evidence:vec![],expected_subject:None,idempotency_key:None };
        assert!(validate_local_observation(&input).is_err());
    }
}
