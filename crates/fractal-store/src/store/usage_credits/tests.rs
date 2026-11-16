use super::*;

const WRITER: &str = "agent/sample/usage-producer";
const OTHER: &str = "agent/sample/other-producer";

fn complete(store: &Store) -> Vec<UsageCreditReading> {
    store.wait_for_usage_credit_backfill().unwrap();
    let snapshot = store.usage_credit_readings().unwrap();
    assert!(!snapshot.catching_up);
    snapshot.readings
}

fn credit(id: &str, at: &str) -> Value {
    json!({"provider":"example", "account":"account/example", "account_ref":"sample/example",
        "credit_id":id, "measured_at":at, "expires_at":null, "state":"available",
        "disposition":{"kind":"unknown", "at":null, "evidence_ref":null},
        "coverage":"reported", "source":"omp-usage", "host":"sample-host"})
}

fn append(store: &Store, actor: &str, credits: Vec<Value>) {
    store.append_claim(&ClaimInput {
        subject:actor.into(), actor:Some(actor.into()), kind:"usage.limits.observed".into(),
        fields:serde_json::from_value(json!({"readings":[], "credit_readings":credits})).unwrap(),
        evidence:vec![], expected_subject:None, idempotency_key:None,
    }).unwrap();
}

#[test]
fn append_preserves_writers_credit_ids_and_exact_provider_time() {
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into(), OTHER.into()]).unwrap();
    append(&store, WRITER, vec![credit("first", "2026-11-07T10:00:00.000000002Z"), credit("second", "2026-11-07T10:00:00Z")]);
    append(&store, WRITER, vec![credit("first", "2026-11-07T11:00:00.000000001+01:00")]);
    append(&store, OTHER, vec![credit("first", "2026-11-07T10:00:01Z")]);
    append(&store, WRITER, vec![]);
    store.set_usage_limits_writers(vec![]).unwrap();
    let rows = complete(&store);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().find(|row| row.actor == WRITER && row.credit["credit_id"] == "first").unwrap().credit["measured_at"], "2026-11-07T10:00:00.000000002Z");
    store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
    assert_eq!(complete(&store), rows);
}

#[test]
fn replication_converges_in_both_delivery_orders_and_rebuilds() {
    use crate::store::glasses_tests_shared_delivery::sync;
    let a = Store::open_memory("sample-a").unwrap();
    let b = Store::open_memory("sample-b").unwrap();
    for store in [&a, &b] {
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        store.set_write_clock_at(1_000).unwrap();
    }
    append(&a, WRITER, vec![credit("same", "2026-11-07T10:00:00.123456789Z")]);
    let equivalent = credit("same", "2026-11-07T11:00:00.123456789+01:00");
    append(&b, WRITER, vec![equivalent]);
    let a_head = complete(&a).remove(0);
    let b_head = complete(&b).remove(0);
    let expected = if a_head.claim_id > b_head.claim_id { a_head } else { b_head };
    let forward = Store::open_memory("sample-forward").unwrap();
    let reverse = Store::open_memory("sample-reverse").unwrap();
    // Targets have no authorized local writers; schema-valid replication still projects.
    sync(&a, &forward);
    sync(&b, &forward);
    sync(&b, &reverse);
    sync(&a, &reverse);
    for store in [&forward, &reverse] {
        assert_eq!(complete(store), vec![expected.clone()]);
        store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
        assert_eq!(complete(store), vec![expected.clone()]);
    }
}

#[test]
fn open_bootstraps_existing_history_and_catches_up_pending_claims() {
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    append(&store, WRITER, vec![credit("first", "2026-11-07T10:00:00Z")]);
    let expected = complete(&store);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-he0dad1782998 owner=fractal_store::store::usage_credits::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER usage_credit_claim_insert;
            DROP TABLE local_usage_credit_heads; DROP TABLE local_usage_credit_pending;
            DROP TABLE local_usage_credit_progress;")?;
        open(tx)?;
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(complete(&store), expected);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-hd2ecc22bed99 owner=fractal_store::store::usage_credits::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_usage_credit_heads", [])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h342a097047a2 owner=fractal_store::store::usage_credits::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO local_usage_credit_pending SELECT id FROM claims WHERE kind='usage.limits.observed'", [])?;
        flush(tx)
    }).unwrap().unwrap();
    assert_eq!(complete(&store), expected);
}

