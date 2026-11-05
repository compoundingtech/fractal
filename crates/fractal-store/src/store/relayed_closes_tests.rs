//! A seat's host's relayed close counts in the client selectors as a close made here does, and
//! nowhere else; a read queues it without waiting for the writer, and the queue stays bounded.
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

/// Hold `store`'s writer on a thread until the returned sender is sent to, or dropped.
fn hold_writer(store: &Arc<Store>) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (held, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
    let holder = {
        let store = store.clone();
        let (held, release) = (held.0, release.1);
        std::thread::spawn(move || {
            let _writer = store.connection.write();
            held.send(()).unwrap();
            let _ = release.recv();
        })
    };
    held.1.recv().unwrap();
    (release.0, holder)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_queues_a_close_without_waiting_for_a_held_writer() {
    let store = reminders();
    let (release, holder) = hold_writer(&store);

    let started = std::time::Instant::now();
    relayed_closes::queue(&store, vec![close("message/r2"), close("message/unknown")]).unwrap();
    assert!(started.elapsed() < Duration::from_millis(500), "a read waited for the writer: {:?}", started.elapsed());
    assert_eq!(relayed_closes::queued(&store), 1, "mail this member does not hold is never queued");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(current(&store), ["message/r2"], "nothing recorded while the writer is held");

    release.send(()).unwrap();
    holder.join().unwrap();
    idle(&store).await;
    assert!(relayed_closes::closed(&store.readers.get(), "message/r2").unwrap(), "the writer recorded the close");
    assert_eq!(current(&store), ["message/r1"]);

    // A close already recorded is not queued again.
    relayed_closes::queue(&store, vec![close("message/r2")]).unwrap();
    assert_eq!(relayed_closes::queued(&store), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_queue_keeps_at_most_its_bound_and_drops_the_rest_for_a_later_read() {
    let store = Arc::new(Store::open_memory("relayed-closes").unwrap());
    let subjects: Vec<String> = (0..QUEUED + BATCH + 10).map(|index| format!("message/held-{index:04}")).collect();
    for (version, subject) in subjects.iter().enumerate() {
        sent(&store, subject, version as u64);
    }
    let (release, holder) = hold_writer(&store);

    let started = std::time::Instant::now();
    relayed_closes::queue(&store, subjects.iter().map(|subject| close(subject)).collect()).unwrap();
    assert!(started.elapsed() < Duration::from_millis(500), "a read waited for the writer: {:?}", started.elapsed());
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The writer may already hold one batch it took before it reached the held lock.
    let queued = relayed_closes::queued(&store);
    assert!(queued <= QUEUED, "the queue grew past its bound: {queued}");
    assert!(queued >= QUEUED - BATCH, "the queue filled to its bound: {queued}");

    release.send(()).unwrap();
    holder.join().unwrap();
    idle(&store).await;
    let recorded = |store: &Store| {
        subjects.iter().filter(|subject| relayed_closes::closed(&store.readers.get(), subject).unwrap()).count()
    };
    assert_eq!(recorded(&store), QUEUED, "only what the queue kept was recorded");

    // A later read queues the dropped ones again.
    relayed_closes::queue(&store, subjects.iter().map(|subject| close(subject)).collect()).unwrap();
    let queued = relayed_closes::queued(&store);
    assert!(queued <= subjects.len() - QUEUED, "only the dropped ones are queued again: {queued}");
    idle(&store).await;
    assert_eq!(recorded(&store), subjects.len());
}

/// Wait until the queue is empty and the writer has committed the last batch it took: an empty
/// queue alone can still have a batch in flight.
async fn idle(store: &Store) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !relayed_closes::idle(store) {
        assert!(std::time::Instant::now() < deadline, "the writer never went idle");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
