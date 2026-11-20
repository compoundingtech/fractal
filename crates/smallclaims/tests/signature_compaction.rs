//! Storage version 24 readers: a compacted claim keeps its facade, count, verdict, nonce use and
//! dependency links, without candidate or source rows. Readers take the facade as its single
//! candidate, so verdicts, serving, revocation, delegation and requeue read as before, and a new
//! candidate expands it first. The writer compacts a claim as it settles and pages through the
//! claims earlier releases kept whole.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use smallclaims::claim::{ClaimInput, ClaimRecord};
use smallclaims::fleet::MemberKey;
use smallclaims::principal::{ClaimSignature, KEY_GRANTED, KEY_REVOKED, Verdict, content_digest};
use smallclaims::store::Store;
use smallclaims::store::principals::store_claim_signature_tx;
use smallclaims::store::runtime::Plain;

fn key() -> Arc<MemberKey> {
    Arc::new(MemberKey::generate().unwrap().0)
}

fn studio() -> (tempfile::TempDir, Store, Arc<MemberKey>) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("claims.db"), "studio", Arc::new(Plain)).unwrap();
    let anchor = key();
    store.set_node_key(anchor.clone()).unwrap();
    (directory, store, anchor)
}

fn append(store: &Store, kind: &str, subject: &str, actor: Option<&str>, fields: Value) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

fn note(store: &Store, actor: Option<&str>, text: &str) -> ClaimRecord {
    append(store, "example.note", "note/plans", actor, json!({"text": text}))
}

fn seal(store: &Store) {
    store.replication_snapshot().unwrap();
}

fn drain(store: &Store) {
    for _ in 0..10_000 {
        store.advance_signature_judgment().unwrap();
        if !store.verdicts_pending().unwrap() {
            return;
        }
    }
    panic!("the paced signature worker must converge");
}

fn verdict(store: &Store, claim: &ClaimRecord) -> Verdict {
    store.claim_verdict(&claim.id).unwrap()
}

