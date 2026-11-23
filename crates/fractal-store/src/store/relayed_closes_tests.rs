//! A seat's host's relayed close counts in client selectors and nowhere else. Durable, bounded
//! admission retains terminal evidence until finalization commits, including across restart.
use super::relayed_closes::{self, BATCH, QUEUED, RelayedClose};
use super::*;
use std::time::Duration;

const SEAT: &str = "agent/cedar";

fn sent(store: &Store, subject: &str, version: u64) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "message.sent".into(),
            actor: Some("agent/sender".into()),
            fields: BTreeMap::from([
                ("from".into(), json!("agent/sender")),
                ("to".into(), json!(SEAT)),
                ("content".into(), json!(format!("about {subject}"))),
                ("status".into(), json!("sent")),
                ("tags".into(), json!(["reminder:standup", format!("version:{version}")])),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn close(subject: &str) -> RelayedClose {
    RelayedClose {
        message: subject.into(),
        host: "seat-host".into(),
        receipt: format!("offset-receipt/{}", subject.trim_start_matches("message/")),
        closed_at_unix_ms: 1_800_000_000_000,
    }
}

/// A reminder group of two: `message/r1` (version 1) and its newer `message/r2` (version 2).
fn reminders() -> Arc<Store> {
    let store = Arc::new(Store::open_memory("relayed-closes").unwrap());
    sent(&store, "message/r1", 1);
    sent(&store, "message/r2", 2);
    while store.maintain_client_message_selectors().unwrap() {}
    store
}

/// Each row of the selector page: its subject, whether it is current, its `closed`, and its
/// revision.
fn page(store: &Store, history: bool) -> Vec<(String, bool, bool, String)> {
    store
        .read_snapshot(|through| {
            Ok(store
                .client_messages_page(None, None, history, through, None, 20)?
                .into_iter()
                .map(|(message, metadata, current)| {
                    (
                        message.subject,
                        current,
                        metadata["closed"] == true,
                        metadata["revision"].as_str().unwrap().to_owned(),
                    )
                })
                .collect())
        })
        .unwrap()
}

fn current(store: &Store) -> Vec<String> {
    page(store, false).into_iter().map(|(subject, ..)| subject).collect()
}

#[test]
fn a_relayed_close_of_a_reminders_winner_makes_the_next_one_current() {
    let store = reminders();
    assert_eq!(current(&store), ["message/r2"], "the newer reminder wins");
    let before = page(&store, true);

    assert_eq!(relayed_closes::record(&store, &[close("message/r2")]).unwrap(), 1);
    assert_eq!(current(&store), ["message/r1"], "the host closed the winner, so the older one is current");
    let history = page(&store, true);
    let row = |rows: &[(String, bool, bool, String)], subject: &str| {
        rows.iter().find(|row| row.0 == subject).cloned().unwrap()
    };
    let closed = row(&history, "message/r2");
    assert!(!closed.1 && closed.2, "the closed winner is history, and closed: {history:?}");
    assert_eq!(closed.3, row(&before, "message/r2").3, "a relayed close never feeds a revision");
    assert_eq!(row(&history, "message/r1").3, row(&before, "message/r1").3);
    // The message's own status is this member's records: the host's close is not a view's.
    assert_eq!(store.message("message/r2").unwrap().unwrap().status, "sent");
}

#[test]
fn a_close_is_recorded_once_and_never_for_mail_this_member_closed_or_does_not_hold() {
    let store = reminders();
    assert_eq!(relayed_closes::record(&store, &[close("message/r2"), close("message/r2")]).unwrap(), 1);
    assert_eq!(relayed_closes::record(&store, &[close("message/r2")]).unwrap(), 0);
    assert_eq!(relayed_closes::record(&store, &[close("message/unknown")]).unwrap(), 0);
    message_marks::record(&store, "message/r1", "message.closed", SEAT).unwrap();
    assert_eq!(relayed_closes::record(&store, &[close("message/r1")]).unwrap(), 0, "closed here already");
    assert!(relayed_closes::closed(&store.readers.get(), "message/r2").unwrap());
    assert!(!relayed_closes::closed(&store.readers.get(), "message/r1").unwrap());
}

#[test]
fn trimming_removes_the_closes_this_members_own_records_cover_and_keeps_the_header_closed() {
    let store = reminders();
    relayed_closes::record(&store, &[close("message/r1"), close("message/r2")]).unwrap();
    assert_eq!(relayed_closes::trim(&store, 1).unwrap(), 0, "nothing covers them yet");
    for kind in ["message.delivered", "message.read", "message.closed"] {
        store
            .append_claim(&ClaimInput {
                subject: "message/r2".into(),
                kind: kind.into(),
                actor: Some(SEAT.into()),
                fields: BTreeMap::from([("status".into(), json!(kind.trim_start_matches("message.")))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    assert_eq!(relayed_closes::trim(&store, 1).unwrap(), 1);
    assert!(!relayed_closes::closed(&store.readers.get(), "message/r2").unwrap());
    assert!(relayed_closes::closed(&store.readers.get(), "message/r1").unwrap());
    assert!(current(&store).is_empty(), "both are closed, one by its claim and one by its host");
}

#[test]
fn a_relayed_close_folds_into_a_rebuilt_selector() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    {
        let store = Store::open(&path, "relayed-closes").unwrap();
        sent(&store, "message/r1", 1);
        sent(&store, "message/r2", 2);
        while store.maintain_client_message_selectors().unwrap() {}
        relayed_closes::record(&store, &[close("message/r2")]).unwrap();
    }
    let reopened = Store::open(&path, "relayed-closes").unwrap();
    while reopened.maintain_client_message_selectors().unwrap() {}
    assert_eq!(current(&reopened), ["message/r1"]);
}

/// Admit without starting a runtime writer, making the pending/finalized boundary deterministic.
fn admit(store: &Arc<Store>, closes: Vec<RelayedClose>) {
    std::thread::scope(|scope| {
        scope.spawn(|| relayed_closes::queue(store, closes).unwrap()).join().unwrap();
    });
}

fn fail_finalization(store: &Store) {
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h4ab0ae4d51d7 owner=fractal_store::store::relayed_closes_tests TODO=bound-or-reference-values-and-reclamation */
        "CREATE TRIGGER reject_relayed_close BEFORE INSERT ON local_relayed_message_closes
         BEGIN SELECT RAISE(ABORT, 'forced close writer failure'); END;",
    ).unwrap();
}

fn recover_finalization(store: &Store) {
    store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h41634bdd2967 owner=fractal_store::store::relayed_closes_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER reject_relayed_close;").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn admitted_close_survives_writer_failure_until_the_source_is_repaired() {
    let store = reminders();
    fail_finalization(&store);
    admit(&store, vec![close("message/r2"), close("message/unknown")]);
    assert_eq!(relayed_closes::queued(&store), 1, "unknown mail is not admitted");
    assert!(!relayed_closes::idle(&store));

    for _ in 0..2 {
        let error = tokio::time::timeout(Duration::from_secs(5), relayed_closes::wait_idle(&store))
            .await.expect("persistent local failure must not hang shutdown").unwrap_err();
        assert!(format!("{error:#}").contains("forced close writer failure"));
        assert_eq!(relayed_closes::queued(&store), 1, "failed transaction retains evidence");
        assert!(!relayed_closes::idle(&store), "failed pending work is not settled");
        assert!(!relayed_closes::closed(&store.readers.get(), "message/r2").unwrap());
        assert_eq!(current(&store), ["message/r2"], "failed finalization rolls selectors back");
    }

    // Repair the real SQLite source, then explicitly recover without another relay/discovery event.
    recover_finalization(&store);
    relayed_closes::wait_idle(&store).await.unwrap();
    assert!(relayed_closes::idle(&store));
    assert_eq!(relayed_closes::queued(&store), 0);
    assert!(relayed_closes::closed(&store.readers.get(), "message/r2").unwrap());
    assert_eq!(current(&store), ["message/r1"]);
    relayed_closes::queue(&store, vec![close("message/r2")]).unwrap();
    relayed_closes::wait_idle(&store).await.unwrap();
    assert_eq!(relayed_closes::queued(&store), 0, "already recorded mail is not readmitted");
}

#[test]
fn failed_admitted_close_is_recovered_after_daemon_restart_without_rediscovery() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    {
        let store = Arc::new(Store::open(&path, "relayed-closes").unwrap());
        sent(&store, "message/r1", 1);
        sent(&store, "message/r2", 2);
        while store.maintain_client_message_selectors().unwrap() {}
        fail_finalization(&store);
        relayed_closes::queue(&store, vec![close("message/r2")]).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert!(runtime.block_on(relayed_closes::wait_idle(&store)).is_err());
        assert_eq!(relayed_closes::queued(&store), 1);
        // Dropping the runtime joins the blocking writer; no old process state can recover it.
        drop(runtime);
    }
    let reopened = Arc::new(Store::open(&path, "relayed-closes").unwrap());
    assert_eq!(relayed_closes::queued(&reopened), 1);
    assert!(!relayed_closes::idle(&reopened));
    recover_finalization(&reopened);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(relayed_closes::wait_idle(&reopened)).unwrap();
    assert!(relayed_closes::closed(&reopened.readers.get(), "message/r2").unwrap());
    assert_eq!(relayed_closes::queued(&reopened), 0);
    assert_eq!(current(&reopened), ["message/r1"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_rejects_overflow_and_completion_admits_the_next_page() {
    let store = Arc::new(Store::open_memory("relayed-closes").unwrap());
    let subjects: Vec<String> = (0..QUEUED + BATCH + 10).map(|index| format!("message/held-{index:04}")).collect();
    for (version, subject) in subjects.iter().enumerate() {
        sent(&store, subject, version as u64);
    }
    assert!(relayed_closes::queue(&store, subjects.iter().map(|subject| close(subject)).collect()).is_err());
    assert_eq!(relayed_closes::queued(&store), 0, "overflow admission is atomic");
    let mut admitted = 0;
    for page in subjects.chunks(QUEUED) {
        relayed_closes::queue(&store, page.iter().map(|subject| close(subject)).collect()).unwrap();
        relayed_closes::wait_idle(&store).await.unwrap();
        admitted += page.len();
        let recorded = subjects.iter().filter(|subject| relayed_closes::closed(&store.readers.get(), subject).unwrap()).count();
        assert_eq!(recorded, admitted);
        assert_eq!(relayed_closes::queued(&store), 0);
    }
}

