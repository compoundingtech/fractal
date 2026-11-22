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