fn verdicts(store: &Store) -> BTreeMap<String, String> {
    let connection = store.readers.get();
    connection
        .prepare("SELECT claim_id, verdict FROM claim_verdicts")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(store: &Store, sql: &str) -> i64 {
    store.readers.get().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn attach(store: &Store, claim: &ClaimRecord, signature: &ClaimSignature) {
    let connection = store.connection.write();
    store_claim_signature_tx(&connection, &claim.id, signature).unwrap();
}

/// Run the version 24 writer to its end: judging compacts each claim as it settles, and the
/// paced pass the rest. Returns how many claims are compacted.
fn compact(store: &Store) -> i64 {
    drain(store);
    for _ in 0..10_000 {
        if store.compact_settled_signatures().unwrap().finished {
            return compacted(store);
        }
    }
    panic!("the paced compaction must finish");
}

fn compacted(store: &Store) -> i64 {
    count(
        store,
        "SELECT COUNT(*) FROM claim_signatures s JOIN claim_signature_counts USING(claim_id)
          WHERE NOT EXISTS(SELECT 1 FROM claim_signature_candidates d WHERE d.claim_id=s.claim_id)",
    )
}

#[test]
fn compacted_claims_read_serve_and_recheck_as_before() {
    let (_directory, store, _anchor) = studio();
    let claims = [
        note(&store, None, "a node note"),
        note(&store, Some("person/ada"), "a delegated note"),
    ];
    seal(&store);
    drain(&store);
    let before = verdicts(&store);
    let served = claims
        .iter()
        .map(|claim| store.claim_signature(&claim.id).unwrap().unwrap())
        .collect::<Vec<_>>();
    let signed = count(&store, "SELECT COUNT(*) FROM claim_signatures");
    assert_eq!(compact(&store), signed, "every settled claim here compacts");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_signature_candidates"), 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_signature_sources"), 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_signature_counts"), signed);

    assert_eq!(verdicts(&store), before);
    for (claim, signature) in claims.iter().zip(&served) {
        assert_eq!(verdict(&store, claim), Verdict::Verified);
        assert_eq!(store.claim_signature(&claim.id).unwrap().as_ref(), Some(signature));
    }
    // A full rejudge takes each facade as its candidate and finds nothing to correct.
    assert!(store.recheck_claim_verdicts().unwrap().is_empty());
    drain(&store);
    assert_eq!(verdicts(&store), before);
    assert_eq!(compacted(&store), signed, "judging never expands a compacted claim");
}

#[test]
fn a_compacted_grant_still_verifies_its_delegated_key_for_new_claims() {
    let (_directory, store, _anchor) = studio();
    let first = note(&store, Some("person/ada"), "first");
    seal(&store);
    drain(&store);
    assert_eq!(verdict(&store, &first), Verdict::Verified);
    assert!(compact(&store) > 0);
    let grants = count(
        &store,
        &format!(
            "SELECT COUNT(*) FROM claims c WHERE c.kind='{KEY_GRANTED}'
               AND NOT EXISTS(SELECT 1 FROM claim_signature_candidates d WHERE d.claim_id=c.id)"
        ),
    );
    assert!(grants > 0, "the delegation grants are compacted");
    // A later claim is judged fresh through the compacted grant chain.
    let later = note(&store, Some("person/ada"), "later");
    seal(&store);
    drain(&store);
    assert_eq!(verdict(&store, &later), Verdict::Verified);
    assert_eq!(
        store.claim_signature(&later.id).unwrap().unwrap().key,
        store.claim_signature(&first.id).unwrap().unwrap().key,
        "the later claim relies on the same delegated key",
    );
    assert!(store.recheck_claim_verdicts().unwrap().is_empty());
}

#[test]
fn a_compacted_revocation_still_revokes_its_key() {
    let (_directory, store, _anchor) = studio();
    store.set_write_clock_at(100).unwrap();
    let first = note(&store, Some("person/ada"), "first");
    seal(&store);
    let device = store.claim_signature(&first.id).unwrap().unwrap();
    store.set_write_clock_at(120).unwrap();
    let revocation = append(&store, KEY_REVOKED, "person/ada", None, json!({"key": device.key}));
    store.set_write_clock_at(130).unwrap();
    let after = note(&store, Some("person/ada"), "after the revocation");
    seal(&store);
    drain(&store);
    assert_eq!(verdict(&store, &revocation), Verdict::Verified);
    assert_eq!(store.claim_signature(&after.id).unwrap().unwrap().key, device.key);
    let revoked = verdict(&store, &after);
    assert!(matches!(&revoked, Verdict::Invalid(_)), "{revoked:?}");
    assert!(compact(&store) > 0);
    assert_eq!(
        count(
            &store,
            &format!(
                "SELECT COUNT(*) FROM claim_signature_candidates WHERE claim_id='{}'",
                revocation.id
            ),
        ),
        0,
        "the revocation is compacted",
    );
    // Rejudging everything reads the compacted revocation and still revokes the key.
    assert!(store.recheck_claim_verdicts().unwrap().is_empty());
    drain(&store);
    assert_eq!(verdict(&store, &after), revoked);
    assert_eq!(verdict(&store, &first), Verdict::Verified);
}

#[test]
fn a_revocation_that_arrives_later_requeues_compacted_dependents() {
    let (_directory, store, anchor) = studio();
    store.set_write_clock_at(100).unwrap();
    let first = note(&store, Some("person/ada"), "first");
    seal(&store);
    let device = store.claim_signature(&first.id).unwrap().unwrap();
    store.set_write_clock_at(120).unwrap();
    let revoked = append(&store, KEY_REVOKED, "person/ada", None, json!({"key": device.key}));
    let content = content_digest(&revoked.subject, &revoked.kind, revoked.actor.as_deref(), &revoked.body);
    // The revocation first arrives with a broken signature, so it does not count yet.
    let mut broken = ClaimSignature::sign(&anchor, &content, "host/studio", None, vec![], 120);
    broken.signature = "broken-until-the-real-signature-arrives".into();
    attach(&store, &revoked, &broken);
    store.set_write_clock_at(130).unwrap();
    let after = (0..3)
        .map(|i| note(&store, Some("person/ada"), &format!("after {i}")))
        .collect::<Vec<_>>();
    seal(&store);
    drain(&store);
    for claim in &after {
        assert_eq!(verdict(&store, claim), Verdict::Verified);
    }
    assert!(compact(&store) >= after.len() as i64);
    for claim in &after {
        assert_eq!(
            count(
                &store,
                &format!("SELECT COUNT(*) FROM claim_signature_candidates WHERE claim_id='{}'", claim.id),
            ),
            0,
        );
    }
    // The genuine revocation signature arrives: its links requeue the compacted dependents.
    let genuine = ClaimSignature::sign(&anchor, &content, "host/studio", None, vec![], 120);
    attach(&store, &revoked, &genuine);
    drain(&store);
    assert_eq!(verdict(&store, &revoked), Verdict::Verified);
    for claim in &after {
        assert!(matches!(verdict(&store, claim), Verdict::Invalid(_)), "{:?}", verdict(&store, claim));
    }
    assert_eq!(verdict(&store, &first), Verdict::Verified);
    assert!(store.recheck_claim_verdicts().unwrap().is_empty());
}

#[test]
fn a_new_candidate_expands_a_compacted_claim_first() {
    let (_directory, store, _anchor) = studio();
    let claim = note(&store, Some("person/ada"), "carried twice");
    seal(&store);
    drain(&store);
    let genuine = store.claim_signature(&claim.id).unwrap().unwrap();
    assert!(compact(&store) > 0);
    let candidates = |store: &Store| {
        count(
            store,
            &format!("SELECT COUNT(*) FROM claim_signature_candidates WHERE claim_id='{}'", claim.id),
        )
    };
    let counted = |store: &Store| {
        count(
            store,
            &format!("SELECT candidates FROM claim_signature_counts WHERE claim_id='{}'", claim.id),
        )
    };

    // An exact duplicate restores the candidate and adds nothing else.
    attach(&store, &claim, &genuine);
    assert_eq!((candidates(&store), counted(&store)), (1, 1));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_signature_sources"), count(&store, "SELECT COUNT(*) FROM claim_signature_candidates"));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_verdict_fresh"), 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_verdict_queue"), 0);
    assert_eq!(verdict(&store, &claim), Verdict::Verified);

    // A second, untrusted candidate is judged beside the restored one, which keeps winning.
    assert!(compact(&store) > 0);
    let content = content_digest(&claim.subject, &claim.kind, claim.actor.as_deref(), &claim.body);
    let untrusted = ClaimSignature::sign(&key(), &content, "!untrusted", None, vec![], genuine.signed_at_unix_ms);
    attach(&store, &claim, &untrusted);
    assert_eq!((candidates(&store), counted(&store)), (2, 2));
    drain(&store);
    assert_eq!(verdict(&store, &claim), Verdict::Verified);
    assert_eq!(store.claim_signature(&claim.id).unwrap(), Some(genuine));
    assert!(store.recheck_claim_verdicts().unwrap().is_empty());
}

