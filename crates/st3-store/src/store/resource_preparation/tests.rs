#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation::tests_shared_seats::{ResetHook, fixture, fixture_for};
use super::*;

#[test]
fn the_native_counter_fences_trigger_without_rowid_replace_delete_and_rollback() {
    let (_store_directory, store, _) = fixture();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h51ebd2697b2d owner=st3_store::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE invented_input(id INTEGER PRIMARY KEY,value TEXT);
         CREATE TABLE invented_output(id INTEGER PRIMARY KEY,value TEXT) WITHOUT ROWID;
         CREATE TABLE invented_child(id INTEGER PRIMARY KEY REFERENCES invented_input(id) ON DELETE CASCADE);
         CREATE TABLE invented_truncate(value TEXT);
         INSERT INTO invented_truncate VALUES('a'),('b');
         CREATE VIEW invented_visible AS SELECT id,value FROM invented_input;
         CREATE TRIGGER invented_copy AFTER INSERT ON invented_input BEGIN
             INSERT OR REPLACE INTO invented_output VALUES(NEW.id,NEW.value); END;
         CREATE TRIGGER invented_view_write INSTEAD OF INSERT ON invented_visible BEGIN
             INSERT INTO invented_input VALUES(NEW.id,NEW.value); END;",
        )
        .unwrap();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h5a695c36f150 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    assert!(fence.current(&store, &transaction, &prepared));
    let count = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-h1276e02f66d3 owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_visible VALUES(1,'first')", [])
        .unwrap();
    assert!(
        transaction.total_changes() >= count + 2,
        "the view-row itself is omitted, its base-table and trigger/WITHOUT ROWID writes count"
    );
    assert!(!fence.current(&store, &transaction, &prepared));
    transaction.rollback().unwrap();
    assert!(
        !fence.current(&store, &writer, &prepared),
        "rolled-back work conservatively invalidates"
    );
    drop(writer);

    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h2a0aef662216 owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_input VALUES(1,'first')", [])
        .unwrap();
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h71864eed6e29 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let count = transaction.total_changes();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-hbc8d9a9e06f5 owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO invented_input VALUES(1,'replacement')",
            [],
        )
        .unwrap();
    assert!(
        transaction.total_changes() > count,
        "REPLACE INSERT counts even when implicit DELETE does not"
    );
    assert!(!fence.current(&store, &transaction, &prepared));
    transaction.commit().unwrap();
    drop(writer);

    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h488b56d2c856 owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_child VALUES(1)", [])
        .unwrap();
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hf1e2831889e9 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let before_delete = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-he1f19ebd2363 owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM invented_input", [])
        .unwrap();
    assert!(
        transaction.total_changes() >= before_delete + 2,
        "base delete and foreign-key action count"
    );
    assert!(
        !fence.current(&store, &transaction, &prepared),
        "base-table delete is counted without an update hook"
    );
    transaction.rollback().unwrap();
    drop(writer);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h84407456cbef owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let count = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-hdbb342d4817f owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM invented_truncate", [])
        .unwrap();
    assert!(transaction.total_changes() >= count + 2);
    assert!(
        !fence.current(&store, &transaction, &prepared),
        "a whole-table delete without triggers/FKs also invalidates"
    );
    transaction.rollback().unwrap();
}

