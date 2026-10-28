//! Sealed-reader authority invalidation and bounded SQL work.
use super::*;
use crate::fleet::{FleetView, MemberKey};
use crate::sqlite::work::{SqliteWork, SqliteWorkScope};
use crate::store::checkpoint::{claim_tombstone, drop_digest};
use crate::store::runtime::Plain;
use std::sync::mpsc;
use std::time::Duration;

mod mutation_tests;
mod lifecycle_tests;
mod migration_tests;
mod index_cost_tests;

fn configure(store: &Store, history: usize) -> String {
    store.bind_fleet("sealed-cache-test").unwrap();
    let (key, _) = MemberKey::generate().unwrap();
    let public = key.public().to_owned();
    store.pin_fleet_anchor(&public).unwrap();
    store.set_member_key(Some(Arc::new(key))).unwrap();
    store
        .admit_fleet_anchor("sealed-cache-test", &public, "listening")
        .unwrap();
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        for index in 0..history {
            append_endpoint_tx(&tx, &public, &format!("https://anchor-{index}.invalid"));
        }
        tx.commit().unwrap();
    }
    store.seal_local_batches().unwrap();
    public
}

fn fixture(history: usize) -> (Store, String) {
    let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
    let public = configure(&store, history);
    (store, public)
}

fn append_endpoint_tx(tx: &Transaction<'_>, key: &str, url: &str) -> ClaimRecord {
    append_claim_record_tx(
        tx,
        "anchor",
        "host/anchor",
        "fleet.member-endpoints",
        None,
        &json!({"fields": {"member_key": key, "mode": "listening",
            "endpoints": [{"url": url}]}}),
        &[],
        None,
    )
    .unwrap()
}

fn append_endpoint(store: &Store, key: &str, url: &str) -> ClaimRecord {
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let claim = append_endpoint_tx(&tx, key, url);
    tx.commit().unwrap();
    claim
}

fn oracle(store: &Store) -> FleetView {
    FleetView::from_membership(&fleet_membership_tx(&store.readers.get()).unwrap())
}

fn refolded(store: &Store) -> FleetView {
    let current = store.fleet_view_sealed().unwrap();
    assert_eq!(current, oracle(store));
    assert_eq!(current, store.fleet_view_sealed().unwrap());
    current
}

#[test]
fn sealed_view_cache_hit_reuses_fold_with_one_generation_statement() {
    let (store, _) = fixture(8);
    let first = store.fleet_view_sealed().unwrap();
    assert_eq!(first.members.len(), 1);
    let snapshot_scope = SqliteWorkScope::start();
    store.read_snapshot(|_| Ok(())).unwrap();
    let snapshot_work = snapshot_scope.finish();
    let scope = SqliteWorkScope::start();
    let second = store.fleet_view_sealed().unwrap();
    let work = scope.finish();
    assert_eq!(first, second);
    assert_eq!(
        work.statements, snapshot_work.statements + 1,
        "warm read adds only fleet_generation to the pinned-snapshot overhead"
    );
    assert_eq!(work.fullscan_steps, snapshot_work.fullscan_steps);
    assert_eq!(work.sorts, 0);
}

