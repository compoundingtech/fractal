use super::*;
use super::desired_batch::DesiredBatchRequest;

#[test]
fn kind_filtered_desired_pages_seek_the_kind_subject_index() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.write();
    // Keep this literal aligned with desired_batch's all-subject, kind-set branch.
    let plan = connection.prepare(
        "EXPLAIN QUERY PLAN
         SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
         FROM desired INDEXED BY desired_kind_subject_index
         WHERE ?1='[]' AND kind=?2 AND subject>?3 ORDER BY subject LIMIT ?4",
    ).unwrap().query_map(params!["[]", "mission", "", 2], |row| row.get::<_, String>(3))
        .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert!(plan.iter().any(|step| step.contains("SEARCH desired USING INDEX desired_kind_subject_index")
        && step.contains("kind=? AND subject>?")), "{plan:?}");
    assert!(!plan.iter().any(|step| step.contains("SCAN ")), "{plan:?}");
}

#[test]
fn mission_desired_pages_seek_both_primary_keys() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.write();
    // Missions use a separate selected-definition family, not a desired kind OR filter.
    let plan = connection.prepare(
        "EXPLAIN QUERY PLAN
         SELECT 'mission/' || d.mission_id, 'mission', r.body, NULL, NULL, NULL, NULL
         FROM mission_definitions d JOIN mission_revisions r
         ON r.mission_id=d.mission_id AND r.revision=d.revision
         WHERE ?1 AND ?2='[]' AND d.mission_id>?3 ORDER BY d.mission_id LIMIT ?4",
    ).unwrap().query_map(params![true, "[]", "", 2], |row| row.get::<_, String>(3))
        .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert!(plan.iter().any(|step| step.contains("SEARCH d USING INDEX sqlite_autoindex_mission_definitions_")
        && step.contains("mission_id>?")), "{plan:?}");
    assert!(plan.iter().any(|step| step.contains("SEARCH r USING INDEX sqlite_autoindex_mission_revisions_")
        && step.contains("mission_id=? AND revision=?")), "{plan:?}");
    assert!(!plan.iter().any(|step| step.contains("SCAN ")), "{plan:?}");
}

#[test]
fn existing_store_startup_installs_desired_kind_index_without_offline_work() {
    const ROWS: u64 = 4096;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let mut kdl = String::from("version 2\n");
    for number in 0..ROWS {
        kdl.push_str(&format!(
            "agent \"index-startup/{number:05}\" {{ command \"true\"; workspace \".\"; restart \"never\"; }}\n",
        ));
    }
    let intent = crate::graph::parse_intent(&kdl, store.origin()).unwrap();
    crate::store::publication_tests_shared::publish(&store, &intent, "index-startup-fixture");
    {
        let connection = store.connection.write();
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM desired", [], |row| row.get::<_, u64>(0)).unwrap(), ROWS);
        // Model an existing member's old schema; normal Store::open must do the upgrade.
        // Test-owned fixed DDL only: no bound values, new columns, or production writer.
        // Preserve this missing-index startup regression through representation/reclamation changes.
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h2abdf327c0d1 owner=fractal_store::store::desired_batch_index_tests TODO=bound-or-reference-values-and-reclamation */ "DROP INDEX desired_kind_subject_index").unwrap();
        assert!(!connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name='desired_kind_subject_index')",
            [], |row| row.get::<_, bool>(0),
        ).unwrap());
    }
    drop(store);
    let started = std::time::Instant::now();
    let reopened = Store::open(&path, "node").unwrap();
    let elapsed = started.elapsed();
    let connection = reopened.connection.write();
    assert!(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name='desired_kind_subject_index')",
        [], |row| row.get::<_, bool>(0),
    ).unwrap());
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM desired", [], |row| row.get::<_, u64>(0)).unwrap(), ROWS);
    eprintln!("desired kind index existing-store startup: desired_rows={ROWS} elapsed={elapsed:?} (normal Store::open, includes all startup work)");
}

#[test]
fn desired_cursor_expires_when_repair_rewrites_at_the_same_claim_cut_and_graph_health() {
    let store = Store::open_memory("node").unwrap();
    for (command, key) in [("true", "before"), ("false", "after")] {
        let intent = crate::graph::parse_intent(&format!(
            "version 2\nagent \"cursor/a\" {{ command \"{command}\"; workspace \".\"; restart \"never\"; }}\n\
             agent \"cursor/b\" {{ command \"true\"; workspace \".\"; restart \"never\"; }}\n",
        ), store.origin()).unwrap();
        crate::store::publication_tests_shared::publish(&store, &intent, key);
    }
    let selected = store.selected_desired_token("agent/cursor/a").unwrap().unwrap();
    let replacement: String = store.readers.get().query_row(
        "SELECT id FROM claims WHERE subject='agent/cursor/a' AND kind='intent.desired'
         AND id<>?1 ORDER BY store_index LIMIT 1",
        [&selected], |row| row.get(0),
    ).unwrap();
    let request = DesiredBatchRequest { kind: Some("agent".into()), limit: Some(1), ..Default::default() };
    let first = store.desired_batch(&request).unwrap();
    let health = || store.readers.get().query_row(
        "SELECT status,last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
    ).optional().unwrap();
    let before_health = health();
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        // Exercise the production repair selector with already-admitted claims: no
        // append or graph projection step is allowed to accidentally expire the page.
        select_desired_repair_tx(&transaction, &selected, &replacement).unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(store.selected_desired_token("agent/cursor/a").unwrap(), Some(replacement));
    let fresh = store.desired_batch(&request).unwrap();
    assert_eq!(first.store_index, fresh.store_index);
    assert_eq!(before_health, health());
    assert_ne!(first.items[0].declaration.desired, fresh.items[0].declaration.desired);
    let error = store.desired_batch(&DesiredBatchRequest {
        cursor: Some(first.next_cursor.unwrap()), ..request
    }).unwrap_err();
    assert!(format!("{error:?}").contains("desired-cursor-expired"), "{error:?}");
}

#[test]
fn desired_write_witness_tracks_noop_updates_and_deletes_transactionally() {
    let store = Store::open_memory("node").unwrap();
    let intent = crate::graph::parse_intent(
        "version 2\nagent \"cursor/a\" { command \"true\"; workspace \".\"; restart \"never\"; }\n",
        store.origin(),
    ).unwrap();
    crate::store::publication_tests_shared::publish(&store, &intent, "witness");
    let mut connection = store.connection.write();
    let generation = |connection: &Connection| connection.query_row(
        "SELECT generation FROM local_desired_write_witness WHERE id=1",
        [], |row| row.get::<_, u64>(0),
    ).unwrap();
    let before = generation(&connection);
    assert!(before > 0, "publication inserts must advance the witness");
    {
        let transaction = connection.transaction().unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h11f5c377b176 owner=fractal_store::store::desired_batch_index_tests TODO=bound-or-reference-values-and-reclamation */
            "UPDATE desired SET body=body WHERE subject='agent/cursor/a'", [],
        ).unwrap();
        assert_eq!(generation(&transaction), before + 1);
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h727c99989afb owner=fractal_store::store::desired_batch_index_tests TODO=bound-or-reference-values-and-reclamation */
            "DELETE FROM desired WHERE subject='agent/cursor/a'", [],
        ).unwrap();
        assert_eq!(generation(&transaction), before + 2);
        transaction.rollback().unwrap();
    }
    assert_eq!(generation(&connection), before, "rolled-back rows must not expire cursors");
}
