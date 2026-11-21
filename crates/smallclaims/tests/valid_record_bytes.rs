//! A valid replica record keeps no bytes of its own (`store::record_bytes`): new records are
//! stored without them, older ones are emptied in pages, and nothing a reader, a peer or a
//! digest sees changes.
use smallclaims::{
    claim::ClaimInput,
    store::{Store, runtime::Plain},
};
use std::{collections::BTreeMap, sync::Arc};

const FLEET: &str = "sample-fleet";

fn note(store: &Store, n: usize) {
    store
        .append_claim(&ClaimInput {
            subject: format!("note/sample-{}", n % 7),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::from([("text".into(), serde_json::json!(format!("note {n}")))]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(format!("note-{n}")),
        })
        .unwrap();
}

/// Bring `peer` level with `source`, as successive exchanges do: one carries a bounded page.
fn sync(source: &Store, peer: &Store) {
    for _ in 0..8 {
        let exchange = source
            .export_replication_exchange(FLEET, &peer.replication_inventory().unwrap())
            .unwrap();
        if exchange.envelopes.is_empty() {
            break;
        }
        peer.receive_replication_exchange("alder", FLEET, &exchange)
            .unwrap();
        peer.validate_replication_backlog().unwrap();
        peer.project_replication_backlog().unwrap();
    }
}

/// A record as readers see it, without its bytes: its ref, state, position and claim.
type RecordRow = (String, String, i64, Option<String>);
/// What readers and peers see of a store: its authority and graph digests, every record and
/// every claim position.
type Observable = (String, String, Vec<RecordRow>, Vec<(String, i64)>);

/// What readers and peers see of a store: its digests and every record but its bytes.
fn observable(store: &Store) -> Observable {
    let status = store.replication_status(true, Some(FLEET), &[]).unwrap();
    let connection = store.connection.write();
    let records = connection
        .prepare("SELECT record_ref, state, position, claim_id FROM replica_records ORDER BY record_ref")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let positions = connection
        .prepare("SELECT claim_id, position FROM claim_positions ORDER BY claim_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    (status.authority_digest, status.graph_digest, records, positions)
}

fn record_bytes(store: &Store, state: &str) -> (usize, usize) {
    store
        .connection
        .write()
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(raw)), 0) FROM replica_records WHERE state=?1",
            [state],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

#[test]
fn new_valid_records_keep_no_bytes_and_still_replicate() {
    let alder = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    for n in 0..40 {
        note(&alder, n);
    }
    let birch = Store::open_memory("birch", Arc::new(Plain)).unwrap();
    sync(&alder, &birch);
    for store in [&alder, &birch] {
        let (valid, bytes) = record_bytes(store, "valid");
        assert!(valid >= 40, "every claim has its valid record");
        assert_eq!(bytes, 0, "a valid record keeps no bytes");
    }
    assert_eq!(observable(&alder).0, observable(&birch).0);
    assert_eq!(observable(&alder).1, observable(&birch).1);
}

#[test]
fn older_valid_record_bytes_empty_in_resumable_pages_and_nothing_else_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("alder.sqlite3");
    let alder = Store::open(&path, "alder", Arc::new(Plain)).unwrap();
    for n in 0..600 {
        note(&alder, n);
    }
    let birch = Store::open_memory("birch", Arc::new(Plain)).unwrap();
    sync(&alder, &birch);
    // A store from an earlier build kept a copy of each valid record's bytes, and keeps the
    // bytes of a record it could not admit.
    {
        let connection = alder.connection.write();
        connection
            .execute("UPDATE replica_records SET raw=randomblob(512) WHERE state='valid'", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO replica_records(record_ref, writer, sequence, envelope_hash, position, raw,
                     state, error_code, updated_at_unix_ms)
                 VALUES ('record/unknown-sample', 'cedar', 1, 'unknown-envelope', 0, X'0102', 'unknown',
                     'unknown-claim-kind', '0')",
                [],
            )
            .unwrap();
    }
    let before = observable(&alder);
    assert!(record_bytes(&alder, "valid").1 > 0);

    let first = alder.release_valid_record_bytes().unwrap();
    assert!(first.emptied > 0 && !first.done);
    assert_eq!(first.bytes, first.emptied as u64 * 512);
    assert!(!alder.valid_record_bytes_released().unwrap());
    // The cursor committed with the page: a restart resumes after it.
    drop(alder);
    let alder = Store::open(&path, "alder", Arc::new(Plain)).unwrap();
    let mut emptied = first.emptied;
    loop {
        let page = alder.release_valid_record_bytes().unwrap();
        emptied += page.emptied;
        if page.done {
            break;
        }
    }
    assert!(alder.valid_record_bytes_released().unwrap());
    assert_eq!(emptied, record_bytes(&alder, "valid").0);
    assert_eq!(record_bytes(&alder, "valid").1, 0);
    assert_eq!(record_bytes(&alder, "unknown"), (1, 2), "a record that is not valid keeps its bytes");
    assert_eq!(observable(&alder), before, "rows, positions and digests are unchanged");
    let again = alder.release_valid_record_bytes().unwrap();
    assert!(again.done && again.emptied == 0);
    // An older build that runs the store after a rollback stores bytes for the records it
    // admits; the next start empties them, and only them.
    note(&alder, 601);
    alder.replication_snapshot().unwrap();
    alder
        .connection
        .write()
        .execute(
            "UPDATE replica_records SET raw=randomblob(64)
             WHERE rowid=(SELECT MAX(rowid) FROM replica_records WHERE state='valid')",
            [],
        )
        .unwrap();
    assert_eq!(record_bytes(&alder, "valid").1, 64);
    assert!(!alder.valid_record_bytes_released().unwrap());
    let mut later = alder.release_valid_record_bytes().unwrap();
    while !later.done {
        let page = alder.release_valid_record_bytes().unwrap();
        later.emptied += page.emptied;
        later.done = page.done;
    }
    assert_eq!(later.emptied, 1);
    assert_eq!(record_bytes(&alder, "valid").1, 0);
    assert!(alder.valid_record_bytes_released().unwrap());

    // Both an existing and a new peer still sync from it, to the same digests. The unknown
    // record stood for one from a peer; it has no envelope here, so it leaves first.
    alder
        .connection
        .write()
        .execute("DELETE FROM replica_records WHERE record_ref='record/unknown-sample'", [])
        .unwrap();
    note(&alder, 600);
    let cedar = Store::open_memory("cedar", Arc::new(Plain)).unwrap();
    for peer in [&birch, &cedar] {
        sync(&alder, peer);
        assert_eq!(
            peer.replication_status(true, Some(FLEET), &[]).unwrap().authority_digest,
            alder.replication_status(true, Some(FLEET), &[]).unwrap().authority_digest,
        );
    }
}