#[test]
fn absorbing_states_converge_on_append_replication_and_rebuild() {
    use crate::store::glasses_tests_shared_delivery::sync;

    for (older_state, newer_state, winner_state) in [
        ("redeemed", "unknown", "redeemed"),
        ("redeemed", "forfeited", "redeemed"),
        ("unknown", "available", "available"),
        ("forfeited", "unknown", "forfeited"),
    ] {
        let mut older = credit("same", "2026-11-07T10:00:00.000000001Z");
        older["state"] = json!(older_state);
        let mut newer = credit("same", "2026-11-07T11:00:00.000000002+01:00");
        newer["state"] = json!(newer_state);
        let expected = if winner_state == older_state { &older } else { &newer };

        // A single writer's independently produced claims can arrive from different hosts.
        let a = Store::open_memory("sample-a").unwrap();
        let b = Store::open_memory("sample-b").unwrap();
        for store in [&a, &b] {
            store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        }
        append(&a, WRITER, vec![older.clone()]);
        append(&b, WRITER, vec![newer.clone()]);
        for reverse in [false, true] {
            let local = Store::open_memory("sample-local").unwrap();
            local.set_usage_limits_writers(vec![WRITER.into(), OTHER.into()]).unwrap();
            let delivered = if reverse { [&newer, &older] } else { [&older, &newer] };
            for value in delivered {
                append(&local, WRITER, vec![value.clone()]);
            }
            // Authorization changes and another writer's newer evidence must not erase
            // this writer's terminal head.
            append(&local, OTHER, vec![newer.clone()]);
            local.set_usage_limits_writers(vec![OTHER.into()]).unwrap();
            let retained = complete(&local);
            assert_eq!(retained.len(), 2);
            assert_eq!(&retained.iter().find(|row| row.actor == WRITER).unwrap().credit, expected);
            local.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
            assert_eq!(complete(&local), retained);
            local.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
            assert_eq!(complete(&local), retained);

            // Replication projects schema-valid evidence without local writer authorization.
            let replica = Store::open_memory("sample-replica").unwrap();
            let sources = if reverse { [&b, &a] } else { [&a, &b] };
            for source in sources {
                sync(source, &replica);
            }
            let rows = complete(&replica);
            assert_eq!(rows.len(), 1);
            assert_eq!(&rows[0].credit, expected);
            replica.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
            assert_eq!(complete(&replica), rows);
        }
    }
}

#[test]
fn open_refolds_timestamp_only_v1_heads_to_recover_terminal_evidence() {
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    let mut unknown = credit("same", "2026-11-07T10:00:01Z");
    unknown["state"] = json!("unknown");
    append(&store, WRITER, vec![unknown]);
    let timestamp_only = complete(&store).remove(0);
    let mut redeemed = credit("same", "2026-11-07T10:00:00Z");
    redeemed["state"] = json!("redeemed");
    append(&store, WRITER, vec![redeemed]);
    let expected = complete(&store);
    store.set_usage_limits_writers(vec![]).unwrap();
    store.connection.batched(|tx| -> Result<()> {
        // Reproduce the old schema and losing timestamp-only head, with no pending claims.
        tx.execute(/* sqlite-large-value: allow-for-now lv-h9706066ba498 owner=fractal_store::store::usage_credits::tests TODO=bound-or-reference-values-and-reclamation */
            "UPDATE local_usage_credit_heads SET measured_seconds=?1,measured_nanos=0,claim_id=?2,credit=?3",
            params![chrono::DateTime::parse_from_rfc3339("2026-11-07T10:00:01Z")?.timestamp(),
                timestamp_only.claim_id, serde_json::to_string(&timestamp_only.credit)?])?;
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h47ef32daa8d1 owner=fractal_store::store::usage_credits::tests TODO=bound-or-reference-values-and-reclamation */
            "DROP TABLE local_usage_credit_progress;")?;
        open(tx)
    }).unwrap().unwrap();
    assert_eq!(complete(&store), expected);
    store.connection.batched(|tx| -> Result<()> { open(tx) }).unwrap().unwrap();
    assert_eq!(complete(&store), expected);
}

