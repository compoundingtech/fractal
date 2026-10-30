use super::runtime::Plain;
use super::contents::{differences, each_in_parallel, scratch_root, stored};
use super::{FRESH_SCHEMA, SCHEMA, Store};
use rusqlite::Connection;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn core_schema_uses_one_commit_and_reopens_without_losing_data() {
    let directory = tempfile::tempdir().unwrap();
    let connection = Connection::open(directory.path().join("schema.sqlite")).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let commits = Arc::new(AtomicUsize::new(0));
    let observed = commits.clone();
    connection.commit_hook(Some(move || {
        observed.fetch_add(1, Ordering::Relaxed);
        false
    }));

    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hbadf3021d128 owner=smallclaims::store::schema_install_tests TODO=trace-statement-and-value-representation */ FRESH_SCHEMA).unwrap();
    assert_eq!(commits.load(Ordering::Relaxed), 1);
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h71896d15d4ea owner=smallclaims::store::schema_install_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta VALUES('fixture', 'preserved')", [])
        .unwrap();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h76a4ff5e7aa3 owner=smallclaims::store::schema_install_tests TODO=trace-statement-and-value-representation */ SCHEMA).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM meta WHERE key='fixture'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "preserved"
    );
    assert!(connection.is_autocommit());
    assert_eq!(
        connection
            .pragma_query_value(None, "synchronous", |row| row.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, u32>(0))
            .unwrap(),
        1
    );
}

#[test]
fn failed_core_schema_install_preserves_existing_data_without_partial_objects() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schema.sqlite");
    {
        let connection = Connection::open(&path).unwrap();
        // A supported-version store with a damaged claims table fails partway through DDL.
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h8588b56bee1f owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "PRAGMA journal_mode=WAL; CREATE TABLE claims(id TEXT PRIMARY KEY); INSERT INTO claims VALUES('existing'); PRAGMA user_version=18;").unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let error = Connection::open(&path)
        .unwrap()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h9b594f38a048 owner=smallclaims::store::schema_install_tests TODO=trace-statement-and-value-representation */ FRESH_SCHEMA)
        .unwrap_err();
    assert!(error.to_string().contains("subject"), "{error:#}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT id FROM claims", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "existing"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('meta', 'batches')",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn existing_incomplete_core_install_keeps_completed_ddl() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schema.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hdde097353ef7 owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims(id TEXT PRIMARY KEY); PRAGMA user_version=18;")
        .unwrap();
    assert!(Store::open(&path, "birch", Arc::new(Plain)).is_err());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('meta','batches')",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn complete_core_ddl_does_not_take_a_held_writer_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schema.sqlite");
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch(/* sqlite-large-value: allow-for-now lv-hf1ae75680d42 owner=smallclaims::store::schema_install_tests TODO=trace-statement-and-value-representation */ FRESH_SCHEMA).unwrap();
    writer
        .execute_batch("PRAGMA user_version=18; BEGIN IMMEDIATE;")
        .unwrap();
    let reader = Connection::open(&path).unwrap();
    reader.busy_timeout(std::time::Duration::ZERO).unwrap();
    let skipped_seeds = Arc::new(AtomicUsize::new(0));
    let observed = skipped_seeds.clone();
    reader.authorizer(Some(move |context: rusqlite::hooks::AuthContext<'_>| {
        use rusqlite::hooks::{AuthAction, Authorization};
        // Keep zero timeout while executing the production script's connection PRAGMAs.
        if matches!(context.action, AuthAction::Pragma { pragma_name, .. } if pragma_name == "busy_timeout") {
            Authorization::Ignore
        } else if matches!(context.action, AuthAction::Insert { table_name } if matches!(table_name, "graph_generation" | "checkpoint_capture_epoch")) {
            // These two pre-existing seed writes already require writer admission. Check the
            // affected DDL independently, without changing either seed in production.
            observed.fetch_add(1, Ordering::Relaxed);
            Authorization::Ignore
        } else {
            Authorization::Allow
        }
    }));
    // The writer remains held until this returns: added write admission would fail, not pass late.
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-h06c8c7732dec owner=smallclaims::store::schema_install_tests TODO=trace-statement-and-value-representation */ SCHEMA).unwrap();
    assert_eq!(skipped_seeds.load(Ordering::Relaxed), 2);
    assert!(reader.is_autocommit());
    assert!(reader.execute_batch(/* sqlite-large-value: allow-for-now lv-h759f01e6116b owner=smallclaims::store::schema_install_tests TODO=trace-statement-and-value-representation */ "BEGIN IMMEDIATE;").is_err());
}