#[test]
fn sealed_view_cache_warm_sql_budget_is_independent_of_endpoint_history() {
    let mut measurements: Vec<(SqliteWork, SqliteWork)> = Vec::new();
    for history in [4, 256] {
        let (store, _) = fixture(history);
        let first = store.fleet_view_sealed().unwrap();
        let reader = store.readers.get();
        let baseline_scope = SqliteWorkScope::start();
        let baseline = FleetView::from_membership(&fleet_membership_tx(&reader).unwrap());
        let baseline_work = baseline_scope.finish();
        drop(reader);
        assert_eq!(first, baseline);
        assert_eq!(
            baseline_work.statements, 2,
            "anchor lookup plus history fold"
        );
        let snapshot_scope = SqliteWorkScope::start();
        store.read_snapshot(|_| Ok(())).unwrap();
        let snapshot_work = snapshot_scope.finish();
        let warm_scope = SqliteWorkScope::start();
        for _ in 0..16 {
            assert_eq!(first, store.fleet_view_sealed().unwrap());
        }
        let warm_work = warm_scope.finish();
        assert_eq!(warm_work.statements, 16 * (snapshot_work.statements + 1));
        assert_eq!(warm_work.fullscan_steps, 16 * snapshot_work.fullscan_steps);
        assert_eq!(warm_work.sorts, 0);
        assert_eq!(warm_work.autoindex_rows, 0);
        measurements.push((baseline_work, warm_work));
    }
    // SQL statement counts alone hid the original regression: the same second statement
    // materialized every historical endpoint claim. VM steps expose that baseline cost.
    assert!(measurements[1].0.vm_steps > measurements[0].0.vm_steps);
    assert_eq!(measurements[0].1, measurements[1].1);
    println!(
        "endpoint_history=4/256 baseline_statements_per_read=2 warm_read=snapshot_overhead_plus_one_generation_statement warm_reads=16"
    );
}

#[test]
fn sealed_view_cache_miss_never_lends_writer_or_seals_local_claims() {
    let (store, key) = fixture(1);
    let pending = append_endpoint(&store, &key, "https://pending.invalid");
    let store = Arc::new(store);
    let writer = store.connection.write();
    let envelopes_before: i64 = writer
        .query_row("SELECT COUNT(*) FROM replica_envelopes", [], |row| {
            row.get(0)
        })
        .unwrap();
    let (send, receive) = mpsc::channel();
    let reading = store.clone();
    let worker = std::thread::spawn(move || {
        send.send(reading.fleet_view_sealed()).unwrap();
    });
    // Retain the actual writer loan until the read returns. Release it before asserting
    // so a regressed writer-taking implementation fails rather than hanging the suite.
    let result = receive.recv_timeout(Duration::from_secs(5));
    drop(writer);
    worker.join().unwrap();
    let view = result
        .expect("sealed cache miss waited for the writer")
        .unwrap();
    assert_eq!(view, oracle(&store));
    let reader = store.readers.get();
    let envelopes_after: i64 = reader
        .query_row("SELECT COUNT(*) FROM replica_envelopes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(envelopes_before, envelopes_after);
    let pending_is_sealed: bool = reader
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_envelopes WHERE batch_id=?1)",
            [&pending.batch_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!pending_is_sealed);
    assert_ne!(
        view.members[0].endpoints,
        vec![json!({"url": "https://pending.invalid"})]
    );
}

#[test]
fn sealed_view_cache_invalidates_on_fleet_claim_update() {
    let (store, key) = fixture(1);
    store.fleet_view_sealed().unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h139299d7fd57 owner=smallclaims::store::sealed_view_cache_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE kind='fleet.member-endpoints'",
            [json!({"fields": {"member_key": key, "mode": "listening",
            "endpoints": [{"url": "https://edited.invalid"}]}})
            .to_string()],
        )
        .unwrap();
    let changed = refolded(&store);
    assert_eq!(
        changed.members[0].endpoints,
        vec![json!({"url": "https://edited.invalid"})]
    );
}

#[test]
fn sealed_view_cache_invalidates_on_envelope_membership_change_and_delete() {
    let (store, _) = fixture(1);
    let first = store.fleet_view_sealed().unwrap();
    let (writer, sequence, hash, batch): (String, u64, String, String) = store
        .readers
        .get()
        .query_row(
            "SELECT e.writer,e.sequence,e.envelope_hash,e.batch_id FROM replica_envelopes e
            JOIN claims c ON c.batch_id=e.batch_id WHERE c.kind='fleet.member-endpoints' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h1125126c4903 owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET batch_id=NULL WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![writer, sequence, hash],
    ).unwrap();
    let detached = refolded(&store);
    assert!(detached.members[0].endpoints.is_empty());
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hc1834a5fefdb owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET batch_id=?4 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![writer, sequence, hash, batch],
    ).unwrap();
    let attached = refolded(&store);
    assert_eq!(attached, first);
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h6242e1454e88 owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
            params![writer, sequence, hash],
        )
        .unwrap();
    let deleted = refolded(&store);
    assert!(deleted.members[0].endpoints.is_empty());
}

