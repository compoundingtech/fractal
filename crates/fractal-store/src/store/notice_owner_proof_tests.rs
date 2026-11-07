use super::*;
use crate::fleet::MemberKey;
use crate::store::fleet_admission_tests_shared_seats::{admit, key, node, note, sync, FLEET};
use smallclaims::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicaEnvelopePayload};

fn fixture() -> (Store, Store, Arc<MemberKey>, ClaimRecord) {
    let owner = key();
    let source = node("owner", Some(&owner), Some(&owner));
    admit(&source, "owner", &owner, "anchor", None);
    let claim = note(&source, "signed owner evidence");
    let target = node("reader", None, Some(&owner));
    sync(&source, &target);
    (source, target, owner, claim)
}

fn envelope(source: &Store, claim: &ClaimRecord) -> ReplicaEnvelope {
    let id = source.readers.get().query_row(
        "SELECT writer,sequence,envelope_hash FROM replica_envelopes WHERE batch_id=?1",
        [&claim.batch_id], |row| Ok(ReplicaEnvelopeId {
            writer: row.get(0)?, sequence: row.get(1)?, hash: row.get(2)?,
        }),
    ).unwrap();
    source.replica_envelopes(vec![id]).unwrap().pop().unwrap()
}

#[test]
fn sealed_owner_position_uses_real_admitted_signature() {
    let (source, target, owner, claim) = fixture();
    let envelope = envelope(&source, &claim);
    assert!(crate::fleet::verify_signature(owner.public(),
        &crate::fleet::envelope_signature_message(FLEET, &envelope.writer,
            envelope.sequence, &envelope.hash), envelope.signature.as_deref().unwrap()));
    for store in [&source, &target] {
        let position = authenticate(&store.readers.get(), &claim, None, owner.public()).unwrap();
        assert_eq!(position.writer, "owner");
        assert_eq!(position.sequence, envelope.sequence);
        assert_eq!(position.position, 0);
    }
}

#[test]
fn another_admitted_member_cannot_author_the_owner_position() {
    let (source, target, owner, claim) = fixture();
    let other = key();
    admit(&source, "other", &other, "sponsor", None);
    sync(&source, &target);
    let membership = smallclaims::store::fleet_membership_tx(&target.readers.get()).unwrap();
    assert!(matches!(membership.state("other"), crate::fleet::MemberState::Current(_)));
    assert_ne!(owner.public(), other.public());
    assert_eq!(authenticate(&target.readers.get(), &claim, None, other.public())
        .unwrap_err().code, "invalid-notice-owner-proof");
}

#[test]
fn missing_signature_and_record_are_unavailable_without_unsigned_fallback() {
    for missing in ["signature", "record"] {
        let (_, target, owner, claim) = fixture();
        let sql = if missing == "signature" {
            "DELETE FROM replica_envelope_signatures WHERE writer='owner' AND sequence=(
             SELECT replica_sequence FROM batches WHERE id=?1)"
        } else {
            "DELETE FROM replica_records WHERE claim_id IN (SELECT id FROM claims WHERE batch_id=?1)"
        };
        target.connection.write().execute(sql, [&claim.batch_id]).unwrap();
        assert_eq!(authenticate(&target.readers.get(), &claim, None, owner.public())
            .unwrap_err().code, "notice-owner-proof-unavailable", "{missing}");
    }
}

#[test]
fn writer_sequence_fork_rejects_both_real_signed_envelopes() {
    let (source, target, owner, claim) = fixture();
    let original = envelope(&source, &claim);
    let fork = node("owner", Some(&owner), Some(&owner));
    admit(&fork, "owner", &owner, "anchor", None);
    let fork_claim = note(&fork, "a genuinely signed competing write");
    let competing = envelope(&fork, &fork_claim);
    assert_eq!(original.sequence, competing.sequence);
    assert_ne!(original.hash, competing.hash);
    let mut exchange = fork.export_replication_exchange(FLEET,
        &target.replication_inventory().unwrap()).unwrap();
    // Ask for the exact competing envelope even when the receiver's writer cursor
    // already covers this sequence. It remains a real signed envelope from fork.
    exchange.envelopes = vec![competing];
    target.receive_replication_exchange("owner", FLEET, &exchange).unwrap();
    let error = authenticate(&target.readers.get(), &claim, None, owner.public()).unwrap_err();
    assert_eq!(error.code, "invalid-notice-owner-proof");
    assert!(error.message.contains("competing envelopes"));
}

#[test]
fn incoming_batch_proof_needs_no_already_admitted_claim_record() {
    let owner = key();
    let source = node("owner", Some(&owner), Some(&owner));
    admit(&source, "owner", &owner, "anchor", None);
    let target = node("reader", None, Some(&owner));
    sync(&source, &target);
    let claim = note(&source, "incoming signed owner evidence");
    let incoming = envelope(&source, &claim);
    let payload: ReplicaEnvelopePayload = ciborium::from_reader(
        incoming.payload.bytes().unwrap()).unwrap();
    let exchange = source.export_replication_exchange(FLEET,
        &target.replication_inventory().unwrap()).unwrap();
    target.receive_replication_exchange("owner", FLEET, &exchange).unwrap();
    assert!(target.claim_by_id(&claim.id).unwrap().is_none());
    let connection = target.readers.get();
    assert_eq!(authenticate(&connection, &claim, None, owner.public()).unwrap_err().code,
        "notice-owner-proof-unavailable");
    let position = authenticate(&connection, &claim, Some(&payload.batch), owner.public()).unwrap();
    assert_eq!(position.sequence, incoming.sequence);
    assert_eq!(position.position, 0);
    let mut duplicate = payload.batch.clone();
    duplicate.claims.push(duplicate.claims[0].clone());
    assert_eq!(authenticate(&connection, &claim, Some(&duplicate), owner.public())
        .unwrap_err().code, "invalid-notice-owner-proof");
    let mut wrong_origin = claim.clone();
    wrong_origin.origin = "other".into();
    assert_eq!(authenticate(&connection, &wrong_origin, Some(&payload.batch), owner.public())
        .unwrap_err().code, "invalid-notice-owner-proof");
}