/// A store the writer fenced at 24, rolled back to this reader release: the fence must not turn
/// on the version 21 reference writer, whose conversion needs every facade's candidate row.
#[test]
fn a_store_fenced_at_24_keeps_the_reference_writer_off() {
    let (_directory, store, _anchor) = studio();
    note(&store, None, "a node note");
    seal(&store);
    drain(&store);
    assert!(compact(&store) > 0);
    store.connection.write().pragma_update(None, "user_version", 24).unwrap();
    let before = verdicts(&store);

    let later = note(&store, Some("person/ada"), "written after the rollback");
    seal(&store);
    drain(&store);
    for _ in 0..8 {
        let migration = store.advance_signature_migration().unwrap();
        assert!(migration.fault.is_none(), "{:?}", migration.fault);
    }
    assert!(store.signature_state_faults().unwrap().is_empty());
    assert_eq!(verdict(&store, &later), Verdict::Verified);
    let after = verdicts(&store);
    assert!(before.iter().all(|(claim, verdict)| after.get(claim) == Some(verdict)));
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM claim_signatures WHERE typeof(signature)<>'text'"),
        0,
        "no facade became a reference"
    );
}

fn user_version(store: &Store) -> u32 {
    store.readers.get().query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap()
}

fn writer_flag(store: &Store) -> Option<String> {
    store
        .readers
        .get()
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [smallclaims::store::principals::compaction::COMPACTION_WRITER_FLAG],
            |row| row.get(0),
        )
        .ok()
}

/// Leave every compacted claim as a release before version 24 kept it: its facade as its one
/// candidate, carried by the node, and the store at version 20 without the writer's flag.
fn whole(store: &Store) -> i64 {
    let connection = store.connection.write();
    connection
        .execute_batch(
            "CREATE TEMP TABLE restoring AS
               SELECT s.claim_id, s.signature FROM claim_signatures s JOIN claim_signature_counts USING(claim_id)
                WHERE NOT EXISTS(SELECT 1 FROM claim_signature_candidates d WHERE d.claim_id=s.claim_id);
             DELETE FROM claim_signature_counts WHERE claim_id IN (SELECT claim_id FROM restoring);
             INSERT INTO claim_signature_candidates(claim_id,signature,key,nonce)
               SELECT claim_id, signature, json_extract(signature,'$.key'), json_extract(signature,'$.nonce') FROM restoring;
             INSERT INTO claim_signature_sources(claim_id,signature,writer)
               SELECT claim_id, signature, 'studio' FROM restoring;
             DELETE FROM meta WHERE key LIKE 'claim_signature_compaction%';
             PRAGMA user_version=20;",
        )
        .unwrap();
    let restored = connection.query_row("SELECT COUNT(*) FROM restoring", [], |row| row.get(0)).unwrap();
    connection.execute_batch("DROP TABLE restoring;").unwrap();
    restored
}

