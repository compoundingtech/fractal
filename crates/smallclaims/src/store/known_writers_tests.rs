//! The writers a checkpoint counts: the work of finding them grows with the writers a node
//! holds envelopes from, not with how many envelopes each wrote.
use super::*;
use crate::sqlite::work::SqliteWorkScope;
use crate::store::runtime::Plain;

/// `count` more envelopes for `writer` in `replica_envelopes`, after any it already has.
fn replica_envelopes(store: &Store, writer: &str, count: u64) {
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hdff59cf457d6 owner=smallclaims::store::known_writers_tests TODO=chunk-or-reference-exact-wire-values */ "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?2)
             INSERT INTO replica_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms,
                 payload, relay, receipt_state, received_at_unix_ms)
             SELECT ?1, i + (SELECT COALESCE(MAX(sequence), 0) FROM replica_envelopes
                             WHERE writer=?1),
                    printf('%064x', i), '0', x'', ?1, 'validated', '0'
             FROM n",
            params![writer, count],
        )
        .unwrap();
}

/// `count` more checkpoint tombstones for `writer`, after any it already has.
fn checkpoint_envelopes(store: &Store, writer: &str, count: u64) {
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hbebd5a08d184 owner=smallclaims::store::known_writers_tests TODO=stage-generation-and-keyed-reclamation */ "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?2)
             INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash,
                 accepted_at_unix_ms, checkpoint)
             SELECT ?1, i + (SELECT COALESCE(MAX(sequence), 0) FROM checkpoint_envelopes
                             WHERE writer=?1),
                    printf('%064x', i), 0, 'invented-checkpoint'
             FROM n",
            params![writer, count],
        )
        .unwrap();
}

fn known_writers_work(store: &Store) -> (BTreeSet<String>, u64) {
    // The first read warms the membership fold, whose cost is not what this measures.
    store.checkpoint_known_writers(&[]).unwrap();
    let scope = SqliteWorkScope::start();
    let known = store
        .checkpoint_known_writers(&["invented-peer".into()])
        .unwrap();
    let work = scope.finish();
    assert!(
        work.statements > 0,
        "SQLite work instrumentation did not run"
    );
    (known, work.vm_steps)
}

#[test]
fn known_writers_seek_each_writer_however_many_envelopes_it_wrote() {
    let store = Store::open_memory("invented-origin", Arc::new(Plain)).unwrap();
    replica_envelopes(&store, "invented-alder", 10);
    replica_envelopes(&store, "invented-birch", 10);
    checkpoint_envelopes(&store, "invented-birch", 10);
    checkpoint_envelopes(&store, "invented-cedar", 10);
    let (small, small_steps) = known_writers_work(&store);
    let expected: BTreeSet<String> = [
        "invented-alder",
        "invented-birch",
        "invented-cedar",
        "invented-origin",
        "invented-peer",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert!(
        expected.is_subset(&small),
        "every envelope writer, tombstone writer, the node and its peers: {small:?}"
    );

    replica_envelopes(&store, "invented-alder", 5_000);
    replica_envelopes(&store, "invented-birch", 5_000);
    checkpoint_envelopes(&store, "invented-cedar", 5_000);
    let (large, large_steps) = known_writers_work(&store);
    assert_eq!(large, small);
    // A DISTINCT over the envelopes takes several steps per envelope: tens of thousands more
    // here. A seek per writer takes the same steps at either size.
    assert!(
        large_steps <= small_steps + 100,
        "known writers read every envelope: {small_steps} steps at 10 per writer, \
         {large_steps} at 5,010"
    );
}
