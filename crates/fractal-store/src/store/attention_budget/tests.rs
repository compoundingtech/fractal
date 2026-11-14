use super::*;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h976b4bd6803a owner=fractal_store::store::attention_budget::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims (id TEXT, store_index INTEGER PRIMARY KEY, batch_id TEXT, subject TEXT,
           kind TEXT, origin TEXT, actor TEXT, body TEXT, predecessors TEXT, accepted_at_unix_ms TEXT);
         CREATE INDEX claims_accepted_order_index ON claims(length(accepted_at_unix_ms), accepted_at_unix_ms, store_index);
         CREATE INDEX claims_subject_kind_index ON claims(subject,kind,store_index);
         CREATE INDEX claims_person_ask_recipient_index ON claims(json_extract(body, '$.fields.person')) WHERE kind='work.person-asked';
         CREATE TABLE batches (id TEXT, origin TEXT, replica_sequence INTEGER);
         CREATE TABLE claim_positions (claim_id TEXT, position INTEGER);
         CREATE TABLE replica_records (claim_id TEXT, position INTEGER);
         CREATE TABLE mission_runs (id TEXT PRIMARY KEY, mission_id TEXT, root_run_id TEXT, parent_step_run TEXT, initial_revision TEXT DEFAULT 'old');
         CREATE TABLE mission_definitions (mission_id TEXT PRIMARY KEY, revision TEXT);
         CREATE TABLE mission_revisions (mission_id TEXT, revision TEXT, body TEXT, PRIMARY KEY(mission_id,revision));
         CREATE TABLE step_runs (subject TEXT PRIMARY KEY, run_id TEXT);
         INSERT INTO mission_runs(id,mission_id,root_run_id,parent_step_run) VALUES
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
fn candidates_seek_only_the_recipient_index_without_sorting() {
    let db = fixture();
    let plan = db
        .query_row(
            format!("EXPLAIN QUERY PLAN {SUBJECTS}").as_str(),
            params!["person/operator", MAX_SUBJECTS + 1],
            |row| row.get::<_, String>(3),
        )
        .unwrap();
    assert!(
        plan.contains("SEARCH claims USING INDEX claims_person_ask_recipient_index"),
        "{plan}"
    );
    let mut plan = db.prepare(format!("EXPLAIN QUERY PLAN {SUBJECTS}").as_str()).unwrap();
    for detail in plan.query_map(params!["person/operator", MAX_SUBJECTS + 1], |row| row.get::<_, String>(3)).unwrap() {
        assert!(!detail.unwrap().contains("TEMP B-TREE"));
    }
}

#[test]
fn attention_request_count_reuses_a_pinned_snapshot() {
    let store = Store::open_memory("node").unwrap();
    let at = WEEK_MS + 100;
    let standalone = store
        .attention_request_count("livestore/cycle", "person/operator", at)
        .unwrap();
    let nested = store
        .read_snapshot(|_| {
            store.attention_request_count("livestore/cycle", "person/operator", at)
        })
        .unwrap();
    let nested_unindexed = store
        .read_snapshot_unindexed(|| {
            store.attention_request_count("livestore/cycle", "person/operator", at)
        })
        .unwrap();
    assert_eq!(standalone, nested);
    assert_eq!(standalone, nested_unindexed);
}

