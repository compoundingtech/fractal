use super::*;

const PREFIX: &str = "step-run/r/s@2@3@";
const UPPER: &str = "step-run/r/s@2@3A";
const ORIGINAL: &str = "SELECT claims.body, claims.accepted_at_unix_ms, claims.subject,
    EXISTS(SELECT 1 FROM claims consumed WHERE consumed.subject=claims.subject
        AND consumed.kind IN ('message.read','message.closed') AND consumed.accepted_at_unix_ms<=?1)
    OR EXISTS(SELECT 1 FROM local_message_marks consumed WHERE consumed.message=claims.subject
        AND consumed.kind IN ('message.read','message.closed') AND consumed.recorded_at_unix_ms<=CAST(?1 AS INTEGER)),
    EXISTS(SELECT 1 FROM claims delivered WHERE delivered.subject=claims.subject
        AND delivered.kind='message.delivered' AND delivered.accepted_at_unix_ms<=?1)
    OR EXISTS(SELECT 1 FROM local_message_marks delivered WHERE delivered.message=claims.subject
        AND delivered.kind='message.delivered' AND delivered.recorded_at_unix_ms<=CAST(?1 AS INTEGER)),
    EXISTS(SELECT 1 FROM claims staged WHERE staged.subject=claims.subject
        AND staged.kind='message.staged' AND staged.accepted_at_unix_ms<=?1)
    OR EXISTS(SELECT 1 FROM local_message_staged staged WHERE staged.message=claims.subject
        AND staged.recorded_at_unix_ms<=CAST(?1 AS INTEGER))
    FROM claims INDEXED BY claims_message_to_index
    WHERE claims.kind='message.sent' AND claims.accepted_at_unix_ms<=?1
        AND (instr(claims.body,'st3-work:'||?2)>0 OR instr(claims.body,'fractal-work:'||?2)>0)
        AND json_extract(claims.body,'$.fields.to') IN (?3,?4)
    ORDER BY CANONICAL_ASC(claims)";

type WakeRow = (String, String, String, bool, bool, bool);

fn rows(connection: &Connection, sql: &str, indexed: bool, subject: &str) -> Vec<WakeRow> {
    let prefix = format!("{subject}@2@3@");
    let upper = format!("{subject}@2@3A");
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-ha2d021c98ccf owner=fractal_store::store::wake_lookup_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
    let read = |row: &rusqlite::Row<'_>| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
    };
    let mut result: Vec<WakeRow> = if indexed {
        statement.query_map(params!["99999999999999999999999999999999999999", prefix, "agent/worker", "worker", upper], read)
            .unwrap().collect::<rusqlite::Result<_>>().unwrap()
    } else {
        statement.query_map(params!["99999999999999999999999999999999999999", prefix, "agent/worker", "worker"], read)
            .unwrap().collect::<rusqlite::Result<_>>().unwrap()
    };
    // Candidate sets may differ: the original raw-body search also admits a work key in content.
    // Compare the rows that the unchanged Rust semantic filter actually uses for wake attempts.
    result.retain(|(body, ..)| {
        let body: Value = serde_json::from_str(body).unwrap();
        let fields = body.get("fields").unwrap_or(&body);
        let tags: Vec<_> = fields.get("tags").and_then(Value::as_array).into_iter()
            .flatten().filter_map(Value::as_str).collect();
        !crate::silent::message_tag_values(tags.iter().copied(), "wake-source:")
            .any(|source| source == "manual")
            && crate::silent::message_tag_values(tags.iter().copied(), "work:")
                .any(|tag| work_wake_tag_incarnation(tag, subject, 2, 3).is_some())
    });
    result
}

fn assert_history(connection: &Connection, complete: bool) -> Vec<WakeRow> {
    let original = rows(connection, &canonical_sql(ORIGINAL), false, "step-run/r/s");
    let indexed = rows(connection, &work_wake_history_query(complete), true, "step-run/r/s");
    assert_eq!(indexed, original);
    indexed
}

fn body(to: &str, tags: Value) -> Value {
    json!({"fields": {"from": "daemon/runtime", "to": to, "content": "", "status": "sent", "tags": tags}})
}

fn insert(connection: &Connection, id: &str, kind: &str, body: &str) {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h80d3ab603997 owner=fractal_store::store::wake_lookup_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
         SELECT ?1,'fixture',COALESCE(MAX(replica_sequence),0)+1,'fixture-hash','10'
         FROM batches WHERE origin='fixture'",
        [id],
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h128a4d8ec35c owner=fractal_store::store::wake_lookup_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
         VALUES(?1,?1,?2,?3,'fixture',?4,'[]','10')",
        params![id, format!("message/{id}"), kind, body],
    ).unwrap();
}

