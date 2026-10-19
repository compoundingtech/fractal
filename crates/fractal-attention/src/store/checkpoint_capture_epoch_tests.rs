#![allow(unused_imports)]
use crate::AttentionStore;
use super::*;
use fractal_seats::store::checkpoint_capture_epoch_tests_shared_attention_tests::{capture_bounds, epoch, projected_store, publish};


#[test]
fn direct_desired_deletion_advances_capture_epoch_without_a_claim_append() {
    let store = projected_store();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h1376c4efa186 owner=fractal_attention::store::checkpoint_capture_epoch_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_run='mission-run/capture-owner' WHERE subject='agent/writer.capture'", [],
    ).unwrap();
    let index = store.index().unwrap();
    let before = epoch(&store);
    assert_eq!(store.discard_desired_owned_by("mission-run/capture-owner").unwrap(), 1);
    assert!(epoch(&store) > before);
    assert_eq!(store.index().unwrap(), index, "deletion unexpectedly appended a claim");
    assert_eq!(store.readers.get().query_row(
        "SELECT COUNT(*) FROM desired WHERE subject='agent/writer.capture'", [], |row| row.get::<_, u64>(0),
    ).unwrap(), 0);
}
