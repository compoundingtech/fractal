//! Members that hold the same claims summarize the same answers, and a claim one member lacks is
//! named by its kind and key.
use smallclaims::{
    claim::ClaimInput,
    replication::ReplicationInventory,
    store::{
        Store,
        answers::{differing_buckets, differing_keys, differing_kinds},
        runtime::Plain,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
const FLEET: &str = "sample-fleet";
/// A cut a day from now, so every claim these tests write is older than it.
fn cut() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 86_400_000
}
fn node(name: &str) -> Store {
    Store::open_memory(name, Arc::new(Plain)).unwrap()
}
fn state(store: &Store, subject: &str, value: &str) -> String {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "example.state".into(),
            actor: None,
            fields: BTreeMap::from([("value".into(), value.into())]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .id
}
fn push(from: &Store, to: &Store) {
    let push = from
        .export_replication_exchange(FLEET, &ReplicationInventory::default())
        .unwrap();
    to.receive_replication_exchange("alder", FLEET, &push).unwrap();
    to.validate_replication_backlog().unwrap();
    to.project_replication_backlog().unwrap();
}

#[test]
fn members_with_the_same_claims_answer_the_same_and_a_missing_claim_is_named() {
    let source = node("alder");
    let target = node("birch");
    state(&source, "item/a", "one");
    state(&source, "item/a", "two");
    state(&source, "item/b", "one");
    push(&source, &target);
    let cut = cut();
    let ours = target.answers_summary(cut).unwrap();
    assert_eq!(ours, source.answers_summary(cut).unwrap());
    let answers = &ours.kinds["example.state"];
    assert_eq!((answers.keys, answers.claims), (2, 3));

    let newest = state(&source, "item/b", "two");
    let theirs = source.answers_summary(cut).unwrap();
    let kinds = differing_kinds(&ours, &theirs, true);
    assert_eq!(kinds.len(), 1);
    assert_eq!(kinds[0].kind, "example.state");
    let none = BTreeSet::new();
    let buckets = differing_buckets(
        &target.answer_buckets("example.state", cut, &none).unwrap().buckets,
        &source.answer_buckets("example.state", cut, &none).unwrap().buckets,
    );
    let keys = differing_keys(
        &target.answer_buckets("example.state", cut, &buckets).unwrap().keys,
        &source.answer_buckets("example.state", cut, &buckets).unwrap().keys,
    );
    assert_eq!(keys.len(), 1);
    assert_eq!((keys[0].subject.as_str(), keys[0].origin.as_str()), ("item/b", "alder"));
    assert_eq!(keys[0].theirs.as_deref(), Some(newest.as_str()));

    push(&source, &target);
    assert!(differing_kinds(&target.answers_summary(cut).unwrap(), &theirs, true).is_empty());
}