#[test]
fn a_sqlite_prefixed_user_table_keeps_the_populated_store_path() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h7623bdc4ebe4 owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE sqlitebirds(value TEXT); INSERT INTO sqlitebirds VALUES('preserved'); PRAGMA user_version=18;").unwrap();
    let began = Arc::new(AtomicUsize::new(0));
    let first_meta = Arc::new(AtomicUsize::new(0));
    let seen_begin = began.clone();
    let observed_meta = first_meta.clone();
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Begin,
            } => {
                seen_begin.store(1, Ordering::Relaxed);
            }
            AuthAction::CreateTable { table_name: "meta" } => {
                let before = seen_begin.load(Ordering::Relaxed);
                let _ = observed_meta.compare_exchange(
                    0,
                    before + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
            _ => {}
        }
        Authorization::Allow
    }));
    Store::create_schema(&connection, &Plain).unwrap();
    assert_eq!(
        first_meta.load(Ordering::Relaxed),
        1,
        "core DDL must run without an explicit fresh-install transaction"
    );
    assert_eq!(
        connection
            .query_row("SELECT value FROM sqlitebirds", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
}

#[test]
fn catalog_eligibility_ignores_only_literal_internal_prefixes() {
    let predicates = |connection: &Connection| {
        let old = connection
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap();
        let new = connection
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap();
        (old, new)
    };
    for (case, setup, expected) in [
        ("empty", "", (true, true)),
        (
            "sequence only",
            /* sqlite-large-value: allow-for-now lv-hd6b76d8e5547 owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE ordinary(id INTEGER PRIMARY KEY AUTOINCREMENT); DROP TABLE ordinary;",
            (true, true),
        ),
        (
            "statistics only",
            /* sqlite-large-value: allow-for-now lv-hdfa643afc189 owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE ordinary(value TEXT); ANALYZE; DROP TABLE ordinary;",
            (true, true),
        ),
        (
            "user prefix",
            /* sqlite-large-value: allow-for-now lv-ha7bb6b3168d1 owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE sqlitebirds(value TEXT);",
            (true, false),
        ),
        (
            "mixed user prefix",
            /* sqlite-large-value: allow-for-now lv-h228af6311daa owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE SQLITEbirds(value TEXT);",
            (true, false),
        ),
        ("core table", /* sqlite-large-value: allow-for-now lv-h4a8e9cf971a2 owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE meta(key TEXT);", (false, false)),
    ] {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hf177f99d95aa owner=smallclaims::store::schema_install_tests TODO=trace-statement-and-value-representation */ setup).unwrap();
        if case == "sequence only" {
            assert!(
                connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='sqlite_sequence')",
                        [],
                        |row| row.get::<_, bool>(0)
                    )
                    .unwrap()
            );
        }
        if case == "statistics only" {
            assert!(
                connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='sqlite_stat1')",
                        [],
                        |row| row.get::<_, bool>(0)
                    )
                    .unwrap()
            );
        }
        assert_eq!(predicates(&connection), expected, "{case}");
    }
    let internal = Connection::open_in_memory().unwrap();
    for reserved in ["sqlite_x", "SQLITE_x"] {
        let error = internal
            .execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-h25d326f14966 owner=smallclaims::store::schema_install_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE {reserved}(value TEXT);"))
            .unwrap_err();
        assert!(error.to_string().contains("reserved"), "{error}");
    }
    assert_eq!(predicates(&internal), (true, true));
    // A synthetic internal catalog row checks the literal prefix without claiming user DDL
    // can create this name. This private in-memory connection is never used as a real store.
    internal.execute_batch(/* sqlite-large-value: allow-for-now lv-ha1f81bed509d owner=smallclaims::store::schema_install_tests TODO=bound-or-reference-values-and-reclamation */ "PRAGMA writable_schema=ON; INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql) VALUES('table','sqlite_x','sqlite_x',0,NULL);").unwrap();
    assert_eq!(predicates(&internal), (true, true));
}

