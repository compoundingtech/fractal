#![allow(unused_imports)]
pub use st3_store::store::work_summaries_ivm::tests_shared_seats::*;
use crate::store::work_summaries_ivm::*;
use smallclaims::ivm::{SourceCut, events};
use crate::store::attention_ivm::tests_shared_seats::*;
use crate::store::directive_notes::tests_shared_seats::*;
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;


// The batched writer runs on its own thread. A controlled accepted timestamp must be
// supplied on this transaction's thread instead of assuming that thread inherits the clock.
pub fn append_at(store: &Store, subject: &str, at: u128, summary: &str) -> ClaimRecord {
    let _reset = freeze(at);
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let claim = append_claim_tx(
        &tx,
        store.origin(),
        subject,
        "work.progress",
        Some("agent/sample/worker"),
        &json!({"fields":{"attempt":1,"summary":summary}}),
        &[],
        None,
    )
    .unwrap();
    assert_eq!(claim.accepted_at_unix_ms, at);
    tx.commit().unwrap();
    claim
}

// Test-only complete capture after a real Store action or a completed replication pass.
// Production must capture every admitted source transaction and certify the contiguous
// projection frontier; this helper is not a production MAX-index adapter.
pub fn capture(store: &Store, views: &Views) {
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let previous = source_cut(&tx).unwrap().unwrap();
    let records=tx.prepare("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE store_index>?1 ORDER BY store_index")
        .unwrap().query_map([previous.projected],claim_from_row).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    for claim in records {
        if views.reads_kind(&claim.kind) {
            let canonical = smallclaims::store::canonical::claim_key(&tx, &claim.id).unwrap();
            let changes = views
                .change(&tx, None, Some((&claim, &canonical)), previous.epoch)
                .unwrap();
            assert!(changes.deferred.is_empty());
        }
    }
    let index = current_index_tx(&tx).unwrap();
    views
        .publish_cut(
            &tx,
            SourceCut {
                admitted: index,
                projected: index,
                ..previous
            },
        )
        .unwrap();
    tx.commit().unwrap();
}