#[test]
fn the_listed_columns_are_every_record_column_but_raw() {
    let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    let columns: Vec<String> = store
        .connection
        .write()
        .prepare("SELECT name FROM pragma_table_info('replica_records') WHERE name<>'raw' ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        smallclaims::store::record_bytes::RECORD_COLUMNS_BUT_RAW
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>(),
        columns
    );
}

/// Every record as readers see it: `replica_record_rows`, without bytes.
fn record_rows(store: &Store) -> Vec<(String, i64, String, i64, String, Option<String>)> {
    store
        .connection
        .write()
        .prepare(
            "SELECT writer, sequence, envelope_hash, position, state, claim_id FROM replica_record_rows
             ORDER BY writer, sequence, envelope_hash, position",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn user_version(store: &Store) -> u32 {
    store.connection.write().query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap()
}

#[test]
fn valid_records_go_after_the_window_and_readers_and_peers_see_the_same_records() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("alder.sqlite3");
    let alder = Store::open(&path, "alder", Arc::new(Plain)).unwrap();
    for n in 0..300 {
        note(&alder, n);
    }
    let birch = Store::open_memory("birch", Arc::new(Plain)).unwrap();
    sync(&alder, &birch);
    let rows = record_rows(&alder);
    let claim_records = rows.iter().filter(|row| row.5.is_some()).count();
    assert!(claim_records >= 300);
    assert_eq!(record_bytes(&alder, "valid").0, rows.len(), "the view shows each stored record once");
    let before = observable(&alder);

    // Inside the rollback window nothing goes, and the store stays at the default version.
    let waiting = alder.release_valid_records(60 * 60 * 1000).unwrap();
    assert!(waiting.waiting_until_unix_ms.is_some() && waiting.deleted == 0);
    assert!(user_version(&alder) < smallclaims::store::record_bytes::VALID_RECORDS_GONE_VERSION);

    // After it, the valid claim records go in pages that resume after a restart.
    let first = alder.release_valid_records(0).unwrap();
    assert!(first.deleted > 0 && !first.done);
    assert_eq!(user_version(&alder), smallclaims::store::record_bytes::VALID_RECORDS_GONE_VERSION);
    drop(alder);
    let alder = Store::open(&path, "alder", Arc::new(Plain)).unwrap();
    while !alder.release_valid_records(0).unwrap().done {}
    assert_eq!(
        alder.connection.write().query_row(
            "SELECT COUNT(*) FROM replica_records WHERE state='valid' AND claim_id IS NOT NULL", [], |row| row.get::<_, usize>(0),
        ).unwrap(),
        0,
        "no valid claim record is stored"
    );
    assert_eq!(record_rows(&alder), rows, "readers see the same records");
    let after = observable(&alder);
    assert_eq!((after.0.clone(), after.1.clone(), after.3.clone()), (before.0, before.1, before.3), "digests and positions are unchanged");

    // New claims, local and replicated, store no valid record and still show in the view, and
    // peers sync both ways to equal digests.
    note(&alder, 300);
    note(&birch, 301);
    sync(&alder, &birch);
    let exchange = birch
        .export_replication_exchange(FLEET, &alder.replication_inventory().unwrap())
        .unwrap();
    alder.receive_replication_exchange("birch", FLEET, &exchange).unwrap();
    alder.validate_replication_backlog().unwrap();
    alder.project_replication_backlog().unwrap();
    alder.replication_snapshot().unwrap();
    assert_eq!(
        alder.connection.write().query_row(
            "SELECT COUNT(*) FROM replica_records WHERE state='valid' AND claim_id IS NOT NULL", [], |row| row.get::<_, usize>(0),
        ).unwrap(),
        0
    );
    assert_eq!(record_rows(&alder).len(), rows.len() + 2);
    assert_eq!(
        alder.replication_status(true, Some(FLEET), &[]).unwrap().authority_digest,
        birch.replication_status(true, Some(FLEET), &[]).unwrap().authority_digest,
    );
}