#[test]
fn fresh_auxiliary_schema_families_share_durable_commits() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::Mutex;
    let directory = tempfile::tempdir().unwrap();
    let connection = Connection::open(directory.path().join("schema.sqlite")).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let commits = Arc::new(AtomicUsize::new(0));
    let observed = commits.clone();
    connection.commit_hook(Some(move || {
        observed.fetch_add(1, Ordering::Relaxed);
        false
    }));
    let installations = Arc::new(Mutex::new(Vec::new()));
    let recorded = installations.clone();
    let committed = commits.clone();
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        let family = match context.action {
            AuthAction::CreateTrigger { trigger_name, .. }
                if trigger_name.starts_with("fleet_generation_") =>
            {
                Some("fleet")
            }
            AuthAction::CreateTrigger { trigger_name, .. }
                if trigger_name.contains("_inventory_") =>
            {
                Some("inventory")
            }
            AuthAction::CreateTrigger { trigger_name, .. }
                if trigger_name.ends_with("_graph_insert")
                    || trigger_name.ends_with("_graph_update")
                    || trigger_name.ends_with("_graph_delete") =>
            {
                Some("graph")
            }
            AuthAction::CreateTable {
                table_name: "claim_signatures" | "claim_verdicts" | "claim_verdict_links" | "held_keys",
            } => Some("principals"),
            _ => None,
        };
        if let Some(family) = family {
            recorded
                .lock()
                .unwrap()
                .push((family, committed.load(Ordering::Relaxed)));
        }
        Authorization::Allow
    }));
    Store::create_schema(&connection, &Plain).unwrap();
    // Plain has no legacy graph tables. Exercise the same fresh-install batch with
    // two existing graph tables, as a runtime that supplies a legacy digest does.
    super::install_schema_batch(&connection, true, |connection| {
        super::create_graph_generation_triggers(
            connection,
            &[
                ("operations", "operations", &["id", "state"], "id"),
                ("documents", "documents", &["name", "hash"], "name"),
            ],
        )
    })
    .unwrap();
    connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let installations = installations.lock().unwrap();
    for family in ["fleet", "inventory", "graph", "principals"] {
        let positions = installations
            .iter()
            .filter(|(name, _)| *name == family)
            .map(|(_, position)| *position)
            .collect::<Vec<_>>();
        assert!(positions.len() > 1, "missing {family} DDL: {positions:?}");
        assert!(
            positions.iter().all(|position| *position == positions[0]),
            "{family} DDL committed separately: {positions:?}"
        );
    }
    assert!(connection.is_autocommit());
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h601898e1e0ad owner=smallclaims::store::schema_install_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta VALUES('fixture','preserved')", [])
        .unwrap();
    Store::create_schema(&connection, &Plain).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM meta WHERE key='fixture'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "preserved"
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "synchronous", |row| row.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, u32>(0))
            .unwrap(),
        1
    );
}

fn writer_synchronous(store: &Store) -> u32 {
    store
        .connection
        .write()
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .unwrap()
}

#[test]
fn a_fresh_open_with_deferred_durability_stores_what_committing_each_step_stores() {
    let directory = tempfile::tempdir_in(scratch_root()).unwrap();
    let deferred = directory.path().join("deferred.sqlite");
    let each = directory.path().join("each.sqlite");
    let store = Store::open(&deferred, "node", Arc::new(Plain)).unwrap();
    assert_eq!(writer_synchronous(&store), 2, "a fresh open leaves the writer at FULL");
    drop(store);
    drop(Store::open_for_initialization_test(&each, "node", Arc::new(Plain), false, |_| {}).unwrap());
    let difference = differences(&stored(&deferred), &stored(&each));
    assert!(difference.is_empty(), "{difference:#?}");
    let reopened = Store::open(&deferred, "node", Arc::new(Plain)).unwrap();
    assert_eq!(writer_synchronous(&reopened), 2, "an existing store opens at FULL");
}

