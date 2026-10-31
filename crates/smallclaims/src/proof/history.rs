//! Sealed historical evidence; no runtime naming or principal policy is assumed here.
use std::collections::{BTreeMap, BTreeSet};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use crate::claim::{ClaimRecord, ReplicaEnvelope, ReplicaEnvelopePayload};
use crate::fleet::{FleetClaim, Membership, envelope_signature_message, verify_signature};
use crate::hash::{claim_id_is_content_hash, replica_envelope_hash, verify_replica_batch_header};
use crate::principal::ClaimSignature;
use super::{ProofError, chain_error, malformed};

pub(super) struct Record {
    pub(super) claim: ClaimRecord,
    pub(super) sequence: u64,
    pub(super) position: usize,
    pub(super) signatures: Vec<ClaimSignature>,
}

/// Authenticated sealed claims and membership at the supplied historical horizons.
/// Fields are private so consumers cannot construct or modify verified evidence.
pub struct VerifiedHistory {
    pub(super) records: BTreeMap<String, Record>,
    pub(super) membership: Membership,
}

impl VerifiedHistory {
    /// Look up an immutable authenticated claim. Principal authorization is a separate layer.
    pub fn claim(&self, id: &str) -> Option<&ClaimRecord> {
        self.records.get(id).map(|record| &record.claim)
    }
}

/// Verify exact sealed bytes, writer prefixes, and anchored member windows offline.
/// This proves a supplied historical cut, never the absence of later envelopes.
pub fn verify_sealed_history(
    envelopes: &[ReplicaEnvelope], fleet_id: &str, fleet_anchor: &str,
) -> Result<VerifiedHistory, ProofError> {
    if fleet_id.is_empty() || fleet_anchor.is_empty() {
        return Err(malformed("empty fleet identity or anchor"));
    }
    let mut records = BTreeMap::<String, Record>::new();
    let mut fleet = Vec::new();
    let mut writers = BTreeMap::<&str, BTreeMap<u64, &ReplicaEnvelope>>::new();
    let mut batch_hashes = BTreeMap::new();
    for envelope in envelopes {
        if envelope.sequence == 0 || writers.entry(&envelope.writer).or_default().insert(envelope.sequence, envelope).is_some() {
            return Err(chain_error("duplicate writer sequence or zero sequence"));
        }
        let bytes = envelope.payload.bytes().map_err(|error| malformed(error.to_string()))?;
        if replica_envelope_hash(&envelope.writer, envelope.sequence, envelope.previous_hash.as_deref(), envelope.accepted_at_unix_ms, bytes) != envelope.hash {
            return Err(ProofError::new("invalid-signature", "envelope content hash mismatch"));
        }
        let (Some(key), Some(signature)) = (&envelope.member_key, &envelope.signature) else {
            return Err(ProofError::new("invalid-signature", "envelope is not sealed by a member key"));
        };
        if !verify_signature(key, &envelope_signature_message(fleet_id, &envelope.writer, envelope.sequence, &envelope.hash), signature) {
            return Err(ProofError::new("invalid-signature", "invalid envelope signature"));
        }
        let mut cursor = std::io::Cursor::new(bytes);
        let payload: ReplicaEnvelopePayload = ciborium::from_reader(&mut cursor).map_err(|error| malformed(error.to_string()))?;
        if cursor.position() != bytes.len() as u64 { return Err(malformed("trailing envelope payload bytes")); }
        verify_replica_batch_header(&payload.batch).map_err(|error| malformed(error.to_string()))?;
        batch_hashes.insert((envelope.writer.as_str(), envelope.sequence), payload.batch.hash.clone());
        let batch = &payload.batch;
        if batch.origin != envelope.writer || batch.replica_sequence != envelope.sequence || batch.previous_hash != envelope.previous_hash || batch.accepted_at_unix_ms != envelope.accepted_at_unix_ms {
            return Err(malformed("envelope and batch header disagree"));
        }
        for (hash, blob) in &payload.blobs {
            if hex::encode(Sha256::digest(blob)) != *hash {
                return Err(ProofError::new("invalid-signature", "blob content hash mismatch"));
            }
        }
        for (position, claim) in batch.claims.iter().enumerate() {
            if claim.batch_id != batch.id || claim.origin != batch.origin || claim.accepted_at_unix_ms != batch.accepted_at_unix_ms || !claim_id_is_content_hash(claim).map_err(|error| malformed(error.to_string()))? {
                return Err(ProofError::new("invalid-signature", "claim identity or batch metadata mismatch"));
            }
            if claim.kind.starts_with("fleet.") {
                fleet.push(FleetClaim {
                    id: claim.id.clone(), kind: claim.kind.clone(), subject: claim.subject.clone(),
                    fields: serde_json::from_value(claim.body["fields"].clone()).map_err(|error| malformed(error.to_string()))?,
                    writer: envelope.writer.clone(), sequence: envelope.sequence,
                    signers: BTreeSet::from([key.clone()]),
                });
            }
            let signatures = payload.claim_signatures.get(&claim.id).cloned().into_iter().collect();
            if records.insert(claim.id.clone(), Record { claim: claim.clone(), sequence: batch.replica_sequence, position, signatures }).is_some() {
                return Err(malformed("duplicate claim ID"));
            }
        }
        if payload.claim_signatures.keys().any(|id| !batch.claims.iter().any(|claim| &claim.id == id)) {
            return Err(malformed("signature does not name a claim in its envelope"));
        }
    }
    let membership = Membership::fold(Some(fleet_anchor), &fleet);
    for envelope in envelopes {
        let key = envelope.member_key.as_deref().expect("checked above");
        // The anchor's admission bootstraps its own writer window.
        let bootstrap = key == fleet_anchor && fleet.iter().any(|claim| {
            claim.writer == envelope.writer && claim.sequence == envelope.sequence
                && claim.kind == "fleet.member-admitted" && membership.counts(&claim.id)
                && claim.fields.get("member_key").and_then(Value::as_str) == Some(fleet_anchor)
        });
        if !bootstrap && !membership.keys_for(&envelope.writer, envelope.sequence).contains(key) {
            return Err(ProofError::new("untrusted-membership", format!("writer {} sequence {} is outside its member window", envelope.writer, envelope.sequence)));
        }
    }
    for envelopes in writers.values() {
        let mut high_water = 0;
        let mut previous = None;
        for envelope in envelopes.values() {
            let incarnation = membership.incarnations().find(|incarnation| {
                incarnation.name == envelope.writer
                    && Some(incarnation.member_key.as_str()) == envelope.member_key.as_deref()
                    && incarnation.start <= envelope.sequence
                    && incarnation.end.is_none_or(|end| envelope.sequence <= end)
            }).ok_or_else(|| chain_error("writer has no authenticated incarnation"))?;
            // A fresh store has a floor but no retained head. Only the authenticated
            // incarnation's first sequence can reset that head; never an interior gap.
            let fresh = envelope.sequence == incarnation.start && envelope.previous_hash.is_none();
            let (sequence, predecessor) = crate::writer_chain::writer_continuation(
                if fresh { 0 } else { high_water }, incarnation.start - 1,
                if fresh { None } else { previous },
            );
            if envelope.sequence != sequence || envelope.previous_hash.as_deref() != predecessor {
                return Err(chain_error("writer envelope chain is incomplete or disconnected"));
            }
            high_water = envelope.sequence;
            previous = batch_hashes.get(&(envelope.writer.as_str(), envelope.sequence)).map(String::as_str);
        }
    }
    Ok(VerifiedHistory { records, membership })
}

#[cfg(test)]
#[path = "history_tests.rs"]
pub(super) mod tests;