fn budget(db: &Connection, mission: &str, revision: &str, asks: Option<u64>) {
    let attention = asks.map_or_else(Vec::new, |asks| vec![json!({"person":"person/operator","asks":asks})]);
    db.execute(/* sqlite-large-value: allow-for-now lv-hee678b78a3e8 owner=fractal_store::store::attention_budget::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_revisions VALUES (?1,?2,?3)",
        params![mission, revision, json!({"declared_budgets":{"attention":attention}}).to_string()]).unwrap();
    db.execute(/* sqlite-large-value: allow-for-now lv-h4d1d6346668f owner=fractal_store::store::attention_budget::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO mission_definitions VALUES (?1,?2)", params![mission,revision]).unwrap();
}

#[test]
fn initial_revision_pinning_undeclared_and_ancestor_multi_charging() {
    let db = fixture();
    let at = WEEK_MS + 100;
    budget(&db, "livestore/cycle", "old", None);
    budget(&db, "livestore/cycle", "new", Some(3));
    budget(&db, "livestore/delegated", "old", Some(2));
    db.execute(/* sqlite-large-value: allow-for-now lv-h38753dc0c8a0 owner=fractal_store::store::attention_budget::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET initial_revision='new' WHERE id='second'", []).unwrap();
    insert(&db, "old-run", at - 2, linked("first"));
    insert(&db, "new-run", at - 1, linked("second"));
    insert(&db, "child", at, linked("delegate"));
    let parent = report::report(&db, "livestore/cycle", "mission/livestore/cycle", "person/operator", at).unwrap();
    assert_eq!((parent.inclusive, parent.charged, parent.not_charged), (3, 1, 2));
    assert_eq!(parent.declared, AttentionDeclaration::Declared { asks: 3 });
    let child = report::report(&db, "livestore/delegated", "mission/livestore/delegated", "person/operator", at).unwrap();
    assert_eq!((child.inclusive, child.charged), (1, 1));
    // The same child ask is charged again to its ancestor after a new child run
    // is parented to the ancestor's budget-declaring run.
    db.execute(/* sqlite-large-value: allow-for-now lv-h16e758f1cacf owner=fractal_store::store::attention_budget::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET parent_step_run='step-run/second/work' WHERE id='delegate'", []).unwrap();
    let parent = report::report(&db, "livestore/cycle", "mission/livestore/cycle", "person/operator", at).unwrap();
    assert_eq!((parent.charged, parent.not_charged), (2, 1));
    budget(&db, "livestore/cycle", "removed", None);
    let parent = report::report(&db, "livestore/cycle", "mission/livestore/cycle", "person/operator", at).unwrap();
    assert_eq!(parent.declared, AttentionDeclaration::Undeclared);
    assert_eq!(parent.charged, 2, "current removal cannot uncharge pinned runs");
    assert_eq!(serde_json::to_value(parent.declared).unwrap(), json!({"state":"undeclared"}));
}

#[test]
fn report_bounds_newest_exact_references_and_never_serializes_text() {
    let db = fixture();
    budget(&db, "livestore/cycle", "old", None);
    let at = WEEK_MS + 1000;
    insert(&db, "excluded-start", 1000, linked("first"));
    insert(&db, "excluded-future", at + 1, linked("first"));
    for offset in 0..130 {
        let mut fields = linked("first");
        fields["title"] = json!("PRIVATE TITLE");
        fields["prompt"] = json!("PRIVATE PROMPT");
        fields["reason"] = json!("PRIVATE REASON");
        fields["answer"] = json!("PRIVATE ANSWER");
        insert(&db, &format!("request-{offset:03}"), at - offset, fields);
    }
    insert(&db, "tie-z", at, linked("first"));
    let report = report::report(&db, "livestore/cycle", "mission/livestore/cycle", "person/operator", at).unwrap();
    assert_eq!((report.window_start_exclusive_unix_ms, report.window_end_inclusive_unix_ms), (1000, at));
    assert_eq!((report.inclusive, report.charged, report.not_charged), (131, 0, 131));
    assert_eq!(report.local_claim_index, 133);
    assert!(!report.fleet_complete && !report.request_refs_complete);
    assert_eq!(report.request_refs.len(), 128);
    assert_eq!(report.request_refs[0].subject, "tie-z");
    assert_eq!(report.request_refs[1].subject, "request-000");
    assert_eq!(report.request_refs[127].subject, "request-126");
    let value = serde_json::to_value(report).unwrap();
    for reference in value["request_refs"].as_array().unwrap() {
        assert_eq!(reference.as_object().unwrap().keys().cloned().collect::<Vec<_>>(), ["claim_id", "subject"]);
    }
    assert!(!value.to_string().contains("PRIVATE"));
}

#[test]
fn candidate_and_canonical_history_limits_refuse_incomplete_counts() {
    let db = fixture();
    let at = WEEK_MS + 20_000;
    let tx = db.unchecked_transaction().unwrap();
    for offset in 0..=MAX_SUBJECTS {
        insert(&tx, "same-subject", at - offset as u128, linked("first"));
    }
    assert!(count(&tx, "livestore/cycle", "person/operator", at).unwrap_err().to_string().contains("request-claim limit"));
    tx.rollback().unwrap();
    insert(&db, "old-original", 1, linked("first"));
    for offset in 0..MAX_CANONICAL_CLAIMS {
        insert(&db, "old-original", at - offset as u128, linked("first"));
    }
    assert!(count(&db, "livestore/cycle", "person/operator", at).unwrap_err().to_string().contains("canonical-history limit"));
    let db = fixture();
    for offset in 0..=MAX_SUBJECTS {
        insert(&db, "all-expired", 1 + offset as u128, linked("first"));
    }
    assert!(count(&db, "livestore/cycle", "person/operator", at).unwrap_err().to_string().contains("lifetime request-claim limit"));
}

#[test]
fn ancestry_is_bounded_at_64_vertices() {
    for length in [64, 65] {
        let db = fixture();
        budget(&db, "target", "old", None);
        budget(&db, "child", "old", Some(1));
        for index in 0..length {
            let run = format!("chain-{index}");
            let parent = (index > 0).then(|| format!("step-run/chain-{}/work", index - 1));
            db.execute(/* sqlite-large-value: allow-for-now lv-h5dae31705c3e owner=fractal_store::store::attention_budget::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id,mission_id,root_run_id,parent_step_run) VALUES (?1,?2,'chain-0',?3)",
                params![run, if index == 0 { "target" } else { "child" }, parent]).unwrap();
            db.execute(/* sqlite-large-value: allow-for-now lv-h63559ed05743 owner=fractal_store::store::attention_budget::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs VALUES (?1,?2)", params![format!("step-run/{run}/work"),run]).unwrap();
        }
        let at = WEEK_MS + 100;
        insert(&db, "deep", at, linked(&format!("chain-{}",length - 1)));
        for mission in ["target", "child"] {
            let counted = count(&db, mission, "person/operator", at);
            let reported = report::report(&db, mission, &format!("mission/{mission}"), "person/operator", at);
            let charged = report::run_charges_mission(&db, &format!("chain-{}", length - 1), mission, "person/operator", &mut BTreeMap::new());
            if length == 64 {
                assert_eq!(counted.unwrap().requests, 1);
                assert_eq!(reported.unwrap().inclusive, 1);
                assert_eq!(charged.unwrap(), mission == "child");
            } else {
                for error in [counted.unwrap_err(), reported.unwrap_err(), charged.unwrap_err()] {
                    assert!(error.to_string().contains("ancestry limit of 64 vertices"), "{error:#}");
                }
            }
        }
    }
}

#[test]
fn unrelated_claim_volume_does_not_consume_recipient_candidate_work() {
    let db = fixture();
    let tx = db.unchecked_transaction().unwrap();
    let at = WEEK_MS + 100;
    let mut unrelated = linked("first");
    unrelated["person"] = json!("person/other");
    for offset in 0..=MAX_SUBJECTS {
        insert(&tx, "unrelated-recipient", at - offset as u128, unrelated.clone());
    }
    insert(&tx, "selected-recipient", at, linked("first"));
    let mut query = tx.prepare(/* sqlite-large-value: allow-for-now lv-hedfb698e9105 owner=fractal_store::store::attention_budget::tests TODO=trace-statement-and-value-representation */ SUBJECTS).unwrap();
    let examined = query.query_map(params!["person/operator", MAX_SUBJECTS + 1], |_| Ok(())).unwrap().count();
    assert_eq!(examined, 1, "recipient index excludes all unrelated rows before LIMIT");
    assert_eq!(count(&tx, "livestore/cycle", "person/operator", at).unwrap().requests, 1);
    assert!(count(&tx, "livestore/cycle", "person/other", at).unwrap_err().to_string().contains("lifetime request-claim limit"));
}
