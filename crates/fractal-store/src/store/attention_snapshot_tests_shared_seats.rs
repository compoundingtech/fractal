#![allow(unused_imports)]
use crate::store::*;
use crate::model::LoopSpec;
use crate::store::attention_snapshot::*;
use crate::model::{ClaimInput, PersonStepResponse};

pub fn step_back_in_time(store: &Store, subject: &str, accepted_at: &str) {
        store
            .connection
            .lock().expect("fixture writer")
            .execute(
                /* sqlite-large-value: allow-for-now lv-hbab360552e56 owner=st3_store::store::attention_snapshot_tests_shared_seats TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?2 WHERE subject=?1 AND kind='work.person-asked'",
                params![subject, accepted_at],
            )
            .unwrap();
    }

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
