use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use smallclaims::store::{
    LATEST_TRANSPORT_OBSERVATION_QUERY, SCHEMA, Store, canonical_sql, runtime::Plain,
};

#[test]
fn transport_observation_latest_seeks_one_origin_and_preserves_canonical_ties() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    let transaction = connection.transaction().unwrap();
    for sequence in 0..3000 {
        for origin in ["local", "remote-a", "remote-b", "remote-c"] {
            let id = format!("{origin}-{sequence}");
            // Reverse insertion order and cross a decimal length boundary. All three
            // newest local observations tie in time, so writer sequence must decide.
            let accepted = (10000 - sequence).to_string();
            transaction.execute(
                "INSERT INTO batches(id, origin, replica_sequence, hash, accepted_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?1, ?4)",
                params![id, origin, sequence, accepted],
            ).unwrap();
            transaction.execute(
                "INSERT INTO claims(id, batch_id, subject, kind, origin, body, predecessors, accepted_at_unix_ms)
                 VALUES (?1, ?1, 'host/peer', 'transport.observed', ?2, ?3, '[]', ?4)",
                params![id, origin, format!("{{\"sequence\":{sequence}}}"), accepted],
            ).unwrap();
        }
    }
    transaction.execute(
        "UPDATE claims SET accepted_at_unix_ms='10000' WHERE id IN ('local-1', 'local-2')",
        [],
    ).unwrap();
    transaction.commit().unwrap();

    let indexed = canonical_sql(LATEST_TRANSPORT_OBSERVATION_QUERY);
    let reference = canonical_sql(
        "SELECT body, accepted_at_unix_ms FROM claims NOT INDEXED
         WHERE subject=?1 AND kind='transport.observed' AND origin=?2
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
    );
    for origin in ["local", "remote-a", "remote-b", "remote-c", "absent"] {
        let latest = |query: &str| {
            connection.query_row(query, params!["host/peer", origin], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }).optional().unwrap()
        };
        assert_eq!(latest(&indexed), latest(&reference));
        if origin == "local" {
            assert_eq!(latest(&indexed), Some(("{\"sequence\":2}".into(), "10000".into())));
        }
    }
    let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {indexed}")).unwrap()
        .query_map(params!["host/peer", "local"], |row| row.get::<_, String>(3)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert!(plan.iter().any(|line| line.contains(
        "SEARCH claims USING INDEX claims_transport_observed_latest_index (subject=? AND origin=?)"
    )), "expected a subject/origin seek: {plan:#?}");
    assert!(!plan.iter().any(|line| line.starts_with("SCAN claims")), "unexpected claims scan: {plan:#?}");
}

#[test]
fn transport_observation_index_is_added_when_an_existing_store_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    drop(Store::open(&path, "local", Arc::new(Plain)).unwrap());
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("DROP INDEX claims_transport_observed_latest_index").unwrap();
    drop(connection);
    drop(Store::open(&path, "local", Arc::new(Plain)).unwrap());
    let connection = Connection::open(&path).unwrap();
    let count: u32 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='claims_transport_observed_latest_index'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
