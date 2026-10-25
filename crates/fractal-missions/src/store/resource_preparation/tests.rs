#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use fractal_store::store::resource_preparation::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use fractal_store::store::resource_preparation::tests_shared_seats::{ResetHook, fixture, fixture_for};
use super::*;


#[test]
fn the_native_counter_fences_trigger_without_rowid_replace_delete_and_rollback() {
    let (_store_directory, store, _) = fixture();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-hce7090013b32 owner=fractal_missions::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE invented_input(id INTEGER PRIMARY KEY,value TEXT);
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
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h3bc5b1a5a473 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    assert!(fence.current(&store, &transaction, &prepared));
    let count = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-h7f801e64b648 owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_visible VALUES(1,'first')", [])
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
        .execute(/* sqlite-large-value: allow-for-now lv-h75a1ca7b5ff0 owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_input VALUES(1,'first')", [])
        .unwrap();
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h52813697cfa2 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let count = transaction.total_changes();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h619595e3d75e owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO invented_input VALUES(1,'replacement')",
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
        .execute(/* sqlite-large-value: allow-for-now lv-he337946add73 owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_child VALUES(1)", [])
        .unwrap();
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h01c86721e28a owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let before_delete = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-hf772fd405c55 owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM invented_input", [])
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
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hf02160169cc7 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let count = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-h1fc00fda0eed owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM invented_truncate", [])
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
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-he2e1b76ab1b7 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-h1650bf6a58bc owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &other, |_| ()).value.is_none());
    assert!(!Fence::new(&other).current(&other, &other.connection.write(), &prepared));
    assert!(!Fence::new(&store).current(&store, &store.connection.write(), &prepared));
    store
        .read_snapshot(|_| {
            assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-hf2ee35959168 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none());
            Ok(())
        })
        .unwrap();
    // Unknown/read-failed publication never authorizes an effect.
    let mut unavailable = fence.prepare(/* sqlite-large-value: allow-for-now lv-hb651b4929ca7 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    unavailable.cut = None;
    assert!(!fence.current(&store, &store.connection.write(), &unavailable));
    let mut wrapped = fence.prepare(/* sqlite-large-value: allow-for-now lv-hfe9c6bf35233 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
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
            /* sqlite-large-value: allow-for-now lv-h26f1ece353b6 owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-fresh-cut','new')",
            [],
        )
        .unwrap();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h0ee836d9bd3f owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |reader| {
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
    let mut foreign = fence.prepare(/* sqlite-large-value: allow-for-now lv-h9d88320be3bd owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    foreign.cut.as_mut().unwrap().file = "invented-other-file".into();
    assert!(!fence.current(&store, &store.connection.write(), &foreign));

    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h2cde553fd1a4 owner=fractal_missions::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE invented_shadow(value TEXT)")
        .unwrap();
    assert!(
        fence.prepare(/* sqlite-large-value: allow-for-now lv-h9e58fda6723a owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none(),
        "a preexisting writer TEMP schema cannot silently change name resolution"
    );
}

#[test]
fn a_later_queued_job_rejects_an_earlier_jobs_uncommitted_write() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (_store_directory, store, _) = fixture();
    let store = Arc::new(store);
    let fence = Arc::new(Fence::new(&store));
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-ha0dce1e45fb7 owner=fractal_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
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
                /* sqlite-large-value: allow-for-now lv-h0e569540b40b owner=fractal_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-same-batch','1')",
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

#[test]
fn observation_preparation_history_decodes_are_independent_of_history_length() {
    use fractal_store::store::ACTUAL_HISTORY_BODIES_DECODED;

    const ITEMS: u64 = 3;
    let (_directory, store, revision) = fixture();
    let resource = "resource/example-repository";
    let facts = json!({"pull_requests": (1..=ITEMS)
        .map(|number| json!({"number": number, "state": "open"}))
        .collect::<Vec<_>>()});
    let mut measurements = Vec::new();
    for history in [1, 8, 32] {
        let prior = measurements.last().map_or(0, |(length, _)| *length);
        for _ in prior..history {
            for number in 0..=ITEMS {
                let (subject, kind, observed) = if number == 0 {
                    (resource.to_owned(), "vcs.repository", json!({}))
                } else {
                    (format!("{resource}/pull-request/{number}"),
                     "vcs.pull-request", json!({"number": number, "state": "open"}))
                };
                store.append_claim(&ClaimInput {
                    subject,
                    kind: "resource.observed".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("kind".into(), json!(kind)),
                        ("facts".into(), observed),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                }).unwrap();
            }
        }
        ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(0));
        let prepared = Fence::new(&store).prepare_with_actuals(&store, |reader, _| {
            read_inputs(reader, Observation {
                observer: "observer/fixture",
                desired_revision: &revision,
                attempt: None,
                resource,
                facts: &facts,
                subscriptions: &[],
            })
        });
        let inputs = prepared.value.expect("file-backed preparation is available").unwrap();
        assert_eq!(inputs.recorded_items.len(), ITEMS as usize);
        assert!(inputs.recorded_items.values().all(Option::is_some));
        let decoded = ACTUAL_HISTORY_BODIES_DECODED.with(std::cell::Cell::get);
        measurements.push((history, decoded));
    }
    eprintln!("preparation history bodies decoded (I={ITEMS}): {measurements:?}");
    let baseline = measurements[0].1;
    assert!(
        measurements.iter().all(|(_, decoded)| *decoded <= baseline),
        "fixed-cardinality preparation must not decode growing claim history: {measurements:?}"
    );
}

#[test]
fn canonical_observer_fallback_history_reads_are_bounded() {
    use fractal_store::store::ACTUAL_HISTORY_BODIES_DECODED;

    let (_directory, store, revision) = fixture();
    let facts = json!({"state": "open"});
    let cursor = "x".repeat(64 * 1024);
    let mut measurements = Vec::new();
    for history in [1, 8, 32] {
        let prior = measurements.last().map_or(0, |(length, _)| *length);
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        for _ in prior..history {
            append_claim_tx(&transaction, &store.origin, "observer/fixture", "observer.observed",
                None, &json!({"fields": {
                    "status": "healthy", "revision": revision, "cursor": cursor, "changed": true,
                }}), &[], None).unwrap();
        }
        transaction.commit().unwrap();
        let transaction = writer.transaction().unwrap();
        ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(0));
        // This is the exact None-preparation branch in record_resource_observation_attempt.
        let prepared = inputs(&transaction, "observer/fixture", &revision, None,
            "resource/fixture", &facts, &[]).unwrap();
        assert!(prepared.active_subscriptions.is_empty());
        let decoded = ACTUAL_HISTORY_BODIES_DECODED.with(std::cell::Cell::get);
        measurements.push((history, decoded));
        transaction.rollback().unwrap();
    }
    eprintln!("canonical observer fallback (history claims, bodies decoded): {measurements:?}");
    assert!(
        measurements.iter().all(|(_, decoded)| *decoded <= measurements[0].1),
        "fixed-input fallback must not decode growing observer history: {measurements:?}"
    );
}
