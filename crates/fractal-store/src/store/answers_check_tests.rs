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
    assert!(moving.values().flatten().all(|key| key.subject != SEAT || key.moving), "{moving:?}");
}

/// Independent histories can retain different superseded writers without changing a reader.
fn retained_history_case(kind: &str) {
    use smallclaims::answers::{differing_kinds, reader_answers};
        let store = Store::open_memory("alder").unwrap();
        let subject = match kind {
            "observer.observed" => "observer/example",
            "resource.observed" => "resource/example/issue/1502",
            _ => "host/example",
        };
        let mut ids = Vec::new();
        for (at, writer, state) in [(100, "old", "idle"), (200, "new", "idle")] {
            let writer = if matches!(kind, "observer.observed" | "resource.observed") { writer } else { "new" };
            store.set_write_clock_at(at).unwrap();
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            let claim = append_claim_tx(
                &transaction, writer, subject, kind, None,
                &json!({"fields": {"state": state, "code": "example", "observer": "observer/example",
                    "incarnation_id": "one", "semantics": "session_cumulative", "total_tokens": 1,
                    "driver": "codex", "kind": "vcs.issue", "facts": {"state": "open"}}}),
                &[], None,
            ).unwrap();
            ids.push(claim.id);
            transaction.commit().unwrap();
        }
        let summary = || {
            let connection = store.readers.get();
            let mut summary = smallclaims::answers::summary(&connection, 1_000).unwrap();
            for (name, keys) in fold_reader_answers(&connection, 1_000).unwrap() {
                let (name, answers, _) = reader_answers(&name, 1_000, keys);
                summary.kinds.insert(name, answers);
            }
            summary
        };
        let full = summary();
        store.connection.write().execute("DELETE FROM claims WHERE id=?1", [&ids[0]]).unwrap();
        let trimmed = summary();
        assert!(differing_kinds(&full, &trimmed, true).is_empty(), "{kind}");
        let update = match kind {
            "harness.usage" => "UPDATE claims SET body=json_set(body,'$.fields.total_tokens',2) WHERE id=?1",
            "resource.observed" => "UPDATE claims SET body=json_set(body,'$.fields.facts.state','closed') WHERE id=?1",
            _ => "UPDATE claims SET body=json_set(body,'$.fields.state','failed') WHERE id=?1",
        };
        store.connection.write().execute(update, [&ids[1]]).unwrap();
        assert!(!differing_kinds(&trimmed, &summary(), true).is_empty(), "{kind}");
}

#[test]
fn retained_history_daemon_diagnostic() { retained_history_case("daemon.diagnostic"); }
#[test]
fn retained_history_harness_observed() { retained_history_case("harness.observed"); }
#[test]
fn retained_history_harness_todo() { retained_history_case("harness.todo.observed"); }
#[test]
fn retained_history_harness_usage() { retained_history_case("harness.usage"); }
#[test]
fn retained_history_observer() { retained_history_case("observer.observed"); }
#[test]
fn retained_history_resource_1502() { retained_history_case("resource.observed"); }
#[test]
fn retained_history_transport() { retained_history_case("transport.observed"); }

#[test]
fn slot_readers_preserve_field_carriers_and_distinct_slots() {
    let store = Store::open_memory("alder").unwrap();
    let mut carriers = Vec::new();
    for (kind, subject, values) in [
        ("observer.observed", "observer/example", [json!({"cursor": "first"}), json!({"state": "healthy"})]),
        ("daemon.diagnostic", "daemon/example", [json!({"code": "a"}), json!({"code": "b"})]),
        ("harness.observed", "host/example", [json!({"incarnation_id": "a"}), json!({"incarnation_id": "b"})]),
        ("harness.usage", "host/example", [
            json!({"semantics": "session_cumulative", "incarnation_id": "a", "total_tokens": 1}),
            json!({"semantics": "session_cumulative", "incarnation_id": "b", "total_tokens": 1}),
        ]),
    ] {
        for (index, fields) in values.into_iter().enumerate() {
            store.set_write_clock_at(100 + index as u128).unwrap();
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            let claim = append_claim_tx(
                &transaction, "alder", subject, kind, None, &json!({"fields": fields}), &[], None,
            ).unwrap();
            if kind == "observer.observed" && index == 0 { carriers.push(claim.id); }
            transaction.commit().unwrap();
        }
    }
    let full = slot_reader_answers(&store.readers.get(), 1_000).unwrap();
    for kind in ["daemon.diagnostic", "harness.observed", "harness.usage"] {
        assert_eq!(full[&format!("slots/{kind}")].len(), 2, "{kind}");
    }
    store.connection.write().execute("DELETE FROM claims WHERE id=?1", [&carriers[0]]).unwrap();
    let lost = slot_reader_answers(&store.readers.get(), 1_000).unwrap();
    assert_ne!(full["slots/observer.observed"], lost["slots/observer.observed"]);
}
