#![allow(unused_imports)]
use crate::store::tests_shared::{exchange_from, receive_and_project};
use crate::store::*;

pub fn agent_subjects(connection: &Connection, index: u64) -> Vec<String> {
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hd14af6227748 owner=fractal_store::store::card_fold_tests_shared TODO=trace-statement-and-value-representation */ RANGE_SUBJECTS)
        .unwrap()
        .query_map(params![index, "agent/", "agent0"], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// The agent card status pass as it was before the fold read subjects a chunk at a time:
/// one runtime view entry and one card reduction per subject.
pub fn per_subject_card_status(
    connection: &Connection,
    names: &[String],
    index: u64,
    history: bool,
) -> Vec<Value> {
    names
        .iter()
        .filter(|name| {
            history
                || !runtime_view_entry(connection, name, index, true)
                    .unwrap()
                    .history
        })
        .map(|name| {
            subject_status_at(
                connection, name, Some(index), None, ClaimsReduction::AgentCard,
            )
            .unwrap()
            .unwrap()
            .0
        })
        .filter(|status| history || status.projection.layer == "current")
        .map(|status| serde_json::to_value(status).unwrap())
        .collect()
}

/// Assert that the chunked fold and the per-subject reductions agree on `store` at `indexes`:
/// each runtime view entry, each card reduction, and the cold roster status pass.
pub fn assert_card_fold_parity(store: &Store, indexes: &[u64]) {
    for &index in indexes {
        let connection = store.readers.get();
        let names = agent_subjects(&connection, index);
        for owners in [false, true] {
            let mut reads = card_fold::CardReads::new(index);
            let mut entries = store
                .runtime_view_entries(&connection, &names, index, owners, &mut reads)
                .unwrap();
            for name in &names {
                let one = runtime_view_entry(&connection, name, index, owners).unwrap();
                let many = entries.remove(name).unwrap();
                assert_eq!(
                    (many.head, many.history, many.declared, many.owners),
                    (one.head, one.history, one.declared, one.owners),
                    "view entry of {name} at {index}, owners={owners}"
                );
            }
            // The status pass shares what the view pass read, as the roster does.
            let mut statuses = store
                .agent_card_statuses(&connection, &names, index, &mut reads)
                .unwrap();
            for name in &names {
                let (status, action) = subject_status_at(
                    &connection, name, Some(index), None, ClaimsReduction::AgentCard,
                )
                .unwrap()
                .unwrap();
                let many = statuses.remove(name).unwrap();
                assert_eq!(
                    serde_json::to_value(&many).unwrap(),
                    serde_json::to_value((status, action)).unwrap(),
                    "card status of {name} at {index}"
                );
            }
        }
        drop(connection);
        assert_card_reads_parity(store, &names, index);
        let connection = store.readers.get();
        for history in [false, true] {
            let expected = per_subject_card_status(&connection, &names, index, history);
            store.forget_current_views();
            let cold = store.agent_card_status_at(None, index, history).unwrap();
            assert_eq!(cold.store_index, index);
            let cold = cold
                .subjects
                .iter()
                .map(|status| serde_json::to_value(status).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(cold, expected, "cold roster status at {index}, history={history}");
        }
    }
}

/// The roster's per-card reads of `names` at `index` against the one-agent readers they replace.
pub fn assert_card_reads_parity(store: &Store, names: &[String], index: u64) {
    // With each agent's selected actual claim, as the roster passes them, and without.
    let connection = store.readers.get();
    let selected = names
        .iter()
        .map(|name| {
            let (status, _) = subject_status_at(
                &connection, name, Some(index), None, ClaimsReduction::AgentCard,
            )
            .unwrap()
            .unwrap();
            (name.clone(), status.actual_claim)
        })
        .collect::<HashMap<_, _>>();
    drop(connection);
    for actual_claims in [selected, HashMap::new()] {
        assert_card_reads_parity_with(store, names, index, &actual_claims);
    }
}

pub fn assert_card_reads_parity_with(
    store: &Store,
    names: &[String],
    index: u64,
    actual_claims: &HashMap<String, Option<String>>,
) {
    let mut reads = store.agent_card_reads(names, index, actual_claims).unwrap();
    for name in names {
        let one = store.observed_harness_at(name, index).unwrap();
        let many = reads.take_harness(store, name).unwrap();
        assert_eq!(
            serde_json::to_value(&many).unwrap(),
            serde_json::to_value(&one).unwrap(),
            "harness of {name} at {index}"
        );
        let incarnation = one.as_ref().map(|harness| harness.incarnation_id.as_str());
        assert_eq!(
            reads.last_activity_at(store, name, incarnation).unwrap(),
            store.agent_last_activity_at(name, incarnation, index).unwrap(),
            "last activity of {name} at {index}"
        );
        if !reads.may_have_suspension(name) {
            assert!(crate::suspension::current(store, name).unwrap().is_none(), "{name}");
        }
        if !reads.may_have_rollout(name) {
            assert_eq!(crate::rollout::status(store, name).unwrap(), None, "{name}");
        }
    }
}
