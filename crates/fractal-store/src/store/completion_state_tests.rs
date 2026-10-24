use super::*;

const OBSERVER: &str = "observer/example-health";
const SUBSCRIPTION: &str = "subscription/example-health";

fn narrow(value: Option<Value>) -> Option<Value> {
    value.map(|value| Value::Object(FIELDS.into_iter().filter_map(|field|
        value.get(field).map(|value| (field.into(), value.clone()))).collect()))
}

fn parity(connection: &Connection, subject: &str) {
    flush(connection).unwrap();
    assert_eq!(actual(connection, subject).unwrap(), narrow(latest_actual(connection, subject).unwrap()));
}

fn append(connection: &Transaction<'_>, origin: &str, subject: &str, kind: &str, fields: Value) -> String {
    append_claim_tx(connection, origin, subject, kind, None, &json!({"fields":fields}), &[], None)
        .unwrap().id
}

// Historical wire shapes can carry fields the current public writer no longer admits.
fn append_legacy(tx: &Transaction<'_>, origin: &str, subject: &str, kind: &str, fields: Value) -> String {
    let id = append(tx, origin, subject, "observer.observed", json!({}));
    tx.execute(/* sqlite-large-value: allow-for-now lv-h8074b0d5efe1 owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET kind=?2,body=?3 WHERE id=?1",
        params![id, kind, json!({"fields":fields}).to_string()]).unwrap();
    id
}

#[test]
fn completion_state_converges_across_reversed_replication_arrival() {
    use super::super::tests::{exchange_from, receive_and_project};

    for reverse in [false, true] {
        let target = Store::open_memory("example-target").unwrap();
        let old = Store::open_memory("example-old").unwrap();
        let new = Store::open_memory("example-new").unwrap();
        for (source, time, state) in [(&old, 10, "degraded"), (&new, 20, "healthy")] {
            source.set_write_clock_at(time).unwrap();
            let mut writer = source.connection.write();
            let tx = writer.transaction().unwrap();
            append(&tx, &source.origin, OBSERVER, "observer.state",
                json!({"state":state,"revision":format!("revision-{time}")}));
            flush(&tx).unwrap();
            tx.commit().unwrap();
        }
        let sources = if reverse { [&new, &old] } else { [&old, &new] };
        for source in sources {
            receive_and_project(&target, &source.origin,
                &exchange_from(source, &ReplicationInventory::default()));
            let writer = target.connection.write();
            parity(&writer, OBSERVER);
        }
        let writer = target.connection.write();
        assert_eq!(actual(&writer, OBSERVER).unwrap().unwrap(),
            json!({"state":"healthy","revision":"revision-20"}));
    }
}

#[test]
fn completion_state_matches_append_carry_forward_transition_clearing_and_json_null() {
    let store = Store::open_memory("example-health").unwrap();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    parity(&tx, OBSERVER);
    append(&tx, &store.origin, OBSERVER, "observer.state",
        json!({"state":"degraded","revision":"old","reason":"offline"}));
    parity(&tx, OBSERVER);
    append(&tx, &store.origin, OBSERVER, "observer.observed",
        json!({"revision":"new","cursor":"opaque"}));
    parity(&tx, OBSERVER);
    assert_eq!(actual(&tx, OBSERVER).unwrap().unwrap(),
        json!({"state":"degraded","revision":"new","reason":"offline"}));
    append(&tx, &store.origin, OBSERVER, "observer.state", json!({"state":"healthy"}));
    parity(&tx, OBSERVER);
    assert_eq!(actual(&tx, OBSERVER).unwrap().unwrap(), json!({"state":"healthy"}));
    append_legacy(&tx, &store.origin, OBSERVER, "observer.observed", json!({"reason":null}));
    parity(&tx, OBSERVER);
    assert!(actual(&tx, OBSERVER).unwrap().unwrap().get("reason").unwrap().is_null());
    append(&tx, &store.origin, SUBSCRIPTION, "subscription.state",
        json!({"state":"pending","reason":"missing-target"}));
    parity(&tx, SUBSCRIPTION);
    append(&tx, &store.origin, SUBSCRIPTION, "subscription.state", json!({"state":"active"}));
    parity(&tx, SUBSCRIPTION);
    assert_eq!(actual(&tx, SUBSCRIPTION).unwrap().unwrap(), json!({"state":"active"}));
    tx.commit().unwrap();
}

#[test]
fn completion_state_repairs_and_out_of_order_claims_use_indexed_predecessors() {
    let store = Store::open_memory("example-health").unwrap();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let old = append(&tx, &store.origin, OBSERVER, "observer.state",
        json!({"state":"degraded","revision":"old","reason":"offline"}));
    let new = append(&tx, &store.origin, OBSERVER, "observer.state",
        json!({"state":"healthy","revision":"new"}));
    tx.execute(/* sqlite-large-value: allow-for-now lv-hc8b5552e1bd4 owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='20' WHERE id=?1", [&old]).unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-ha255f4d678bf owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='30' WHERE id=?1", [&new]).unwrap();
    parity(&tx, OBSERVER);
    // Arrives last but sorts between the prior states. It must not resurrect a cleared reason.
    let late = append_legacy(&tx, &store.origin, OBSERVER, "observer.observed",
        json!({"revision":"late","reason":"late-reason"}));
    tx.execute(/* sqlite-large-value: allow-for-now lv-hd3dd3521b2b8 owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='25' WHERE id=?1", [&late]).unwrap();
    parity(&tx, OBSERVER);
    assert_eq!(actual(&tx, OBSERVER).unwrap().unwrap(), json!({"state":"healthy","revision":"new"}));
    tx.execute(/* sqlite-large-value: allow-for-now lv-h81a06432eb48 owner=fractal_store::store::completion_state_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)", [&new]).unwrap();
    parity(&tx, OBSERVER);
    assert_eq!(actual(&tx, OBSERVER).unwrap().unwrap(),
        json!({"state":"degraded","revision":"late","reason":"late-reason"}));
    tx.execute(/* sqlite-large-value: allow-for-now lv-h59110ab2cf1d owner=fractal_store::store::completion_state_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM projection_digest_repaired_claims WHERE id=?1", [&new]).unwrap();
    parity(&tx, OBSERVER);
    // Changing canonical batch inputs and explicit positions also invalidates candidates.
    tx.execute(/* sqlite-large-value: allow-for-now lv-h00a05218092d owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "UPDATE batches SET origin='example-other' WHERE id=(SELECT batch_id FROM claims WHERE id=?1)", [&new]).unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h4f250c788e7b owner=fractal_store::store::completion_state_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_positions SET position=99 WHERE claim_id=?1", [&old]).unwrap();
    parity(&tx, OBSERVER);
    tx.execute(/* sqlite-large-value: allow-for-now lv-hba992e26e63e owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&new]).unwrap();
    parity(&tx, OBSERVER);
    tx.rollback().unwrap();
}

#[test]
fn completion_state_reads_uncommitted_changes_and_rolls_back_with_the_batch() {
    let store = Store::open_memory("example-health").unwrap();
    let mut writer = store.connection.write();
    {
        let tx = writer.transaction().unwrap();
        append(&tx, &store.origin, OBSERVER, "observer.state", json!({"state":"healthy","revision":"first"}));
        parity(&tx, OBSERVER);
        tx.commit().unwrap();
    }
    let before = actual(&writer, OBSERVER).unwrap();
    let tx = writer.transaction().unwrap();
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-hd439a4489ff4 owner=fractal_store::store::completion_state_tests TODO=trace-statement-and-value-representation */ "SAVEPOINT earlier_job").unwrap();
    let id = append(&tx, &store.origin, OBSERVER, "observer.state", json!({"state":"degraded","revision":"second"}));
    parity(&tx, OBSERVER);
    assert_ne!(actual(&tx, OBSERVER).unwrap(), before);
    // The historical API stays historical even while current-cut candidates include later DML.
    let cut: u64 = tx.query_row("SELECT store_index-1 FROM claims WHERE id=?1", [&id], |r| r.get(0)).unwrap();
    assert_eq!(narrow(latest_actual_at(&tx, OBSERVER, Some(cut)).unwrap()), before);
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h5b27b5de7324 owner=fractal_store::store::completion_state_tests TODO=trace-statement-and-value-representation */ "ROLLBACK TO earlier_job; RELEASE earlier_job").unwrap();
    parity(&tx, OBSERVER);
    assert_eq!(actual(&tx, OBSERVER).unwrap(), before);
    append(&tx, &store.origin, OBSERVER, "observer.state", json!({"state":"stopped"}));
    parity(&tx, OBSERVER);
    tx.rollback().unwrap();
    assert_eq!(actual(&writer, OBSERVER).unwrap(), before);
}

