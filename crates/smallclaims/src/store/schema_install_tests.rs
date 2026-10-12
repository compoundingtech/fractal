use super::runtime::Plain;
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

    connection.execute_batch(FRESH_SCHEMA).unwrap();
    assert_eq!(commits.load(Ordering::Relaxed), 1);
    connection
        .execute("INSERT INTO meta VALUES('fixture', 'preserved')", [])
        .unwrap();
    connection.execute_batch(SCHEMA).unwrap();
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
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE claims(id TEXT PRIMARY KEY); INSERT INTO claims VALUES('existing'); PRAGMA user_version=18;").unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let error = Connection::open(&path)
        .unwrap()
        .execute_batch(FRESH_SCHEMA)
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
        .execute_batch("CREATE TABLE claims(id TEXT PRIMARY KEY); PRAGMA user_version=18;")
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
    writer.execute_batch(FRESH_SCHEMA).unwrap();
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
    reader.execute_batch(SCHEMA).unwrap();
    assert_eq!(skipped_seeds.load(Ordering::Relaxed), 2);
    assert!(reader.is_autocommit());
    assert!(reader.execute_batch("BEGIN IMMEDIATE;").is_err());
}
