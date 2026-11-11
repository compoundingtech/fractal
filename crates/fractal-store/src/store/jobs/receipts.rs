use super::*;
use fractal_jobs::identity::{IdempotencyDecision, RequestDigest, decide_idempotency};
use rusqlite::OptionalExtension as _;
use sha2::{Digest as _, Sha256};

/// One subject per requester/key prevents a receipt conflict from requiring another
/// subject's refold. The digest deliberately excludes the request digest: changed
/// input must conflict with the same subject, never become another job.
pub fn job_id(requester: &str, key: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"fractal-job-id-v1\n");
    hash.update((requester.len() as u64).to_be_bytes());
    hash.update(requester.as_bytes());
    hash.update(key.as_bytes());
    format!("job/{}", hex::encode(hash.finalize()))
}

pub(super) fn reserve_receipt(tx: &Transaction<'_>, identity: &IdempotencyKey, id: &str) -> Result<()> {
    ensure!(id == job_id(&identity.requester, &identity.key), "job-idempotency-subject-mismatch");
    let existing = receipt_for_key(tx, &identity.requester, &identity.key)?;
    match decide_idempotency(identity, existing) {
        Ok(IdempotencyDecision::NewJob) => {
            tx.execute("INSERT INTO job_receipts VALUES(?1,?2,?3,?4)",
                params![identity.requester, identity.key, serde_json::to_string(&identity.request_digest)?, id])?;
        }
        Ok(IdempotencyDecision::ExistingJob(existing)) => ensure!(existing == id, "job-receipt-conflict"),
        Err(reason) => anyhow::bail!(serde_json::to_string(&reason)?),
    }
    Ok(())
}

pub fn receipt_for_key(connection: &Connection, requester: &str, key: &str) -> Result<Option<(RequestDigest, String)>> {
    let existing: Option<(String, String)> = connection.query_row(
        "SELECT digest,job_id FROM job_receipts WHERE requester=?1 AND key=?2",
        params![requester, key], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    existing.map(|(digest, job)| {
        Ok((serde_json::from_str::<RequestDigest>(&digest)?, job))
    }).transpose()
}
