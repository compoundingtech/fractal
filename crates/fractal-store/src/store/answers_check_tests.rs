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
// SQL mutations in these regressions touch only explicitly constructed, bounded in-memory fixtures.
fn retained_history_case(kind: &str) {
    use smallclaims::answers::{differing_kinds, reader_answers};
        let store = Store::open_memory("alder").unwrap();
        let subject = match kind {
            "daemon.diagnostic" => "daemon/example",
            "harness.observed" | "harness.todo.observed" | "harness.usage" => "agent/example",
            "observer.observed" => "observer/example",
            "resource.observed" => "resource/example/issue/1502",
            _ => "host/example",
        };
        let mut ids = Vec::new();
        for (at, writer, state) in [(100, "old", "idle"), (150, "old", "idle"), (200, "new", "idle")] {
            let writer = if matches!(kind, "observer.observed" | "resource.observed") { writer } else { "new" };
            store.set_write_clock_at(at).unwrap();
            let fields = match kind {
                "daemon.diagnostic" => json!({"severity":"error","code":"example","reason":"example"}),
                "harness.observed" => json!({"state":state,"driver":"codex","incarnation_id":"one"}),
                "harness.todo.observed" => json!({"harness":"omp","session_id":"session","incarnation_id":"one",
                    "observed_at":"2026-10-03T14:00:00Z","source_op":"hydrate","phases":[],"truncated":false,
                    "totals":{"pending":0,"in_progress":0,"completed":0,"blocked":0}}),
                "harness.usage" => json!({"semantics":"session_cumulative","driver":"codex","incarnation_id":"one","total_tokens":1}),
                "observer.observed" => json!({"status":state}),
                "resource.observed" => json!({"kind":"vcs.issue","observer":"observer/example","facts":{"state":"open"}}),
                "transport.observed" => json!({"status":"up"}),
                _ => unreachable!(),
            };
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            let claim = append_claim_tx(
                &transaction, writer, subject, kind, None,
                &json!({"fields": fields}),
                &[], None,
            ).unwrap();
            ids.push(claim.id);
            transaction.commit().unwrap();
        }
        let summary = || {
            if kind == "resource.observed" {
                let mut connection = store.connection.write();
                let transaction = connection.transaction().unwrap();
                resources::refresh(&transaction, subject).unwrap();
                transaction.commit().unwrap();
            }
            let connection = store.readers.get();
            let mut summary = smallclaims::answers::summary(&connection, 1_000).unwrap();
            for (name, keys) in fold_reader_answers(&connection, 1_000).unwrap() {
                let (name, answers, _) = reader_answers(&name, 1_000, keys);
                summary.kinds.insert(name, answers);
            }
            summary
        };
        let full = summary();
        let first = usize::from(!matches!(kind, "observer.observed" | "resource.observed"));
        store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h82b449284c39 owner=fractal_store::store::answers_check_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id IN (?1,?2)", [&ids[first], &ids[1]],
        ).unwrap();
        let trimmed = summary();
        if kind != "observer.observed" {
            assert!(differing_kinds(&full, &trimmed, true).iter().any(|difference| difference.kind == kind),
                "incomplete production-reader coverage must retain {kind}'s conservative inventory check");
            return;
        }
        assert!(differing_kinds(&full, &trimmed, true).is_empty(), "{kind}");
        let update = /* sqlite-large-value: allow-for-now lv-h02c49cdc0193 owner=fractal_store::store::answers_check_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$.fields.status','failed') WHERE id=?1";
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hea7a342bde64 owner=fractal_store::store::answers_check_tests TODO=trace-statement-and-value-representation */ update, [&ids[2]]).unwrap();
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
fn observer_reader_preserves_field_carriers() {
    let store = Store::open_memory("alder").unwrap();
    let original = observation(&store, "observer/example", "observer.observed", "alder", 100, json!({"cursor":"first"}));
    observation(&store, "observer/example", "observer.observed", "alder", 200, json!({"status":"healthy"}));
    let full = slot_reader_answers(&store.readers.get(), 1_000).unwrap();
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h67e53372c4b6 owner=fractal_store::store::answers_check_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&original]).unwrap();
    assert_ne!(full, slot_reader_answers(&store.readers.get(), 1_000).unwrap());
}

fn observation(store: &Store, subject: &str, kind: &str, writer: &str, at: u128, fields: Value) -> String {
    store.set_write_clock_at(at).unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    let claim = append_claim_tx(&transaction, writer, subject, kind, None, &json!({"fields":fields}), &[], None).unwrap();
    transaction.commit().unwrap();
    claim.id
}


