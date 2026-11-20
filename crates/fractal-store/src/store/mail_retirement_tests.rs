//! Closed agent and daemon mail retires a week after it closes; person mail and cited mail stay.
use super::*;
use crate::store::mail_retirement::{CURSOR, RETAIN_AFTER_CLOSE_MS};
use crate::store::tests_shared_seats::TEST_FLEET;
use std::collections::BTreeSet;
use std::time::Duration;

const SENDER: &str = "agent/node.alder";
const RECIPIENT: &str = "agent/node.birch";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn member(path: &std::path::Path, trims_locally: bool) -> Store {
    let store = Store::open(path, "node").unwrap();
    store.bind_fleet(TEST_FLEET).unwrap();
    store.import_retired_identities_paced(|_| {}).unwrap();
    store.trim_locally(trims_locally).unwrap();
    store
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

fn sent(subject: &str, from: &str, to: &str, evidence: Vec<String>) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some(from.into()),
        fields: serde_json::from_value(json!({
            "from": from, "to": to, "content": format!("about {subject}"), "status": "sent"
        }))
        .unwrap(),
        evidence,
        expected_subject: None,
        idempotency_key: None,
    }
}

fn lifecycle(subject: &str, kind: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some(RECIPIENT.into()),
        fields: BTreeMap::from([("status".into(), json!(kind.trim_start_matches("message.")))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }
}

/// A thread from `from` to `to`, read and, when `closed`, closed. Returns its claim IDs.
fn thread(store: &Store, subject: &str, from: &str, to: &str, closed: bool) -> Vec<String> {
    let mut ids = vec![store.append_claim(&sent(subject, from, to, Vec::new())).unwrap().id];
    let mut kinds = vec!["message.delivered", "message.read"];
    if closed {
        kinds.push("message.closed");
    }
    for kind in kinds {
        ids.push(store.append_claim(&lifecycle(subject, kind)).unwrap().id);
    }
    ids
}

fn strings(statement: &mut rusqlite::Statement<'_>) -> BTreeSet<String> {
    statement.query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn claims(store: &Store) -> BTreeSet<String> {
    strings(&mut store.readers.get().prepare("SELECT id FROM claims").unwrap())
}

fn retired(store: &Store) -> BTreeSet<String> {
    strings(&mut store.readers.get()
        .prepare("SELECT CASE WHEN typeof(id)='blob' THEN lower(hex(id)) ELSE id END FROM retired_claim_ids")
        .unwrap())
}

fn indexed(store: &Store) -> BTreeSet<String> {
    strings(&mut store.readers.get().prepare("SELECT subject FROM message_index").unwrap())
}

fn retire_all(store: &Store, now: u64) -> (usize, usize) {
    let (mut threads, mut claims) = (0, 0);
    loop {
        let step = store.retire_closed_mail_step(now).unwrap();
        threads += step.threads;
        claims += step.claims;
        if !step.more {
            return (threads, claims);
        }
    }
}

#[test]
fn closed_agent_mail_retires_a_week_after_it_closes_and_the_rest_stays() {
    let dir = tempfile::tempdir().unwrap();
    let store = member(&dir.path().join("claims.sqlite3"), true);
    let agents = thread(&store, "message/agents", SENDER, RECIPIENT, true);
    let daemon = thread(&store, "message/daemon", "daemon/runtime", RECIPIENT, true);
    let to_person = thread(&store, "message/to-person", SENDER, "person/avery", true);
    let from_person = thread(&store, "message/from-person", "person/avery", RECIPIENT, true);
    let open = thread(&store, "message/open", SENDER, RECIPIENT, false);
    let before = claims(&store);

    // Six days after close nothing is old enough.
    assert_eq!(retire_all(&store, now_ms() + 6 * DAY_MS), (0, 0));
    assert_eq!(claims(&store), before);

    let durability = |store: &Store| -> (i64, i64) {
        let writer = store.connection.write();
        (
            writer.query_row("PRAGMA synchronous", [], |row| row.get(0)).unwrap(),
            writer.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0)).unwrap(),
        )
    };
    let ordinary = durability(&store);
    let (threads, deleted) = retire_all(&store, now_ms() + RETAIN_AFTER_CLOSE_MS + DAY_MS);
    assert_eq!((threads, deleted), (2, agents.len() + daemon.len()));
    // Retirement commits without waiting on the disk; the next message send commits as before.
    assert_eq!(durability(&store), ordinary);
    let gone: BTreeSet<String> = agents.iter().chain(&daemon).cloned().collect();
    assert_eq!(retired(&store), gone, "each deleted claim is retired, and nothing else");
    let kept: BTreeSet<String> = to_person.iter().chain(&from_person).chain(&open).cloned().collect();
    assert_eq!(claims(&store), before.difference(&gone).cloned().collect::<BTreeSet<_>>());
    assert!(kept.is_subset(&claims(&store)), "person threads and open mail stay");
    // Retained and retired together are still the set this member held: replication compares both.
    assert_eq!(claims(&store).union(&retired(&store)).cloned().collect::<BTreeSet<_>>(), before);
    // The projections a deleted claim fed forget it.
    let indexed = indexed(&store);
    assert!(!indexed.contains("message/agents") && !indexed.contains("message/daemon"), "{indexed:?}");
    assert!(indexed.contains("message/to-person") && indexed.contains("message/open"));
    store.append_claim(&sent("message/after", SENDER, RECIPIENT, Vec::new())).unwrap();
    assert_eq!(durability(&store), ordinary);
}

