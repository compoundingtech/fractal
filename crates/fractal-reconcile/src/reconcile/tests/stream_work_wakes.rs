//! A work wake for a seat on this member is a stream item (4c-3, part b). Its whole life reads
//! as a graph wake's did: the reconciler counts it as an attempt, the step view shows it, its
//! read and close are this member's shard records, and the exhaustion fault names the latest.
use super::*;
use serde_json::json;

const START: u128 = 1_800_000_000_000;

struct Clock;
impl Clock {
    fn at(store: &Store, at: u128) -> Self {
        smallclaims::store::set_thread_clock(Some(at));
        store.set_write_clock_at(at).unwrap();
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

fn fixture() -> (tempfile::TempDir, Clock, SeatQueueFixture) {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "node").unwrap());
    crate::store::messages_shard::open(&store, root.path()).unwrap();
    let clock = Clock::at(&store, START);
    let mut seat = SeatQueueFixture::with_store(store);
    // As the daemon runs it: a pass evaluates only the items whose reads changed or fell due.
    seat.reconciler = seat.reconciler.skipping_unneeded(true);
    (root, clock, seat)
}

fn faults(seat: &SeatQueueFixture) -> Vec<crate::model::ClaimRecord> {
    seat.store
        .claims_for(SEAT, Some("harness.diagnostic"))
        .unwrap()
        .into_iter()
        .filter(|claim| claim.body["fields"]["code"] == "work-wake-exhausted")
        .collect()
}

fn step_wake(seat: &SeatQueueFixture, step: &str) -> crate::model::WorkWakeView {
    seat.store.step_run(step).unwrap().unwrap().wake.expect("the step shows its wake")
}

#[test]
fn a_stream_wake_is_counted_shown_closed_and_named_as_a_graph_wake_was() {
    let (_root, _clock, seat) = fixture();
    let run = seat.start("queued", "stream-wake");
    let selected = SeatQueueFixture::step(&run, "work");
    let wakes = seat.store.messages(Some(SEAT), false).unwrap();
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    let wake = &wakes[0];
    assert!(crate::store::messages_shard::mailbox_reads::is_stream_item(wake), "{wake:?}");
    assert!(seat.store.latest_claim(&wake.subject, Some("message.sent")).unwrap().is_none());
    assert_eq!(work_message_target(wake).map(|(step, ..)| step.to_owned()), Some(selected.clone()));
    let sent = crate::store::messages_shard::mailbox_reads::sent_claim(&seat.store, &wake.subject)
        .unwrap()
        .unwrap();
    let shown = step_wake(&seat, &selected);
    assert_eq!(shown.attempts, 1);
    assert_eq!(shown.last_attempt_at_unix_ms, Some(sent.accepted_at_unix_ms));

    // Inside the retry interval the reconciler counts the stream wake and sends no second one.
    let _clock = Clock::at(&seat.store, START + 1);
    seat.reconciler.reconcile_once().unwrap();
    assert_eq!(seat.store.messages(Some(SEAT), true).unwrap().len(), 1);

    // Past it, the second attempt is the next stream wake, and the step view counts both.
    let _clock = Clock::at(&seat.store, sent.accepted_at_unix_ms + WORK_WAKE_RETRY_MS);
    seat.reconciler.reconcile_once().unwrap();
    let wakes = seat.store.messages(Some(SEAT), true).unwrap();
    assert_eq!(wakes.len(), 2, "{wakes:?}");
    assert!(wakes.iter().all(crate::store::messages_shard::mailbox_reads::is_stream_item));
    assert_eq!(step_wake(&seat, &selected).attempts, 2);

    // The seat takes both; the exhaustion fault cites no graph claim for them and names them.
    for wake in &wakes {
        for (kind, status) in [("message.delivered", "delivered"), ("message.read", "read")] {
            crate::store::messages_shard::record_lifecycle(
                &seat.store,
                &wake.subject,
                kind,
                Some(SEAT),
                &BTreeMap::from([("status".into(), json!(status))]),
            )
            .unwrap();
        }
    }
    let last = wakes
        .iter()
        .map(|wake| {
            crate::store::messages_shard::mailbox_reads::sent_claim(&seat.store, &wake.subject)
                .unwrap()
                .unwrap()
                .accepted_at_unix_ms
        })
        .max()
        .unwrap();
    let _clock = Clock::at(&seat.store, last + 1);
    seat.store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "harness.observed".into(),
            actor: Some(SEAT.into()),
            fields: BTreeMap::from([
                ("state".into(), json!("idle")),
                ("driver".into(), json!("codex")),
                ("incarnation_id".into(), json!("seat-one")),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("seat-one-idle".into()),
        })
        .unwrap();
    seat.reconciler.reconcile_once().unwrap();
    assert!(faults(&seat).is_empty(), "inside the grace the consumed wakes hold");
    let _clock = Clock::at(&seat.store, last + WORK_WAKE_EXHAUST_GRACE_MS);
    seat.reconciler.reconcile_once().unwrap();
    let found = faults(&seat);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].body["fields"]["step_run"], selected);
    assert_eq!(found[0].body["evidence"], json!([]));
    let latest = wakes
        .iter()
        .max_by_key(|wake| wake.created_index)
        .map(|wake| wake.subject.clone())
        .unwrap();
    assert_eq!(found[0].body["fields"]["message_id"], json!(latest));
}

#[test]
fn a_claimed_steps_stream_wake_is_closed_in_the_shard() {
    let (_root, _clock, seat) = fixture();
    let run = seat.start("queued", "stream-close");
    let selected = SeatQueueFixture::step(&run, "work");
    let wake = seat.store.messages(Some(SEAT), false).unwrap().remove(0);
    assert!(crate::store::messages_shard::mailbox_reads::is_stream_item(&wake));
    crate::store::messages_shard::record_lifecycle(
        &seat.store,
        &wake.subject,
        "message.delivered",
        Some(SEAT),
        &BTreeMap::from([("status".into(), json!("delivered"))]),
    )
    .unwrap();
    seat.work(&selected, "claim", "stream-claim").unwrap();
    let _clock = Clock::at(&seat.store, START + 1);
    seat.reconciler.reconcile_once().unwrap();
    let closed = seat.store.message(&wake.subject).unwrap().unwrap();
    assert_eq!(closed.status, "closed", "{closed:?}");
    // The reader took the read for the seat and the close as it; no graph claim was written.
    let read = crate::store::messages_shard::mailbox_reads::lifecycle_step(&seat.store, &wake.subject, "message.read")
        .unwrap();
    assert_eq!(read.map(|(actor, _)| actor).as_deref(), Some(SEAT));
    assert!(seat.store.claims_for(&wake.subject, None).unwrap().is_empty());
    // A closed wake still counts as an attempt.
    assert_eq!(seat.store.work_wake_messages_for_reconcile(SEAT).unwrap().len(), 1);
}

