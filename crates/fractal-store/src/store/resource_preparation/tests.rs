#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation::tests_shared_seats::{ResetHook, fixture, fixture_for};
use super::*;

#[test]
fn a_second_connection_commit_is_detected_by_the_same_writer_data_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite");
    let store = Store::open(&path, "prepared-other-writer").unwrap();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h3817e732295a owner=fractal_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let expected = prepared.cut.as_ref().unwrap();
    let other = Connection::open(&path).unwrap();
    other
        .execute(
            /* sqlite-large-value: allow-for-now lv-h952b22a9cf9b owner=fractal_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-external-write','1')",
            [],
        )
        .unwrap();
    let writer = store.connection.write();
    let actual = NativeCut::read(&writer).unwrap();
    assert_eq!(actual.connection, expected.connection);
    assert_eq!(
        actual.changes, expected.changes,
        "external writes do not change this connection's total_changes"
    );
    assert_ne!(
        actual.data, expected.data,
        "compare data_version on the SAME writer handle"
    );
    assert!(!fence.current(&store, &writer, &prepared));
}

// Use the public client admission and the same local AUTOINCREMENT publication as log_diet.
// Warm up its actor key before pinning so a key-creation failure cannot imitate the lock.
fn unrelated_timeline(store: &Store, sequence: u64) -> Result<(ClaimRecord, bool), St3Error> {
    let subject = "agent/invented.sequence-writer";
    store.append_client_claim_outcome(&ClaimInput {
        subject: subject.into(),
        kind: "harness.timeline".into(),
        actor: Some(subject.into()),
        fields: BTreeMap::from([
            ("operation".into(), json!("append")),
            ("entry_id".into(), json!(format!("entry-{sequence}"))),
            ("sequence".into(), json!(sequence)),
            ("revision".into(), json!(1)),
            ("role".into(), json!("assistant")),
            ("entry_type".into(), json!("content")),
            ("final".into(), json!(true)),
            (
                "body".into(),
                json!({"media_type":"text/plain", "text":"fixture"}),
            ),
            ("driver".into(), json!("codex")),
            ("incarnation_id".into(), json!("invented-incarnation")),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(format!("sequence-writer:{sequence}")),
    })
}

fn concurrent_timeline(store: &Store, sequence: u64) -> Result<(ClaimRecord, bool), St3Error> {
    // A separate thread exercises an unrelated caller, not a write through the pinned reader.
    // Join before returning; no task or guard escapes on the error path.
    std::thread::scope(|scope| {
        scope
            .spawn(|| unrelated_timeline(store, sequence))
            .join()
            .unwrap()
    })
}

#[test]
fn a_shared_cache_frontier_blocks_an_unrelated_sequence_publication_but_wal_does_not() {
    let memory = Store::open_memory("sequence-lock").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let file = Store::open(&directory.path().join("claims.sqlite3"), "sequence-lock").unwrap();
    for store in [&memory, &file] {
        assert!(unrelated_timeline(store, 1).unwrap().1);
        // This is the original read_snapshot frontier, before any resource input is read.
        // The worker finishes while the snapshot is held, on both modes, without sleeps.
        let result = store
            .read_snapshot(|_| Ok(concurrent_timeline(store, 2)))
            .unwrap();
        if store.shared_memory {
            let error = result.unwrap_err();
            assert_eq!(error.code, "database-locked", "{error:?}");
            assert!(error.message.contains("sqlite_sequence"), "{error:?}");
            // The same admitted publication succeeds once PinnedRead has ended.
            assert!(unrelated_timeline(store, 2).unwrap().1);
        } else {
            assert!(result.unwrap().1);
        }
        assert!(
            !unrelated_timeline(store, 2).unwrap().1,
            "publication stays idempotent"
        );
    }
}

#[test]
fn preparation_never_pins_shared_cache_before_an_unrelated_publication() {
    let memory = Store::open_memory("preparation-lock").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let file = Store::open(&directory.path().join("claims.sqlite3"), "preparation-lock").unwrap();
    for store in [&memory, &file] {
        assert!(unrelated_timeline(store, 1).unwrap().1);
        let fence = Fence::new(store);
        take_counts();
        let mut entered = false;
        let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h80504f4844d9 owner=fractal_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ store, |_| {
            entered = true;
            concurrent_timeline(store, 2)
        });
        let counts = take_counts();
        // Removing the shared-memory guard runs the writer under the frontier pin: this
        // assertion then fails with the exact sqlite_sequence error, after releasing the pin.
        if let Some(result) = &prepared.value {
            assert!(result.is_ok(), "unrelated publication failed: {result:?}");
        }
        if store.shared_memory {
            assert!(
                !entered,
                "shared-cache preparation must not acquire a snapshot"
            );
            assert!(prepared.cut.is_none() && prepared.value.is_none());
            assert!(counts.seed.is_empty() && counts.preparation.is_empty());
            assert!(unrelated_timeline(store, 2).unwrap().1);
        } else {
            assert!(entered);
            assert!(prepared.value.as_ref().unwrap().as_ref().unwrap().1);
            assert_eq!((counts.seed.len(), counts.preparation.len()), (1, 1));
            // A successful concurrent WAL write invalidates the old seed, as before.
            assert!(!fence.current(store, &store.connection.write(), &prepared));
        }
    }
}

