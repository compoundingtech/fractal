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

#[test]
fn runtime_membership_decodes_only_the_status_it_needs_as_history_grows() {
    let store = Store::open_memory("alder").unwrap();
    let intent = crate::graph::parse_intent(
        "version 2\nagent \"declared\" { command \"true\" }", "alder",
    ).unwrap();
    declare(&store, &intent, "declared-seat");
    let subjects = ["agent/alder.declared", "agent/stray-live", "agent/stray-stopped"]
        .map(str::to_owned);
    let mut written = 0;
    for history in [8, 80] {
        for sequence in written..history {
            for subject in &subjects {
                running(&store, subject, &format!("incarnation-{sequence}"));
            }
        }
        written = history;
        append(&store, &subjects[2], "runtime.observed", Some(&subjects[2]),
            json!({"status": "stopped", "incarnation_id": format!("incarnation-{}", history - 1)}));
        let index = store.index().unwrap();
        let connection = store.readers.get();
        let expected = subjects.iter().map(|subject| (
            subject.clone(), runtime_view_entry(&connection, subject, index, true).unwrap(),
        )).collect::<HashMap<_, _>>();

        ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(0));
        let mut full = card_fold::CardReads::new(index);
        store.agent_card_statuses(&connection, &subjects, index, &mut full).unwrap();
        let full_bodies = ACTUAL_HISTORY_BODIES_DECODED.with(std::cell::Cell::get);
        assert!(full_bodies >= history * subjects.len(), "{full_bodies}");

        ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(0));
        let mut reads = card_fold::CardReads::new(index);
        let views = store.runtime_view_entries(
            &connection, &subjects, index, true, &mut reads,
        ).unwrap();
        let membership_bodies = ACTUAL_HISTORY_BODIES_DECODED.with(std::cell::Cell::get);
        assert_eq!(membership_bodies, 2, "history={history}; full={full_bodies}");
        for subject in &subjects {
            let (actual, expected) = (&views[subject], &expected[subject]);
            assert_eq!((actual.head, actual.history, actual.declared, &actual.owners),
                (expected.head, expected.history, expected.declared, &expected.owners), "{subject}");
        }
        assert!(!views[&subjects[0]].history);
        assert!(!views[&subjects[1]].history);
        assert!(views[&subjects[2]].history);
    }
}

