//! The GitHub plugin's own database, `<state>/plugins/github/github.sqlite3`: the poll cursor of
//! each observer this member polls. It never syncs and is not in the graph's backup. It is a
//! cache that GitHub rebuilds: a member without a cursor polls once from nothing, a baseline.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, params};

/// The schema version this build writes, in `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 1;

pub struct GithubPluginStore {
    connection: Mutex<Connection>,
}

impl GithubPluginStore {
    /// The database beneath `state_dir`, created on first use.
    pub fn open(state_dir: &Path) -> Result<Self> {
        let directory = state_dir.join("plugins").join("github");
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("create {}", directory.display()))?;
        let path = directory.join("github.sqlite3");
        let connection =
            Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Self::migrate(connection)
    }

    /// A database that lasts as long as this value: every restart starts without cursors.
    pub fn memory() -> Self {
        Self::migrate(Connection::open_in_memory().expect("an in-memory database opens"))
            .expect("an in-memory database migrates")
    }

    fn migrate(connection: Connection) -> Result<Self> {
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        anyhow::ensure!(
            version <= SCHEMA_VERSION,
            "the GitHub plugin database is version {version}; this build reads up to {SCHEMA_VERSION}"
        );
        if version < 1 {
            connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS observer_cursors(
                     observer TEXT PRIMARY KEY,
                     resource TEXT NOT NULL,
                     cursor TEXT NOT NULL,
                     updated_at_unix_ms INTEGER NOT NULL
                 ) WITHOUT ROWID;
                 PRAGMA user_version = 1;",
            )?;
        }
        Ok(Self { connection: Mutex::new(connection) })
    }

    /// The cursor `observer` last polled to on this member.
    pub fn cursor(&self, observer: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .query_row(
                "SELECT cursor FROM observer_cursors WHERE observer=?1",
                [observer],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Record where `observer` polled `resource` to. No cursor forgets it, so the next poll is a
    /// baseline.
    pub fn set_cursor(&self, observer: &str, resource: &str, cursor: Option<&str>) -> Result<()> {
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        match cursor {
            Some(cursor) => connection.execute(
                "INSERT INTO observer_cursors(observer, resource, cursor, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(observer) DO UPDATE SET
                     resource=excluded.resource, cursor=excluded.cursor,
                     updated_at_unix_ms=excluded.updated_at_unix_ms",
                params![observer, resource, cursor, super::now_ms() as i64],
            )?,
            None => connection
                .execute("DELETE FROM observer_cursors WHERE observer=?1", [observer])?,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_outlives_the_process_that_wrote_it() {
        let state = tempfile::tempdir().unwrap();
        let first = GithubPluginStore::open(state.path()).unwrap();
        first.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", Some("c1")).unwrap();
        first.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", Some("c2")).unwrap();
        drop(first);
        assert!(state.path().join("plugins/github/github.sqlite3").is_file());
        let reopened = GithubPluginStore::open(state.path()).unwrap();
        assert_eq!(reopened.cursor("observer/github/acme/garden").unwrap().as_deref(), Some("c2"));
        assert_eq!(reopened.cursor("observer/github/acme/other").unwrap(), None);
        reopened.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", None).unwrap();
        assert_eq!(reopened.cursor("observer/github/acme/garden").unwrap(), None);
    }

    #[test]
    fn a_newer_database_is_refused() {
        let state = tempfile::tempdir().unwrap();
        drop(GithubPluginStore::open(state.path()).unwrap());
        let path = state.path().join("plugins/github/github.sqlite3");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 2).unwrap();
        let error = GithubPluginStore::open(state.path()).err().unwrap();
        assert!(error.to_string().contains("version 2"), "{error}");
    }
}
