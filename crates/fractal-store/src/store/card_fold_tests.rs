//! The agent card fold reads many subjects per statement. Its answers must be the ones the
//! per-subject reductions give, at every snapshot, whatever order replication delivered the
//! claims in, after a rolled-back write and after a checkpoint trimmed history.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared_missions::{append, birch, declare, running, snapshots};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::{agent_subjects, assert_card_fold_parity, assert_card_reads_parity, assert_card_reads_parity_with, per_subject_card_status};

use super::*;

#[test]
fn card_fold_matches_across_chunk_boundaries() {
    let store = Store::open_memory("alder").unwrap();
    let source = (0..300)
        .map(|n| format!("agent \"seat-{n:03}\" {{ command \"true\" }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let intent = crate::graph::parse_intent(&format!("version 2\n{source}"), "alder").unwrap();
    declare(&store, &intent, "many-seats");
    for n in (0..300).step_by(2) {
        running(&store, &format!("agent/alder.seat-{n:03}"), &format!("seat-{n}"));
    }
    for n in 0..20 {
        running(&store, &format!("agent/stray-{n:02}"), &format!("stray-{n}"));
    }
    let head = store.index().unwrap();
    assert_card_fold_parity(&store, &[head]);
}

#[test]
fn newest_claim_reads_sort_only_the_newest_millisecond() {
    let store = Store::open_memory("alder").unwrap();
    let connection = store.readers.get();
    let plan = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-haacfae1ab071 owner=fractal_store::store::card_fold_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {}", card_fold::newest_claims_sql()))
        .unwrap()
        .query_map(params!["[]", 1], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("; ");
    assert!(plan.contains("USING INDEX claims_subject_accepted_index"), "{plan}");
    // A sort of only the order's last terms ends at the first newer time block.
    assert!(plan.contains("USE TEMP B-TREE FOR LAST"), "{plan}");
}

