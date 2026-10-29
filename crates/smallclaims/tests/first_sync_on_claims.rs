//! A first sync settles on claims: it never waits for a peer's projections.
use smallclaims::{
    claim::ClaimInput,
    replication::ReplicationInventory,
    store::{Store, runtime::Plain},
};
use std::{collections::BTreeMap, sync::Arc};
const FLEET: &str = "sample-fleet";
fn node(name: &str) -> Store {
    Store::open_memory(name, Arc::new(Plain)).unwrap()
}
fn note(store: &Store) {
    store
        .append_claim(&ClaimInput {
            subject: "note/sample".into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}
fn first_sync_state(store: &Store) -> String {
    store.first_sync().unwrap().unwrap().state
}

#[test]
fn a_first_sync_ends_on_claims_from_a_peer_that_sends_no_projection_digests() {
    for compare in [false, true] {
        let source = node("alder");
        let target = node("birch");
        target.capture_divergences(compare);
        note(&source);
        target.begin_first_sync("alder").unwrap();
        let push = source
            .export_replication_exchange(FLEET, &ReplicationInventory::default())
            .unwrap();
        target
            .receive_replication_exchange("alder", FLEET, &push)
            .unwrap();
        target.project_replication_backlog().unwrap();
        assert_eq!(first_sync_state(&target), "syncing", "envelopes just arrived");
        let mut summary = source.export_replication_summary(FLEET).unwrap();
        summary.projection_digests.clear();
        summary.graph_digest.clear();
        target
            .receive_replication_exchange("alder", FLEET, &summary)
            .unwrap();
        assert_eq!(first_sync_state(&target), "verified", "compare={compare}");
        assert!(!target.replication_diverged());
    }
}

#[test]
fn without_the_check_a_settled_pair_reports_no_comparison() {
    let source = node("alder");
    let target = node("birch");
    note(&source);
    let push = source
        .export_replication_exchange(FLEET, &ReplicationInventory::default())
        .unwrap();
    target
        .receive_replication_exchange("alder", FLEET, &push)
        .unwrap();
    target.project_replication_backlog().unwrap();
    let mut summary = source.export_replication_summary(FLEET).unwrap();
    summary
        .projection_digests
        .insert("documents".into(), "different".into());
    target
        .receive_replication_exchange("alder", FLEET, &summary)
        .unwrap();
    let status = target
        .replication_status_sealed(true, Some(FLEET), &["alder".into()])
        .unwrap();
    let peer = &status.peers[0];
    assert!(peer.differing_tables.is_empty());
    assert!(!peer.projection_comparison_waiting);
    assert_eq!(peer.projection_graphs_equal, None);
    assert!(!target.replication_diverged());
    assert!(!target.replication_projections_agree("alder"));
}
