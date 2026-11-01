use super::*;
use crate::store::{
    checkpoint_agreement::{CHECKPOINT_SEALED, CheckpointContext, DAY_MS},
    runtime::Plain,
};
use std::sync::Arc;

fn context(scratch: &Path) -> CheckpointContext {
    CheckpointContext {
        now_unix_ms: 30 * DAY_MS,
        configured_peers: Vec::new(),
        scratch: scratch.to_path_buf(),
        reviewer: "person/example".into(),
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
        .query_row("SELECT COUNT(*) FROM claims WHERE kind=?1", [CHECKPOINT_SEALED], |row| row.get(0))
        .unwrap()
}

fn state(store: &Store, checkpoint: &str) -> String {
    store
        .readers
        .get()
        .query_row("SELECT state FROM checkpoints WHERE id=?1", [checkpoint], |row| row.get(0))
        .unwrap()
}

fn manifest() -> CheckpointManifest {
    CheckpointManifest {
        checkpoint: "checkpoint/1970-01-29".into(),
        cut_unix_ms: 28 * DAY_MS,
        envelopes: Vec::new(),
        claims: Vec::new(),
    }
}

#[test]
fn a_member_that_trims_locally_seals_verifies_and_adopts_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    // Control: an ordinary member seals the due checkpoint and takes up a manifest.
    let ordinary = Store::open_memory("writer", Arc::new(Plain)).unwrap();
    let actions = ordinary.checkpoint_step(&context(scratch.path())).unwrap();
    assert!(actions.iter().any(|action| matches!(action, CheckpointAction::Sealed { .. })));
    assert_eq!(seals(&ordinary), 1);
    let refused = ordinary.adopt_checkpoint(&manifest()).err().map(|error| error.code);
    assert_ne!(refused, Some("trim-is-local"));

    let local = local_store();
    local.trim_locally(true).unwrap();
    assert!(local.trims_locally());
    // It trims its own due cut, once, and never seals, verifies or asks for a manifest.
    let first = local.checkpoint_step(&context(scratch.path())).unwrap();
    assert!(
        first.iter().all(|action| matches!(action, CheckpointAction::Trimmed { .. })),
        "{first:?}"
    );
    assert!(local.checkpoint_step(&context(scratch.path())).unwrap().is_empty());
    assert_eq!(seals(&local), 0);
    assert!(local.checkpoint_manifest_need().unwrap().is_none());
    assert_eq!(local.adopt_checkpoint(&manifest()).unwrap_err().code, "trim-is-local");
}

#[test]
fn a_member_that_trims_locally_finishes_a_committed_trim_and_keeps_an_uncommitted_one() {
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
