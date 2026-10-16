//! The guard keeps snapshot freshness work, and omits only full-summary assembly.
use super::*;
use crate::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::cell::Cell;

thread_local! {
    static CLONED: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

pub(super) fn record_summary_clones(buckets: usize, projections: usize) {
    CLONED.with(|c| {
        let (b, p) = c.get();
        c.set((b + buckets, p + projections));
    });
}

fn measured<T>(work: impl FnOnce() -> T) -> (T, SqliteWork, (usize, usize)) {
    let before = CLONED.with(Cell::get);
    let scope = SqliteWorkScope::start();
    let result = work();
    let sql = scope.finish();
    let after = CLONED.with(Cell::get);
    (result, sql, (after.0 - before.0, after.1 - before.1))
}

fn note(store: &Store, name: &str) -> crate::claim::ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: format!("note/{name}"), kind: "example.note".into(), actor: None,
        fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None,
        idempotency_key: None,
    }).unwrap()
}

fn parity(store: &Store) -> String {
    let compact = store.replication_authority_digest().unwrap();
    assert_eq!(compact, store.export_replication_summary("fixture").unwrap().authority_digest);
    compact
}

#[test]
// This is smallclaims physical-trim freshness, not Smalltalk checkpoint planning or
// desired/mission/document protected-reference coverage. The st3 companion exercises a
// real Smalltalk capture/plan separately; no application schema is fabricated here.
fn authority_digest_keeps_write_reopen_and_real_trim_freshness() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("authority.sqlite3");
    let store = Store::open(&path, "alder", Arc::new(runtime::Plain)).unwrap();
    let empty = parity(&store);
    let claim = note(&store, "first");
    let written = parity(&store);
    assert_ne!(empty, written, "local writes cannot be hidden by a stale guard digest");
    // Plain has no application projections for checkpoint planning's protected-reference
    // reads. This control tests trimming, not planning: derive the exact tombstones from
    // the claim just committed and its actual sealed envelope, without fake app tables.
    let envelope = store.readers.get().query_row(
        "SELECT writer, sequence, envelope_hash, accepted_at_unix_ms
         FROM replica_envelopes WHERE batch_id=?1",
        [&claim.batch_id], |row| Ok(EnvelopeTombstone {
            writer: row.get(0)?, sequence: row.get(1)?, envelope_hash: row.get(2)?,
            accepted_at_unix_ms: row.get::<_, String>(3)?.parse::<u128>().unwrap(),
        })).unwrap();
    let records: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM claims WHERE batch_id=?1", [&claim.batch_id], |r| r.get(0)).unwrap();
    assert_eq!(records, 1, "the disposable trim must drop only this note");
    // This note has no operation (no idempotency key), so both operation fields are
    // None, matching claim_tombstone's extraction from the body. This is not an
    // equivalence check for operation-bearing claims.
    let tombstone = ClaimTombstone {
        id: claim.id.clone(), writer: envelope.writer.clone(), sequence: envelope.sequence,
        envelope_hash: envelope.envelope_hash.clone(), subject: claim.subject.clone(),
        kind: claim.kind.clone(), actor: claim.actor.clone(), predecessors: claim.predecessors.clone(),
        operation_id: claim.operation_id.clone(), request_digest: claim.request_digest.clone(),
        accepted_at_unix_ms: claim.accepted_at_unix_ms,
    };
    store.trim_checkpoint("checkpoint/guard-fixture", now_ms() + 1, "fixture",
        &[envelope], &[tombstone], false, &mut Vec::new()).unwrap();
    assert_eq!(parity(&store), written, "a trim replaces payload with the same authority identities");
    assert_eq!(store.readers.get().query_row(
        "SELECT COUNT(*) FROM claims WHERE subject='note/first'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    drop(store);
    let store = Store::open(&path, "alder", Arc::new(runtime::Plain)).unwrap();
    assert_eq!(parity(&store), written);
    note(&store, "after-reopen");
    assert_ne!(parity(&store), written);
}

#[test]
fn authority_digest_preserves_snapshot_sql_and_removes_only_summary_queries_and_clones() {
    let store = Store::open_memory("alder", Arc::new(runtime::Plain)).unwrap();
    let mut previous_bucket_count = 0;
    for count in [1usize, 64] {
        // Real checkpoint identities on the same store make distinct writer buckets.
        // Populate and invalidate outside every measurement scope.
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            for writer in previous_bucket_count..count {
                for id in test_envelope_ids(&format!("retired-{writer}"), 0..1, "guard-fixture") {
                    tx.execute("INSERT INTO checkpoint_envelopes VALUES (?1,?2,?3,0,'checkpoint/fixture')",
                        params![id.writer, id.sequence, id.hash]).unwrap();
                }
            }
            tx.commit().unwrap();
        }
        store.replica_rows_changed();
        // Warm every complete path before scoped comparison, including reader setup.
        store.replication_snapshot().unwrap();
        store.replication_authority_digest().unwrap();
        store.export_replication_summary("fixture").unwrap();
        let (snapshot, shared_work, shared_copies) = measured(|| store.replication_snapshot().unwrap());
        let (digest, compact_work, compact_copies) = measured(|| store.replication_authority_digest().unwrap());
        let (full, full_work, full_copies) = measured(|| store.export_replication_summary("fixture").unwrap());
        eprintln!("buckets={} shared={shared_work:?} compact={compact_work:?} full={full_work:?} copies={full_copies:?}", snapshot.buckets.len());
        assert_eq!(digest, snapshot.authority_digest);
        assert_eq!(digest, full.authority_digest);
        assert!(shared_work.statements > 0, "retained snapshot work is actually observed");
        assert_eq!(compact_work, shared_work, "sealing, judging and snapshot SQL stays");
        assert_eq!(full_work.statements, shared_work.statements + 2,
            "only the signature-request and advertised-checkpoint queries disappear");
        assert_eq!(shared_copies, (0, 0));
        assert_eq!(compact_copies, (0, 0));
        assert_eq!(full_copies, (snapshot.buckets.len(), snapshot.projection_digests.len()));
        assert!(snapshot.buckets.len() >= count);
        previous_bucket_count = count;
    }
}

#[test]
fn authority_digest_propagates_snapshot_failure() {
    let store = Store::open_memory("alder", Arc::new(runtime::Plain)).unwrap();
    parity(&store);
    store.connection.write().execute_batch("DROP TABLE replica_envelopes").unwrap();
    assert!(store.replication_authority_digest().is_err());
    assert!(store.export_replication_summary("fixture").is_err());
}
