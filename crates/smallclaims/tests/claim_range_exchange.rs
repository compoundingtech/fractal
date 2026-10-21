//! Peers compare the claims they retain by range digest, and only peers that speak the
//! claim-range protocol are compared.
use smallclaims::{
    claim::ClaimInput,
    replication::{CLAIM_RANGE_VERSION, ReplicationExchange, ReplicationInventory, ReplicationPeerSync},
    store::{Store, runtime::Plain},
};
use std::{collections::BTreeMap, sync::Arc};
const FLEET: &str = "sample-fleet";
fn node(name: &str) -> Store {
    Store::open_memory(name, Arc::new(Plain)).unwrap()
}
fn note(store: &Store, subject: &str) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}
fn fold(store: &Store) {
    store.fold_claim_ranges_paced(|_| {}).unwrap();
}
fn push(from: &Store, to: &Store) {
    let push = from
        .export_replication_exchange(FLEET, &ReplicationInventory::default())
        .unwrap();
    to.receive_replication_exchange("alder", FLEET, &push).unwrap();
    to.project_replication_backlog().unwrap();
}
fn receive(to: &Store, exchange: &ReplicationExchange) -> ReplicationPeerSync {
    to.receive_replication_exchange("alder", FLEET, exchange).unwrap();
    to.replication_peer_sync(&["alder".into()]).remove("alder").unwrap()
}

#[test]
fn peers_compare_the_claims_they_retain() {
    let source = node("alder");
    let target = node("birch");
    note(&source, "note/first");
    push(&source, &target);
    fold(&source);
    fold(&target);
    let summary = source.export_replication_summary(FLEET).unwrap();
    let retained = summary.claim_ranges.as_ref().unwrap().retained.as_ref().unwrap();
    assert_eq!(retained.count, 1);
    let sync = receive(&target, &summary);
    assert!(sync.claims_compared_at_unix_ms.is_some());
    assert_eq!(sync.claims_differ_since_unix_ms, None, "the same claim on both");

    note(&source, "note/second");
    fold(&source);
    let sync = receive(&target, &source.export_replication_summary(FLEET).unwrap());
    let differs = sync.claims_differ_since_unix_ms.expect("a claim only the peer holds");
    assert!(!sync.diverged, "a claim difference is not a divergence");

    push(&source, &target);
    fold(&target);
    let sync = receive(&target, &source.export_replication_summary(FLEET).unwrap());
    assert!(sync.claims_compared_at_unix_ms.unwrap() >= differs);
    assert_eq!(sync.claims_differ_since_unix_ms, None, "fetched by normal sync");
}

#[test]
fn a_peer_without_the_protocol_or_a_folded_index_is_not_compared() {
    let source = node("alder");
    let target = node("birch");
    note(&source, "note/first");
    push(&source, &target);
    let unfolded = source.export_replication_summary(FLEET).unwrap();
    assert_eq!(unfolded.claim_ranges.as_ref().unwrap().version, CLAIM_RANGE_VERSION);
    assert_eq!(unfolded.claim_ranges.as_ref().unwrap().retained, None, "still folding");
    fold(&source);
    fold(&target);

    let mut older = source.export_replication_summary(FLEET).unwrap();
    older.claim_ranges = None;
    let json = serde_json::to_value(&older).unwrap();
    assert!(json.get("claim_ranges").is_none(), "an older peer's exchange has no such field");
    let mut newer = source.export_replication_summary(FLEET).unwrap();
    newer.claim_ranges.as_mut().unwrap().version = CLAIM_RANGE_VERSION + 1;
    for exchange in [&unfolded, &older, &newer] {
        let sync = receive(&target, exchange);
        assert_eq!(sync.claims_compared_at_unix_ms, None);
    }
}
