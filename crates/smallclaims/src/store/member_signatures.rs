//! Each claim carries its writer's own signature, so a claim authenticates without the envelope
//! that carried it, and a member can release envelope history and still serve and verify every
//! claim it holds. The writer's member key signs `st3-claim-v1`, the fleet, the writer, the batch's
//! sequence and the claim id (`fleet::keys::claim_signature_message`). The claim id already
//! hashes the claim's content and its batch, so the signature binds both.
//!
//! A writer signs its own claims in a paced background pass, the new ones and those it held
//! before this build (`sign_own_claims`). Signatures travel beside the claims in an exchange's
//! `claim_member_signatures` and are kept only after the signature verifies with a key the
//! membership window admits for that writer and sequence. An older peer ignores the field.
use super::*;
use std::time::{Duration, Instant};

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-hc057e3fd627d owner=smallclaims::store::member_signatures TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS claim_member_signatures (
    id INTEGER PRIMARY KEY,
    claim_id TEXT NOT NULL UNIQUE,
    member_key TEXT NOT NULL,
    signature TEXT NOT NULL
);
";

const SIGN_CURSOR: &str = "claim_member_signature_cursor";
const SIGN_PAGE: usize = 512;
const STORE_BUDGET: Duration = Duration::from_millis(10);

/// One claim's writer signature, as an exchange carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimMemberSignature {
    pub claim_id: String,
    pub writer: String,
    pub sequence: u64,
    pub member_key: String,
    pub signature: String,
}

/// What one page of [`Store::sign_own_claims`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OwnClaimSigning {
    pub scanned: usize,
    pub signed: usize,
    pub done: bool,
}

