use super::*;
use crate::sqlite::work::SqliteWorkScope;

fn store() -> Store {
    Store::open_memory("fixture", Arc::new(runtime::Plain)).unwrap()
}

fn claim(tx: &Transaction<'_>, operation: &str, digest: &str) -> ClaimRecord {
    append_claim_record_tx(tx, "peer", "note/scan", "example.note", None,
        &json!({"_operation":{"id":operation,"request_digest":digest}}), &[], None).unwrap()
}

fn tombstone(claim: &ClaimRecord) -> checkpoint::ClaimTombstone {
    checkpoint::ClaimTombstone {
        id: claim.id.clone(), writer: claim.origin.clone(), sequence: 1,
        envelope_hash: "fixture-envelope".into(), subject: claim.subject.clone(),
        kind: claim.kind.clone(), actor: None, predecessors: vec![],
        operation_id: claim.operation_id.clone(), request_digest: claim.request_digest.clone(),
        accepted_at_unix_ms: 0,
    }
}

fn trim(store: &Store, claim: &ClaimRecord) {
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    let dropped = [tombstone(claim)];
    checkpoint::record_checkpoint_tombstones_tx(&tx, "checkpoint/fixture", &[], &dropped).unwrap();
    checkpoint::delete_dropped_rows_tx(&tx, &[], &dropped).unwrap();
    tx.commit().unwrap();
}

fn fence(store: &Store, operation: &str) -> bool {
    store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate=?1)",
        [format!("projection:operation-conflict-fence:{operation}")], |r| r.get(0),
    ).unwrap()
}

#[test]
fn trim_preserves_discovery_work_and_later_conflict_is_found() {
    for count in [32, 2048] {
        let store = store();
        let first = {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            let first = claim(&tx, "op/trimmed", "a");
            for number in 1..count { claim(&tx, &format!("op/{number}"), "a"); }
            tx.commit().unwrap();
            first
        };
        let target = store.index().unwrap();
        assert!(store.discover_operation_conflicts(target, false).unwrap());
        let saved_generation = generation(&store.readers.get()).unwrap();
        let scope = SqliteWorkScope::start();
        assert!(store.discover_operation_conflicts(target, false).unwrap());
        let before = scope.finish();
        trim(&store, &first);
        assert_eq!(generation(&store.readers.get()).unwrap(), saved_generation);
        let scope = SqliteWorkScope::start();
        assert!(store.discover_operation_conflicts(target, false).unwrap());
        let after = scope.finish();
        assert_eq!(after, before, "trim must not add any history-read work: {count}");
        assert!(!fence(&store, "op/trimmed"));
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            claim(&tx, "op/trimmed", "b");
            tx.commit().unwrap();
        }
        assert!(store.discover_operation_conflicts(store.index().unwrap(), false).unwrap());
        assert!(fence(&store, "op/trimmed"), "the checkpoint side still participates");
    }
}

#[test]
fn removed_conflict_evidence_stays_conservative_without_hiding_a_later_conflict() {
    let store = store();
    let first = {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let first = claim(&tx, "op/conflict", "a");
        claim(&tx, "op/conflict", "b");
        tx.commit().unwrap();
        first
    };
    assert!(store.discover_operation_conflicts(store.index().unwrap(), false).unwrap());
    assert!(fence(&store, "op/conflict"));
    trim(&store, &first);
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-he65ca22f303e owner=smallclaims::store::operation_conflict_fence_tests TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_claims WHERE id=?1", [&first.id]).unwrap();
        assert!(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate='projection:operation-conflict-fence:op/conflict')",
            [], |r| r.get::<_, bool>(0),
        ).unwrap(), "deletion retains the conservative replay guard until explicit repair");
        // Explicit repair consumes conservative evidence, even after its sources retire.
        repaired(&tx).unwrap();
        tx.commit().unwrap();
    }
    assert!(!fence(&store, "op/conflict"));
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        claim(&tx, "op/conflict", "c");
        tx.commit().unwrap();
    }
    assert!(store.discover_operation_conflicts(store.index().unwrap(), false).unwrap());
    assert!(fence(&store, "op/conflict"));
}

#[test]
fn checkpoint_restore_and_prefix_reinsertion_invalidate_discovery() {
    let store = store();
    let first = {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let first = claim(&tx, "op/restored", "a");
        claim(&tx, "op/restored", "b");
        tx.commit().unwrap();
        first
    };
    let through = store.index().unwrap();
    assert!(store.discover_operation_conflicts(through, false).unwrap());
    trim(&store, &first);
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h44bbdca22607 owner=smallclaims::store::operation_conflict_fence_tests TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_claims WHERE id=?1", [&first.id]).unwrap();
    let before = generation(&tx).unwrap();
    checkpoint::record_checkpoint_tombstones_tx(&tx, "checkpoint/restore", &[], &[tombstone(&first)]).unwrap();
    assert_ne!(generation(&tx).unwrap(), before, "restoring checkpoint-only evidence invalidates");
    repaired(&tx).unwrap();
    let before = generation(&tx).unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h616de255ee50 owner=smallclaims::store::operation_conflict_fence_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(store_index,id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
        VALUES(1,?1,?2,?3,?4,?5,NULL,?6,'[]','0')",
        params![first.id,first.batch_id,first.subject,first.kind,first.origin,serde_json::to_string(&first.body).unwrap()],
    ).unwrap();
    assert_ne!(generation(&tx).unwrap(), before, "insertion below the frontier invalidates");
    assert!(!valid(&tx, through).unwrap());
    tx.commit().unwrap();
    drop(connection);
    assert!(store.discover_operation_conflicts(through, false).unwrap());
    assert!(fence(&store, "op/restored"));
}

#[test]
fn daemon_turn_bounds_discovery_and_resumes_without_another_receive() {
    let store = store();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for number in 0..(PROJECTION_CHUNK_CLAIMS * PROJECTION_TURN_CHUNKS + 1) {
            claim(&tx, &format!("op/{number}"), "a");
        }
        tx.commit().unwrap();
    }
    let target = store.index().unwrap();
    let scope = SqliteWorkScope::start();
    assert_eq!(store.project_replication_backlog_unless_catching_up().unwrap(), None);
    let work = scope.finish();
    assert_eq!(work.fullscan_steps, 0);
    let scanned: u64 = store.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='projection:operation-conflict-scan'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(scanned, (PROJECTION_CHUNK_CLAIMS * PROJECTION_TURN_CHUNKS) as u64);
    assert!(scanned < target);
    assert!(store.replication_projection_deferred());
    for _ in 0..4 {
        if store.project_replication_backlog_unless_catching_up().unwrap() == Some(true) { break; }
    }
    assert!(!store.replication_projection_deferred());
    assert!(valid(&store.readers.get(), target).unwrap());
}
