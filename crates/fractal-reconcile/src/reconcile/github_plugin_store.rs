//! The GitHub plugin's own database, `<state>/plugins/github/github.sqlite3`: the poll cursor of
//! each observer this member polls. It never syncs and is not in the graph's backup. It is a cache:
//! each observer's claim keeps how far it has read, so a member without a row reads on from there
//! and delivers only what differs from the facts the graph already recorded.

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
    /// The database beneath `state_dir`, created on first use. A file this build cannot use,
    /// damaged or from a newer build, is set aside beside it and replaced by an empty one, since
    /// the claims rebuild what it held. Only a directory that cannot be made is an error.
    pub fn open(state_dir: &Path) -> Result<Self> {
        let directory = state_dir.join("plugins").join("github");
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("create {}", directory.display()))?;
        let path = directory.join("github.sqlite3");
        match Self::open_at(&path) {
            Ok(store) => Ok(store),
            Err(error) => {
                let at = super::now_ms();
                let aside = (0..)
                    .map(|n| directory.join(format!("github.sqlite3.set-aside-{at}-{n}")))
                    .find(|aside| !aside.exists())
                    .expect("an unused name");
                for suffix in ["", "-wal", "-shm"] {
                    let from = directory.join(format!("github.sqlite3{suffix}"));
                    if from.exists() {
                        let to = aside.with_file_name(format!(
                            "{}{suffix}",
                            aside.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        std::fs::rename(&from, &to).with_context(|| {
                            format!("set aside {} after: {error:#}", from.display())
                        })?;
                    }
                }
                tracing::warn!(
                    path = %path.display(),
                    aside = %aside.display(),
                    "the GitHub plugin database could not be used and was set aside; observers read on from their claims: {error:#}"
                );
                Self::open_at(&path)
            }
        }
    }

    fn open_at(path: &Path) -> Result<Self> {
        let connection =
            Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Self::migrate(connection)
    }

    /// A database that lasts as long as this value, for tests.
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
                /* sqlite-large-value: allow-for-now lv-h8ad3192acc9b owner=fractal_reconcile::reconcile::github_plugin_store TODO=chunk-or-reference-variable-columns */
                "CREATE TABLE IF NOT EXISTS observer_cursors(
                     observer TEXT PRIMARY KEY,
                     resource TEXT NOT NULL,
                     cursor TEXT,
                     updated_at_unix_ms INTEGER NOT NULL
                 ) WITHOUT ROWID;
                 PRAGMA user_version = 1;",
            )?;
        }
        Ok(Self { connection: Mutex::new(connection) })
    }

    /// Where `observer` last polled to on this member: `None` when it never polled here (or
    /// this database is new), `Some(None)` when its last poll here ended without a cursor.
    pub fn cursor(&self, observer: &str) -> Result<Option<Option<String>>> {
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

    /// Record where `observer` polled `resource` to. The row stays when the cursor is gone, so
    /// this member reads its own state from then on, never a cursor in a claim.
    pub fn set_cursor(&self, observer: &str, resource: &str, cursor: Option<&str>) -> Result<()> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .execute(
                /* sqlite-large-value: allow-for-now lv-he3683455757b owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */
                "INSERT INTO observer_cursors(observer, resource, cursor, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(observer) DO UPDATE SET
                     resource=excluded.resource, cursor=excluded.cursor,
                     updated_at_unix_ms=excluded.updated_at_unix_ms",
                params![observer, resource, cursor, super::now_ms() as i64],
            )?;
        Ok(())
    }

    /// Drop `observer`'s row, once this member no longer polls it: it stopped, or another member
    /// declares it. Polling here again reads on from the observer's claim.
    pub fn forget(&self, observer: &str) -> Result<()> {
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let known = connection
            .query_row("SELECT 1 FROM observer_cursors WHERE observer=?1", [observer], |_| Ok(()))
            .optional()?
            .is_some();
        if known {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-hf4c868e4e1e9 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */
                "DELETE FROM observer_cursors WHERE observer=?1",
                [observer],
            )?;
        }
        Ok(())
    }

    /// Refuse every later write, as a full disk or a read-only file does, or accept them again.
    #[cfg(test)]
    pub(crate) fn refuse_writes(&self, refuse: bool) {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pragma_update(None, "query_only", refuse)
            .unwrap();
    }

    /// Fail every later read, as a damaged file does, or read again.
    #[cfg(test)]
    pub(crate) fn fail_reads(&self, fail: bool) {
        let (from, to) = if fail { ("observer_cursors", "unreadable") } else { ("unreadable", "observer_cursors") };
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}"))
            .unwrap();
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
        assert_eq!(reopened.cursor("observer/github/acme/garden").unwrap(), Some(Some("c2".into())));
        assert_eq!(reopened.cursor("observer/github/acme/other").unwrap(), None);
        reopened.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", None).unwrap();
        assert_eq!(reopened.cursor("observer/github/acme/garden").unwrap(), Some(None));
        reopened.forget("observer/github/acme/garden").unwrap();
        reopened.forget("observer/github/acme/other").unwrap();
        assert_eq!(reopened.cursor("observer/github/acme/garden").unwrap(), None);
    }

    #[test]
    fn a_database_that_cannot_open_is_an_error() {
        let state = tempfile::tempdir().unwrap();
        std::fs::create_dir(state.path().join("plugins")).unwrap();
        std::fs::write(state.path().join("plugins/github"), b"not a directory").unwrap();
        assert!(GithubPluginStore::open(state.path()).is_err());
    }

    /// A newer build's file and a damaged one are each set aside, not read, and the member
    /// starts with an empty database instead of refusing to start.
    #[test]
    fn a_newer_or_damaged_database_is_set_aside() {
        let state = tempfile::tempdir().unwrap();
        let directory = state.path().join("plugins/github");
        let path = directory.join("github.sqlite3");
        let set_aside = || std::fs::read_dir(&directory).unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("github.sqlite3.set-aside-") && !name.ends_with("-wal") && !name.ends_with("-shm"))
            .count();
        let first = GithubPluginStore::open(state.path()).unwrap();
        first.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", Some("c1")).unwrap();
        drop(first);
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 2).unwrap();
        let reopened = GithubPluginStore::open(state.path()).unwrap();
        assert_eq!(reopened.cursor("observer/github/acme/garden").unwrap(), None);
        assert_eq!(set_aside(), 1);
        drop(reopened);
        std::fs::write(&path, b"not a database").unwrap();
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(directory.join(format!("github.sqlite3{suffix}")));
        }
        let rebuilt = GithubPluginStore::open(state.path()).unwrap();
        rebuilt.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", Some("c2")).unwrap();
        assert_eq!(rebuilt.cursor("observer/github/acme/garden").unwrap(), Some(Some("c2".into())));
        assert_eq!(set_aside(), 2);
    }
}
