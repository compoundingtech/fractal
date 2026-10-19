//! Invented sources for the production actual-state reader. Synthetic SQL rows are not
//! admitted replication evidence; the existing actual-purity replication controls remain.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::actual_state_sorter_tests_shared_attention_tests::{SUBJECT, written_bytes};
use super::*;

fn synthetic_source() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h525da7cafce5 owner=st3_store::store::actual_state_sorter_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
         CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT,
             id TEXT UNIQUE, batch_id TEXT, subject TEXT, kind TEXT, body TEXT,
             accepted_at_unix_ms TEXT);
         CREATE INDEX claims_subject_kind_index ON claims(subject, kind, store_index);
         CREATE TABLE replica_records(claim_id TEXT, position INTEGER);
         CREATE TABLE projection_digest_repaired_claims(id TEXT PRIMARY KEY);
         INSERT INTO batches VALUES ('batch/alder', 'alder', 7), ('batch/birch', 'birch', 1);",
        )
        .unwrap();
    connection
}

fn synthetic_row(
    connection: &Connection,
    id: &str,
    batch: &str,
    time: &str,
    position: u64,
    kind: &str,
    body: &Value,
) -> u64 {
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h6c6e63d35313 owner=st3_store::store::actual_state_sorter_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,body,accepted_at_unix_ms)
         VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, batch, SUBJECT, kind, body.to_string(), time],
        )
        .unwrap();
    let index = connection.last_insert_rowid() as u64;
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-he88cb37c1bff owner=st3_store::store::actual_state_sorter_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(claim_id,position) VALUES (?1,?2)",
            params![id, position],
        )
        .unwrap();
    index
}

#[test]
fn actual_identity_stream_keeps_canonical_wire_order_and_field_carriers() {
    let values = [
        (
            "old",
            "batch/alder",
            "9",
            0,
            json!({"fields":{"phase":"old","carried":"kept"}}),
        ),
        (
            "middle",
            "batch/birch",
            "10",
            0,
            json!({"fields":{"phase":"middle"}}),
        ),
        (
            "new",
            "batch/birch",
            "10",
            1,
            json!({"fields":{"phase":"new","nullable":null}}),
        ),
    ];
    for order in [[0, 1, 2], [2, 1, 0]] {
        let connection = synthetic_source();
        for n in order {
            let (id, batch, time, position, body) = &values[n];
            synthetic_row(
                &connection,
                id,
                batch,
                time,
                *position,
                "resource.observed",
                body,
            );
        }
        let arrival = connection
            .prepare("SELECT id FROM claims ORDER BY store_index")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let canonical = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h78e994b48144 owner=st3_store::store::actual_state_sorter_tests TODO=trace-statement-and-value-representation */ &format!(
                "SELECT claims.id FROM claims ORDER BY {CANONICAL_ORDER}"
            ))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(canonical, ["old", "middle", "new"]);
        if order == [2, 1, 0] {
            assert_eq!(arrival, ["new", "middle", "old"]);
            assert_ne!(
                arrival, canonical,
                "reverse arrival must exercise canonical reordering"
            );
        }
        assert_eq!(
            latest_actual_at(&connection, SUBJECT, None).unwrap(),
            Some(json!({"phase":"new","carried":"kept","nullable":null}))
        );
    }
}

#[test]
fn actual_identity_stream_keeps_historical_cut_and_repaired_exclusion() {
    let connection = synthetic_source();
    let old = synthetic_row(
        &connection,
        "old",
        "batch/alder",
        "9",
        0,
        "resource.observed",
        &json!({"fields":{"carried":"kept","phase":"old"}}),
    );
    synthetic_row(
        &connection,
        "new",
        "batch/birch",
        "10",
        0,
        "resource.observed",
        &json!({"fields":{"phase":"new"}}),
    );
    synthetic_row(
        &connection,
        "repaired",
        "batch/birch",
        "11",
        1,
        "resource.observed",
        &json!({"fields":{"phase":"retired","carried":"wrong"}}),
    );
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h8d161cb826e8 owner=st3_store::store::actual_state_sorter_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims VALUES ('repaired')",
            [],
        )
        .unwrap();
    // Neither upper-case harness reports nor desired state enters the body fold.
    synthetic_row(
        &connection,
        "harness",
        "batch/birch",
        "12",
        2,
        "HARNESS.observed",
        &json!({"fields":{"phase":"harness"}}),
    );
    synthetic_row(
        &connection,
        "desired",
        "batch/birch",
        "13",
        3,
        "intent.desired",
        &json!({"fields":{"phase":"desired"}}),
    );
    assert_eq!(
        latest_actual_at(&connection, SUBJECT, Some(old)).unwrap(),
        Some(json!({"carried":"kept","phase":"old"}))
    );
    assert_eq!(
        latest_actual_at(&connection, SUBJECT, None).unwrap(),
        Some(json!({"carried":"kept","phase":"new"}))
    );
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-hc1ae20adaaa2 owner=st3_store::store::actual_state_sorter_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='invalid' WHERE id='new'", [])
        .unwrap();
    assert!(latest_actual_at(&connection, SUBJECT, None).is_err());
    assert_eq!(
        latest_actual_at(&connection, "resource/absent", None).unwrap(),
        None
    );
}

fn append_observation(store: &Store, time: u128, fields: Value) -> ClaimRecord {
    store.set_write_clock_at(time).unwrap();
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let claim = append_claim_tx(
        &transaction,
        "alder",
        SUBJECT,
        "resource.observed",
        None,
        &json!({"fields":fields}),
        &[],
        None,
    )
    .unwrap();
    transaction.commit().unwrap();
    claim
}

#[test]
fn actual_identity_stream_reads_the_pinned_native_reader_cut() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("claims.sqlite3"), "alder").unwrap();
    let old = append_observation(&store, 9, json!({"phase":"old","carried":"kept"}));
    let reader = store.readers.get();
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-h66a58277583a owner=st3_store::store::actual_state_sorter_tests TODO=trace-statement-and-value-representation */ "BEGIN").unwrap();
    assert_eq!(
        latest_actual_at(&reader, SUBJECT, None).unwrap(),
        Some(json!({"phase":"old","carried":"kept"}))
    );
    append_observation(&store, 10, json!({"phase":"new"}));
    assert_eq!(
        latest_actual_at(&reader, SUBJECT, None).unwrap(),
        Some(json!({"phase":"old","carried":"kept"}))
    );
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-h2bba6a389985 owner=st3_store::store::actual_state_sorter_tests TODO=trace-statement-and-value-representation */ "ROLLBACK").unwrap();
    assert_eq!(
        latest_actual_at(&reader, SUBJECT, Some(old.store_index)).unwrap(),
        Some(json!({"phase":"old","carried":"kept"}))
    );
    assert_eq!(
        store.latest_actual_value(SUBJECT).unwrap(),
        Some(json!({"phase":"new","carried":"kept"}))
    );
}