#[test]
fn completion_state_rebuild_and_reopen_are_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.db");
    let expected = {
        let store = Store::open(&path, "example-health").unwrap();
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        append(&tx, &store.origin, OBSERVER, "observer.state", json!({"state":"healthy","revision":"current"}));
        parity(&tx, OBSERVER);
        let expected = actual(&tx, OBSERVER).unwrap();
        rebuild(&tx).unwrap();
        assert_eq!(actual(&tx, OBSERVER).unwrap(), expected);
        tx.commit().unwrap();
        expected
    };
    let store = Store::open(&path, "example-health").unwrap();
    let writer = store.connection.write();
    assert_eq!(actual(&writer, OBSERVER).unwrap(), expected);
}

#[test]
fn completion_state_maintenance_and_reads_do_not_scale_with_observer_history() {
    let store = Store::open_memory("example-health").unwrap();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let cursor = "x".repeat(64 * 1024);
    let mut costs = Vec::new();
    let mut prior = 0;
    for history in [1, 32, 256] {
        for _ in prior..history {
            append(&tx, &store.origin, OBSERVER, "observer.observed", json!({"revision":"current","cursor":cursor}));
            flush(&tx).unwrap();
        }
        prior = history;
        BODIES_DECODED.with(|count| count.set(0));
        let work = smallclaims::sqlite::work::SqliteWorkScope::start();
        append(&tx, &store.origin, OBSERVER, "observer.observed", json!({"revision":"current","cursor":cursor}));
        let append_work = work.finish();
        let work = smallclaims::sqlite::work::SqliteWorkScope::start();
        flush(&tx).unwrap();
        let maintenance = work.finish();
        assert_eq!(BODIES_DECODED.with(std::cell::Cell::get), 1);
        BODIES_DECODED.with(|count| count.set(0));
        let work = smallclaims::sqlite::work::SqliteWorkScope::start();
        actual(&tx, OBSERVER).unwrap();
        let read = work.finish();
        assert_eq!(BODIES_DECODED.with(std::cell::Cell::get), 0);
        assert_eq!(maintenance.sorts, 0);
        assert_eq!(read.fullscan_steps, 0);
        assert_eq!(read.sorts, 0);
        costs.push((history, maintenance, read, append_work));
    }
    eprintln!("completion health maintenance/read work: {costs:?}");
    assert!(costs[0].1.vm_steps > 0, "native work accounting must be active");
    for (_, maintenance, read, append_work) in &costs {
        assert!(maintenance.vm_steps <= costs[0].1.vm_steps * 2, "{costs:?}");
        assert_eq!(maintenance.statements, costs[0].1.statements, "{costs:?}");
        assert_eq!(read.vm_steps, costs[0].2.vm_steps, "{costs:?}");
        assert!(append_work.vm_steps <= costs[0].3.vm_steps * 2, "{costs:?}");
    }
    tx.rollback().unwrap();
}

