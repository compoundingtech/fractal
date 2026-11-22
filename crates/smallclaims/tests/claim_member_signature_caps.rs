//! The caps on waiting claim signatures, in a process of their own: the caps are process-wide.
//! Each claim carries its writer's signature (`store::member_signatures`): the writer signs its
//! own claims, the signatures travel beside the envelopes that carry the claims, and a peer keeps
//! only those that verify with a key the membership admits for that writer and that name a claim
//! of that writer.
use smallclaims::{
    fleet::{MemberKey, keys::claim_signature_message},
    store::{Store, member_signatures::ClaimMemberSignature, runtime::Plain},
};
use std::sync::Arc;

const FLEET: &str = "sample-fleet";

fn node(name: &str, key: Option<&Arc<MemberKey>>, anchor: &Arc<MemberKey>) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    if let Some(key) = key {
        store.set_member_key(Some(key.clone())).unwrap();
    }
    store
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
fn a_signature_under_an_unknown_key_never_waits_and_the_caps_refuse_rather_than_evict() {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let birch = node("birch", None, &key);
    let invented = |n: usize| {
        let claim = format!("{:064x}", n);
        ClaimMemberSignature {
            signature: key.sign(&claim_signature_message(FLEET, "alder", 1, &claim)),
            claim_id: claim,
            writer: "alder".into(),
            sequence: 1,
            member_key: key.public().to_owned(),
        }
    };
    // A fresh key membership has never known is dropped, not parked.
    birch.receive_claim_member_signatures("cedar", &[fresh_signature("dogwood", 1, &format!("{:064x}", 9999))]).unwrap();
    assert_eq!(waiting(&birch), 0);

    // Past the per-peer cap a peer's signatures are refused and counted; those that wait stay.
    smallclaims::store::member_signatures::limit_waiting_claim_signatures(3, 5);
    birch.receive_claim_member_signatures("cedar", &(0..4).map(invented).collect::<Vec<_>>()).unwrap();
    assert_eq!(waiting(&birch), 3);
    assert_eq!(birch.claim_member_signatures_refused().unwrap(), 1);
    // Past the total cap another peer's are refused too.
    birch.receive_claim_member_signatures("dogwood", &(10..14).map(invented).collect::<Vec<_>>()).unwrap();
    assert_eq!(waiting(&birch), 5);
    assert_eq!(birch.claim_member_signatures_refused().unwrap(), 3);
    assert_eq!(birch.replication_status(true, Some(FLEET), &[]).unwrap().claim_signatures_refused, 3);
    for n in 0..3 {
        let kept: usize = birch.connection.write().query_row(
            "SELECT COUNT(*) FROM claim_member_signature_pending WHERE claim_id=?1",
            [format!("{:064x}", n)],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(kept, 1, "a waiting signature is never evicted");
    }
    smallclaims::store::member_signatures::limit_waiting_claim_signatures(
        smallclaims::store::member_signatures::PENDING_PER_PEER,
        smallclaims::store::member_signatures::PENDING_TOTAL,
    );
}
