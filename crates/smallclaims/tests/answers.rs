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
fn a_claim_a_repair_replaced_counts_on_no_member() {
    let alder = node("alder");
    let birch = node("birch");
    let original = state(&alder, "item/a", "one");
    let replacement = state(&alder, "item/a", "two");
    push(&alder, &birch);
    // birch found the original's record invalid and repaired it with the newer claim. birch had
    // admitted the original already, and alder wrote it, so both keep its claim row; a member
    // that received the repair first never admits the original at all.
    let record = {
        let connection = birch.connection.write();
        let record: String = connection
            .query_row("SELECT record_ref FROM replica_records WHERE claim_id=?1", [&original], |row| row.get(0))
            .unwrap();
        connection.execute("UPDATE replica_records SET state='invalid' WHERE record_ref=?1", [&record]).unwrap();
        record
    };
    birch.repair_replica_record(&record, &replacement, "replace the original", "person/example", "repair-1").unwrap();
    let back = birch.export_replication_exchange(FLEET, &alder.replication_inventory().unwrap()).unwrap();
    alder.receive_replication_exchange("birch", FLEET, &back).unwrap();
    alder.validate_replication_backlog().unwrap();
    alder.project_replication_backlog().unwrap();
    alder.apply_replication_repairs().unwrap();

    let cut = cut();
    for store in [&alder, &birch] {
        assert!(store.claim_by_id(&original).unwrap().is_some(), "{} keeps the original's row", store.origin);
        let answers = store.answers_summary(cut).unwrap().kinds["example.state"].clone();
        assert_eq!((answers.keys, answers.claims), (1, 1), "{} counts only the replacement", store.origin);
    }
    assert_eq!(alder.answers_summary(cut).unwrap().kinds, birch.answers_summary(cut).unwrap().kinds);
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
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER);
             CREATE TABLE projection_digest_repaired_claims(id TEXT PRIMARY KEY);",
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
    let answers = keys
        .iter()
        .map(|key| (key.subject.as_str(), key.origin.as_str(), key.claim.as_str(), key.moving))
        .collect::<Vec<_>>();
    assert_eq!(
        answers,
        [("item/a", "alder", "a2", false), ("item/a", "birch", "a3", false), ("item/b", "alder", "b2", true)]
    );
    let answers = kind_answers(&connection, "example.state", 50).unwrap();
    assert_eq!((answers.keys, answers.claims), (2, 4));
}

