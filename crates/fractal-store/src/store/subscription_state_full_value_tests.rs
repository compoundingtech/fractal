// subscription.state is a full-value family (fractal_schema::full_value): the canonical newest
// claim of each (subject, origin) is the whole value. Any trim that keeps those claims must leave
// every reader's answer unchanged, however the claims arrived. This simulates such a trim by
// deleting the other state claims, which is what local removal would do; nothing removes them
// in production yet.
const SUBJECTS: [&str; 2] = ["subscription/full-value/one", "subscription/full-value/two"];
const WRITERS: [&str; 3] = ["alder", "birch", "cedar"];
const STATES: [&str; 3] = ["pending", "active", "stopped"];

/// One writer's state claims: (time, subject, state, reason), at controlled accepted times.
fn state_source(writer: &str, writes: &[(u128, usize, usize, bool)]) -> Store {
    let source = Store::open_memory(writer).unwrap();
    for &(time, subject, state, reason) in writes {
        source.set_write_clock_at(time).unwrap();
        let mut connection = source.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_tx(
            &tx,
            &source.origin,
            SUBJECTS[subject],
            "subscription.state",
            None,
            &json!({"fields": {
                "state": STATES[state],
                "reason": reason.then_some("the delivery target is absent"),
            }}),
            &[],
            None,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    source
}

type Readings = Vec<(Option<(Value, u128)>, Option<(String, Option<u128>)>, Option<Value>)>;

/// What each reader answers for each subject: the list and detail state, attention's target
/// state, and the actual-state fold.
fn readings(store: &Store) -> Readings {
    store
        .read_snapshot(|index| {
            SUBJECTS
                .iter()
                .map(|subject| {
                    let connection = store.readers.get();
                    Ok((
                        store.observer_subscription_state(subject, "subscription.state", index)?,
                        super::attention::attention_target_state_tx(&connection, subject)?,
                        latest_actual_at(&connection, subject, Some(index))?,
                    ))
                })
                .collect()
        })
        .unwrap()
}

/// Every state claim that is not its (subject, origin)'s canonical newest.
fn superseded_states(store: &Store) -> Vec<String> {
    let connection = store.readers.get();
    let mut statement = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-h51c5a99f5584 owner=fractal_store::store::subscription_state_full_value_tests TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT claims.id, claims.subject, batches.origin FROM claims
             JOIN batches ON batches.id=claims.batch_id
             WHERE claims.kind='subscription.state' ORDER BY CANONICAL_DESC(claims)",
        ))
        .unwrap();
    let mut newest = std::collections::HashSet::new();
    statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .filter(|(_, subject, origin)| !newest.insert((subject.clone(), origin.clone())))
        .map(|(id, ..)| id)
        .collect()
}