#[test]
fn a_thread_any_other_claim_cites_stays() {
    let dir = tempfile::tempdir().unwrap();
    let store = member(&dir.path().join("claims.sqlite3"), true);
    let cited = thread(&store, "message/cited", SENDER, RECIPIENT, true);
    // A later message cites the first thread's sent claim as its evidence.
    store.append_claim(&sent("message/citer", SENDER, "person/avery", vec![cited[0].clone()])).unwrap();
    retire_all(&store, now_ms() + RETAIN_AFTER_CLOSE_MS + DAY_MS);
    assert!(cited.iter().all(|id| claims(&store).contains(id)), "a cited thread stays");
    assert!(retired(&store).is_empty());
}

#[test]
fn only_a_member_whose_retired_ledger_is_the_authority_retires() {
    let dir = tempfile::tempdir().unwrap();
    let store = member(&dir.path().join("claims.sqlite3"), false);
    thread(&store, "message/agents", SENDER, RECIPIENT, true);
    let before = claims(&store);
    let step = store.retire_closed_mail_step(now_ms() + 30 * DAY_MS).unwrap();
    assert_eq!(step, Default::default());
    assert_eq!(claims(&store), before);
}

#[test]
fn a_step_stops_at_its_budget_and_the_next_finishes_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let store = member(&dir.path().join("claims.sqlite3"), true);
    let first = thread(&store, "message/first", SENDER, RECIPIENT, true);
    let second = thread(&store, "message/second", SENDER, RECIPIENT, true);
    let later = now_ms() + RETAIN_AFTER_CLOSE_MS + DAY_MS;
    // A spent budget still deletes one claim a hold; the cursor stays before the cut thread.
    let step = store.retire_closed_mail_step_within(later, Duration::ZERO).unwrap();
    assert_eq!((step.threads, step.claims, step.more), (0, 1, true));
    assert_eq!(retired(&store), BTreeSet::from([first[1].clone()]), "the delivered claim goes first");
    let cursor = |store: &Store| -> Option<String> {
        store.readers.get().query_row("SELECT value FROM meta WHERE key=?1", [CURSOR], |row| row.get(0)).optional().unwrap()
    };
    assert_eq!(cursor(&store).as_deref(), Some("0"), "the cursor waits before the cut thread");
    // A store reopened (a restart) resumes from the committed cursor and finishes both.
    drop(store);
    let store = member(&dir.path().join("claims.sqlite3"), true);
    assert_eq!(retire_all(&store, later), (2, first.len() + second.len() - 1));
    assert_ne!(cursor(&store).as_deref(), Some("0"));
    assert_eq!(retired(&store), first.iter().chain(&second).cloned().collect());
}

