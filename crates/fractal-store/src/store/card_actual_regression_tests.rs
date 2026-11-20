use super::*;

const SUBJECT: &str = "agent/example";

fn append(tx: &Transaction<'_>, status: &str, batch: Option<&str>) -> ClaimRecord {
    append_claim_tx(tx, "alder", SUBJECT, "runtime.observed", None,
        &json!({"fields":{"status":status}}), &[], batch).unwrap()
}

fn read(store: &Store, connection: &Connection, index: u64, status: &str) -> usize {
    let mut reads = CardReads::new(index);
    ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(0));
    reads.load_actual(store, connection, &[SUBJECT.into()]).unwrap();
    let decoded = ACTUAL_HISTORY_BODIES_DECODED.with(std::cell::Cell::get);
    let actual = &reads.actual[SUBJECT];
    assert_eq!(actual.latest.as_ref().unwrap()["status"], status);
    assert_eq!(actual.latest, latest_actual_at(connection, SUBJECT, Some(index)).unwrap());
    let expected = selected_actual_source_at(connection, SUBJECT, Some(index), None).unwrap();
    assert_eq!(actual.source.as_ref().map(|(id, origin, _)| (Some(id.clone()), Some(origin.clone())))
        .unwrap_or((None, None)), (expected.0, expected.1));
    decoded
}

fn cached(store: &Store) -> Arc<card_actual::Reduction> {
    store.fractal.card_actuals.lock().unwrap().get(&(SUBJECT.into(), false)).unwrap().clone()
}

#[test]
fn writer_transactions_neither_reuse_nor_publish_shared_actuals_after_rollback() {
    let store = Store::open_memory("alder").unwrap();
    store.set_write_clock_at(100).unwrap();
    let first = {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let claim = append(&tx, "running", None);
        tx.commit().unwrap();
        claim
    };
    assert_eq!(read(&store, &store.readers.get(), first.store_index, "running"), 1);
    let committed = cached(&store);
    let rolled_back_index = {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        // A transaction must not even reuse an otherwise matching committed prefix.
        assert_eq!(read(&store, &tx, first.store_index, "running"), 1);
        let pending = append(&tx, "stopped", None);
        assert_eq!(read(&store, &tx, pending.store_index, "stopped"), 2);
        assert!(Arc::ptr_eq(&committed, &cached(&store)));
        tx.rollback().unwrap();
        pending.store_index
    };
    let replacement = {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let replacement = append(&tx, "running", None);
        assert_eq!(replacement.store_index, rolled_back_index);
        assert_eq!(read(&store, &tx, replacement.store_index, "running"), 2);
        assert!(Arc::ptr_eq(&committed, &cached(&store)));
        tx.commit().unwrap();
        replacement
    };
    read(&store, &store.readers.get(), replacement.store_index, "running");
}

#[test]
fn duplicate_envelope_position_join_invalidates_the_canonical_prefix() {
    let store = Store::open_memory("alder").unwrap();
    store.set_write_clock_at(100).unwrap();
    let (first, second) = {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let first = append(&tx, "running", None);
        let second = append(&tx, "stopped", Some(&first.batch_id));
        tx.execute(/* sqlite-large-value: allow-for-now lv-h4aeb57f40a61 owner=fractal_store::store::card_actual_regression_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='100' WHERE id IN (?1,?2)",
            params![first.id, second.id]).unwrap();
        // Local admission already assigned 0 and 1. Seed the pre-duplicate wire
        // positions directly: the production MIN join cannot raise those values.
        tx.execute(/* sqlite-large-value: allow-for-now lv-hf09583e4d191 owner=fractal_store::store::card_actual_regression_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_positions SET position=10 WHERE claim_id=?1", [&first.id]).unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h5cdac82c1d39 owner=fractal_store::store::card_actual_regression_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_positions SET position=20 WHERE claim_id=?1", [&second.id]).unwrap();
        tx.commit().unwrap();
        (first, second)
    };
    assert_eq!(read(&store, &store.readers.get(), second.store_index, "stopped"), 2);
    assert_eq!(read(&store, &store.readers.get(), second.store_index, "stopped"), 0);
    let before = cached(&store);
    {
        let writer = store.connection.write();
        // Duplicate-envelope admission joins the supplied position before noticing
        // the existing claim/record. No new arrival index or replica row is needed.
        canonical::record_position_tx(&writer, &second.id, 1).unwrap();
    }
    assert_eq!(store.index().unwrap(), second.store_index);
    assert_eq!(read(&store, &store.readers.get(), second.store_index, "running"), 2);
    assert_ne!(before.revision, cached(&store).revision);
    {
        let writer = store.connection.write();
        // Repeating the identical join does not change effective canonical order.
        canonical::record_position_tx(&writer, &second.id, 1).unwrap();
    }
    assert_eq!(read(&store, &store.readers.get(), second.store_index, "running"), 0);
    assert_ne!(first.id, second.id);
}

#[test]
fn physically_readonly_snapshots_can_reuse_shared_actuals() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("test.db"), "alder").unwrap();
    store.set_write_clock_at(100).unwrap();
    let claim = {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let claim = append(&tx, "running", None);
        tx.commit().unwrap();
        claim
    };
    store.read_snapshot(|_| {
        let connection = store.readers.get();
        assert!(!connection.is_autocommit());
        assert!(connection.is_readonly(rusqlite::DatabaseName::Main)?);
        assert_eq!(read(&store, &connection, claim.store_index, "running"), 1);
        assert_eq!(read(&store, &connection, claim.store_index, "running"), 0);
        Ok(())
    }).unwrap();
}