#[test]
fn a_key_moving_on_either_member_is_no_difference() {
    // alder computed after item/b's newer claim arrived and birch before.
    let ours = fixture();
    claim(&ours, "a1", "example.state", "item/a", "alder", 10, 1);
    claim(&ours, "b1", "example.state", "item/b", "alder", 10, 2);
    claim(&ours, "b2", "example.state", "item/b", "alder", 100, 3);
    let theirs = fixture();
    claim(&theirs, "a1", "example.state", "item/a", "alder", 10, 1);
    claim(&theirs, "b1", "example.state", "item/b", "alder", 10, 2);
    let (ours, theirs) = (all_keys(&ours, "example.state", 50), all_keys(&theirs, "example.state", 50));
    assert_eq!(differing_keys(&ours, &theirs), []);
    assert_eq!(differing_keys(&theirs, &ours), []);

    // Both still hold item/b's claim before the cut, so the kind agrees without narrowing.
    let summary = |connection: &Connection| {
        let mut summary = AnswersSummary { cut_unix_ms: 50, ..AnswersSummary::default() };
        summary.kinds.insert("example.state".into(), kind_answers(connection, "example.state", 50).unwrap());
        summary
    };
    let (moved, behind) = (fixture(), fixture());
    for connection in [&moved, &behind] {
        claim(connection, "a1", "example.state", "item/a", "alder", 10, 1);
        claim(connection, "b1", "example.state", "item/b", "alder", 10, 2);
    }
    claim(&moved, "b2", "example.state", "item/b", "alder", 100, 3);
    assert_eq!(differing_kinds(&summary(&moved), &summary(&behind), true), []);
    // A member that trimmed item/b's older claim no longer agrees on the claims before the cut,
    // so the kind differs until narrowing finds only the moving key.
    delete(&moved, "b1");
    assert_eq!(differing_kinds(&summary(&moved), &summary(&behind), false).len(), 1);
    let keys = |connection: &Connection| all_keys(connection, "example.state", 50);
    assert_eq!(differing_keys(&keys(&moved), &keys(&behind)), []);

    // A fold reader's moving subject is listed but not counted.
    let key = |subject: &str, moving| AnswerKey { subject: subject.into(), origin: String::new(), claim: String::new(), moving };
    let (_, with_moving, buckets) = reader_answers("status_history", 50, vec![key("agent/a", false), key("agent/b", true)]);
    let (_, without, _) = reader_answers("status_history", 50, vec![key("agent/a", false)]);
    assert_eq!(with_moving, without);
    assert_eq!(buckets.keys.len(), 2);
    assert_eq!(differing_keys(&buckets.keys, &[key("agent/a", false), key("agent/b", false)]), []);
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

/// Ask `cache` until it stops answering `pending`; it computes on a thread of its own.
fn settled(cache: &Arc<AnswersCache>, store: &Arc<Store>, request: &AnswersRequest) -> AnswersReply {
    for _ in 0..500 {
        let reply = cache.answer(store, request);
        if !reply.pending {
            return reply;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the cache never answered {request:?}");
}

#[test]
fn the_cache_covers_fold_readers_and_narrows_them_by_subject() {
    let store = Arc::new(node("alder"));
    state(&store, "item/a", "one");
    let cache = Arc::new(AnswersCache::default());
    cache.set_readers(Arc::new(|_: &Connection, _: u128| {
        Ok(BTreeMap::from([(
            "status_history".to_owned(),
            vec![AnswerKey { subject: "agent/alder.worker".into(), origin: String::new(), claim: "answer".into(), moving: false }],
        )]))
    }));
    let cut = cut();
    let summary = AnswersRequest { cut_unix_ms: cut, ..AnswersRequest::default() };
    let reply = settled(&cache, &store, &summary).summary.unwrap();
    assert_eq!(reply.kinds["reader/status_history"].keys, 1);
    assert_eq!(reply.kinds["example.state"].keys, 1);

    let subject = BTreeSet::from([bucket("agent/alder.worker", "")]);
    let ask = |kind: &str, buckets: BTreeSet<usize>| AnswersRequest { cut_unix_ms: cut, kind: Some(kind.into()), buckets };
    let reader = settled(&cache, &store, &ask("reader/status_history", subject.clone())).buckets.unwrap();
    assert_eq!(reader.keys.len(), 1);
    assert_eq!(reader.keys[0].subject, "agent/alder.worker");
    // A reader this member does not run answers with nothing, rather than pending forever.
    let unknown = settled(&cache, &store, &ask("reader/unknown", subject)).buckets.unwrap();
    assert!(unknown.keys.is_empty());
    assert!(unknown.buckets.iter().all(String::is_empty));
}

/// A fold reader over the seats in `connection`, as the daemon's are: one key per subject, its
/// answer the newest claim, and moving once the subject has a claim at or after the cut.
fn seats(connection: &Connection, cut: u128) -> anyhow::Result<BTreeMap<String, Vec<AnswerKey>>> {
    let keys = connection
        .prepare(
            "SELECT subject, MAX(id), MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims
             WHERE subject LIKE 'agent/%' GROUP BY subject ORDER BY subject",
        )?
        .query_map([], |row| {
            let moving = row.get::<_, i64>(2)? as u128 >= cut;
            Ok(AnswerKey {
                subject: row.get(0)?,
                origin: String::new(),
                claim: if moving { String::new() } else { row.get(1)? },
                moving,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(BTreeMap::from([("harness".to_owned(), keys)]))
}

#[test]
fn members_that_kept_fold_readers_hours_apart_agree_once_a_seat_moves_past_the_cut() {
    let store = Arc::new(node("alder"));
    state(&store, "agent/sample.worker", "idle");
    state(&store, "agent/sample.reviewer", "idle");
    let cut = cut();
    let summary = AnswersRequest { cut_unix_ms: cut, ..AnswersRequest::default() };
    let member = || {
        let cache = Arc::new(AnswersCache::default());
        cache.set_readers(Arc::new(seats));
        cache
    };
    // One member computes, then a seat with claims before the cut is written again after it, as
    // a late flush of its readings is, and another member computes only then.
    let early = member();
    assert_eq!(settled(&early, &store, &summary).summary.unwrap().kinds["reader/harness"].keys, 2);
    store.set_write_clock_at(cut + 1_000).unwrap();
    state(&store, "agent/sample.worker", "busy");
    let late = member();
    let later = settled(&late, &store, &summary).summary.unwrap();
    assert_eq!(later.kinds["reader/harness"].keys, 1);

    let earlier = settled(&early, &store, &summary).summary.unwrap();
    assert_eq!(earlier.kinds["reader/harness"], later.kinds["reader/harness"], "the kept reader counts the seat it moved");
    let every = AnswersRequest { cut_unix_ms: cut, kind: Some("reader/harness".into()), buckets: (0..256).collect() };
    let (ours, theirs) = (settled(&early, &store, &every).buckets.unwrap(), settled(&late, &store, &every).buckets.unwrap());
    assert_eq!(ours.buckets, theirs.buckets);
    assert!(differing_keys(&ours.keys, &theirs.keys).is_empty());
}

#[test]
fn readers_compare_only_between_members_that_both_run_them() {
    let answers = |digest: &str| KindAnswers { keys: 1, claims: 0, digest: digest.into(), before_cut: String::new() };
    let summary = |kinds: &[(&str, &str)]| AnswersSummary {
        kinds: kinds.iter().map(|(kind, digest)| (kind.to_string(), answers(digest))).collect(),
        ..AnswersSummary::default()
    };
    let older = summary(&[("example.state", "a")]);
    let newer = summary(&[("example.state", "a"), ("reader/status_history", "b")]);
    assert!(differing_kinds(&older, &newer, true).is_empty(), "an older build runs no readers");
    let other = summary(&[("example.state", "a"), ("reader/status_history", "c")]);
    assert_eq!(differing_kinds(&newer, &other, true)[0].kind, "reader/status_history");
}


#[test]
fn a_restarted_member_serves_the_summary_it_kept_until_a_claim_older_than_the_cut_arrives() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Arc::new(Store::open(&path, "alder", Arc::new(Plain)).unwrap());
    state(&store, "item/a", "one");
    let summary = AnswersRequest { cut_unix_ms: cut(), ..AnswersRequest::default() };
    let first = settled(&Arc::new(AnswersCache::default()), &store, &summary).summary.unwrap();
    assert_eq!(first.kinds["example.state"].keys, 1);

    // A restarted daemon starts with an empty cache and answers with the kept summary. Mark it,
    // so a summary computed again would not carry the mark.
    let kept = directory.path().join("claims.sqlite3.answers.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let mut file: serde_json::Value = serde_json::from_slice(&std::fs::read(&kept).unwrap()).unwrap();
    file["summary"]["kinds"]["example.state"]["digest"] = "kept".into();
    std::fs::write(&kept, serde_json::to_vec(&file).unwrap()).unwrap();
    let cache = Arc::new(AnswersCache::default());
    assert_eq!(settled(&cache, &store, &summary).summary.unwrap().kinds["example.state"].digest, "kept");

    // A peer that was offline replays a claim its writer accepted before the cut. The kept
    // summary lacks it, so the member computes again and counts it.
    let birch = node("birch");
    state(&birch, "item/b", "one");
    let exchange = birch.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap();
    store.receive_replication_exchange("birch", FLEET, &exchange).unwrap();
    store.validate_replication_backlog().unwrap();
    store.project_replication_backlog().unwrap();
    let again = settled(&cache, &store, &summary).summary.unwrap();
    assert_eq!(again.kinds["example.state"].keys, 2);
    assert_ne!(again.kinds["example.state"].digest, "kept");

    // A store that now trims differently counts claims differently, so it computes again, as it
    // does at any other cut.
    store.trim_locally(true).unwrap();
    let trimming = settled(&Arc::new(AnswersCache::default()), &store, &summary).summary.unwrap();
    assert!(trimming.trims_locally);
    let tomorrow = AnswersRequest { cut_unix_ms: cut() + 86_400_000, ..AnswersRequest::default() };
    let later = settled(&Arc::new(AnswersCache::default()), &store, &tomorrow).summary.unwrap();
    assert_eq!(later.kinds["example.state"].keys, 2);
}

#[test]
fn claims_compare_only_between_members_that_finished_the_same_trim() {
    let alder = node("alder");
    let birch = node("birch");
    state(&alder, "item/a", "one");
    push(&alder, &birch);
    let day = 86_400_000;
    let trim = |store: &Store, trimmed: u128, state: &str| {
        store.set_checkpoint_state(&format!("checkpoint/{trimmed}"), trimmed, state).unwrap();
        store.answers_summary(cut()).unwrap()
    };
    let alder_summary = trim(&alder, day, "trimmed");
    let birch_summary = trim(&birch, day, "trimmed");
    assert!(claims_compare(&alder_summary, &birch_summary));

    // birch is part way through the next day's trim, then finishes it: until alder has too, the
    // members hold different claims older than the cut and still answer the same.
    let birch_summary = trim(&birch, 2 * day, "trimming");
    assert!(birch_summary.trim_unfinished);
    assert!(!claims_compare(&alder_summary, &birch_summary));
    let birch_summary = trim(&birch, 2 * day, "trimmed");
    assert_eq!(birch_summary.trimmed_cut_unix_ms, 2 * day);
    assert!(!claims_compare(&alder_summary, &birch_summary));
    let alder_summary = trim(&alder, 2 * day, "trimmed");
    assert!(claims_compare(&alder_summary, &birch_summary));
    assert!(differing_kinds(&alder_summary, &birch_summary, true).is_empty());
}