#[test]
fn tokens_do_not_cross_store_or_fence_incarnations_and_nested_pins_are_unavailable() {
    let (_store_directory, store, _) = fixture();
    let (_other_directory, other, _) = fixture();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hcef56f485dc0 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-h33bc96c975ec owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &other, |_| ()).value.is_none());
    assert!(!Fence::new(&other).current(&other, &other.connection.write(), &prepared));
    assert!(!Fence::new(&store).current(&store, &store.connection.write(), &prepared));
    store
        .read_snapshot(|_| {
            assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-hb290f2b346dd owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none());
            Ok(())
        })
        .unwrap();
    // Unknown/read-failed publication never authorizes an effect.
    let mut unavailable = fence.prepare(/* sqlite-large-value: allow-for-now lv-h4246fa1c3753 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    unavailable.cut = None;
    assert!(!fence.current(&store, &store.connection.write(), &unavailable));
    let mut wrapped = fence.prepare(/* sqlite-large-value: allow-for-now lv-h5342ed0237f7 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    wrapped.cut.as_mut().unwrap().changes = u64::MAX;
    assert!(!fence.current(&store, &store.connection.write(), &wrapped));
}

#[test]
fn preparation_reads_the_post_seed_cut_and_refuses_a_foreign_reader_file() {
    let (_store_directory, store, _) = fixture();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h70061f0e2590 owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-fresh-cut','new')",
            [],
        )
        .unwrap();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h09837ff10b59 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |reader| {
        assert!(
            !reader.is_autocommit(),
            "the canonical inputs share a real read transaction"
        );
        reader
            .query_row(
                "SELECT value FROM meta WHERE key='invented-fresh-cut'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    });
    assert_eq!(prepared.value.as_deref(), Some("new"));
    let mut foreign = fence.prepare(/* sqlite-large-value: allow-for-now lv-h857c8f43a788 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    foreign.cut.as_mut().unwrap().file = "invented-other-file".into();
    assert!(!fence.current(&store, &store.connection.write(), &foreign));

    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hba58047fa590 owner=st3_store::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE invented_shadow(value TEXT)")
        .unwrap();
    assert!(
        fence.prepare(/* sqlite-large-value: allow-for-now lv-h40ef39be4279 owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none(),
        "a preexisting writer TEMP schema cannot silently change name resolution"
    );
}

#[test]
fn a_second_connection_commit_is_detected_by_the_same_writer_data_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite");
    let store = Store::open(&path, "prepared-other-writer").unwrap();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h48d3ef01843e owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let expected = prepared.cut.as_ref().unwrap();
    let other = Connection::open(&path).unwrap();
    other
        .execute(
            /* sqlite-large-value: allow-for-now lv-ha0c76ae0907f owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-external-write','1')",
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

#[test]
fn a_later_queued_job_rejects_an_earlier_jobs_uncommitted_write() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (_store_directory, store, _) = fixture();
    let store = Arc::new(store);
    let fence = Arc::new(Fence::new(&store));
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h1933a33479ed owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let commits = Arc::new(AtomicUsize::new(0));
    let seen = commits.clone();
    let _observer = store.observe_commits(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    let accepted = Arc::new(AtomicBool::new(true));
    let before_commit = Arc::new(AtomicBool::new(false));
    // Prequeue both jobs while a loan keeps the native writer from starting the first.
    // This exercises the real queue/savepoints, not a post-commit observer approximation.
    let writer = store.connection.write();
    let mut done = Vec::new();
    let (answer, result) = std::sync::mpsc::sync_channel(1);
    done.push(result);
    store.connection.send(WriterJob::Batched {
        run: Box::new(|tx| {
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h8e48e2e77211 owner=st3_store::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-same-batch','1')",
                [],
            )
            .is_ok()
        }),
        profile: None,
        wait: None,
        holder: smallclaims::windows::holds::Holder::current(),
        enqueued: std::time::Instant::now(),
        done: answer,
    });
    let (answer, result) = std::sync::mpsc::sync_channel(1);
    done.push(result);
    let selected = accepted.clone();
    let inside = before_commit.clone();
    let counter = commits.clone();
    let current_store = store.clone();
    store.connection.send(WriterJob::Batched {
        run: Box::new(move |tx| {
            inside.store(counter.load(Ordering::SeqCst) == 1, Ordering::SeqCst);
            selected.store(
                fence.current(&current_store, tx, &prepared),
                Ordering::SeqCst,
            );
            true
        }),
        profile: None,
        wait: None,
        holder: smallclaims::windows::holds::Holder::current(),
        enqueued: std::time::Instant::now(),
        done: answer,
    });
    // Returning the loan publishes one hint; neither queued job has committed yet.
    drop(writer);
    for result in done {
        result.recv().unwrap().unwrap();
    }
    assert!(
        before_commit.load(Ordering::SeqCst),
        "the two prequeued jobs must share the native batch for this discriminator"
    );
    assert!(
        !accepted.load(Ordering::SeqCst),
        "the native counter must reject before commit publication"
    );
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
        let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-heb0858aa879b owner=st3_store::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ store, |_| {
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

