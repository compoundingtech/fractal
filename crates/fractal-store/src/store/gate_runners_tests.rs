//! The live gate-runner table holds exactly what reading every gate request from the claims
//! finds, through appends, stops, deletions and rewrites, and a store that predates it is
//! filled once when it opens.
use super::*;
use serde_json::json;

fn request(store: &Store, subject: &str, fields: serde_json::Value) -> String {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: subject.into(),
            kind: "gate.requested".into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .id
}

fn observe(store: &Store, subject: &str, status: &str) -> String {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: subject.into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: serde_json::from_value(json!({"status": status})).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .id
}

type Row = (String, String, String, String);

fn rows(connection: &rusqlite::Connection, sql: &str) -> Vec<Row> {
    let mut rows = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-hae6010f8fa83 owner=fractal_store::store::gate_runners_tests TODO=trace-statement-and-value-representation */ sql)
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<Row>>>()
        .unwrap();
    rows.sort();
    rows
}

fn assert_kept(store: &Store) -> Vec<Row> {
    let connection = store.readers.get();
    let kept = rows(&connection, "SELECT claim_id, subject, origin, owner FROM local_gate_runners");
    assert_eq!(kept, rows(&connection, gate_runners::FROM_CLAIMS));
    kept
}

#[test]
fn the_table_holds_what_reading_every_gate_request_finds() {
    let store = Store::open_memory("node").unwrap();
    let exec = request(&store, "gate-operation/step-run.a/check", json!({"runner": "exec", "owner": "step-run/a"}));
    request(&store, "gate-operation/step-run.b/metric", json!({"runner": "loop-metric", "owner": "step-run/b"}));
    request(&store, "gate-operation/step-run.c/model", json!({"model": "invented-model", "owner": "step-run/c"}));
    // A person's review needs no runner; a request without an owner names it in its subject.
    request(&store, "gate-operation/step-run.d/review", json!({"reviewer": "person/alex", "owner": "step-run/d"}));
    request(&store, "gate-operation/step-run.legacy/check", json!({"runner": "exec"}));
    let kept = assert_kept(&store);
    assert_eq!(kept.len(), 4);
    assert!(kept.iter().any(|(_, subject, _, owner)| subject == "gate-operation/step-run.legacy/check" && owner == "legacy:step-run.legacy"));

    // A running observation keeps the runner; a stop retires every request on that subject.
    observe(&store, "gate-operation/step-run.a/check", "running");
    assert_eq!(assert_kept(&store).len(), 4);
    let stop = observe(&store, "gate-operation/step-run.a/check", "stopped");
    assert_eq!(assert_kept(&store).len(), 3);

    let connection = store.connection.write();
    // Deleting the stop brings the request back; deleting a request takes only its row.
    connection.execute(/* sqlite-large-value: allow-for-now lv-h9702b2285d87 owner=fractal_store::store::gate_runners_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&stop]).unwrap();
    drop(connection);
    assert_eq!(assert_kept(&store).len(), 4);
    let connection = store.connection.write();
    connection.execute(/* sqlite-large-value: allow-for-now lv-hdcff2425c62f owner=fractal_store::store::gate_runners_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&exec]).unwrap();
    drop(connection);
    assert_eq!(assert_kept(&store).len(), 3);
    // A rewritten claim is taken out under its old shape and in under its new one.
    let connection = store.connection.write();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h24c98c95d905 owner=fractal_store::store::gate_runners_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body, '$.fields.runner', 'manual') WHERE kind='gate.requested' AND subject='gate-operation/step-run.b/metric'",
            [],
        )
        .unwrap();
    drop(connection);
    assert_eq!(assert_kept(&store).len(), 2);
}

/// A store an older build wrote has no table: this build's first open installs the triggers
/// with a ceiling, the list reads the claims until the paced copy below it completes, and the
/// rebuild refills it.
#[test]
fn a_store_without_the_table_is_copied_after_it_opens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("node.sqlite3");
    {
        let store = Store::open(&path, "node").unwrap();
        let connection = store.connection.write();
        connection
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-h66ea9662b56e owner=fractal_store::store::gate_runners_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER gate_runners_request_insert; DROP TRIGGER gate_runners_stop_insert;
                 DROP TRIGGER gate_runners_claim_delete; DROP TRIGGER gate_runners_claim_update; DROP TABLE local_gate_runners;
                 DROP TABLE local_gate_runners_backfill;",
            )
            .unwrap();
        drop(connection);
        let pages = 2 * gate_runners::BACKFILL_PAGE as usize + 7;
        for index in 0..pages {
            request(&store, &format!("gate-operation/step-run.s{index}/check"), json!({"runner": "exec", "owner": format!("step-run/s{index}")}));
        }
        observe(&store, "gate-operation/step-run.s7/check", "stopped");
    }
    let store = Store::open(&path, "node").unwrap();
    assert!(!gate_runners::complete(&store.readers.get()).unwrap());
    // Written after the triggers: in the table at once, with nothing older copied yet.
    request(&store, "gate-operation/step-run.new/check", json!({"runner": "exec", "owner": "step-run/new"}));
    let copied = || store.readers.get().query_row("SELECT COUNT(*) FROM local_gate_runners", [], |row| row.get::<_, usize>(0)).unwrap();
    assert_eq!(copied(), 1);
    // A stop that lands before its request is copied keeps it out.
    observe(&store, "gate-operation/step-run.s3/check", "stopped");
    let mut pages = 0;
    while !store.advance_gate_runner_backfill().unwrap() {
        pages += 1;
        assert!(pages < 10, "the copy did not finish");
    }
    assert_eq!(pages, 2);
    assert!(gate_runners::complete(&store.readers.get()).unwrap());
    let all = 2 * gate_runners::BACKFILL_PAGE as usize + 7;
    assert_eq!(assert_kept(&store).len(), all + 1 - 2);
    let connection = store.connection.write();
    connection.execute(/* sqlite-large-value: allow-for-now lv-h1abbf8648a66 owner=fractal_store::store::gate_runners_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_gate_runners", []).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    gate_runners::rebuild(&transaction).unwrap();
    transaction.commit().unwrap();
    drop(connection);
    assert_eq!(assert_kept(&store).len(), all + 1 - 2);
}

fn plan(connection: &rusqlite::Connection, sql: &str) -> Vec<String> {
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hc0ab63c1e6fe owner=fractal_store::store::gate_runners_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let count = statement.parameter_count();
    statement
        .query_map(rusqlite::params_from_iter(std::iter::repeat_n("x", count)), |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// The steps of the trigger lookups that do not find their claims by subject: a scan, or a search
/// by kind alone, reads every gate request or observation in history.
fn unbounded(connection: &rusqlite::Connection) -> Vec<String> {
    gate_runners::TRIGGER_LOOKUPS
        .iter()
        .flat_map(|sql| plan(connection, sql))
        .filter(|step| {
            (step.starts_with("SCAN ") && step != "SCAN CONSTANT ROW")
                || (step.starts_with("SEARCH ") && !step.contains("(subject=?") && !step.contains("(claim_id=?"))
        })
        .collect()
}

#[test]
fn the_triggers_search_an_index_without_naming_one() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.write();
    // A trigger that names an index fails every claim write once that index is dropped.
    let named: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'gate_runners_%' AND sql LIKE '%INDEXED BY%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(named.is_empty(), "{named:?}");
    // Without the hint the planner must still find each lookup's claims by subject, so a planner
    // or index change shows up here rather than as a slower claim write.
    assert!(unbounded(&connection).is_empty(), "{:?}", unbounded(&connection));
    connection
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h9f0db61bdab1 owner=fractal_store::store::gate_runners_tests TODO=bound-or-reference-values-and-reclamation */ "DROP INDEX claims_subject_kind_index; DROP INDEX claims_subject_kind_accepted_index;")
        .unwrap();
    assert!(!unbounded(&connection).is_empty());
}
