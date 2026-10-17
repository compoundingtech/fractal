use super::*;
use super::super::tests::{exchange_from, receive_and_project};
use proptest::prelude::*;

const SUBJECT: &str = "resource/reuse-alder";

fn append(store: &Store, subject: &str, at: u128, fields: Value) -> String {
    store.set_write_clock_at(at).unwrap();
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let claim = append_claim_tx(&transaction, &store.origin, subject, "resource.observed",
        None, &json!({"fields":fields}), &[], None).unwrap();
    transaction.commit().unwrap();
    claim.id
}

fn read(store: &Store) -> (Option<Value>, u64) {
    let fence = resource_preparation::Fence::new(store);
    let prepared = fence.prepare_with_actuals(store, |connection, serial| {
        let work = smallclaims::sqlite::work::SqliteWorkScope::start();
        let value = store.smalltalk.observer_actuals.read(connection, SUBJECT, serial.unwrap()).unwrap();
        let statements = work.finish().statements;
        assert_eq!(value, latest_actual(connection, SUBJECT).unwrap());
        (value, statements)
    });
    assert!(fence.current(store, &store.connection.write(), &prepared));
    prepared.value.unwrap()
}

#[test]
fn observer_actuals_reuse_survives_other_subject_writes_without_history_reads() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("store.db"), "alder").unwrap();
    append(&store, SUBJECT, 10, json!({"facts":{"state":"open"},"carried":"kept"}));
    let first = read(&store);
    assert!(first.1 > 0);
    append(&store, "resource/other", 20, json!({"facts":{"state":"closed"}}));
    let second = read(&store);
    assert_eq!(second.0, first.0);
    assert_eq!(second.1, 0, "unchanged subject must execute no history SQL");
    assert_eq!(read(&store).1, 0);
    let remote = Store::open_memory("birch").unwrap();
    append(&remote, "resource/replicated-other", 30, json!({"facts":{"state":"open"}}));
    receive_and_project(&store, "birch", &exchange_from(&remote, &ReplicationInventory::default()));
    assert_eq!(read(&store).1, 0, "unrelated replication must preserve the reduction");
}

#[test]
fn observer_actuals_does_not_publish_a_reduction_from_an_invalidated_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("store.db"), "alder").unwrap();
    append(&store, SUBJECT, 10, json!({"facts":{"state":"open"}}));
    read(&store);
    let fence = resource_preparation::Fence::new(&store);
    let stale = fence.prepare_with_actuals(&store, |connection, serial| {
        append(&store, SUBJECT, 20, json!({"facts":{"state":"closed"}}));
        store.smalltalk.observer_actuals.read(connection, SUBJECT, serial.unwrap()).unwrap()
    });
    assert!(!fence.current(&store, &store.connection.write(), &stale));
    assert_eq!(read(&store).0.unwrap()["facts"]["state"], "closed");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn observer_actuals_equal_full_fold_across_late_arrivals_repairs_and_nulls(
        reverse in any::<bool>(), nullable in any::<bool>(),
    ) {
        let directory = tempfile::tempdir().unwrap();
        let target = Store::open(&directory.path().join("store.db"), "cedar").unwrap();
        let old = Store::open_memory("alder").unwrap();
        let new = Store::open_memory("birch").unwrap();
        let old_id = append(&old, SUBJECT, 10, json!({"facts":{"state":"open"},"carried":"kept"}));
        let new_id = append(&new, SUBJECT, 20, json!({"facts":{"state":"closed"},
            "nullable":if nullable { Value::Null } else { json!("set") }}));
        prop_assert_eq!(read(&target).0, None);
        let sources = if reverse { [&new, &old] } else { [&old, &new] };
        for source in sources {
            receive_and_project(&target, &source.origin, &exchange_from(source, &ReplicationInventory::default()));
            read(&target);
            prop_assert_eq!(read(&target).1, 0);
        }
        let actual = read(&target).0.unwrap();
        prop_assert_eq!(&actual["carried"], &json!("kept"));
        prop_assert_eq!(&actual["nullable"], &if nullable { Value::Null } else { json!("set") });
        // A named repair excludes an original already represented in a warm reduction.
        target.connection.write().execute(
            "INSERT INTO projection_digest_repaired_claims(id) VALUES (?1)", [&new_id]).unwrap();
        let repaired = read(&target);
        prop_assert!(repaired.1 > 0);
        prop_assert_eq!(&repaired.0.unwrap()["facts"]["state"], &json!("open"));
        // Undo and direct source mutations are conservatively global invalidations.
        target.connection.write().execute(
            "DELETE FROM projection_digest_repaired_claims WHERE id=?1", [&new_id]).unwrap();
        read(&target);
        target.connection.write().execute(
            "UPDATE claims SET body=json_set(body,'$.fields.carried','changed') WHERE id=?1", [&old_id]).unwrap();
        prop_assert_eq!(&read(&target).0.unwrap()["carried"], &json!("changed"));
    }
}

#[test]
fn observer_actuals_external_writer_invalidates_warm_result() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let store = Store::open(&path, "alder").unwrap();
    let id = append(&store, SUBJECT, 10, json!({"facts":{"state":"open"},"carried":"kept"}));
    read(&store);
    assert_eq!(read(&store).1, 0);
    let external = Connection::open(&path).unwrap();
    external.execute("UPDATE claims SET body=json_set(body,'$.fields.carried','external') WHERE id=?1", [&id]).unwrap();
    let changed = read(&store);
    assert!(changed.1 > 0);
    assert_eq!(changed.0.unwrap()["carried"], "external");
}

#[test]
fn observer_actuals_lru_is_bounded() {
    let cache = Actuals::default();
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE batches(id TEXT PRIMARY KEY,origin TEXT,replica_sequence INTEGER);
        CREATE TABLE claims(store_index INTEGER PRIMARY KEY,id TEXT,batch_id TEXT,subject TEXT,kind TEXT,body TEXT,accepted_at_unix_ms TEXT);
        CREATE INDEX claims_subject_kind_index ON claims(subject,kind,store_index);
        CREATE TABLE replica_records(claim_id TEXT,position INTEGER);
        CREATE TABLE projection_digest_repaired_claims(id TEXT PRIMARY KEY);").unwrap();
    let serial = cache.seed(&connection).unwrap();
    for index in 0..LIMIT + 10 {
        cache.read(&connection, &format!("resource/{index}"), serial).unwrap();
    }
    let state = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(state.entries.len(), LIMIT);
    assert!(state.bytes <= BYTE_LIMIT);
}
