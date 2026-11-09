use super::*;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h1b58bf81b4c7 owner=fractal_store::store::attention_budget::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims (id TEXT, store_index INTEGER PRIMARY KEY, batch_id TEXT, subject TEXT,
           kind TEXT, origin TEXT, actor TEXT, body TEXT, predecessors TEXT, accepted_at_unix_ms TEXT);
         CREATE INDEX claims_accepted_order_index ON claims(length(accepted_at_unix_ms), accepted_at_unix_ms, store_index);
         CREATE TABLE batches (id TEXT, origin TEXT, replica_sequence INTEGER);
         CREATE TABLE claim_positions (claim_id TEXT, position INTEGER);
         CREATE TABLE replica_records (claim_id TEXT, position INTEGER);
         CREATE TABLE mission_runs (id TEXT PRIMARY KEY, mission_id TEXT, root_run_id TEXT, parent_step_run TEXT);
         CREATE TABLE step_runs (subject TEXT PRIMARY KEY, run_id TEXT);
         INSERT INTO mission_runs VALUES
           ('daily', 'livestore/daily', 'daily', NULL),
           ('first', 'livestore/cycle', 'daily', 'schedule/daily'),
           ('second', 'livestore/cycle', 'daily', 'schedule/daily'),
           ('delegate', 'livestore/delegated', 'daily', 'step-run/first/work'),
           ('unrelated', 'other/cycle', 'unrelated', NULL),
           ('missing', 'other/delegated', 'daily', 'step-run/unknown/work');
         INSERT INTO step_runs VALUES
           ('step-run/first/work', 'first'), ('step-run/second/work', 'second'),
           ('step-run/delegate/work', 'delegate'), ('step-run/unrelated/work', 'unrelated'),
           ('step-run/missing/work', 'missing');",
    ).unwrap();
    db
}

fn insert(db: &Connection, subject: &str, at: u128, fields: Value) {
    db.execute(
        /* sqlite-large-value: allow-for-now lv-hcd713de06991 owner=fractal_store::store::attention_budget::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
         VALUES (?1, 'batch', ?2, 'work.person-asked', 'node', 'agent/test', ?3, '[]', ?4)",
        params![format!("{subject}:{at}"), subject, json!({"fields":fields}).to_string(), at.to_string()],
    ).unwrap();
}

fn linked(run: &str) -> Value {
    json!({
        "person": "person/operator",
        "run": format!("mission-run/{run}"),
        "origin_step": format!("step-run/{run}/work"),
    })
}

#[test]
fn recurring_mission_and_delegated_work_share_one_budget_without_guessing_ownership() {
    let db = fixture();
    let at = WEEK_MS + 100;
    insert(&db, "first", at - 11, linked("first"));
    insert(&db, "second", at - 10, linked("second"));
    insert(&db, "delegated", at - 9, linked("delegate"));
    insert(&db, "delegated", at - 8, linked("delegate"));
    insert(&db, "unrelated", at - 7, linked("unrelated"));
    insert(&db, "missing", at - 6, linked("missing"));
    insert(
        &db,
        "independent",
        at - 5,
        json!({"person":"person/operator"}),
    );
    let mut update = linked("first");
    update["request"] = json!({"type":"update"});
    insert(&db, "update", at - 4, update);
    let mut converted = linked("first");
    converted["legacy_request"] = json!("legacy-id");
    insert(&db, "converted", at - 3, converted);
    let mut different_person = linked("second");
    different_person["person"] = json!("person/other");
    insert(&db, "other-person", at - 2, different_person);

    let report = count(&db, "livestore/cycle", "person/operator", at).unwrap();
    assert_eq!(report.requests, 3);
    assert_eq!(report.unattributed_for_person, 2);
    assert_eq!(
        report.requests_by_originating_run,
        BTreeMap::from([
            ("mission-run/first".to_owned(), 1),
            ("mission-run/second".to_owned(), 1),
            ("mission-run/delegate".to_owned(), 1),
        ])
    );
}

#[test]
fn an_older_canonical_request_cannot_be_charged_again_inside_the_window() {
    let db = fixture();
    let at = WEEK_MS + 99;
    insert(&db, "expired", 99, linked("first"));
    insert(&db, "expired", at, linked("first"));
    insert(&db, "first-inside", 100, linked("first"));
    insert(&db, "last-inside", at, linked("first"));
    insert(&db, "future", at + 1, linked("first"));
    let report = count(&db, "livestore/cycle", "person/operator", at).unwrap();
    assert_eq!(report.window_start_exclusive_unix_ms, 99);
    assert_eq!(report.requests, 2);
}

#[test]
fn the_window_seeks_the_accepted_time_index() {
    let db = fixture();
    let plan = db
        .query_row(
            format!("EXPLAIN QUERY PLAN {SUBJECTS}").as_str(),
            params![3, "100", "999", "person/operator", 100],
            |row| row.get::<_, String>(3),
        )
        .unwrap();
    assert!(
        plan.contains("SEARCH claims USING INDEX claims_accepted_order_index"),
        "{plan}"
    );
}