fn view() -> StepRunView {
    serde_json::from_value(json!({
        "subject": "step-run/r/s", "run": "mission-run/r", "generation": "g", "step": "s",
        "definition_hash": "", "status": "ready", "attempt": 2, "readiness_epoch": 3,
        "assigned_to": "agent/worker", "agentless": false, "worker_reported": false,
        "created_at_unix_ms": 0, "updated_at_unix_ms": 0
    })).unwrap()
}

fn remove_lookup(connection: &Connection) {
    let triggers = connection.prepare(
        "SELECT name FROM sqlite_schema WHERE type='trigger' AND instr(sql,'local_work_wake_tags')>0",
    ).unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    for trigger in triggers {
        connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-h9d284b24fd9b owner=fractal_store::store::wake_lookup_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER \"{}\"", trigger.replace('"', "\"\""))).unwrap();
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h6fe4bc0f91de owner=fractal_store::store::wake_lookup_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE local_work_wake_tags; DROP TABLE local_work_wake_backfill;").unwrap();
}

fn install_lookup(store: &Store) {
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    wake_lookup::open(&transaction).unwrap();
    transaction.commit().unwrap();
}

fn finish_backfill(store: &Store) {
    let (_, ceiling, _) = wake_lookup::status(&store.readers.get()).unwrap();
    // A finite bound makes a nonadvancing cursor fail without a timeout or sleep.
    for _ in 0..=ceiling {
        if wake_lookup::status(&store.readers.get()).unwrap().2 {
            return;
        }
        wake_lookup::advance(store).unwrap();
    }
    panic!("wake backfill did not finish within its claim-count bound");
}

#[test]
fn wake_lookup_matches_original_history_and_semantic_tag_precedence() {
    let store = Store::open_memory("fixture").unwrap();
    let connection = store.connection.write();
    let cases = [
        ("legacy", body("agent/worker", json!(["st3-work:step-run/r/s@2@3@legacy"]))),
        ("current", body("worker", json!(["fractal-work:step-run/r/s@2@3@current"]))),
        ("both", body("agent/worker", json!(["st3-work:other@2@3@old", "fractal-work:step-run/r/s@2@3@new"]))),
        ("multiple", body("agent/worker", json!(["fractal-work:other@2@3@other", "fractal-work:step-run/r/s@2@3@multi", "fractal-work:step-run/r/s@2@3@multi"]))),
        ("manual", body("agent/worker", json!(["fractal-work:step-run/r/s@2@3@manual", "fractal-wake-source:manual"]))),
        ("legacy-manual-shadowed", body("agent/worker", json!(["fractal-work:step-run/r/s@2@3@auto", "st3-wake-source:manual", "fractal-wake-source:auto"]))),
        ("legacy-work-shadowed", body("agent/worker", json!(["st3-work:step-run/r/s@2@3@old", "fractal-work:other@2@3@new"]))),
        ("long-step", body("agent/worker", json!(["fractal-work:step-run/r/ss@2@3@other"]))),
        ("long-epoch", body("agent/worker", json!(["fractal-work:step-run/r/s@2@30@other"]))),
        ("wrong-attempt", body("agent/worker", json!(["fractal-work:step-run/r/s@20@3@other"]))),
        ("other-recipient", body("agent/other", json!(["fractal-work:step-run/r/s@2@3@other"]))),
        ("nonstrings", body("agent/worker", json!([null, 42, {"tag": "fractal-work:step-run/r/s@2@3@other"}]))),
        ("middle-tag", body("agent/worker", json!(["not-a-prefix:fractal-work:step-run/r/s@2@3@other"]))),
    ];
    for (id, value) in cases {
        insert(&connection, id, "message.sent", &value.to_string());
    }
    let mut middle = body("agent/worker", json!([]));
    middle["fields"]["content"] = json!("middle fractal-work:step-run/r/s@2@3@raw only");
    insert(&connection, "middle-content", "message.sent", &middle.to_string());
    let escaped = body("agent/worker", json!(["fractal-work:step-run/r/s@2@3@escaped"])).to_string()
        .replace("fractal-work:", "fractal-work\\u003a");
    insert(&connection, "escaped", "message.sent", &escaped);
    insert(&connection, "wrong-kind", "message.other", &body("agent/worker", json!(["fractal-work:step-run/r/s@2@3@wrong"])).to_string());
    let found = assert_history(&connection, true);
    let subjects: Vec<_> = found.iter().map(|row| row.2.as_str()).collect();
    for expected in ["message/legacy", "message/current", "message/multiple", "message/legacy-work-shadowed"] {
        assert!(subjects.contains(&expected), "missing {expected}: {subjects:?}");
    }
    assert_eq!(found.len(), 4);
    let mut wake = view();
    enrich_step_wake_at(&connection, &mut wake, u128::MAX).unwrap();
    assert_eq!(wake.wake.unwrap().attempts as usize, found.len());
}