#[test]
fn credit_read_work_is_bounded_by_heads_not_retained_history() {
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    let measure = |count: usize| {
        for _ in 0..count {
            append(&store, WRITER, vec![credit("same", "2026-11-07T10:00:00Z")]);
        }
        // Compare warmed readers: the first connection read also initializes its SQL tracing.
        assert_eq!(complete(&store).len(), 1);
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        assert_eq!(store.usage_credit_readings().unwrap().readings.len(), 1);
        scope.finish()
    };
    let small = measure(10);
    let large = measure(300);
    assert!(large.vm_steps <= small.vm_steps * 2, "{} vs {}", small.vm_steps, large.vm_steps);
    assert_eq!(large.statements, small.statements);
}

#[test]
fn persisted_batches_resume_at_several_interruptions_with_full_fold_equivalence() {
    for interrupted_after in [1, 2, 3] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("store.sqlite3");
        let store = Store::open(&path, "sample").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into(), OTHER.into()]).unwrap();
        for index in 0..(BACKFILL_BATCH * 4 + 3) {
            let mut value = credit(&format!("credit-{}", index % 7), if index % 2 == 0 {
                "2026-11-07T10:00:00.000000001Z"
            } else { "2026-11-07T11:00:00.000000002+01:00" });
            value["state"] = json!(["unknown", "available", "forfeited", "redeemed"][index % 4]);
            append(&store, if index % 3 == 0 { OTHER } else { WRITER }, vec![value]);
        }
        let expected = complete(&store);
        store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
        for step in 1..=interrupted_after {
            store.connection.batched(|tx| -> Result<()> {
                assert!(!backfill_step_tx(tx)?);
                let (through, target) = progress(tx)?.unwrap();
                assert_eq!(through, (step * BACKFILL_BATCH) as u64);
                assert!(through < target);
                Ok(())
            }).unwrap().unwrap();
        }
        drop(store);
        let resumed = Store::open(&path, "sample").unwrap();
        assert_eq!(complete(&resumed), expected);
    }
}

#[test]
fn bounded_worker_releases_writer_and_incremental_append_does_not_complete_partial_heads() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    for index in 0..(BACKFILL_BATCH * 3) {
        append(&store, WRITER, vec![credit(&format!("credit-{index}"), "2026-11-07T10:00:00Z")]);
    }
    complete(&store);
    store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
    let (committed, committed_here) = channel();
    let (resume, resume_here) = channel();
    *store.usage_credit_backfill.lock().unwrap() = Some(spawn_backfill_with(&store.connection, move |complete| {
        committed.send(complete).unwrap();
        resume_here.recv().unwrap();
    }).unwrap());
    assert!(!committed_here.recv_timeout(Duration::from_secs(5)).unwrap());
    let before = store.usage_credit_readings().unwrap();
    assert!(before.catching_up);
    assert_eq!(before.through_index, BACKFILL_BATCH as u64);
    append(&store, WRITER, vec![credit("new-credit", "2026-11-07T10:00:01Z")]);
    let after = store.usage_credit_readings().unwrap();
    assert!(after.catching_up);
    assert_eq!(after.through_index, before.through_index);
    assert!(after.readings.iter().any(|row| row.credit["credit_id"] == "new-credit"));
    // Interrupt at a committed cursor. With the owner dropped, no next batch may run.
    let mut worker = store.usage_credit_backfill.lock().unwrap().take().unwrap();
    let handle = worker.handle.take().unwrap();
    drop(worker);
    resume.send(()).unwrap();
    handle.join().unwrap().unwrap();
    assert_eq!(complete(&store).len(), BACKFILL_BATCH * 3 + 1);
}

