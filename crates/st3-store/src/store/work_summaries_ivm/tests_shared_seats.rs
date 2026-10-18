#![allow(unused_imports)]
use crate::store::work_summaries_ivm::*;
use smallclaims::ivm::{SourceCut, events};
use crate::store::attention_ivm::tests_shared_seats::*;
use crate::store::directive_notes::tests_shared_seats::*;
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;

pub fn register(store: &Store) -> Arc<Views> {
    let views = Arc::new(Views::new(definitions()).unwrap());
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    assert_eq!(current_index_tx(&tx).unwrap(), 0);
    views.create_schema(&tx).unwrap();
    views
        .initialize_empty(
            &tx,
            SourceCut {
                epoch: 1,
                admitted: 0,
                projected: 0,
                local_generation: 0,
            },
        )
        .unwrap();
    events::install(&tx, 1024).unwrap();
    tx.commit().unwrap();
    views
}

pub fn fixture() -> (Store, Arc<Views>) {
    let store = Store::open_memory("birch").unwrap();
    let views = register(&store);
    (store, views)
}

pub fn append(store: &Store, subject: &str, kind: &str, attempt: u32, summary: &str) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: Some("agent/sample/worker".into()),
            fields: BTreeMap::from([
                ("attempt".into(), json!(attempt)),
                ("summary".into(), json!(summary)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

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

pub fn advance(store: &Store, views: &Views, at: u128, limit: usize) -> usize {
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let count = clock_page(&tx, views, at, limit).unwrap();
    tx.commit().unwrap();
    count
}

pub fn parity(store: &Store, views: &Views, steps: &[StepRunView], at: u128) {
    capture(store, views);
    while advance(store, views, at, 2) != 0 {}
    store
        .read_snapshot(|_| {
            let connection = store.readers.get();
            let selected = steps
                .iter()
                .map(|step| (step.subject.clone(), step.attempt))
                .collect::<Vec<_>>();
            let batch = rows(&connection, views, &selected, at)?;
            for step in steps {
                let mut expected = step.clone();
                enrich_step_summaries_at(&connection, &mut expected, at)?;
                let expected = Summaries {
                    progress_summary: expected.progress_summary,
                    progress_at_unix_ms: expected.progress_at_unix_ms,
                    completion_summary: expected.completion_summary,
                };
                assert_eq!(
                    row(&connection, views, &step.subject, step.attempt, at)?,
                    expected
                );
                assert_eq!(batch[&(step.subject.clone(), step.attempt)], expected);
            }
            Ok(())
        })
        .unwrap();
}

pub struct ResetClock;

impl Drop for ResetClock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

pub fn freeze(at: u128) -> ResetClock {
    smallclaims::store::set_thread_clock(Some(at));
    ResetClock
}
