#![allow(unused_imports)]
use crate::store::tests_shared::{exchange_from, receive_and_project};
use crate::store::*;

pub fn agent_subjects(connection: &Connection, index: u64) -> Vec<String> {
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h9301b1ad6d72 owner=st3_store::store::card_fold_tests_shared TODO=trace-statement-and-value-representation */ RANGE_SUBJECTS)
        .unwrap()
        .query_map(params![index, "agent/", "agent0"], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