impl Store {
    /// Sign one page of this writer's own claims that have no writer signature yet, in claim
    /// order. The page is read and signed outside the writer and stored in one short write; the
    /// cursor commits with it. A store without a member key or fleet signs nothing.
    pub fn sign_own_claims(&self) -> Result<OwnClaimSigning> {
        let Some(key) = self.member_key.read().unwrap_or_else(PoisonError::into_inner).clone() else {
            return Ok(OwnClaimSigning { done: true, ..Default::default() });
        };
        let (fleet_id, after, page) = {
            let connection = self.readers.get();
            let Some(fleet_id) = self.signing_fleet_id(&connection)? else {
                return Ok(OwnClaimSigning { done: true, ..Default::default() });
            };
            let after: i64 = connection
                .query_row("SELECT value FROM meta WHERE key=?1", [SIGN_CURSOR], |row| row.get::<_, String>(0))
                .optional()?
                .map(|value| value.parse::<i64>())
                .transpose()?
                .unwrap_or(0);
            let page = connection
                .prepare_cached(
                    "SELECT claims.store_index, claims.id, batches.replica_sequence,
                            EXISTS(SELECT 1 FROM claim_member_signatures WHERE claim_id=claims.id)
                     FROM claims JOIN batches ON batches.id=claims.batch_id
                     WHERE claims.store_index>?1 AND claims.origin=?2
                     ORDER BY claims.store_index LIMIT ?3",
                )?
                .query_map(params![after, self.origin, SIGN_PAGE], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, bool>(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            (fleet_id, after, page)
        };
        let mut report = OwnClaimSigning { scanned: page.len(), done: page.is_empty(), ..Default::default() };
        let through = page.last().map_or(after, |row| row.0);
        let signatures: Vec<ClaimMemberSignature> = page
            .into_iter()
            .filter(|(_, _, _, signed)| !signed)
            .map(|(_, claim_id, sequence, _)| ClaimMemberSignature {
                signature: key.sign(&crate::fleet::keys::claim_signature_message(
                    &fleet_id, &self.origin, sequence, &claim_id,
                )),
                member_key: key.public().to_owned(),
                writer: self.origin.clone(),
                sequence,
                claim_id,
            })
            .collect();
        report.signed = signatures.len();
        if report.scanned == 0 {
            return Ok(report);
        }
        self.connection
            .batched(|tx| -> Result<()> {
                for signature in &signatures {
                    insert_signature_tx(tx, signature)?;
                }
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hf875f5474570 owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![SIGN_CURSOR, through.to_string()],
                )?;
                Ok(())
            })
            .map_err(anyhow::Error::msg)??;
        Ok(report)
    }

    /// Keep the writer signatures a peer sent that verify with a key the membership window
    /// admits for that writer and sequence. Returns how many it kept; one that does not verify is
    /// dropped.
    pub fn receive_claim_member_signatures(&self, signatures: &[ClaimMemberSignature]) -> Result<usize> {
        if signatures.is_empty() {
            return Ok(0);
        }
        let (fleet_id, membership) = {
            let connection = self.readers.get();
            let Some(fleet_id) = self.signing_fleet_id(&connection)? else { return Ok(0) };
            (fleet_id, fleet_membership_tx(&connection)?)
        };
        let verified: Vec<&ClaimMemberSignature> = signatures
            .iter()
            .filter(|signature| {
                let admitted = match membership.window(&signature.writer, signature.sequence) {
                    crate::fleet::Window::Keyed(keys) => keys.contains(&signature.member_key),
                    crate::fleet::Window::Legacy => true,
                    crate::fleet::Window::Fenced => false,
                };
                admitted
                    && crate::fleet::verify_signature(
                        &signature.member_key,
                        &crate::fleet::keys::claim_signature_message(
                            &fleet_id, &signature.writer, signature.sequence, &signature.claim_id,
                        ),
                        &signature.signature,
                    )
            })
            .collect();
        if verified.is_empty() {
            return Ok(0);
        }
        self.connection
            .batched(|tx| -> Result<usize> {
                let started = Instant::now();
                let mut kept = 0;
                for signature in verified {
                    // The claim id hashes its batch, which hashes this writer and sequence, so a
                    // signature that verifies names exactly one claim, held here yet or not:
                    // signatures travel beside envelopes that are admitted after they arrive.
                    kept += insert_signature_tx(tx, signature)?;
                    if started.elapsed() >= STORE_BUDGET {
                        break;
                    }
                }
                Ok(kept)
            })
            .map_err(anyhow::Error::msg)?
    }

    /// The writer signatures of every claim the envelopes `envelopes` carry, found by each
    /// envelope's batch, for an exchange that sends those envelopes.
    pub fn claim_member_signatures_for_envelopes(
        &self,
        envelopes: &[crate::ReplicaEnvelopeId],
    ) -> Result<Vec<ClaimMemberSignature>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT signatures.claim_id, batches.origin, batches.replica_sequence,
                    signatures.member_key, signatures.signature
             FROM replica_envelopes AS envelopes
             JOIN batches ON batches.id=envelopes.batch_id
             JOIN claims INDEXED BY claims_batch_index ON claims.batch_id=envelopes.batch_id
             JOIN claim_member_signatures AS signatures ON signatures.claim_id=claims.id
             WHERE envelopes.writer=?1 AND envelopes.sequence=?2 AND envelopes.envelope_hash=?3",
        )?;
        let mut found = Vec::new();
        for envelope in envelopes {
            let rows = statement.query_map(params![envelope.writer, envelope.sequence, envelope.hash], |row| {
                Ok(ClaimMemberSignature {
                    claim_id: row.get(0)?,
                    writer: row.get(1)?,
                    sequence: row.get(2)?,
                    member_key: row.get(3)?,
                    signature: row.get(4)?,
                })
            })?;
            for row in rows {
                found.push(row?);
            }
        }
        Ok(found)
    }

    /// The writer signatures this store holds for `claim_ids`, as an exchange carries them.
    pub fn claim_member_signatures_for(&self, claim_ids: &[String]) -> Result<Vec<ClaimMemberSignature>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT signatures.claim_id, batches.origin, batches.replica_sequence,
                    signatures.member_key, signatures.signature
             FROM claim_member_signatures AS signatures
             JOIN claims ON claims.id=signatures.claim_id
             JOIN batches ON batches.id=claims.batch_id
             WHERE signatures.claim_id=?1",
        )?;
        let mut found = Vec::new();
        for claim_id in claim_ids {
            if let Some(signature) = statement
                .query_row(/* sqlite-large-value: allow-for-now lv-h4f1be29c47a1 owner=smallclaims::store::member_signatures TODO=trace-statement-and-value-representation */ [claim_id], |row| {
                    Ok(ClaimMemberSignature {
                        claim_id: row.get(0)?,
                        writer: row.get(1)?,
                        sequence: row.get(2)?,
                        member_key: row.get(3)?,
                        signature: row.get(4)?,
                    })
                })
                .optional()?
            {
                found.push(signature);
            }
        }
        Ok(found)
    }
}

fn insert_signature_tx(connection: &Connection, signature: &ClaimMemberSignature) -> Result<usize> {
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-hcf3fa63cc69a owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_member_signatures(claim_id, member_key, signature) VALUES (?1, ?2, ?3)",
        params![signature.claim_id, signature.member_key, signature.signature],
    )?)
}
