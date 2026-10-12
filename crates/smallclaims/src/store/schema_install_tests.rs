use super::runtime::Plain;
use super::{SCHEMA, Store};
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

    connection.execute_batch(SCHEMA).unwrap();
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
        connection.execute_batch("CREATE TABLE claims(id TEXT PRIMARY KEY); INSERT INTO claims VALUES('existing'); PRAGMA user_version=18;").unwrap();
    }
    let error = match Store::open(&path, "birch", Arc::new(Plain)) {
        Ok(_) => panic!("the damaged schema must refuse to open"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("subject"), "{error:#}");
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