#[test]
fn completion_state_external_work_rejects_preparation_until_canonical_flush() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.db");
    let store = Store::open(&path, "example-health").unwrap();
    let id = {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let id = append(&tx, &store.origin, OBSERVER, "observer.state",
            json!({"state":"healthy","revision":"first"}));
        flush(&tx).unwrap();
        tx.commit().unwrap();
        id
    };
    let external = Connection::open(&path).unwrap();
    external.execute(/* sqlite-large-value: allow-for-now lv-hcdaaf2c5696b owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$.fields.state','degraded') WHERE id=?1",
        [&id]).unwrap();
    let fence = resource_preparation::Fence::new(&store);
    assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-hcba78a651068 owner=fractal_store::store::completion_state_tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none());
    {
        let writer = store.connection.write();
        assert!(!current(&writer).unwrap());
        parity(&writer, OBSERVER);
        assert_eq!(actual(&writer, OBSERVER).unwrap().unwrap()["state"], "degraded");
    }
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h2ecd69fcc8f6 owner=fractal_store::store::completion_state_tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    assert!(fence.current(&store, &store.connection.write(), &prepared));
}

#[test]
fn completion_state_external_commit_between_seed_and_snapshot_rejects_preparation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.db");
    let store = Store::open(&path, "example-health").unwrap();
    let id = {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let id = append(&tx, &store.origin, OBSERVER, "observer.state",
            json!({"state":"healthy","revision":"first"}));
        flush(&tx).unwrap();
        tx.commit().unwrap();
        id
    };
    let external = Connection::open(&path).unwrap();
    resource_preparation::BEFORE_SNAPSHOT.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            assert!(current(&external).unwrap(), "seed starts with a complete projection");
            external.execute(/* sqlite-large-value: allow-for-now lv-hf7e3da82da36 owner=fractal_store::store::completion_state_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$.fields.state','degraded') WHERE id=?1",
                [&id]).unwrap();
            assert!(!current(&external).unwrap());
        }));
    });
    let fence = resource_preparation::Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h899ee994aebb owner=fractal_store::store::completion_state_tests TODO=trace-statement-and-value-representation */ &store, |_| panic!("pending work must reject before inputs"));
    assert!(prepared.cut.is_some(), "the writer seed preceded the external commit");
    assert!(prepared.value.is_none());
}

#[test]
#[ignore = "explicit cold-backfill benchmark writes about 2 GB of synthetic cursor history"]
fn completion_state_cold_backfill_twenty_thousand_large_cursors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.db");
    let store = Store::open(&path, "example-backfill").unwrap();
    let cursor = "x".repeat(99_000);
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        for _ in 0..20_000 {
            append(&tx, &store.origin, OBSERVER, "observer.observed",
                json!({"revision":"current","cursor":cursor}));
        }
        tx.commit().unwrap();
    }
    let mut writer = store.connection.write();
    let started = std::time::Instant::now();
    let tx = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let locked = std::time::Instant::now();
    rebuild(&tx).unwrap();
    tx.commit().unwrap();
    let lock_time = locked.elapsed();
    let elapsed = started.elapsed();
    let peak = std::fs::read_to_string("/proc/self/status").ok().and_then(|status|
        status.lines().find(|line| line.starts_with("VmHWM:")).map(str::to_owned));
    eprintln!("cold completion backfill: rows=20000 cursor_bytes=99000 duration={elapsed:?} writer_lock={lock_time:?} peak={peak:?}");
    assert_eq!(actual(&writer, OBSERVER).unwrap().unwrap(), json!({"revision":"current"}));
}
