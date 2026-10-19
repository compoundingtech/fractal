#![allow(unused_imports)]
pub use st3_store::store::work_summaries_ivm::tests_shared_seats::*;
use crate::store::work_summaries_ivm::*;
use smallclaims::ivm::{SourceCut, events};
use crate::store::attention_ivm::tests_shared_seats::*;
use st3_store::store::directive_notes::tests_shared_seats::*;
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