#[test]
fn wake_lookup_triggers_follow_updates_deletions_and_deduplicate_families() {
    let store = Store::open_memory("fixture").unwrap();
    let connection = store.connection.write();
    insert(&connection, "mutable", "message.sent", &body("agent/worker", json!([
        "st3-work:step-run/r/s@2@3@inc", "fractal-work:step-run/r/s@2@3@inc",
        "fractal-work:step-run/r/s@2@3@inc", false
    ])).to_string());
    let entries = || connection.prepare("SELECT recipient,tag,claim_id FROM local_work_wake_tags ORDER BY recipient,tag,claim_id")
        .unwrap().query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(entries(), vec![("agent/worker".into(), "step-run/r/s@2@3@inc".into(), "mutable".into())]);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h781aeff5629d owner=fractal_store::store::wake_lookup_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE id='mutable'", [body("worker", json!(["fractal-work:step-run/r/s@2@3@changed"])).to_string()]).unwrap();
    assert_eq!(entries(), vec![("worker".into(), "step-run/r/s@2@3@changed".into(), "mutable".into())]);
    assert_eq!(assert_history(&connection, true).len(), 1);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h1214e8529adb owner=fractal_store::store::wake_lookup_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET kind='message.other' WHERE id='mutable'", []).unwrap();
    assert!(entries().is_empty());
    connection.execute(/* sqlite-large-value: allow-for-now lv-h8e63805d1a3e owner=fractal_store::store::wake_lookup_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET kind='message.sent' WHERE id='mutable'", []).unwrap();
    assert_eq!(entries().len(), 1);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h235f04a102e8 owner=fractal_store::store::wake_lookup_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id='mutable'", []).unwrap();
    assert!(entries().is_empty());
    assert!(assert_history(&connection, true).is_empty());
}

#[test]
fn wake_lookup_backfill_resumes_across_reopen_and_keeps_post_ceiling_writes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    let store = Store::open(&path, "fixture").unwrap();
    {
        let connection = store.connection.write();
        remove_lookup(&connection);
        for index in 0..(wake_lookup::BACKFILL_PAGE * 2 + 3) {
            insert(&connection, &format!("old-{index:04}"), "message.sent",
                &body("agent/worker", json!(["fractal-work:step-run/r/s@2@3@old"])).to_string());
        }
    }
    install_lookup(&store);
    let initial = wake_lookup::status(&store.readers.get()).unwrap();
    assert_eq!(initial.0, 0);
    assert!(!initial.2);
    let expected = assert_history(&store.readers.get(), false);
    assert_eq!(expected.len(), usize::try_from(wake_lookup::BACKFILL_PAGE * 2 + 3).unwrap());
    wake_lookup::advance(&store).unwrap();
    let partial = wake_lookup::status(&store.readers.get()).unwrap();
    assert!(partial.0 > initial.0 && partial.0 < partial.1 && !partial.2);
    assert_eq!(partial.1, initial.1);
    assert_eq!(assert_history(&store.readers.get(), false), expected);
    {
        let connection = store.connection.write();
        insert(&connection, "after-ceiling", "message.sent", &body("worker", json!(["st3-work:step-run/r/s@2@3@new"])).to_string());
        connection.execute(/* sqlite-large-value: allow-for-now lv-h56baa7d4afcb owner=fractal_store::store::wake_lookup_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id='old-0000'", []).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-hb4bff4ba2ab8 owner=fractal_store::store::wake_lookup_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE id='old-0001'", [body("agent/other", json!(["fractal-work:step-run/r/s@2@3@removed"])).to_string()]).unwrap();
    }
    let expected = assert_history(&store.readers.get(), false);
    assert!(expected.iter().any(|row| row.2 == "message/after-ceiling"));
    drop(store);
    let store = Store::open(&path, "fixture").unwrap();
    assert_eq!(wake_lookup::status(&store.readers.get()).unwrap(), partial);
    assert_eq!(assert_history(&store.readers.get(), false), expected);
    finish_backfill(&store);
    let finished = wake_lookup::status(&store.readers.get()).unwrap();
    assert!(finished.2);
    assert_eq!(finished.1, initial.1);
    assert_eq!(assert_history(&store.readers.get(), true), expected);
    wake_lookup::advance(&store).unwrap();
    assert_eq!(wake_lookup::status(&store.readers.get()).unwrap(), finished);
}

#[test]
fn wake_lookup_rebuild_refills_derived_rows_and_completes_backfill() {
    let store = Store::open_memory("fixture").unwrap();
    {
        let connection = store.connection.write();
        remove_lookup(&connection);
        insert(&connection, "rebuild", "message.sent", &body("agent/worker", json!(["fractal-work:step-run/r/s@2@3@rebuilt"])).to_string());
    }
    install_lookup(&store);
    let expected = assert_history(&store.readers.get(), false);
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        wake_lookup::rebuild(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    assert!(wake_lookup::status(&store.readers.get()).unwrap().2);
    assert_eq!(assert_history(&store.readers.get(), true), expected);
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h8b57d42dc8e0 owner=fractal_store::store::wake_lookup_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_work_wake_tags", []).unwrap();
        wake_lookup::rebuild(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(assert_history(&store.readers.get(), true), expected);
}

fn plan(connection: &Connection, complete: bool) -> Vec<String> {
    connection.prepare(/* sqlite-large-value: allow-for-now lv-h76b20db50772 owner=fractal_store::store::wake_lookup_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {}", work_wake_history_query(complete)))
        .unwrap().query_map(params!["99999999999999999999999999999999999999", PREFIX, "agent/worker", "worker", UPPER], |row| row.get(3))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

#[test]
fn wake_lookup_complete_plan_uses_recipient_tag_primary_key_without_claim_scans() {
    let store = Store::open_memory("fixture").unwrap();
    let connection = store.readers.get();
    assert_eq!(wake_lookup::status(&connection).unwrap(), (0, 0, true));
    let plan = plan(&connection, true);
    assert!(plan.iter().any(|step| step.contains("SEARCH local_work_wake_tags USING PRIMARY KEY")
        && step.contains("recipient=?") && step.contains("tag>?") && step.contains("tag<?")), "{plan:#?}");
    assert!(!plan.iter().any(|step| step.starts_with("SCAN ") && (step.contains("claims") || step.contains("message"))), "{plan:#?}");
    assert!(plan.iter().any(|step| step.starts_with("SEARCH claims ") && step.contains("id=?")), "{plan:#?}");
}

#[test]
fn wake_lookup_incomplete_plan_ranges_tail_before_reading_claim_bodies() {
    let store = Store::open_memory("fixture").unwrap();
    let connection = store.readers.get();
    let plan = plan(&connection, false);
    assert!(plan.iter().any(|step| step.contains("SEARCH pending USING INDEX claims_message_to_order_index")
        && step.contains("<expr>=?") && step.contains("store_index>?") && step.contains("store_index<?")), "{plan:#?}");
    assert!(!plan.iter().any(|step| step.starts_with("SCAN claims") || step.starts_with("SCAN pending")), "{plan:#?}");
}

#[test]
fn wake_lookup_preserves_numeric_tag_spelling_and_subject_at_signs() {
    for subject in ["step-run/r/s", "step-run/r@nested/s@inner"] {
        let store = Store::open_memory("fixture").unwrap();
        let connection = store.connection.write();
        for (index, (attempt, epoch)) in [("02", "3"), ("+2", "3"), ("2", "03"), ("2", "+3")].into_iter().enumerate() {
            let mut value = body("agent/worker", json!([format!("st3-work:{subject}@{attempt}@{epoch}@inc")]));
            // The old SQL finds its canonical needle here, while the semantic parser accepts
            // the noncanonical numeric spelling in the actual tag.
            value["fields"]["content"] = json!(format!("middle st3-work:{subject}@2@3@needle"));
            insert(&connection, &format!("numeric-{index}"), "message.sent", &value.to_string());
        }
        let original = rows(&connection, &canonical_sql(ORIGINAL), false, subject);
        assert_eq!(original.len(), 4);
        assert_eq!(rows(&connection, &work_wake_history_query(true), true, subject), original);
        assert_eq!(rows(&connection, &work_wake_history_query(false), true, subject), original);
    }
}

#[test]
fn wake_lookup_full_replay_restores_missing_derived_candidates() {
    let store = Store::open_memory("fixture").unwrap();
    let input: ClaimInput = serde_json::from_value(json!({
        "subject": "message/replay-wake", "kind": "message.sent", "actor": "daemon/runtime",
        "fields": {"from": "daemon/runtime", "to": "agent/worker", "content": "",
            "status": "sent", "tags": ["fractal-work:step-run/r/s@2@3@replay"]},
        "evidence": [], "expected_subject": null, "idempotency_key": null
    })).unwrap();
    store.append_claim(&input).unwrap();
    let expected = assert_history(&store.readers.get(), true);
    assert_eq!(expected.len(), 1);
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hea974d72d5e3 owner=fractal_store::store::wake_lookup_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_work_wake_tags", []).unwrap();
        replay_graph_from_nothing_tx(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    assert!(wake_lookup::status(&store.readers.get()).unwrap().2);
    assert_eq!(assert_history(&store.readers.get(), true), expected);
}
