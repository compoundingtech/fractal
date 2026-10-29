//! Members that hold the same claims summarize the same answers, and a claim one member lacks is
//! named by its kind and key. The tables in `fixture` hold only the columns the summary reads,
//! so a test can write any history, including one a trim has deleted from.
use proptest::prelude::*;
use rusqlite::{Connection, params};
use smallclaims::{
    answers::*,
    claim::ClaimInput,
    replication::ReplicationInventory,
    store::{Store, canonical, runtime::Plain},
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

/// The columns the summary reads, without the store's triggers and keys.
fn fixture() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
             CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE,
                 batch_id TEXT, subject TEXT, kind TEXT, origin TEXT, accepted_at_unix_ms TEXT);
             CREATE INDEX claims_kind_index ON claims(kind, store_index);
             CREATE INDEX claims_batch_index ON claims(batch_id, store_index);
             CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER);
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER);",
        )
        .unwrap();
    connection
}

/// A claim in its own batch, the writer's `sequence`th.
fn claim(connection: &Connection, id: &str, kind: &str, subject: &str, origin: &str, at: u128, sequence: u64) {
    let batch = format!("batch/{id}");
    connection
        .execute(
            "INSERT INTO batches(id, origin, replica_sequence) VALUES (?1, ?2, ?3)",
            params![batch, origin, sequence],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO claims(id, batch_id, subject, kind, origin, accepted_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, batch, subject, kind, origin, at.to_string()],
        )
        .unwrap();
}

fn delete(connection: &Connection, id: &str) {
    connection.execute("DELETE FROM claims WHERE id=?1", [id]).unwrap();
}

fn all_keys(connection: &Connection, kind: &str, cut: u128) -> Vec<AnswerKey> {
    let every = (0..BUCKETS).collect();
    kind_buckets(connection, kind, cut, &every).unwrap().keys
}

#[test]
fn a_key_answers_with_its_newest_claim_older_than_the_cut() {
    let connection = fixture();
    claim(&connection, "a1", "example.state", "item/a", "alder", 10, 1);
    claim(&connection, "a2", "example.state", "item/a", "alder", 20, 2);
    claim(&connection, "a3", "example.state", "item/a", "birch", 15, 1);
    // Still moving at the cut: its newest claim is not older than the cut.
    claim(&connection, "b1", "example.state", "item/b", "alder", 10, 3);
    claim(&connection, "b2", "example.state", "item/b", "alder", 100, 4);
    let keys = all_keys(&connection, "example.state", 50);
    let answers = keys.iter().map(|key| (key.subject.as_str(), key.origin.as_str(), key.claim.as_str())).collect::<Vec<_>>();
    assert_eq!(answers, [("item/a", "alder", "a2"), ("item/a", "birch", "a3")]);
    let answers = kind_answers(&connection, "example.state", 50).unwrap();
    assert_eq!((answers.keys, answers.claims), (2, 4));
}

#[test]
fn claims_of_one_millisecond_answer_in_canonical_order() {
    let connection = fixture();
    claim(&connection, "z-first", "example.state", "item/a", "alder", 10, 1);
    claim(&connection, "a-second", "example.state", "item/a", "alder", 10, 2);
    assert_eq!(all_keys(&connection, "example.state", 50)[0].claim, "a-second");
}

#[test]
fn deleting_a_superseded_claim_changes_no_answer_but_the_count() {
    let connection = fixture();
    claim(&connection, "a1", "example.state", "item/a", "alder", 10, 1);
    claim(&connection, "a2", "example.state", "item/a", "alder", 20, 2);
    claim(&connection, "a3", "example.state", "item/a", "alder", 70, 3);
    let full = summary(&connection, 50).unwrap();
    // A trim keeps a3, the newest, and drops a1 and a2, including the one the cut once chose.
    delete(&connection, "a1");
    delete(&connection, "a2");
    let trimmed = summary(&connection, 50).unwrap();
    assert!(differing_kinds(&full, &trimmed, false).is_empty());
    let differences = differing_kinds(&full, &trimmed, true);
    assert_eq!(differences.len(), 1, "two members that trim differently hold different counts");
}