#[test]
fn sealed_view_cache_invalidates_on_signature_insert_update_and_delete() {
    let (store, key) = fixture(1);
    let first = store.fleet_view_sealed().unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h299c9d664791 owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET member_key='untrusted-key' WHERE member_key=?1",
            [&key],
        )
        .unwrap();
    let untrusted = refolded(&store);
    assert!(untrusted.members.is_empty());
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-haa9dbff521a0 owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET member_key=?1 WHERE member_key='untrusted-key'",
            [&key],
        )
        .unwrap();
    let restored = refolded(&store);
    assert_eq!(restored, first);
    let signatures: Vec<(String, u64, String, String, String)> = store.readers.get()
        .prepare("SELECT writer,sequence,envelope_hash,member_key,signature FROM replica_envelope_signatures")
        .unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap();
    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h5ac6de1e48b8 owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures", [])
        .unwrap();
    let unsigned = refolded(&store);
    assert!(unsigned.members.is_empty());
    {
        let writer = store.connection.write();
        for (origin, sequence, hash, signer, signature) in signatures {
            writer.execute(/* sqlite-large-value: allow-for-now lv-hc901cd51dd6c owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures(writer,sequence,envelope_hash,member_key,signature,stored_at_unix_ms)
                VALUES(?1,?2,?3,?4,?5,?6)", params![origin, sequence, hash, signer, signature, now_ms().to_string()]).unwrap();
        }
    }
    assert_eq!(refolded(&store), first);
}

#[test]
fn sealed_view_cache_invalidates_on_anchor_change() {
    let (store, key) = fixture(1);
    let first = store.fleet_view_sealed().unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hfbdd4e5f5dfe owner=smallclaims::store::sealed_view_cache_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='different-anchor' WHERE key='fleet_anchor_key'",
            [],
        )
        .unwrap();
    let changed = refolded(&store);
    assert_eq!(changed.anchor.as_deref(), Some("different-anchor"));
    assert!(changed.members.is_empty());
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h511e73a5f92f owner=smallclaims::store::sealed_view_cache_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value=?1 WHERE key='fleet_anchor_key'",
            [&key],
        )
        .unwrap();
    assert_eq!(refolded(&store), first);
}

#[test]
fn sealed_view_cache_local_claim_invalidates_but_is_invisible_until_sealed() {
    let (store, key) = fixture(1);
    let first = store.fleet_view_sealed().unwrap();
    append_endpoint(&store, &key, "https://next.invalid");
    let unsealed = refolded(&store);
    assert_eq!(unsealed, first);
    store.seal_local_batches().unwrap();
    let sealed = refolded(&store);
    assert_eq!(
        sealed.members[0].endpoints,
        vec![json!({"url": "https://next.invalid"})]
    );
}

#[test]
fn sealed_view_cache_invalidates_after_actual_checkpoint_trim() {
    let (store, _) = fixture(1);
    // Plain has no domain projections; sealed-set capture consults these retention roots.
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-he041f7aad05f owner=smallclaims::store::sealed_view_cache_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS desired(claim_id TEXT);
         CREATE TABLE IF NOT EXISTS mission_definitions(claim_id TEXT);
         CREATE TABLE IF NOT EXISTS mission_revisions(claim_id TEXT);
         CREATE TABLE IF NOT EXISTS documents(binding_claim_id TEXT);",
        )
        .unwrap();
    store.fleet_view_sealed().unwrap();
    let cut = now_ms() + 60_000;
    let capture = store.checkpoint_sealed_set(cut).unwrap();
    let dropped = capture
        .claims
        .iter()
        .find(|claim| claim.claim.kind == "fleet.member-endpoints")
        .unwrap();
    let claim_id = dropped.claim.id.clone();
    let tombstone = claim_tombstone(dropped);
    drop(capture);
    let digest = drop_digest(&[], std::slice::from_ref(&tombstone));
    let mut actions = Vec::new();
    store
        .trim_checkpoint(
            "checkpoint/sealed-cache",
            cut,
            &digest,
            &[],
            &[tombstone],
            false,
            &mut actions,
        )
        .unwrap();
    let exists: bool = store
        .readers
        .get()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)",
            [&claim_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !exists,
        "test must exercise deletion, not merely checkpoint recording"
    );
    let trimmed = refolded(&store);
    assert_eq!(trimmed.members.len(), 1);
    assert!(trimmed.members[0].endpoints.is_empty());
    assert!(matches!(
        actions.last(),
        Some(checkpoint_agreement::CheckpointAction::Trimmed { .. })
    ));
}

#[test]
fn sealed_view_cache_generation_and_fold_share_the_pinned_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            &dir.path().join("claims.sqlite3"),
            "anchor",
            Arc::new(Plain),
        )
        .unwrap(),
    );
    let key = configure(&store, 1);
    let first = store.fleet_view_sealed().unwrap();
    store
        .read_snapshot(|_| {
            let changing = store.clone();
            let key = key.clone();
            let newer = std::thread::spawn(move || {
                append_endpoint(&changing, &key, "https://new-snapshot.invalid");
                changing.seal_local_batches().unwrap();
                changing.fleet_view_sealed().unwrap()
            })
            .join()
            .unwrap();
            assert_ne!(newer, first);
            // The global cache now contains the newer generation. A miss inside the old
            // read transaction must fold old facts and key them with that same old generation.
            let pinned = store.fleet_view_sealed().unwrap();
            assert_eq!(pinned, first);
            assert_ne!(pinned, newer);
            assert_eq!(pinned, oracle(&store));
            assert_eq!(pinned, store.fleet_view_sealed().unwrap());
            Ok(())
        })
        .unwrap();
    let latest = store.fleet_view_sealed().unwrap();
    assert_eq!(
        latest.members[0].endpoints,
        vec![json!({"url": "https://new-snapshot.invalid"})]
    );
    assert_eq!(latest, oracle(&store));
}

#[test]
fn sealed_view_cache_trigger_predicates_seek_the_covering_batch_kind_index() {
    let (store, _) = fixture(1);
    let reader = store.readers.get();
    let mut statements = reader.prepare(
        "SELECT name,sql FROM sqlite_schema WHERE type='trigger'
         AND (name LIKE 'fleet_generation_batch_%'
              OR name LIKE 'fleet_generation_envelope_%'
              OR name LIKE 'fleet_generation_signature_%')",
    ).unwrap();
    let triggers = statements.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(triggers.len(), 9);
    for (name, sql) in triggers {
        // Explain the installed trigger's actual WHEN expression, not a copy of its query.
        let predicate = sql.split_once("WHEN ").unwrap().1.split_once(" BEGIN ").unwrap().0;
        let values = "(SELECT 'batch' AS id, 'batch' AS batch_id, 'writer' AS writer,
                       1 AS sequence, 'hash' AS envelope_hash)";
        let explain = format!("EXPLAIN QUERY PLAN SELECT ({predicate})
                              FROM {values} AS NEW CROSS JOIN {values} AS OLD");
        let plan = reader.prepare(/* sqlite-large-value: allow-for-now lv-hb2709eca1246 owner=smallclaims::store::sealed_view_cache_tests TODO=trace-statement-and-value-representation */ &explain).unwrap().query_map([], |row| row.get::<_, String>(3))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(plan.iter().any(|detail| detail.contains("COVERING INDEX claims_batch_kind")),
            "{name}: {plan:?}");
        assert!(!plan.iter().any(|detail| detail.contains("SCAN claims")),
            "{name}: {plan:?}");
    }
}
