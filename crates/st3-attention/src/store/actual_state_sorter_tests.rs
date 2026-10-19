#![allow(unused_imports)]
use super::*;
use st3_store::store::actual_state_sorter_tests_shared_attention_tests::{SUBJECT, written_bytes};


// The former body-bearing sorter, retained only as an invented-fixture comparison.
#[cfg(target_os = "linux")]
fn body_sort_reference(connection: &Connection) -> Option<Value> {
    let mut statement = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-h2bde60ec13a7 owner=st3_attention::store::actual_state_sorter_tests TODO=trace-statement-and-value-representation */ &format!(
            "SELECT claims.kind, claims.body FROM claims INDEXED BY claims_subject_kind_index
         JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND {ACTUAL_STATE_CLAIM}
           AND claims.kind NOT LIKE 'harness.%' AND claims.store_index<=?2
           AND {UNREPAIRED_CLAIM} ORDER BY {CANONICAL_ORDER}"
        ))
        .unwrap();
    let mut rows = statement.query(params![SUBJECT, i64::MAX]).unwrap();
    fold_latest_values(std::iter::from_fn(|| {
        rows.next().unwrap().map(|row| {
            let kind: String = row.get(0).unwrap();
            let body: String = row.get(1).unwrap();
            (kind, serde_json::from_str::<Value>(&body).unwrap())
        })
    }))
    .unwrap()
}

#[cfg(target_os = "linux")]
fn byte_comparison(connection: &Connection, count: usize, width: usize, caller: &str) -> u64 {
    connection
        .execute_batch("PRAGMA temp_store=FILE; PRAGMA cache_size=-64")
        .unwrap();
    let before = written_bytes();
    let started = std::time::Instant::now();
    let expected = body_sort_reference(connection);
    let reference_us = started.elapsed().as_micros();
    let reference_bytes = written_bytes() - before;
    let before = written_bytes();
    let started = std::time::Instant::now();
    let actual = latest_actual_at(connection, SUBJECT, None).unwrap();
    let identity_us = started.elapsed().as_micros();
    let identity_bytes = written_bytes() - before;
    assert_eq!(actual, expected);
    assert_eq!(actual.as_ref().unwrap()["rank"], json!(0));
    // Compact keys for at most 256 rows stay comfortably inside this byte budget even
    // when payload width grows eightfold. The old query fails this same assertion.
    assert!(
        identity_bytes < 2 * 1024 * 1024,
        "{caller}: {count}x{width} actual-state read wrote {identity_bytes} bytes"
    );
    if count == 256 && width == 128 * 1024 {
        assert!(
            reference_bytes > 8 * 1024 * 1024,
            "reference did not exercise a body sorter spill: {reference_bytes}"
        );
    }
    println!(
        "actual sorter {caller}: rows={count} width={width} body_sort_bytes={reference_bytes} identity_sort_bytes={identity_bytes} body_sort_read_us={reference_us} identity_sort_read_us={identity_us} point_lookups={count}"
    );
    identity_bytes
}

#[cfg(target_os = "linux")]
#[test]
fn actual_identity_stream_bounds_native_reader_and_writer_temp_bytes() {
    // Two independent growth axes; at most 32 MiB of body data in one file-backed fixture.
    // Setup is excluded from thread-local syscall-byte measurements. No copied live data.
    for count in [128, 256] {
        let mut narrow = None;
        for width in [16 * 1024, 128 * 1024] {
            let directory = tempfile::tempdir().unwrap();
            let store = Store::open(&directory.path().join("claims.sqlite3"), "alder").unwrap();
            {
                let mut writer = store.connection.write();
                let transaction = writer.transaction().unwrap();
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-hd0736c142abd owner=st3_attention::store::actual_state_sorter_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,previous_hash,hash,accepted_at_unix_ms)
                     VALUES ('batch/alder/sorter','alder',1,NULL,'sorter','1')", [],
                ).unwrap();
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-h83319416432d owner=st3_attention::store::actual_state_sorter_tests TODO=prepare-body-and-predecessor-values */ "WITH RECURSIVE history(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM history WHERE n+1<?1)
                     INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                     SELECT printf('%064x',n),'batch/alder/sorter',?2,'resource.observed','alder',
                            json_object('fields',json_object('rank',n,'cursor',?3)), '[]',CAST(?1-n AS TEXT)
                     FROM history",
                    params![count, SUBJECT, "x".repeat(width)],
                ).unwrap();
                transaction.commit().unwrap();
            }
            let reader_bytes = {
                let reader = store.readers.get();
                byte_comparison(&reader, count, width, "reader")
            };
            // Observer ingestion calls the same fold while holding the native writer.
            let writer = store.connection.write();
            let writer_bytes = byte_comparison(&writer, count, width, "writer");
            if let Some((narrow_reader, narrow_writer)) = narrow {
                assert_eq!(
                    reader_bytes, narrow_reader,
                    "reader sorter writes must not grow with payload width at {count} identities"
                );
                assert_eq!(
                    writer_bytes, narrow_writer,
                    "writer sorter writes must not grow with payload width at {count} identities"
                );
            } else {
                narrow = Some((reader_bytes, writer_bytes));
            }
        }
    }
}
