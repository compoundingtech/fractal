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