fn pace(store: &Store) -> (usize, usize) {
    let (mut pages, mut total) = (0, 0);
    for _ in 0..10_000 {
        let page = store.compact_settled_signatures().unwrap();
        pages += 1;
        total += page.compacted;
        assert!(page.examined <= smallclaims::store::principals::compaction::COMPACTION_PAGE);
        if page.finished {
            return (pages, total);
        }
    }
    panic!("the paced compaction must finish");
}

#[test]
fn the_writer_compacts_each_claim_as_it_settles_and_fences_the_store() {
    let (_directory, store, _anchor) = studio();
    assert_eq!(user_version(&store), 20, "a store with nothing settled stays at 20");
    assert_eq!(writer_flag(&store), None);
    let claims = [note(&store, None, "a node note"), note(&store, Some("person/ada"), "delegated")];
    seal(&store);
    drain(&store);
    let signed = count(&store, "SELECT COUNT(*) FROM claim_signatures");
    assert_eq!(compacted(&store), signed, "judging compacts each claim as it settles");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM claim_signature_sources"), 0);
    assert_eq!(user_version(&store), 24);
    assert_eq!(writer_flag(&store).as_deref(), Some("1"));
    for claim in &claims {
        assert_eq!(verdict(&store, claim), Verdict::Verified);
        assert!(store.claim_signature(&claim.id).unwrap().is_some());
    }
    assert!(store.recheck_claim_verdicts().unwrap().is_empty());
    assert!(store.signature_state_faults().unwrap().is_empty());
}

#[test]
fn the_paced_pass_compacts_what_an_earlier_release_kept_whole() {
    let (_directory, store, _anchor) = studio();
    for index in 0..1_200 {
        note(&store, if index % 2 == 0 { None } else { Some("person/ada") }, &format!("note {index}"));
    }
    seal(&store);
    drain(&store);
    let signed = count(&store, "SELECT COUNT(*) FROM claim_signatures");
    assert_eq!(whole(&store), signed);
    assert_eq!(compacted(&store), 0);
    let before = verdicts(&store);

    let (pages, total) = pace(&store);
    assert!(pages >= 3, "{signed} claims take several short loans, not one ({pages})");
    assert_eq!(total as i64, signed);
    assert_eq!(compacted(&store), signed);
    assert_eq!(user_version(&store), 24);
    assert_eq!(verdicts(&store), before);
    assert!(store.recheck_claim_verdicts().unwrap().is_empty());
    drain(&store);
    assert_eq!(verdicts(&store), before);
    assert_eq!(pace(&store), (1, 0), "once done, a pass is one cheap look");
}

#[test]
fn an_unsettled_claim_stays_whole_until_it_settles() {
    let (_directory, store, _anchor) = studio();
    let waiting = note(&store, None, "queued again");
    let other = note(&store, None, "settled");
    seal(&store);
    drain(&store);
    whole(&store);
    store
        .connection
        .write()
        .execute("INSERT INTO claim_verdict_queue(claim_id) VALUES (?1)", [&waiting.id])
        .unwrap();
    pace(&store);
    let candidates = |claim: &ClaimRecord| {
        store
            .readers
            .get()
            .query_row(
                "SELECT COUNT(*) FROM claim_signature_candidates WHERE claim_id=?1",
                [&claim.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(candidates(&waiting), 1, "a queued claim keeps its candidate");
    assert_eq!(candidates(&other), 0);
    drain(&store);
    assert_eq!(candidates(&waiting), 0, "it compacts when its verdict settles");
    assert_eq!(verdict(&store, &waiting), Verdict::Verified);
}

#[test]
fn a_claim_a_rollback_left_whole_is_found_after_the_pass_is_done() {
    let (_directory, store, _anchor) = studio();
    note(&store, None, "before the rollback");
    seal(&store);
    drain(&store);
    whole(&store);
    pace(&store);
    let signed = count(&store, "SELECT COUNT(*) FROM claim_signatures");
    assert_eq!(compacted(&store), signed);
    // The reader release keeps the fence and the flag but compacts nothing it judges.
    let restored = {
        let connection = store.connection.write();
        connection
            .execute_batch(
                "DELETE FROM claim_signature_counts WHERE claim_id=(SELECT MIN(claim_id) FROM claim_signatures);
                 INSERT INTO claim_signature_candidates(claim_id,signature,key,nonce)
                   SELECT claim_id, signature, json_extract(signature,'$.key'), json_extract(signature,'$.nonce')
                     FROM claim_signatures WHERE claim_id=(SELECT MIN(claim_id) FROM claim_signatures);",
            )
            .unwrap();
        connection.changes()
    };
    assert_eq!(restored, 1);
    assert_eq!(compacted(&store), signed - 1);
    assert_eq!(pace(&store).1, 1);
    assert_eq!(compacted(&store), signed);
}
