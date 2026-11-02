use super::*;
use std::time::Instant;

#[test]
#[ignore = "manual two-million-claim index build measurement; run with --ignored --nocapture"]
fn sealed_view_cache_measure_index_build_on_two_million_claims() {
    const ROWS: usize = 2_000_000;
    const BATCHES: usize = 20_000;
    const CLAIMS_PER_BATCH: usize = 100;
    const BODY_BYTES: usize = 256;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let wal_path = dir.path().join("claims.sqlite3-wal");
    // Synthetic, not signed/admitted claims: fixed-width padded counters stand in for
    // 64-byte hex claim IDs and hashes. Batch IDs are 87 bytes:
    // batch/fixture/<8-digit sequence>/<64-byte hex hash>. Each batch has 100 claims;
    // subjects cycle through 10,000 values and kinds uniformly through 16 values.
    // Every body is the same 256-byte JSON object (231-byte ASCII payload), with no
    // _operation field; the production partial operation index therefore stays empty.
    // All other production indexes remain installed and maintained during setup.
    let body = json!({"fields": {"payload": "x".repeat(231)}}).to_string();
    assert_eq!(body.len(), BODY_BYTES);
    assert_eq!(BATCHES * CLAIMS_PER_BATCH, ROWS);
    {
        let mut connection = Connection::open(&path).unwrap();
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h79b67f329d5d owner=smallclaims::store::sealed_view_cache_tests::index_cost_tests TODO=trace-statement-and-value-representation */ SCHEMA).unwrap();
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h97e1b46dc727 owner=smallclaims::store::sealed_view_cache_tests::index_cost_tests TODO=bound-or-reference-values-and-reclamation */ "DROP INDEX IF EXISTS claims_batch_kind;").unwrap();
        let tx = connection.transaction().unwrap();
        let batches = tx.execute(
            /* sqlite-large-value: allow-for-now lv-h62c6f148a781 owner=smallclaims::store::sealed_view_cache_tests::index_cost_tests TODO=chunk-or-reference-exact-wire-values */ "WITH RECURSIVE counters(n) AS (
                 VALUES(1) UNION ALL SELECT n+1 FROM counters WHERE n<?1
             )
             INSERT INTO batches(id,origin,replica_sequence,previous_hash,hash,accepted_at_unix_ms)
             SELECT printf('batch/fixture/%08d/%064x',n,n),'fixture',n,NULL,
                 printf('%064x',n),'1791849600000'
             FROM counters",
            [BATCHES as i64],
        ).unwrap();
        assert_eq!(batches, BATCHES);
        let rows = tx.execute(
            /* sqlite-large-value: allow-for-now lv-hf0d539537e15 owner=smallclaims::store::sealed_view_cache_tests::index_cost_tests TODO=prepare-body-and-predecessor-values */ "WITH RECURSIVE counters(n) AS (
                 VALUES(1) UNION ALL SELECT n+1 FROM counters WHERE n<?1
             )
             INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             SELECT printf('%064x',n),
                 printf('batch/fixture/%08d/%064x',(n-1)/?2+1,(n-1)/?2+1),
                 printf('note/fixture/%08d',(n-1)%10000),
                 printf('fixture.kind%02d',(n-1)%16),'fixture',NULL,?3,'[]',
                 '1791849600000'
             FROM counters",
            params![ROWS as i64, CLAIMS_PER_BATCH as i64, body],
        ).unwrap();
        assert_eq!(rows, ROWS);
        tx.commit().unwrap();
        let checkpoint: (i64, i64, i64) = connection.query_row(
            "PRAGMA wal_checkpoint(TRUNCATE)", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(checkpoint, (0, 0, 0), "setup WAL must be fully truncated");
        // No prepared statement, reader snapshot, or setup connection pins the WAL.
        connection.close().unwrap();
    }

    let connection = Connection::open(&path).unwrap();
    // Match SCHEMA's connection settings without rerunning SCHEMA (which would build
    // the index). Leave wal_autocheckpoint at SQLite's production default: elapsed
    // time includes CREATE INDEX's durable commit and any automatic checkpoint.
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;",
    ).unwrap();
    let started = Instant::now();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h707dd277ce20 owner=smallclaims::store::sealed_view_cache_tests::index_cost_tests TODO=chunk-or-reference-variable-columns */ "CREATE INDEX claims_batch_kind ON claims(batch_id,kind);",
    ).unwrap();
    let elapsed = started.elapsed();
    // Read physical lengths while the writer remains open, before any explicit
    // checkpoint or close. WAL bytes include recycled/backfilled frames, not just
    // pending frames; db_bytes is the main file's physical length at this instant.
    let wal_bytes = std::fs::metadata(&wal_path).unwrap().len();
    let db_bytes = std::fs::metadata(&path).unwrap().len();
    println!(
        "sealed_view_cache_index_build rows={ROWS} time_ms={} wal_bytes={wal_bytes} db_bytes={db_bytes} batches={BATCHES} claims_per_batch={CLAIMS_PER_BATCH} claim_id_bytes=64 batch_id_bytes=87 body_bytes={BODY_BYTES} subjects=10000 kinds=16 kind_distribution=uniform body_distribution=constant_json wal_kind=physical wal_autocheckpoint=default synchronous=FULL",
        elapsed.as_millis(),
    );
    connection.close().unwrap();
    dir.close().unwrap();
}
