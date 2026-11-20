//! Each claim carries its writer's signature (`store::member_signatures`): the writer signs its
//! own claims, the signatures travel beside the envelopes that carry the claims, and a peer keeps
//! only those that verify with a key the membership admits for that writer and that name a claim
//! of that writer.
use smallclaims::{
    claim::ClaimInput,
    fleet::{MemberKey, keys::claim_signature_message},
    store::{Store, member_signatures::ClaimMemberSignature, runtime::Plain},
};
use std::{collections::BTreeMap, sync::Arc};

const FLEET: &str = "sample-fleet";

fn note(store: &Store, n: usize) {
    store
        .append_claim(&ClaimInput {
            subject: format!("note/sample-{n}"),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::from([("text".into(), serde_json::json!(format!("note {n}")))]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(format!("note-{n}")),
        })
        .unwrap();
}

fn node(name: &str, key: Option<&Arc<MemberKey>>, anchor: &Arc<MemberKey>) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    if let Some(key) = key {
        store.set_member_key(Some(key.clone())).unwrap();
    }
    store
}

fn signed(store: &Store) -> usize {
    store
        .connection
        .write()
        .query_row("SELECT COUNT(*) FROM claim_member_signatures", [], |row| row.get(0))
        .unwrap()
}

fn own_claims(store: &Store) -> usize {
    store
        .connection
        .write()
        .query_row("SELECT COUNT(*) FROM claims WHERE origin=?1", [store.origin()], |row| row.get(0))
        .unwrap()
}

/// Bring `peer` level with `source`, keeping the signatures each exchange carries.
fn sync(source: &Store, peer: &Store) {
    for _ in 0..8 {
        let exchange = source
            .export_replication_exchange(FLEET, &peer.replication_inventory().unwrap())
            .unwrap();
        if exchange.envelopes.is_empty() {
            break;
        }
        peer.receive_replication_exchange(source.origin(), FLEET, &exchange).unwrap();
        peer.validate_replication_backlog().unwrap();
        peer.project_replication_backlog().unwrap();
    }
}

