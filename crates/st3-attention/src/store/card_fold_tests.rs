#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::agent_subjects;
use st3_store::store::tests_shared::{exchange_from, receive_and_project};
use super::*;
use st3_store::store::card_fold_tests_shared_attention_tests::{alder, append, birch, declare, owning_run, running, snapshots};


#[test]
fn card_fold_matches_per_subject_reductions_in_any_replication_order() {
    let alder = alder();
    let birch = birch();
    assert_card_fold_parity(&alder, &snapshots(&alder));
    // The fixture reaches every per-card read it compares.
    let index = alder.index().unwrap();
    let names = ["agent/alder.plain", "agent/alder.halted"].map(str::to_owned);
    let mut reads = alder.agent_card_reads(&names, index, &HashMap::new()).unwrap();
    let plain = reads.take_harness(&alder, &names[0]).unwrap().unwrap();
    assert!(reads.last_activity_at(&alder, &names[0], Some(&plain.incarnation_id)).unwrap().is_some());
    assert!(reads.may_have_suspension(&names[0]));
    assert!(crate::suspension::current(&alder, &names[0]).unwrap().is_some());
    assert_eq!(reads.take_harness(&alder, &names[1]).unwrap().unwrap().state, "indeterminate");
    let from_alder = exchange_from(&alder, &ReplicationInventory::default());
    let from_birch = exchange_from(&birch, &ReplicationInventory::default());
    assert!(from_alder.envelopes.len() >= 3, "a meaningful envelope permutation");
    for (reverse, birch_first) in [(false, false), (true, false), (false, true), (true, true)] {
        let target = Store::open_memory("cedar").unwrap();
        let mut from_alder = from_alder.clone();
        if reverse {
            from_alder.envelopes.reverse();
        }
        if birch_first {
            receive_and_project(&target, "birch", &from_birch);
        }
        receive_and_project(&target, "alder", &from_alder);
        if !birch_first {
            receive_and_project(&target, "birch", &from_birch);
        }
        // A duplicate delivery changes nothing.
        receive_and_project(&target, "alder", &from_alder);
        assert_card_fold_parity(&target, &snapshots(&target));
        let index = target.index().unwrap();
        let status = target.agent_card_status_at(None, index, true).unwrap();
        let plain = status.subjects.iter().find(|s| s.subject == "agent/alder.plain").unwrap();
        assert!(plain.actual_origin.is_some(), "the rival origins were both read");
    }
}

fn card_statuses(store: &Store) -> Value {
    let index = store.index().unwrap();
    serde_json::to_value(&store.agent_card_status_at(None, index, true).unwrap().subjects).unwrap()
}

/// Both writers' envelopes, and the cards a store that received them in order reads.
fn card_fold_fixture() -> &'static (Vec<ReplicaEnvelope>, Value) {
    static FIXTURE: std::sync::OnceLock<(Vec<ReplicaEnvelope>, Value)> = std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        let mut envelopes = exchange_from(&alder(), &ReplicationInventory::default()).envelopes;
        envelopes.extend(exchange_from(&birch(), &ReplicationInventory::default()).envelopes);
        let reference = Store::open_memory("cedar").unwrap();
        receive_and_project(&reference, "relay", &crate::store::tests_shared_seats::exchange_of("relay", envelopes.clone()));
        (envelopes, card_statuses(&reference))
    })
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(16))]

    #[test]
    fn card_fold_is_pure_under_shuffled_split_duplicate_delivery_and_reopen(
        ranks in proptest::collection::vec(proptest::prelude::any::<u64>(), 64),
        width in 1usize..5,
        copies in 1usize..3,
    ) {
        let (envelopes, expected) = card_fold_fixture();
        proptest::prop_assert!(envelopes.len() <= ranks.len());
        let mut order = (0..envelopes.len()).collect::<Vec<_>>();
        order.sort_by_key(|&index| (ranks[index], index));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cards.sqlite3");
        let target = Store::open(&path, "dogwood").unwrap();
        for chunk in order.chunks(width) {
            let batch = chunk.iter().flat_map(|&index| std::iter::repeat_n(envelopes[index].clone(), copies)).collect();
            receive_and_project(&target, "relay", &crate::store::tests_shared_seats::exchange_of("relay", batch));
            // Each read warms the views the next delivery must change.
            card_statuses(&target);
        }
        assert_card_fold_parity(&target, &[target.index().unwrap()]);
        proptest::prop_assert_eq!(&card_statuses(&target), expected);
        drop(target);
        let target = Store::open(&path, "dogwood").unwrap();
        proptest::prop_assert_eq!(&card_statuses(&target), expected);
    }
}