fn remove_claims(store: &Store, ids: &[String]) {
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    for id in ids {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h386774c458da owner=fractal_store::store::subscription_state_full_value_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_positions WHERE claim_id=?1", [id]).unwrap();
        assert_eq!(tx.execute(/* sqlite-large-value: allow-for-now lv-hd52843b6d50d owner=fractal_store::store::subscription_state_full_value_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [id]).unwrap(), 1);
    }
    tx.commit().unwrap();
}

fn full_value_config() -> ProptestConfig {
    if std::env::var_os("ST_TEST_SCALE").is_some_and(|scale| scale == "full") {
        ProptestConfig::with_cases(32)
    } else {
        ProptestConfig {
            cases: 8,
            rng_seed: proptest::test_runner::RngSeed::Fixed(FULL_VALUE_SEED),
            ..ProptestConfig::default()
        }
    }
}

const FULL_VALUE_SEED: u64 = 7;

proptest! {
    #![proptest_config(full_value_config())]

    #[test]
    fn keeping_each_writers_newest_state_leaves_every_reader_unchanged(
        // Few distinct times, so writers often tie and the canonical tie-break decides.
        writes in proptest::collection::vec(
            (0usize..3, 1u64..6, 0usize..2, 0usize..3, any::<bool>()), 3..14),
        ranks in proptest::collection::vec(any::<u64>(), 16),
        width in 1usize..4,
        copies in 1usize..3,
        keep in proptest::collection::vec(any::<bool>(), 16),
    ) {
        let sources = WRITERS.iter().enumerate().map(|(writer, name)| {
            let mut own = writes.iter()
                .filter(|write| write.0 == writer)
                .map(|&(_, time, subject, state, reason)| (time as u128, subject, state, reason))
                .collect::<Vec<_>>();
            own.sort();
            state_source(name, &own)
        }).collect::<Vec<_>>();
        // Every writer's envelopes, delivered in a shuffled, chunked and duplicated order.
        let mut deliveries = sources.iter().zip(WRITERS)
            .flat_map(|(source, name)| {
                exchange_from(source, &ReplicationInventory::default()).envelopes
                    .into_iter().map(move |envelope| (name, envelope))
            })
            .collect::<Vec<_>>();
        let mut order = (0..deliveries.len()).collect::<Vec<_>>();
        order.sort_by_key(|&index| (ranks[index % ranks.len()], index));
        deliveries = order.into_iter().map(|index| deliveries[index].clone()).collect();
        let target = Store::open_memory("dune").unwrap();
        for chunk in deliveries.chunks(width) {
            for (writer, envelope) in chunk {
                let envelopes = std::iter::repeat_n(envelope.clone(), copies).collect();
                let admitted = receive_and_project(&target, writer, &exchange_of(writer, envelopes));
                prop_assert_eq!(admitted.invalid, 0);
            }
        }
        let before = readings(&target);
        // Remove an arbitrary subset of the superseded states, then all of them.
        let superseded = superseded_states(&target);
        // Each subject a writer wrote has an answer, so the comparison is not of absent values.
        for (subject, reading) in before.iter().enumerate() {
            if writes.iter().any(|write| write.2 == subject) {
                prop_assert!(reading.0.is_some() && reading.1.is_some() && reading.2.is_some(), "{reading:?}");
            }
        }
        let some = superseded.iter().zip(keep.iter().cycle())
            .filter(|(_, keep)| !**keep)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        remove_claims(&target, &some);
        prop_assert_eq!(&readings(&target), &before);
        remove_claims(&target, &superseded_states(&target));
        prop_assert_eq!(&readings(&target), &before);
    }
}

#[test]
fn removing_a_writers_newest_state_does_change_an_answer() {
    // The negative control: the property above is not vacuous. Removing the one newest state
    // that every reader chooses changes what they answer.
    let source = state_source("alder", &[(1, 0, 0, true), (2, 0, 1, false)]);
    let target = Store::open_memory("dune").unwrap();
    receive_and_project(&target, "alder", &exchange_from(&source, &ReplicationInventory::default()));
    let before = readings(&target);
    let state = |reading: &(Option<(Value, u128)>, Option<(String, Option<u128>)>, Option<Value>)| {
        (
            reading.0.as_ref().unwrap().0["fields"]["state"].clone(),
            reading.1.as_ref().unwrap().0.clone(),
            reading.2.as_ref().unwrap()["state"].clone(),
        )
    };
    assert_eq!(state(&before[0]), (json!("active"), "active".into(), json!("active")));
    let newest: String = target
        .readers
        .get()
        .query_row(
            "SELECT id FROM claims WHERE subject=?1 AND kind='subscription.state'
             AND json_extract(body, '$.fields.state')='active'",
            [SUBJECTS[0]],
            |row| row.get(0),
        )
        .unwrap();
    remove_claims(&target, &[newest]);
    // Every reader answers, and every reader's answer changes.
    let after = readings(&target);
    assert_eq!(state(&after[0]), (json!("pending"), "pending".into(), json!("pending")));
}