/// One of `store`'s signatures, for a claim alder wrote, with its batch's sequence.
fn some_signature(store: &Store) -> ClaimMemberSignature {
    let claim: String = store
        .connection
        .write()
        .query_row(
            "SELECT claims.id FROM claims JOIN claim_member_signatures s ON s.claim_id=claims.id
             WHERE claims.kind='example.note' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    store.claim_member_signatures_for(&[claim]).unwrap().remove(0)
}

#[test]
fn a_writer_signs_its_claims_and_a_peer_keeps_only_signatures_that_verify() {
    // alder is the fleet's anchor member, so its key is the one membership admits for it.
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let alder = node("alder", Some(&key), &key);
    alder.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    for n in 0..700 {
        note(&alder, n);
    }
    // Pages until every own claim is signed; a second pass signs nothing new.
    while !alder.sign_own_claims().unwrap().done {}
    assert_eq!(signed(&alder), own_claims(&alder));
    assert_eq!(alder.sign_own_claims().unwrap().signed, 0);

    // The signatures ride with the envelopes that carry their claims, and the peer keeps each.
    let birch = node("birch", None, &key);
    sync(&alder, &birch);
    while birch.promote_claim_member_signatures().unwrap() > 0 {}
    assert_eq!(signed(&birch), signed(&alder), "the peer keeps every signature of alder's claims");

    let genuine = some_signature(&birch);
    let other = MemberKey::generate().unwrap().0;
    let message = |writer: &str, sequence: u64, claim: &str| claim_signature_message(FLEET, writer, sequence, claim);

    // A signature that verifies, by a key membership does not admit for alder, is dropped.
    let unadmitted = ClaimMemberSignature {
        member_key: other.public().to_owned(),
        signature: other.sign(&message(&genuine.writer, genuine.sequence, &genuine.claim_id)),
        ..genuine.clone()
    };
    // A signature that verifies for a writer membership does not key (cedar), naming alder's
    // claim, is dropped: the claim's batch is alder's.
    let foreign = ClaimMemberSignature {
        writer: "cedar".into(),
        member_key: other.public().to_owned(),
        signature: other.sign(&message("cedar", genuine.sequence, &genuine.claim_id)),
        ..genuine.clone()
    };
    // Altered bytes do not verify.
    let forged = ClaimMemberSignature { signature: other.sign(b"something else"), ..genuine.clone() };
    assert_eq!(birch.receive_claim_member_signatures(&[unadmitted, foreign.clone(), forged]).unwrap(), 0);
    assert_eq!(signed(&birch), signed(&alder), "no signature was added beside the genuine ones");

    // A signature for a claim not held yet waits, and is dropped once the claim turns out to be
    // another writer's.
    let cedar = node("cedar", None, &key);
    assert_eq!(cedar.receive_claim_member_signatures(&[foreign]).unwrap(), 0);
    sync(&alder, &cedar);
    while cedar.promote_claim_member_signatures().unwrap() > 0 {}
    assert_eq!(
        cedar.claim_member_signatures_for(&[genuine.claim_id.clone()]).unwrap(),
        vec![genuine],
        "only alder's own signature is kept for alder's claim"
    );
}

/// A signature that verifies for `writer` and `sequence` over `claim`, by a fresh key.
fn fresh_signature(writer: &str, sequence: u64, claim: &str) -> ClaimMemberSignature {
    let key = MemberKey::generate().unwrap().0;
    ClaimMemberSignature {
        claim_id: claim.into(),
        writer: writer.into(),
        sequence,
        member_key: key.public().to_owned(),
        signature: key.sign(&claim_signature_message(FLEET, writer, sequence, claim)),
    }
}

fn waiting(store: &Store) -> usize {
    store
        .connection
        .write()
        .query_row("SELECT COUNT(*) FROM claim_member_signature_pending", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn only_a_key_membership_admits_counts_and_waiting_signatures_never_hide_later_ones() {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let alder = node("alder", Some(&key), &key);
    alder.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    note(&alder, 0);
    while !alder.sign_own_claims().unwrap().done {}
    let genuine = some_signature(&alder);

    // cedar is a writer membership never keyed. Its claim is held here, signed with a fresh key
    // for its own writer and sequence: only the window can refuse it.
    let birch = node("birch", None, &key);
    let cedar = node("cedar", None, &key);
    note(&cedar, 1);
    cedar.replication_snapshot().unwrap();
    sync(&cedar, &birch);
    let (cedar_claim, cedar_sequence): (String, u64) = birch
        .connection
        .write()
        .query_row(
            "SELECT claims.id, batches.replica_sequence FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.origin='cedar' AND claims.kind='example.note'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(birch.receive_claim_member_signatures(&[fresh_signature("cedar", cedar_sequence, &cedar_claim)]).unwrap(), 0);
    birch.promote_claim_member_signatures().unwrap();
    assert!(birch.claim_member_signatures_for(&[cedar_claim]).unwrap().is_empty(), "an unkeyed writer's signature is not kept");

    // A made-up writer's signatures for claims that never arrive wait, more than a page of them,
    // all sorting before alder's claim. They never hide alder's genuine signature, which waits
    // until alder's claim arrives and is then kept.
    let invented: Vec<ClaimMemberSignature> = (0..600)
        .map(|n| fresh_signature("dogwood", 1, &format!("{:064x}", n)))
        .collect();
    assert!(invented.iter().all(|signature| signature.claim_id < genuine.claim_id));
    birch.receive_claim_member_signatures(&invented).unwrap();
    assert_eq!(birch.receive_claim_member_signatures(std::slice::from_ref(&genuine)).unwrap(), 0);
    assert_eq!(waiting(&birch), 601);
    sync(&alder, &birch);
    for _ in 0..4 {
        birch.promote_claim_member_signatures().unwrap();
    }
    assert_eq!(birch.claim_member_signatures_for(&[genuine.claim_id.clone()]).unwrap(), vec![genuine]);
    assert_eq!(waiting(&birch), 600, "the invented ones still wait for claims that never come");
    for signature in &invented {
        assert!(birch.claim_member_signatures_for(&[signature.claim_id.clone()]).unwrap().is_empty());
    }
}
