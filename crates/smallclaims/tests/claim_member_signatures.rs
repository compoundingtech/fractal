//! Each claim carries its writer's signature (`store::member_signatures`): the writer signs its
//! own claims, the signatures travel beside the envelopes that carry the claims, and a peer keeps
//! only those that verify for that writer.
use smallclaims::{
    claim::ClaimInput,
    fleet::MemberKey,
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

fn node(name: &str, key: Option<&Arc<MemberKey>>) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
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

#[test]
fn a_writer_signs_its_claims_and_a_peer_keeps_only_signatures_that_verify() {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let alder = node("alder", Some(&key));
    for n in 0..700 {
        note(&alder, n);
    }
    // Pages until every own claim is signed; a second pass signs nothing new.
    while !alder.sign_own_claims().unwrap().done {}
    assert_eq!(signed(&alder), own_claims(&alder));
    assert_eq!(alder.sign_own_claims().unwrap().signed, 0);

    // The signatures ride with the envelopes that carry their claims.
    let birch = node("birch", None);
    let mut kept = 0;
    for _ in 0..8 {
        let exchange = alder
            .export_replication_exchange(FLEET, &birch.replication_inventory().unwrap())
            .unwrap();
        if exchange.envelopes.is_empty() {
            break;
        }
        assert!(!exchange.claim_member_signatures.is_empty());
        birch.receive_replication_exchange("alder", FLEET, &exchange).unwrap();
        birch.validate_replication_backlog().unwrap();
        kept = signed(&birch);
    }
    assert_eq!(kept, signed(&alder), "the peer keeps every signature of alder's claims");

    // A forged signature, or one by a key that is not the writer's, is dropped.
    let cedar = node("cedar", None);
    let mut signature: ClaimMemberSignature = alder
        .claim_member_signatures_for(&[alder
            .connection
            .write()
            .query_row("SELECT claim_id FROM claim_member_signatures LIMIT 1", [], |row| row.get::<_, String>(0))
            .unwrap()])
        .unwrap()
        .remove(0);
    let other = MemberKey::generate().unwrap().0;
    let mut wrong_key = signature.clone();
    wrong_key.member_key = other.public().to_owned();
    signature.signature = other.sign(b"something else");
    assert_eq!(cedar.receive_claim_member_signatures(&[signature, wrong_key]).unwrap(), 0);
}
