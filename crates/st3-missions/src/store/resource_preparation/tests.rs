#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use st3_delivery::store::resource_preparation::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use st3_delivery::store::resource_preparation::tests_shared_seats::{ResetHook, fixture, fixture_for};
use super::*;


#[test]
fn the_native_counter_fences_trigger_without_rowid_replace_delete_and_rollback() {
    let (_store_directory, store, _) = fixture();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h2acfb7234297 owner=st3_missions::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE invented_input(id INTEGER PRIMARY KEY,value TEXT);
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
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h69af57ac3ae9 owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    assert!(fence.current(&store, &transaction, &prepared));
    let count = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-hb1ebf81c52b2 owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_visible VALUES(1,'first')", [])
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
        .execute(/* sqlite-large-value: allow-for-now lv-h27d0206afb3d owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_input VALUES(1,'first')", [])
        .unwrap();
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hb42c1328e1fa owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let count = transaction.total_changes();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-hc92db0d6cf23 owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO invented_input VALUES(1,'replacement')",
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
        .execute(/* sqlite-large-value: allow-for-now lv-h37bc2a7cbf37 owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO invented_child VALUES(1)", [])
        .unwrap();
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hb071e9117a3c owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let before_delete = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-h514e2ed24f3a owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM invented_input", [])
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
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-he713a1d4df56 owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let count = transaction.total_changes();
    transaction
        .execute(/* sqlite-large-value: allow-for-now lv-h047d4e1653aa owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM invented_truncate", [])
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
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hd4688780e9ec owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-h7affbcffe3ce owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &other, |_| ()).value.is_none());
    assert!(!Fence::new(&other).current(&other, &other.connection.write(), &prepared));
    assert!(!Fence::new(&store).current(&store, &store.connection.write(), &prepared));
    store
        .read_snapshot(|_| {
            assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-ha9d09c7653fb owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none());
            Ok(())
        })
        .unwrap();
    // Unknown/read-failed publication never authorizes an effect.
    let mut unavailable = fence.prepare(/* sqlite-large-value: allow-for-now lv-h1d02e1711126 owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    unavailable.cut = None;
    assert!(!fence.current(&store, &store.connection.write(), &unavailable));
    let mut wrapped = fence.prepare(/* sqlite-large-value: allow-for-now lv-h379316ee1efc owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
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
            /* sqlite-large-value: allow-for-now lv-h78f342cb6275 owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-fresh-cut','new')",
            [],
        )
        .unwrap();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h13551bbe1555 owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |reader| {
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
    let mut foreign = fence.prepare(/* sqlite-large-value: allow-for-now lv-h5917353ea5a7 owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    foreign.cut.as_mut().unwrap().file = "invented-other-file".into();
    assert!(!fence.current(&store, &store.connection.write(), &foreign));

    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hd0811f73c21f owner=st3_missions::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE invented_shadow(value TEXT)")
        .unwrap();
    assert!(
        fence.prepare(/* sqlite-large-value: allow-for-now lv-h8e6997015091 owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none(),
        "a preexisting writer TEMP schema cannot silently change name resolution"
    );
}

#[test]
fn a_later_queued_job_rejects_an_earlier_jobs_uncommitted_write() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (_store_directory, store, _) = fixture();
    let store = Arc::new(store);
    let fence = Arc::new(Fence::new(&store));
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-ha7590e83029a owner=st3_missions::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
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
                /* sqlite-large-value: allow-for-now lv-h23ece28a4306 owner=st3_missions::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('invented-same-batch','1')",
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