#[test]
fn served_cache_tracks_observer_actual_state_dependencies() {
    use smallclaims::answers::AnswersCache;
    let store = Arc::new(Store::open_memory("alder").unwrap());
    let subject = "observer/example";
    observation(&store, subject, "observer.observed", "alder", 100, json!({"status":"healthy"}));
    let cache = Arc::new(AnswersCache::default());
    cache.set_readers(Arc::new(fold_reader_answers));
    cache.set_reader_movement(Arc::new(slot_reader_movement));
    assert_eq!(served_summary(&cache, &store).kinds["reader/slots/observer.observed"].keys, 1);
    observation(&store, subject, "publication.operation", "alder", 2_000, json!({"status":"accepted"}));
    assert_eq!(served_summary(&cache, &store).kinds["reader/slots/observer.observed"].keys, 0);
}

fn served_summary(
    cache: &Arc<smallclaims::answers::AnswersCache>, store: &Arc<Store>,
) -> smallclaims::answers::AnswersSummary {
    let request = smallclaims::answers::AnswersRequest { cut_unix_ms:1_000, ..Default::default() };
    for _ in 0..500 {
        let reply = cache.answer(store, &request);
        if !reply.pending { return reply.summary.unwrap(); }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("cache did not answer");
}


#[test]
fn observer_reader_covers_newest_cursor_not_only_merged_fields() {
    let mut answers = Vec::new();
    for cursor in [None, Some("first")] {
        let store = Store::open_memory("alder").unwrap();
        observation(&store, "observer/example", "observer.observed", "alder", 100, json!({"cursor":"first"}));
        let mut fields = json!({"status":"healthy"});
        if let Some(cursor) = cursor { fields["cursor"] = json!(cursor); }
        observation(&store, "observer/example", "observer.observed", "alder", 200, fields);
        assert_eq!(store.claimed_observer_cursor("observer/example").unwrap().as_deref(), cursor);
        answers.push(slot_reader_answers(&store.readers.get(), 1_000).unwrap());
    }
    assert_ne!(answers[0], answers[1], "merged fields agree but the newest cursor differs");
}

#[test]
fn conservative_usage_inventory_keeps_required_hourly_boundaries_visible() {
    let store = Store::open_memory("alder").unwrap();
    let mut ids = Vec::new();
    for hour in 1..=3u128 {
        ids.push(observation(&store, "agent/example", "harness.usage", "alder", hour * 3_600_000,
            json!({"semantics":"response_rollup","driver":"codex","incarnation_id":"one","model":"example","total_tokens":hour * 10})));
    }
    let full = smallclaims::answers::summary(&store.readers.get(), 20_000_000).unwrap();
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hb78dff70f153 owner=fractal_store::store::answers_check_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&ids[1]]).unwrap();
    let missing = smallclaims::answers::summary(&store.readers.get(), 20_000_000).unwrap();
    assert!(smallclaims::answers::differing_kinds(&full, &missing, true).iter()
        .any(|difference| difference.kind == "harness.usage"));
}

#[test]
fn conservative_transport_inventory_keeps_legacy_recency_visible() {
    let stores = [Store::open_memory("alder").unwrap(), Store::open_memory("alder").unwrap()];
    for (store, at) in stores.iter().zip([100, 86_400_000]) {
        observation(store, "host/example", "transport.observed", "alder", at, json!({"status":"up"}));
    }
    assert_ne!(stores[0].transport_links_at(86_400_001).unwrap(), stores[1].transport_links_at(86_400_001).unwrap());
    let summaries = stores.iter().map(|store| smallclaims::answers::summary(&store.readers.get(), 100_000_000).unwrap())
        .collect::<Vec<_>>();
    assert!(smallclaims::answers::differing_kinds(&summaries[0], &summaries[1], true).iter()
        .any(|difference| difference.kind == "transport.observed"));
}


#[test]
fn repaired_only_observer_is_absent_unless_its_production_cursor_survives() {
    let absent = Store::open_memory("alder").unwrap();
    let absent_answers = slot_reader_answers(&absent.readers.get(), 1_000).unwrap();
    for (at, cursor) in [(100, None), (100, Some("first")), (2_000, None)] {
        let retained = Store::open_memory("alder").unwrap();
        let mut fields = json!({"status":"healthy"});
        if let Some(cursor) = cursor { fields["cursor"] = json!(cursor); }
        let original = observation(&retained, "observer/example", "observer.observed", "alder", at, fields);
        retained.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h4a2ed0ed1e0c owner=fractal_store::store::answers_check_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)", [&original],
        ).unwrap();
        assert!(latest_actual(&retained.readers.get(), "observer/example").unwrap().is_none());
        assert_eq!(retained.claimed_observer_cursor("observer/example").unwrap().as_deref(), cursor);
        let retained_answers = slot_reader_answers(&retained.readers.get(), 1_000).unwrap();
        assert_eq!(retained_answers == absent_answers, cursor.is_none() && at < 1_000);
        if at >= 1_000 {
            let keys = &retained_answers["slots/observer.observed"];
            assert_eq!(keys.len(), 1);
            assert!(keys[0].moving, "an empty post-cut answer must still hide its peer's stable key");
        }
    }
}