/// How a fresh open stopped after its first `kept` commits reopens: the store it then holds,
/// or why it does not open. `defer` picks deferred durability or a commit per step.
fn cut_and_reopen(directory: &std::path::Path, defer: bool, kept: usize) -> Result<(Vec<String>, Vec<String>), String> {
    let path = directory.join(format!("cut-{defer}-{kept}.sqlite"));
    let seen = Arc::new(AtomicUsize::new(0));
    // Every commit after the first `kept` rolls back, as if the process stopped there.
    let opened = Store::open_for_initialization_test(&path, "node", Arc::new(Plain), defer, move |connection| {
        connection.commit_hook(Some(move || seen.fetch_add(1, Ordering::Relaxed) >= kept));
    });
    assert!(opened.is_err(), "the open cut after {kept} commits still succeeded");
    drop(opened);
    let reopened = Store::open(&path, "node", Arc::new(Plain)).map_err(|error| format!("{error:#}"))?;
    assert_eq!(writer_synchronous(&reopened), 2);
    drop(reopened);
    Ok(stored(&path))
}

fn fresh_commits(path: &std::path::Path, defer: bool) -> usize {
    let commits = Arc::new(AtomicUsize::new(0));
    let counted = commits.clone();
    drop(
        Store::open_for_initialization_test(path, "node", Arc::new(Plain), defer, move |connection| {
            connection.commit_hook(Some(move || {
                counted.fetch_add(1, Ordering::Relaxed);
                false
            }));
        })
        .unwrap(),
    );
    commits.load(Ordering::Relaxed)
}

#[test]
fn a_fresh_open_cut_after_any_commit_reopens_as_committing_each_step_did() {
    // Deferring durability skips fsyncs, not commits: the same commits happen in the same order,
    // so any prefix a crash leaves is one the old sequence could leave too, and reopens alike.
    let directory = tempfile::tempdir_in(scratch_root()).unwrap();
    let commits = fresh_commits(&directory.path().join("deferred.sqlite"), true);
    assert_eq!(commits, fresh_commits(&directory.path().join("each.sqlite"), false));
    assert!(commits > 2, "a fresh open commits more than once: {commits}");
    each_in_parallel(commits, |kept| {
        // Each cut's stores go once it is checked, so the scratch holds a few at a time.
        let cut = tempfile::tempdir_in(directory.path()).unwrap();
        let deferred = cut_and_reopen(cut.path(), true, kept);
        let each = cut_and_reopen(cut.path(), false, kept);
        match (&deferred, &each) {
            (Ok(deferred), Ok(each)) => {
                let difference = differences(deferred, each);
                assert!(difference.is_empty(), "cut after {kept} of {commits}: {difference:#?}");
            }
            (Err(deferred), Err(each)) => assert_eq!(deferred, each, "cut after {kept} of {commits}"),
            _ => panic!("cut after {kept} of {commits}: deferred {:?} but each step {:?}",
                deferred.as_ref().map(|_| "reopens"), each.as_ref().map(|_| "reopens")),
        }
    });
}

#[test]
fn a_fresh_open_cut_after_any_commit_reopens_to_the_store_a_whole_open_makes() {
    // The schema version commits with the first tables, so no cut leaves tables at version 0,
    // which a reopen refuses as an old store (#3777); the reopen completes the store instead.
    // The test above shows that committing each step reopens alike.
    let directory = tempfile::tempdir_in(scratch_root()).unwrap();
    let whole = directory.path().join("whole.sqlite");
    let commits = fresh_commits(&whole, true);
    let expected = stored(&whole);
    each_in_parallel(commits, |kept| {
        let cut = tempfile::tempdir_in(directory.path()).unwrap();
        let reopened = cut_and_reopen(cut.path(), true, kept)
            .unwrap_or_else(|error| panic!("cut after {kept} of {commits}: {error}"));
        let difference = differences(&reopened, &expected);
        assert!(difference.is_empty(), "cut after {kept} of {commits}: {difference:#?}");
    });
}