#[test]
fn empty_configuration_skips_absent_projection_until_later_configured_or_direct_access() {
    for configured in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("store.sqlite3");
        let store = Store::open(&path, "sample").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        for index in 0..(BACKFILL_BATCH * 3) {
            append(&store, WRITER, vec![credit(&format!("credit-{index}"), "2026-11-07T10:00:00Z")]);
        }
        let expected = complete(&store);
        store.connection.batched(|tx| -> Result<()> {
            tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h311136cb3033 owner=fractal_store::store::usage_credits::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER usage_credit_claim_insert;
                DROP TABLE local_usage_credit_heads;
                DROP TABLE local_usage_credit_pending;
                DROP TABLE local_usage_credit_progress;")?;
            Ok(())
        }).unwrap().unwrap();
        drop(store);
        let store = Store::open(&path, "sample").unwrap();
        store.set_usage_limits_writers(vec![]).unwrap();
        assert!(!exists(&store.readers.get()).unwrap());
        assert!(store.usage_credit_backfill.lock().unwrap().is_none());
        if configured {
            store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        } else {
            let _ = store.usage_credit_readings().unwrap();
        }
        store.wait_for_usage_credit_backfill().unwrap();
        assert!(exists(&store.readers.get()).unwrap());
        assert!(!store.usage_credit_readings().unwrap().catching_up);
        assert_eq!(complete(&store), expected);
    }
}

#[test]
fn open_projection_work_is_constant_and_never_advances_historical_cursor() {
    let measure = |count| {
        let store = Store::open_memory("sample").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        for _ in 0..count {
            append(&store, WRITER, vec![credit("same", "2026-11-07T10:00:00Z")]);
        }
        complete(&store);
        store.connection.batched(|tx| -> Result<_> {
            rebuild(tx)?;
            let before = progress(tx)?;
            let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
            open(tx)?;
            let work = scope.finish();
            assert_eq!(progress(tx)?, before);
            Ok(work)
        }).unwrap().unwrap()
    };
    let small = measure(8);
    let large = measure(BACKFILL_BATCH * 4);
    assert!(large.vm_steps <= small.vm_steps * 2, "{} vs {}", small.vm_steps, large.vm_steps);
    assert_eq!(large.statements, small.statements);
}

#[test]
fn store_open_sql_work_is_bounded_independently_of_retained_usage_history() {
    let root = tempfile::tempdir().unwrap();
    let paths = [root.path().join("small.sqlite3"), root.path().join("large.sqlite3")];
    for (path, count) in paths.iter().zip([8, BACKFILL_BATCH * 8]) {
        let store = Store::open(path, "sample").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        for _ in 0..count {
            append(&store, WRITER, vec![credit("same", "2026-11-07T10:00:00Z")]);
        }
        complete(&store);
        drop(store);
        // Both databases have already installed every open-time schema/version marker.
        drop(Store::open(path, "sample").unwrap());
    }
    let measured = paths.map(|path| {
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        let store = Store::open(&path, "sample").unwrap();
        let work = scope.finish();
        drop(store);
        work
    });
    let [small, large] = measured;
    assert!(large.vm_steps <= small.vm_steps * 2, "{} vs {}", small.vm_steps, large.vm_steps);
    assert_eq!(large.statements, small.statements);
}

#[test]
fn replication_defers_absent_empty_receiver_projection_until_config_or_read_demand() {
    use crate::store::glasses_tests_shared_delivery::sync;
    let source = Store::open_memory("source").unwrap();
    source.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    for index in 0..(BACKFILL_BATCH * 2 + 1) {
        append(&source, WRITER, vec![credit(&format!("credit-{index}"), "2026-11-07T10:00:00Z")]);
    }
    let expected = complete(&source);
    for configured in [false, true] {
        let receiver = Store::open_memory("receiver").unwrap();
        sync(&source, &receiver);
        assert!(!exists(&receiver.readers.get()).unwrap());
        assert!(receiver.usage_credit_backfill.lock().unwrap().is_none());
        if configured {
            receiver.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        } else {
            let _ = receiver.usage_credit_readings().unwrap();
        }
        assert_eq!(complete(&receiver), expected);
    }
}

#[test]
fn configured_append_before_worker_initializes_is_included_in_its_target() {
    let store = Store::open_memory("sample").unwrap();
    // Deterministically stage the setter's authorization update before its worker's
    // first queued transaction, without relying on thread scheduling.
    store.fractal.usage_limits_writers.lock().unwrap().insert(WRITER.into());
    append(&store, WRITER, vec![credit("before-init", "2026-11-07T10:00:00Z")]);
    assert!(!exists(&store.readers.get()).unwrap());
    store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    let rows = complete(&store);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].credit["credit_id"], "before-init");
}