#[test]
fn a_thread_the_budget_cuts_keeps_its_closed_claim_until_the_rest_goes() {
    let dir = tempfile::tempdir().unwrap();
    let store = member(&dir.path().join("claims.sqlite3"), true);
    let ids = thread(&store, "message/cut", SENDER, RECIPIENT, true);
    let closed = ids.last().unwrap().clone();
    let later = now_ms() + RETAIN_AFTER_CLOSE_MS + DAY_MS;
    // Delete one claim at a time: each hold ends after its first delete.
    let mut steps = 0;
    loop {
        let step = store.retire_closed_mail_step_within(later, Duration::ZERO).unwrap();
        steps += 1;
        if claims(&store).contains(&closed) {
            assert!(step.more, "a thread cut part way is not left behind");
        }
        if !step.more {
            break;
        }
        assert!(steps < 20);
    }
    assert!(!claims(&store).contains(&closed));
    assert_eq!(retired(&store), ids.into_iter().collect());
}

#[test]
fn a_thread_closed_before_its_sent_claim_arrives_retires_once_it_does() {
    let dir = tempfile::tempdir().unwrap();
    let store = member(&dir.path().join("claims.sqlite3"), true);
    let ids = thread(&store, "message/late", SENDER, RECIPIENT, true);
    // A member that received the lifecycle ahead of the sent claim: hold the sent row back.
    {
        let writer = store.connection.write();
        writer.execute_batch(/* sqlite-large-value: allow-for-now lv-h83c3f2a081d3 owner=fractal_store::store::mail_retirement_tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE held AS SELECT * FROM claims WHERE subject='message/late' AND kind='message.sent'").unwrap();
        writer.execute(/* sqlite-large-value: allow-for-now lv-hd1bf57d459d7 owner=fractal_store::store::mail_retirement_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&ids[0]]).unwrap();
    }
    let later = now_ms() + RETAIN_AFTER_CLOSE_MS + DAY_MS;
    assert_eq!(retire_all(&store, later), (0, 0), "its people are unknown yet");
    let deferred: String = store
        .readers
        .get()
        .query_row("SELECT value FROM meta WHERE key=?1", [crate::store::mail_retirement::DEFERRED], |row| row.get(0))
        .unwrap();
    assert_eq!(deferred, r#"["message/late"]"#);
    // The sent claim arrives after the cursor passed the closed claim.
    store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h7740f7cc9409 owner=fractal_store::store::mail_retirement_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims SELECT * FROM held; DROP TABLE held").unwrap();
    // The deferred thread is judged again and retires, as on a member that stored the claims in
    // sending order.
    assert_eq!(retire_all(&store, later), (1, 4));
    assert_eq!(retired(&store), ids.into_iter().collect());
}

#[test]
fn a_spent_budget_stops_before_the_next_sent_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = member(&dir.path().join("claims.sqlite3"), true);
    // Threads of only a sent and a closed claim, as a member holds when the rest went to the
    // stream: the sent claim comes first in deletion order. Appending refuses sent to closed, so
    // drop the delivered and read rows afterwards.
    let short = |subject: &str| {
        let ids = thread(&store, subject, SENDER, RECIPIENT, true);
        for id in &ids[1..3] {
            store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-had322b3de5b9 owner=fractal_store::store::mail_retirement_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [id]).unwrap();
        }
        vec![ids[0].clone(), ids[3].clone()]
    };
    let first = short("message/short-1");
    let second = short("message/short-2");
    let later = now_ms() + RETAIN_AFTER_CLOSE_MS + DAY_MS;
    let step = store.retire_closed_mail_step_within(later, Duration::ZERO).unwrap();
    assert_eq!((step.threads, step.claims, step.more), (1, 2, true), "one thread a spent hold, not every thread judged");
    assert_eq!(retired(&store), first.iter().cloned().collect());
    assert_eq!(retire_all(&store, later), (1, 2));
    assert_eq!(retired(&store), first.iter().chain(&second).cloned().collect());
}