#[test]
fn a_differing_key_is_named_through_its_bucket() {
    let ours = fixture();
    let theirs = fixture();
    for connection in [&ours, &theirs] {
        for n in 0..40 {
            claim(connection, &format!("c{n}"), "example.state", &format!("item/{n}"), "alder", 10, n);
        }
        claim(connection, "other", "example.other", "item/0", "alder", 10, 99);
    }
    // A claim only one member holds, which a reader of item/7 would answer with.
    claim(&theirs, "late", "example.state", "item/7", "alder", 20, 100);
    let differences = differing_kinds(&summary(&ours, 50).unwrap(), &summary(&theirs, 50).unwrap(), true);
    assert_eq!(differences.iter().map(|difference| difference.kind.as_str()).collect::<Vec<_>>(), ["example.state"]);
    let none = BTreeSet::new();
    let buckets = differing_buckets(
        &kind_buckets(&ours, "example.state", 50, &none).unwrap().buckets,
        &kind_buckets(&theirs, "example.state", 50, &none).unwrap().buckets,
    );
    assert_eq!(buckets, BTreeSet::from([bucket("item/7", "alder")]));
    let keys = differing_keys(
        &kind_buckets(&ours, "example.state", 50, &buckets).unwrap().keys,
        &kind_buckets(&theirs, "example.state", 50, &buckets).unwrap().keys,
    );
    assert_eq!(
        keys,
        [KeyDifference {
            subject: "item/7".into(),
            origin: "alder".into(),
            ours: Some("c7".into()),
            theirs: Some("late".into()),
        }]
    );
}

#[test]
fn a_kind_only_one_member_holds_differs() {
    let ours = fixture();
    let theirs = fixture();
    claim(&theirs, "x", "example.state", "item/a", "alder", 10, 1);
    let differences = differing_kinds(&summary(&ours, 50).unwrap(), &summary(&theirs, 50).unwrap(), false);
    assert_eq!(differences.len(), 1);
    assert_eq!(differences[0].ours, None);
    // A claim after the cut is not yet an answer, so it is no difference.
    let later = fixture();
    claim(&later, "y", "example.state", "item/a", "alder", 60, 1);
    assert!(differing_kinds(&summary(&ours, 50).unwrap(), &summary(&later, 50).unwrap(), true).is_empty());
}

/// A history of claims: (key, accepted time) per claim, over a few subjects and writers.
fn history() -> impl Strategy<Value = Vec<((u8, u8), u128)>> {
    prop::collection::vec(((0u8..4, 0u8..3), 0u128..100), 0..60)
}

proptest! {
    /// For any history, any trim that keeps every key's newest claim answers as the full log
    /// does, at every cut.
    #[test]
    fn a_trim_that_keeps_each_newest_claim_answers_the_same(
        claims in history(),
        dropped in prop::collection::vec(any::<bool>(), 60),
        cut in 0u128..110,
    ) {
        let connection = fixture();
        let mut by_key: BTreeMap<(u8, u8), Vec<String>> = BTreeMap::new();
        for (n, ((subject, origin), at)) in claims.iter().enumerate() {
            let id = format!("c{n:03}");
            claim(&connection, &id, "example.state", &format!("item/{subject}"), &format!("w{origin}"), *at, n as u64);
            by_key.entry((*subject, *origin)).or_default().push(id);
        }
        let full = all_keys(&connection, "example.state", cut);
        let newest = |connection: &Connection, ids: &[String]| {
            let mut keyed = ids.iter().map(|id| canonical::claim_key(connection, id).unwrap()).collect::<Vec<_>>();
            keyed.sort();
            keyed.pop().unwrap().5
        };
        for ids in by_key.values() {
            let keep = newest(&connection, ids);
            for id in ids {
                let n: usize = id[1..].parse().unwrap();
                if *id != keep && dropped[n] {
                    delete(&connection, id);
                }
            }
        }
        prop_assert_eq!(all_keys(&connection, "example.state", cut), full);
    }
}
