use super::*;

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
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        for number in 0..ROWS {
            let subject = format!("exec/index-startup-{number:05}");
            append_claim_tx(
                &transaction, "node", &subject, "intent.desired", None,
                &json!({"subject":subject,"kind":"exec","desired":{"number":number}}),
                &[], None,
            ).unwrap();
        }
        transaction.commit().unwrap();
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM desired", [], |row| row.get::<_, u64>(0)).unwrap(), ROWS);
        // Model an existing member's old schema; normal Store::open must do the upgrade.
        connection.execute_batch("DROP INDEX desired_kind_subject_index").unwrap();
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
