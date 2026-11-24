#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::{AGENT, LIMITS_HOUR, dropped, input, limits_history, limits_reading, rollup};
use super::checkpoint::*;
use super::*;
use proptest::prelude::*;


#[test]
fn a_write_queued_during_sealing_runs_before_the_backlog_finishes() {
    use crate::fleet::{MemberKey, envelope_signature_message, verify_signature};
    use smallclaims::sqlite::WriterJob;
    use std::sync::atomic::Ordering;
    pub const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
    // Sealing prepares the next chunk on a read connection while the queued write runs. A
    // shared-memory store answers that contention with SQLITE_LOCKED, so this uses a file.
    let scratch = tempfile::tempdir().unwrap();
    let store = Store::open(&scratch.path().join("claims.sqlite3"), "seal-queue-node").unwrap();
    store.bind_fleet(FLEET).unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    store.set_member_key(Some(key.clone())).unwrap();
    store.seal_local_batches().unwrap();
    for n in 0..200 {
        store
            .append_claim(&input(
                "agent/seal-queue",
                "harness.observed",
                Some("agent/seal-queue"),
                json!({"state":"idle", "incarnation_id":format!("seal-{n}")}),
                &format!("seal-{n}"),
            ))
            .unwrap();
    }
    let target = smallclaims::store::max_batch_rowid(&store.readers.get()).unwrap();
    let jobs = store
        .connection
        .jobs
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    let (sample, sampled) = std::sync::mpsc::channel();
    let (done, committed) = std::sync::mpsc::sync_channel(1);
    // Queue a real write after the first sealing chunk commits, while that writer loan is
    // still held. The writer owns the native trace hook now and clears any raw profile
    // registration when a loan installs it, so the store's committed-write observer — the
    // subscription the writer itself publishes at each loan boundary — is the hook.
    // FIFO order makes the observation deterministic, without sleeping.
    static QUEUED_WRITE: std::sync::Mutex<Option<(std::sync::mpsc::Sender<WriterJob>, WriterJob)>> =
        std::sync::Mutex::new(None);
    fn after_chunk_commit(_: &Connection) {
        if let Some((jobs, job)) = QUEUED_WRITE.lock().unwrap().take() {
            jobs.send(job).unwrap();
        }
    }
    *QUEUED_WRITE.lock().unwrap() = Some((
        jobs,
        WriterJob::Batched {
            run: Box::new(move |tx| {
                let count: i64 = tx
                    .query_row("SELECT COUNT(*) FROM replica_envelopes", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                sample.send(count).unwrap();
                append_claim_tx(
                    tx,
                    "seal-queue-node",
                    "agent/queued-write",
                    "harness.observed",
                    Some("agent/queued-write"),
                    &json!({"fields":{"state":"idle", "incarnation_id":"queued-i1"}}),
                    &[],
                    None,
                )
                .unwrap();
                true
            }),
            profile: None,
            wait: None,
            holder: smallclaims::windows::holds::Holder::current(),
            enqueued: std::time::Instant::now(),
            done: done.clone(),
        },
    ));
    let _queued_write_observer = store.connection.observe_tables(&["replica_envelopes"], after_chunk_commit);
    store.seal_local_batches().unwrap();
    drop(_queued_write_observer);
    committed.recv().unwrap().unwrap();
    let count = sampled.recv().unwrap();
    assert!(
        count > 0 && count < 200,
        "the queued write saw {count} envelopes: it must run between seal chunks"
    );
    assert_eq!(
        store.seeded_batch_rowid.load(Ordering::Acquire),
        target,
        "a concurrent write is left for the next pass, not skipped by the high water"
    );
    store.seal_local_batches().unwrap();
    assert_eq!(store.seeded_batch_rowid.load(Ordering::Acquire), target + 1);
    let exchange = store
        .export_replication_exchange(FLEET, &ReplicationInventory::default())
        .unwrap();
    assert!(exchange.envelopes.len() >= 201);
    for envelope in &exchange.envelopes {
        assert!(verify_signature(
            key.public(),
            &envelope_signature_message(
                FLEET,
                "seal-queue-node",
                envelope.sequence,
                &envelope.hash
            ),
            envelope.signature.as_deref().unwrap()
        ));
    }
}