#[test]
fn an_admitted_claim_is_one_with_an_envelope_and_no_record_in_another_state() {
    let alder = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    note(&alder, 0);
    alder.replication_snapshot().unwrap();
    let admitted = || -> bool {
        alder
            .connection
            .write()
            .query_row(
                &format!(
                    "SELECT {} FROM claims WHERE kind='example.note'",
                    smallclaims::store::record_bytes::admitted_claim_sql("claims")
                ),
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert!(admitted(), "a sealed claim with its valid record");
    let set_state = |state: Option<&str>| {
        let connection = alder.connection.write();
        match state {
            Some(state) => connection
                .execute(
                    "UPDATE replica_records SET state=?1 WHERE claim_id=(SELECT id FROM claims WHERE kind='example.note')",
                    [state],
                )
                .unwrap(),
            None => connection
                .execute(
                    "DELETE FROM replica_records WHERE claim_id=(SELECT id FROM claims WHERE kind='example.note')",
                    [],
                )
                .unwrap(),
        };
    };
    set_state(Some("invalid"));
    assert!(!admitted(), "a record marked invalid excludes the claim");
    set_state(Some("repaired"));
    assert!(!admitted(), "a repaired record excludes the claim");
    set_state(None);
    assert!(admitted(), "a valid claim whose record is not stored is admitted");
}

#[test]
fn an_envelopes_record_count_seeks_by_identity_and_scans_nothing() {
    // The local trim capture counts each envelope's records through the view; on a store with
    // millions of claims each count must stay a few index seeks.
    let alder = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    note(&alder, 0);
    alder.replication_snapshot().unwrap();
    // Without statistics the planner assumes large tables, as on a real store; statistics of a
    // one-claim store would make scans the cheapest plan.
    let connection = alder.connection.write();
    let plan: Vec<String> = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM replica_record_rows records
             WHERE records.writer=?1 AND records.sequence=?2 AND records.envelope_hash=?3",
        )
        .unwrap()
        .query_map(rusqlite::params!["alder", 1, "hash"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // `SCAN records` reads the view's own rows as they are produced; every table is searched.
    assert!(plan.iter().all(|step| !step.starts_with("SCAN ") || step == "SCAN records"), "{plan:#?}");
    assert!(plan.iter().any(|step| step.contains("SEARCH claims USING INDEX claims_batch_index")), "{plan:#?}");
}
