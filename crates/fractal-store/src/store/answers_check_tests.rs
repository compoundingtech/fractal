//! The fold readers the daily answers check covers see a dropped claim that a trim keeping each
//! key's newest claim would not.
use super::*;

const SEAT: &str = "agent/alder.worker";

/// A seat whose harness reported `states` at the given times, in one incarnation.
fn seat(states: &[(u128, &str)]) -> Store {
    let store = Store::open_memory("alder").unwrap();
    for (at, state) in states {
        store.set_write_clock_at(*at).unwrap();
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            SEAT,
            "harness.observed",
            Some(SEAT),
            &json!({"fields": {"state": state, "driver": "codex", "incarnation_id": "inc-1"}}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    store
}

fn readers(store: &Store, cut: u128) -> BTreeMap<String, Vec<smallclaims::answers::AnswerKey>> {
    fold_reader_answers(&store.readers.get(), cut).unwrap()
}

#[test]
fn a_dropped_transition_changes_the_seat_readers_that_fold_it() {
    let history = [(1_000, "starting"), (2_000, "ready"), (3_000, "working"), (4_000, "idle")];
    let cut = 10_000;
    let full = readers(&seat(&history), cut);
    assert_eq!(full, readers(&seat(&history), cut), "the same claims answer the same");
    assert!(full["status_history"].iter().any(|key| key.subject == SEAT));

    // The newest claim of the seat's one key is the same idle report, but the working episode
    // is gone from the history a reader folds.
    let dropped = readers(&seat(&[(1_000, "starting"), (2_000, "ready"), (4_000, "idle")]), cut);
    let differing = full
        .keys()
        .chain(dropped.keys())
        .filter(|reader| full.get(*reader) != dropped.get(*reader))
        .collect::<BTreeSet<_>>();
    assert!(differing.contains(&"status_history".to_owned()), "{differing:?}");

    // A seat still written at the cut is left out until a later cut.
    let moving = readers(&seat(&history), 3_500);
    assert!(moving.values().flatten().all(|key| key.subject != SEAT), "{moving:?}");
}
