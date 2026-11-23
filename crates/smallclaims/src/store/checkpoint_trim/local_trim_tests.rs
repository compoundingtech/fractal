use super::*;
use crate::store::{
    checkpoint_agreement::{CheckpointContext, CheckpointPhase, DAY_MS},
    runtime::Plain,
};
use std::sync::Arc;

fn context(scratch: &Path) -> CheckpointContext {
    CheckpointContext {
        now_unix_ms: 30 * DAY_MS,
        scratch: scratch.to_path_buf(),
    }
}

/// A plain store with the projection tables a local cut's capture reads for protected claims.
fn local_store() -> Store {
    let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h0e67e1bad682 owner=smallclaims::store::checkpoint_trim::local_trim_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS desired(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS mission_definitions(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS mission_revisions(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS documents(binding_claim_id TEXT);",
        )
        .unwrap();
    store
}

fn seals(store: &Store) -> i64 {
    store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claims WHERE kind LIKE 'checkpoint.%'", [], |row| row.get(0))
        .unwrap()
}

fn state(store: &Store, checkpoint: &str) -> String {
    store
        .readers
        .get()
        .query_row("SELECT state FROM checkpoints WHERE id=?1", [checkpoint], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_member_trims_its_own_cut_and_writes_no_checkpoint_claim() {
    let scratch = tempfile::tempdir().unwrap();
    // With trimming off, a step does nothing.
    let off = local_store();
    assert!(off.checkpoint_step(&context(scratch.path())).unwrap().is_empty());
    assert_eq!(off.trim_status().unwrap().worker.unwrap().phase, CheckpointPhase::Disabled);
    assert!(off.trimmed_checkpoint().unwrap().is_none());

    let local = local_store();
    local.trim_locally(true).unwrap();
    assert!(local.trims_locally());
    // It trims its own due cut, once, and writes no seal, verification or excusal.
    let first = local.checkpoint_step(&context(scratch.path())).unwrap();
    assert!(
        first.iter().all(|action| matches!(action, CheckpointAction::Trimmed { .. })),
        "{first:?}"
    );
    assert!(local.checkpoint_step(&context(scratch.path())).unwrap().is_empty());
    assert_eq!(seals(&local), 0);
    let status = local.trim_status().unwrap();
    assert_eq!(status.trimmed, Some(checkpoint_name(checkpoint::newest_due_cut(30 * DAY_MS))));
    assert!(!status.halted);
    assert_eq!(status.blocking_summary(), None);
}

#[test]
fn a_member_finishes_a_committed_trim_and_keeps_an_uncommitted_one() {
    let scratch = tempfile::tempdir().unwrap();
    let store = local_store();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h2c2af993af01 owner=smallclaims::store::checkpoint_trim::local_trim_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoints(id, cut_unix_ms, state, updated_at_unix_ms)
                 VALUES('checkpoint/committed', 1, 'trimming', 1),
                       ('checkpoint/uncommitted', 2, 'recording', 1);
             INSERT INTO checkpoint_claims(id, writer, sequence, envelope_hash, subject, kind,
                     predecessors, accepted_at_unix_ms, checkpoint)
                 VALUES('candidate', 'oak', 9, 'hash', 'note/one', 'example.note', '[]', 1,
                        'checkpoint/uncommitted');",
        )
        .unwrap();
    store.trim_locally(true).unwrap();
    let actions = store.checkpoint_step(&context(scratch.path())).unwrap();
    assert!(actions.iter().any(|action| matches!(
        action,
        CheckpointAction::Trimmed { checkpoint, .. } if checkpoint == "checkpoint/committed"
    )));
    assert_eq!(state(&store, "checkpoint/committed"), "trimmed");
    // A recording that never committed stays a labelled candidate; nothing converts it.
    assert_eq!(state(&store, "checkpoint/uncommitted"), "recording");
    assert!(checkpoint::claim_tombstoned(&store.readers.get(), "candidate").unwrap());
    assert_eq!(seals(&store), 0);
}

#[test]
fn trim_metadata_updates_in_place_and_skips_unchanged_daily_counts() {
    // Missing keys cover resuming a checkpoint recorded by an older build.
    for initialized in [false, true] {
        let store = local_store();
        let checkpoint = "checkpoint/metadata";
        store.set_checkpoint_state(checkpoint, 1, "trimming").unwrap();
        let claim = store
            .append_claim(&ClaimInput {
                subject: "note/metadata".into(),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let tombstone = deletable_claim(claim.id.clone(), claim.operation_id, claim.subject, claim.kind);
        let day = store.trim_day.load(Ordering::Relaxed);
        let walk_key = local_walk_key(checkpoint);
        let initial_count = if initialized { 7 } else { 0 };
        let original_walk_rowid = {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            for id in ["absent-a", "absent-b"] {
                let mut absent = tombstone.clone();
                absent.id = id.into();
                record_checkpoint_tombstones_tx(&tx, checkpoint, &[], &[absent]).unwrap();
            }
            record_checkpoint_tombstones_tx(&tx, checkpoint, &[], &[tombstone]).unwrap();
            if initialized {
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hcccb27bfbc0f owner=smallclaims::store::checkpoint_trim::local_trim_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)",
                    params![LOCAL_TRIM_DAY, format!("{day} {initial_count}")],
                )
                .unwrap();
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-ha6f9f42881ce owner=smallclaims::store::checkpoint_trim::local_trim_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,'0 0')",
                    [&walk_key],
                )
                .unwrap();
            }
            let rowid: Option<i64> = tx
                .query_row("SELECT rowid FROM meta WHERE key=?1", [&walk_key], |row| row.get(0))
                .optional()
                .unwrap();
            tx.execute_batch(
                /* sqlite-large-value: allow-for-now lv-h52f9c9da8bbe owner=smallclaims::store::checkpoint_trim::local_trim_tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE trim_meta_writes(key TEXT, operation TEXT);
                 CREATE TEMP TRIGGER trim_meta_insert AFTER INSERT ON main.meta
                 BEGIN INSERT INTO trim_meta_writes VALUES(NEW.key,'insert'); END;
                 CREATE TEMP TRIGGER trim_meta_update AFTER UPDATE ON main.meta
                 BEGIN INSERT INTO trim_meta_writes VALUES(NEW.key,'update'); END;",
            )
            .unwrap();
            tx.commit().unwrap();
            rowid
        };
        store.set_trim_chunk_envelopes(1);
        store.set_trim_fault(Some(TrimFault::AfterChunk(1)));
        let mut actions = Vec::new();
        assert!(store.finish_trim_paced(checkpoint, &mut actions, |_| {}).is_err());
        assert_eq!(state(&store, checkpoint), "trimming");
        assert_eq!(deleted_on(&store.readers.get(), day).unwrap(), initial_count);
        let saved = store.local_walk(checkpoint).unwrap();
        assert_eq!(saved.claim, 1);
        if let Some(rowid) = original_walk_rowid {
            let current: i64 = store
                .readers
                .get()
                .query_row("SELECT rowid FROM meta WHERE key=?1", [&walk_key], |row| row.get(0))
                .unwrap();
            assert_eq!(current, rowid, "updating the walk must not replace its row");
        }
        store.set_trim_fault(None);
        store.finish_trim_paced(checkpoint, &mut actions, |_| {}).unwrap();
        assert_eq!(state(&store, checkpoint), "trimmed");
        assert_eq!(deleted_on(&store.readers.get(), day).unwrap(), initial_count + 1);
        let reader = store.connection.write();
        let writes = |key: &str, operation: &str| -> i64 {
            reader
                .query_row(
                    "SELECT COUNT(*) FROM trim_meta_writes WHERE key=?1 AND operation=?2",
                    params![key, operation],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(writes(LOCAL_TRIM_DAY, "insert"), i64::from(!initialized));
        // Two cursor-only chunks do not update the daily count; the real delete does once.
        assert_eq!(writes(LOCAL_TRIM_DAY, "update"), 1);
        assert_eq!(writes(&walk_key, "insert"), i64::from(!initialized));
        assert_eq!(writes(&walk_key, "update"), if initialized { 3 } else { 2 });
        assert!(checkpoint::claim_tombstoned(&reader, &claim.id).unwrap());
        assert_eq!(
            reader
                .query_row("SELECT COUNT(*) FROM claims WHERE id=?1", [&claim.id], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