#[test]
fn card_fold_matches_after_a_rolled_back_write() {
    let store = alder();
    let index = store.index().unwrap();
    let before = store.agent_card_status_at(None, index, true).unwrap();
    {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        append_claim_tx(
            &transaction,
            "alder",
            "agent/drifter",
            "runtime.observed",
            Some("agent/drifter"),
            &json!({"fields": {"status": "stopped", "runtime_id": "agent/drifter",
                               "incarnation_id": "drifter-1"}}),
            &[],
            None,
        )
        .unwrap();
        transaction.rollback().unwrap();
    }
    assert_eq!(store.index().unwrap(), index);
    assert_card_fold_parity(&store, &snapshots(&store));
    store.forget_current_views();
    let after = store.agent_card_status_at(None, index, true).unwrap();
    assert_eq!(
        serde_json::to_value(&after.subjects).unwrap(),
        serde_json::to_value(&before.subjects).unwrap()
    );
}

#[test]
fn card_fold_marks_unknown_kinds_and_conflicting_operations_as_the_reduction_does() {
    let store = alder();
    // Two hosts used one idempotency key for different restarts of the plain seat.
    let restart = |store: &Store, incarnation: &str| {
        store
            .append_claim(&ClaimInput {
                subject: "agent/alder.plain".into(),
                kind: "runtime.observed".into(),
                actor: Some("agent/alder.plain".into()),
                fields: serde_json::from_value(json!({"status": "running",
                    "runtime_id": "agent/alder.plain", "incarnation_id": incarnation}))
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some("plain-restart".into()),
            })
            .unwrap()
    };
    restart(&store, "plain-2");
    let birch = Store::open_memory("birch").unwrap();
    restart(&birch, "plain-birch-2");
    store.import_replication("birch", &birch.export_replication(0).unwrap()).unwrap();
    // A newer member's claim kind this build does not know, about an undeclared seat.
    let cypress = Store::open_memory("cypress").unwrap();
    running(&cypress, "agent/drifter", "drifter-cypress");
    let mut batch = cypress.export_replication(0).unwrap();
    let claim = &mut batch.batches[0].claims[0];
    claim.kind = "future.seat-note".into();
    claim.id = claim_hash(
        &claim.batch_id,
        &claim.subject,
        &claim.kind,
        &claim.origin,
        claim.actor.as_deref(),
        &claim.body,
        &claim.predecessors,
    )
    .unwrap();
    store.import_replication("cypress", &batch).unwrap();
    assert_card_fold_parity(&store, &snapshots(&store));
    let index = store.index().unwrap();
    let status = store.agent_card_status_at(None, index, true).unwrap();
    for subject in ["agent/alder.plain", "agent/drifter"] {
        let card = status.subjects.iter().find(|s| s.subject == subject).unwrap();
        assert_eq!(card.reachability, "indeterminate", "{subject}");
    }
}

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
        .prepare(/* sqlite-large-value: allow-for-now lv-hea3ec003665c owner=st3_attention::store::card_fold_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {}", card_fold::newest_claims_sql()))
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
fn card_fold_orders_claims_that_tie_in_one_batch_by_their_position() {
    let store = alder();
    // Put the retired seat's running and stopped observations in one batch at one time, so
    // only their positions in the batch, and then their ids, order them.
    {
        let connection = store.connection.write();
        let (first_batch, accepted) = connection
            .query_row(
                "SELECT batch_id, accepted_at_unix_ms FROM claims
                 WHERE subject='agent/retired' AND kind='runtime.observed' ORDER BY store_index LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h9f809d41c4d7 owner=st3_attention::store::card_fold_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET batch_id=?1, accepted_at_unix_ms=?2
                 WHERE subject='agent/retired' AND kind='runtime.observed'",
                params![first_batch, accepted],
            )
            .unwrap();
    }
    store.forget_current_views();
    let index = store.index().unwrap();
    assert_card_fold_parity(&store, &[index]);
    let status = store.agent_card_status_at(None, index, true).unwrap();
    let retired = status.subjects.iter().find(|s| s.subject == "agent/retired").unwrap();
    assert_eq!(
        retired.actual.as_ref().unwrap()["status"],
        latest_actual_at(&store.readers.get(), "agent/retired", Some(index)).unwrap().unwrap()["status"]
    );
}
