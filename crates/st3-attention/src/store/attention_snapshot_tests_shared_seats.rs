#![allow(unused_imports)]
pub use st3_store::store::attention_snapshot_tests_shared_seats::*;
use crate::store::*;
use crate::model::LoopSpec;
use crate::store::attention_snapshot::*;
use crate::model::{ClaimInput, PersonStepResponse};


/// The collection snapshot's visible rows must equal a direct read at the same cut, in
    /// the same order, for every person and boundary.
    pub fn parity(store: &Store, person: Option<&str>, now: u128) {
        let (rows, _) = store.attention_collection_snapshot(person, now).unwrap();
        let direct = store.attention_snapshot(person, now).unwrap();
        let keys = |items: &[AttentionItemView]| {
            items
                .iter()
                .map(|item| {
                    (
                        item.subject.clone(),
                        item.episode.clone(),
                        item.requested_at_unix_ms,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&rows), keys(&direct), "{person:?} at {now}");
    }
