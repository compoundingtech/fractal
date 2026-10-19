#![allow(unused_imports)]
pub use st3_store::store::attention_ivm::tests_shared_seats::*;
use crate::store::attention_ivm::*;
use crate::store::custom::RegistrationRequest;
use smallclaims::ivm::{Readiness, SourceCut, events};


pub fn views(store: &Store) -> Views {
    let views = Views::new(definitions()).unwrap();
    views.create_schema(&store.connection.write()).unwrap();
    views
}

pub fn install(store: &Store, views: &Views) {
    store.project_replication_backlog().unwrap();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let index = smallclaims::store::current_index_tx(&tx).unwrap();
    views
        .initialize_empty(
            &tx,
            SourceCut {
                epoch: 1,
                admitted: index,
                projected: index,
                local_generation: 0,
            },
        )
        .unwrap();
    events::install(&tx, 1024).unwrap();
    for view in [PERSON_VIEW, CUSTOM_VIEW] {
        let mut after = String::new();
        loop {
            let keys = seed_page(&tx, view, &after, 2).unwrap();
            if keys.is_empty() {
                break;
            }
            after = keys.last().unwrap().clone();
        }
    }
    while !backfill_page(&tx, 0, 2).unwrap() {}
    tx.execute(/* sqlite-large-value: allow-for-now lv-h66de73468174 owner=st3_attention::store::attention_ivm::tests_shared_seats TODO=bound-or-reference-values-and-reclamation */ "UPDATE ivm_views SET ready=1", []).unwrap();
    tx.commit().unwrap();
}
