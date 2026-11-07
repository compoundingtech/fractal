//! Owner identity is an admitted envelope signer, never a claim actor or a timestamp.
use super::*;
use smallclaims::claim::ReplicaBatch;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Position {
    pub writer: String,
    pub sequence: u64,
    pub position: u64,
}

fn unavailable(message: &str) -> FractalError {
    FractalError::new("notice-owner-proof-unavailable", message)
}

fn invalid(message: &str) -> FractalError {
    FractalError::new("invalid-notice-owner-proof", message)
}

/// `batch` is exclusively the writer-bound Runtime classification context, after
/// smallclaims' prepare_payload verified the envelope hash and bound its header to
/// this batch. Receipt has already persisted the envelope and verified signatures.
/// A unique writer/sequence therefore identifies that exact incoming envelope;
/// forks are rejected rather than choosing a received or canonical-time winner.
/// Without that context only the original, valid durable record is evidence.
pub(super) fn authenticate(
    connection: &Connection,
    claim: &ClaimRecord,
    batch: Option<&ReplicaBatch>,
    member_key: &str,
) -> Result<Position, FractalError> {
    let (position, record_hash) = if let Some(batch) = batch {
        if claim.origin != batch.origin || claim.batch_id != batch.id {
            return Err(invalid("the claim names a different writer or batch"));
        }
        let mut matches = batch.claims.iter().enumerate().filter(|(_, item)| item.id == claim.id);
        let Some((offset, item)) = matches.next() else {
            return Err(invalid("the claim is absent from the authenticated batch"));
        };
        if matches.next().is_some() {
            return Err(invalid("the claim has ambiguous positions in the authenticated batch"));
        }
        // The id commits to claim contents upstream; also reject a caller passing a
        // different record under that id. store_index is local, not signed identity.
        if item.batch_id != claim.batch_id || item.origin != claim.origin
            || item.subject != claim.subject || item.kind != claim.kind
            || item.actor != claim.actor || item.operation_id != claim.operation_id
            || item.request_digest != claim.request_digest || item.body != claim.body
            || item.predecessors != claim.predecessors
            || item.accepted_at_unix_ms != claim.accepted_at_unix_ms
        {
            return Err(invalid("the claim differs from its authenticated batch record"));
        }
        (Position { writer: batch.origin.clone(), sequence: batch.replica_sequence,
            position: offset as u64 }, None)
    } else {
        let mut statement = connection.prepare_cached(
            "SELECT r.writer,r.sequence,r.position,r.envelope_hash,b.origin,b.replica_sequence
             FROM replica_records r JOIN batches b ON b.id=?2
             WHERE r.claim_id=?1 AND r.state='valid' LIMIT 2",
        ).map_err(internal)?;
        let mut rows = statement.query(params![claim.id, claim.batch_id]).map_err(internal)?;
        let Some(row) = rows.next().map_err(internal)? else {
            return Err(unavailable("the original valid claim record or batch is unavailable"));
        };
        let position = Position { writer: row.get(0).map_err(internal)?,
            sequence: row.get(1).map_err(internal)?, position: row.get(2).map_err(internal)? };
        let hash: String = row.get(3).map_err(internal)?;
        let origin: String = row.get(4).map_err(internal)?;
        let sequence: u64 = row.get(5).map_err(internal)?;
        if rows.next().map_err(internal)?.is_some() {
            return Err(invalid("the claim has ambiguous durable positions"));
        }
        if origin != claim.origin || position.writer != origin || position.sequence != sequence {
            return Err(invalid("the original record does not match the claim writer and batch"));
        }
        (position, Some(hash))
    };
    let mut statement = connection.prepare_cached(
        "SELECT envelope_hash,batch_id FROM replica_envelopes WHERE writer=?1 AND sequence=?2 LIMIT 2",
    ).map_err(internal)?;
    let mut rows = statement.query(params![position.writer, position.sequence]).map_err(internal)?;
    let Some(row) = rows.next().map_err(internal)? else {
        return Err(unavailable("the original envelope is unavailable"));
    };
    let hash: String = row.get(0).map_err(internal)?;
    let envelope_batch: Option<String> = row.get(1).map_err(internal)?;
    if rows.next().map_err(internal)?.is_some() {
        return Err(invalid("the writer sequence has competing envelopes"));
    }
    if record_hash.as_ref().is_some_and(|expected| expected != &hash)
        || envelope_batch.as_deref().is_some_and(|id| id != claim.batch_id)
    {
        return Err(invalid("the envelope does not match the claim record or batch"));
    }
    if batch.is_none() && envelope_batch.is_none() {
        return Err(unavailable("the durable envelope has no admitted batch"));
    }
    let (signed, any_signature): (bool, bool) = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM replica_envelope_signatures
           WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4),
         EXISTS(SELECT 1 FROM replica_envelope_signatures
           WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3)",
        params![position.writer, position.sequence, hash, member_key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(internal)?;
    // Receipt inserts these rows only after VerifiedEnvelopeSignature::verify;
    // local sealing signs with the configured MemberKey. Both bind this exact hash.
    if !signed {
        return Err(if any_signature { invalid("the envelope is not signed by the bound owner key") }
            else { unavailable("the verified owner envelope signature is unavailable") });
    }
    let membership = smallclaims::store::fleet_membership_tx(connection).map_err(internal)?;
    if !membership.keys_for(&position.writer, position.sequence).contains(member_key) {
        return Err(invalid("the owner key is outside its admitted writer sequence window"));
    }
    Ok(position)
}

#[cfg(test)]
#[path = "notice_owner_proof_tests.rs"]
mod tests;
