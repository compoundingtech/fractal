//! Offline preparation of the mandatory conversation-owner query indexes.
use std::{io::Write, path::Path, time::Duration};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};

pub const INDEX_NAMES: [&str; 5] = [
    "claims_transport_pair_accepted_index",
    "claims_actual_accepted_index",
    "claims_runtime_origin_accepted_index",
    "claims_identity_runtime_id_accepted_index",
    "claims_identity_incarnation_id_accepted_index",
];

fn install(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h00d5d1e046d7 owner=fractal_store::owner_indexes TODO=trace-statement-and-value-representation */ smallclaims::store::TRANSPORT_PAIR_INDEX)?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hc41ef8140f43 owner=fractal_store::owner_indexes TODO=trace-statement-and-value-representation */ &crate::store::RUNTIME_ID_INDEX)?;
    Ok(())
}

fn definitions(connection: &Connection) -> Result<Vec<Option<String>>> {
    INDEX_NAMES.iter().map(|name| {
        Ok(connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name=?1",
            [name], |row| row.get(0),
        ).optional()?)
    }).collect()
}

fn report(output: &mut impl Write, phase: &str, definitions: &[Option<String>]) -> Result<()> {
    for (name, sql) in INDEX_NAMES.iter().zip(definitions) {
        writeln!(output, "{phase}: {name}: {}", if sql.is_some() { "exists" } else { "missing" })?;
    }
    Ok(())
}

fn expected_definitions() -> Result<Vec<Option<String>>> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h2d43c40cfa36 owner=fractal_store::owner_indexes TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims (store_index INTEGER PRIMARY KEY, subject TEXT, kind TEXT, origin TEXT, body TEXT, accepted_at_unix_ms TEXT);")?;
    install(&connection)?;
    definitions(&connection)
}

/// Never opens the full Store: no migrations, projection work, or implicit store creation.
/// Hold both the daemon lifetime fence and SQLite's writer reservation through the build.
pub fn prepare(store: &Path, output: &mut impl Write) -> Result<()> {
    let store = store.canonicalize().context("resolve explicit existing store path")?;
    let parent = store.parent().context("store has no parent directory")?;
    let _state_lock = crate::node_identity::lock(parent)
        .context("stop all daemon writers before preparing owner indexes")?;
    let mut connection = Connection::open_with_flags(&store, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::ZERO)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)
        .context("store writer lock is held; stop all writers before preparing owner indexes")?;
    report(output, "before", &definitions(&transaction)?)?;
    install(&transaction)?;
    let after = definitions(&transaction)?;
    report(output, "after", &after)?;
    anyhow::ensure!(after == expected_definitions()?, "owner-index schema gate failed: missing or unexpected index definitions");
    transaction.commit()?;
    let (busy, _, _): (i64, i64, i64) = connection.query_row(
        "PRAGMA wal_checkpoint(TRUNCATE)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    anyhow::ensure!(busy == 0, "owner-index checkpoint is busy; keep the daemon stopped");
    writeln!(output, "owner-index schema gate passed; wal_checkpoint(TRUNCATE) completed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(smallclaims::store::SCHEMA).unwrap();
        for name in INDEX_NAMES {
            connection.execute_batch(&format!("DROP INDEX IF EXISTS {name}")).unwrap();
        }
        (directory, path)
    }

    #[test]
    fn prepares_idempotently_with_startup_identical_definitions() {
        let (_directory, path) = fixture();
        let mut output = Vec::new();
        prepare(&path, &mut output).unwrap();
        let connection = Connection::open(&path).unwrap();
        let before: i64 = connection.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
        let sql = definitions(&connection).unwrap();
        prepare(&path, &mut output).unwrap();
        let after: i64 = connection.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
        assert_eq!(before, after);
        assert_eq!(sql, definitions(&connection).unwrap());
        let startup = crate::store::Store::open_memory("owner-index-startup").unwrap();
        assert_eq!(sql, definitions(&startup.connection.write()).unwrap());
        let output = String::from_utf8(output).unwrap();
        for name in INDEX_NAMES {
            assert!(output.contains(&format!("before: {name}: missing")));
            assert!(output.contains(&format!("after: {name}: exists")));
        }
        let wal = std::fs::metadata(format!("{}-wal", path.display())).unwrap();
        assert_eq!(wal.len(), 0);
    }

    #[test]
    fn refuses_daemon_lifetime_lock() {
        let (directory, path) = fixture();
        let _lock = crate::node_identity::lock(directory.path()).unwrap();
        assert!(prepare(&path, &mut Vec::new()).unwrap_err().to_string().contains("stop all daemon writers"));
    }

    #[test]
    fn refuses_sqlite_writer_lock() {
        let (_directory, path) = fixture();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert!(prepare(&path, &mut Vec::new()).unwrap_err().to_string().contains("store writer lock is held"));
        connection.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn rejects_wrong_definition_and_missing_store() {
        let (_directory, path) = fixture();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE INDEX claims_actual_accepted_index ON claims(subject)").unwrap();
        assert!(prepare(&path, &mut Vec::new()).unwrap_err().to_string().contains("schema gate failed"));
        assert!(prepare(&path.with_file_name("absent.sqlite3"), &mut Vec::new()).is_err());
        assert!(!path.with_file_name("absent.sqlite3").exists());
    }
}
