//! The GitHub plugin's own database, `<state>/plugins/github/github.sqlite3`, under the plugin
//! store contract ([`super::plugin_store`]). It never syncs and is not in the graph's backup. It
//! holds, for the repositories this member polls:
//!
//! - each observer's poll cursor. Each observer's claim keeps how far it has read, so a member
//!   without a row reads on from there and delivers only what differs from the recorded facts;
//! - the mirror: every pull request's and issue's full facts, every head SHA with its checks, and
//!   the keyed events derived from each change. The poller serves `/v1/plugins/github/*` from it,
//!   and other members relay those reads here. It is filled from GitHub itself, by a look the
//!   poller records nowhere else, and then follows each look the poller records. Until it is
//!   filled, and again whenever a look finds it behind the poll cursor, reads answer from the
//!   graph and say so. It needs no backup: GitHub is its source.
//!
//! `PRAGMA user_version` stays 1, the cursor table's version, so a build from before the mirror
//! still opens this file after a rollback and ignores the mirror tables. The mirror's own
//! version is the `mirror_version` row of `plugin_meta`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Map, Value, json};

use super::github_events;
use super::plugin_store::{GraphKind, Manifest};

/// The cursor table's version, in `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 1;
/// The mirror tables' version, in `plugin_meta`.
const MIRROR_VERSION: i64 = 1;
/// How long the mirror keeps an event.
const EVENT_RETENTION_MS: u128 = 30 * 24 * 60 * 60 * 1000;
/// The most rows one mirror read returns.
pub const MAX_READ_ROWS: usize = 500;

/// The GitHub plugin and the only graph writes it may make: one compact state per item, written
/// on change, and one claim per keyed event. Neither is written before step 3b.
pub const MANIFEST: Manifest = Manifest {
    name: "github",
    graph_kinds: &[
        GraphKind {
            kind: fractal_schema::github_plugin::ITEM_STATE,
            subject_prefix: "resource/",
            max_fields_bytes: fractal_schema::github_plugin::MAX_ITEM_STATE_BYTES,
        },
        GraphKind {
            kind: fractal_schema::github_plugin::EVENT,
            subject_prefix: "github-event/",
            max_fields_bytes: fractal_schema::github_plugin::MAX_EVENT_BYTES,
        },
    ],
};

/// One item one look recorded: its resource subject, kind (`vcs.pull-request` or `vcs.issue`),
/// full facts, and the graph claim that recorded them.
#[derive(Clone, Debug, PartialEq)]
pub struct MirrorItem {
    pub subject: String,
    pub item_kind: String,
    pub facts: Map<String, Value>,
    pub claim: Option<String>,
}

pub struct GithubPluginStore {
    connection: Mutex<Connection>,
    /// Observers this process knows have no row, so forgetting them again reads nothing.
    absent: Mutex<HashSet<String>>,
    /// Each repository whose fill failed: when the next may start, and the wait after that.
    refills: Mutex<HashMap<String, (u128, u128)>>,
    /// Whether each repository's last look was in compact mode, to log each switch once.
    compact: Mutex<HashMap<String, bool>>,
}

/// The first wait after a failed fill, doubled after each further failure up to an hour.
const REFILL_RETRY_MS: u128 = 60_000;
const REFILL_RETRY_MAX_MS: u128 = 60 * 60_000;

impl GithubPluginStore {
    /// The database beneath `state_dir`, created on first use. A file this build cannot use,
    /// damaged or from a newer build, is set aside beside it and replaced by an empty one, since
    /// the claims rebuild what it held. It is an error, and the daemon does not start, only when
    /// the directory cannot be made, the file cannot be moved aside, or no database opens in its
    /// place.
    pub fn open(state_dir: &Path) -> Result<Self> {
        let connection = super::plugin_store::open(state_dir, MANIFEST.name, Self::migrate)?;
        Ok(Self { connection: Mutex::new(connection), absent: Mutex::default(), refills: Mutex::default(), compact: Mutex::default() })
    }

    /// The one handle this process keeps on the database beneath `state_dir`, opened on first
    /// use: the reconciler polls into it and the API reads from it.
    pub fn shared(state_dir: &Path) -> Result<Arc<Self>> {
        let mut handles = shared_handles().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(handle) = handles.get(state_dir) {
            return Ok(handle.clone());
        }
        let handle = Arc::new(Self::open(state_dir)?);
        handles.insert(state_dir.to_owned(), handle.clone());
        Ok(handle)
    }

    /// Make `store` the handle [`Self::shared`] returns for `state_dir`, for tests.
    pub fn share(state_dir: &Path, store: Arc<Self>) {
        shared_handles().lock().unwrap_or_else(PoisonError::into_inner).insert(state_dir.to_owned(), store);
    }

    /// A database that lasts as long as this value, for tests.
    pub fn memory() -> Self {
        let connection = Connection::open_in_memory().expect("an in-memory database opens");
        Self::migrate(&connection).expect("an in-memory database migrates");
        Self { connection: Mutex::new(connection), absent: Mutex::default(), refills: Mutex::default(), compact: Mutex::default() }
    }

    fn migrate(connection: &Connection) -> Result<()> {
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
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-ha1763c1a36dd owner=fractal_reconcile::reconcile::github_plugin_store TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS plugin_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;",
        )?;
        let mirror: i64 = connection
            .query_row("SELECT value FROM plugin_meta WHERE key='mirror_version'", [], |row| row.get::<_, String>(0))
            .optional()?
            .map_or(Ok(0), |value| value.parse())?;
        anyhow::ensure!(
            mirror <= MIRROR_VERSION,
            "the GitHub plugin mirror is version {mirror}; this build reads up to {MIRROR_VERSION}"
        );
        if mirror < 1 {
            connection.execute_batch(
                /* sqlite-large-value: allow-for-now lv-h4b94ad1aa193 owner=fractal_reconcile::reconcile::github_plugin_store TODO=chunk-or-reference-variable-columns */ "BEGIN;
                 CREATE TABLE IF NOT EXISTS mirror_repositories(
                     repository TEXT PRIMARY KEY,
                     observer TEXT NOT NULL,
                     locator TEXT NOT NULL,
                     mirrored_at_unix_ms INTEGER,
                     filled_at_unix_ms INTEGER,
                     cursor_digest TEXT
                 ) WITHOUT ROWID;
                 CREATE TABLE IF NOT EXISTS mirror_items(
                     repository TEXT NOT NULL,
                     number INTEGER NOT NULL,
                     item_kind TEXT NOT NULL,
                     subject TEXT NOT NULL,
                     state TEXT NOT NULL,
                     facts TEXT NOT NULL,
                     claim TEXT,
                     observed_at_unix_ms INTEGER NOT NULL,
                     PRIMARY KEY(repository, number)
                 ) WITHOUT ROWID;
                 CREATE INDEX IF NOT EXISTS mirror_items_state ON mirror_items(repository, item_kind, state, number);
                 CREATE TABLE IF NOT EXISTS mirror_heads(
                     repository TEXT NOT NULL,
                     number INTEGER NOT NULL,
                     head_sha TEXT NOT NULL,
                     first_seen_unix_ms INTEGER NOT NULL,
                     last_seen_unix_ms INTEGER NOT NULL,
                     checks_state TEXT,
                     checks TEXT,
                     required_checks TEXT,
                     PRIMARY KEY(repository, number, head_sha)
                 ) WITHOUT ROWID;
                 CREATE TABLE IF NOT EXISTS mirror_events(
                     subject TEXT PRIMARY KEY,
                     repository TEXT NOT NULL,
                     number INTEGER NOT NULL,
                     key TEXT NOT NULL,
                     event TEXT NOT NULL,
                     head_sha TEXT,
                     detail TEXT NOT NULL,
                     at TEXT NOT NULL,
                     recorded_at_unix_ms INTEGER NOT NULL,
                     sequence INTEGER NOT NULL
                 ) WITHOUT ROWID;
                 CREATE INDEX IF NOT EXISTS mirror_events_item ON mirror_events(repository, number, recorded_at_unix_ms);
                 CREATE TABLE IF NOT EXISTS mirror_event_keys(
                     subject TEXT PRIMARY KEY,
                     repository TEXT NOT NULL
                 ) WITHOUT ROWID;
                 CREATE INDEX IF NOT EXISTS mirror_event_keys_repository ON mirror_event_keys(repository);
                 CREATE INDEX IF NOT EXISTS mirror_events_recorded ON mirror_events(repository, recorded_at_unix_ms);
                 INSERT OR REPLACE INTO plugin_meta(key, value) VALUES ('mirror_version', '1');
                 COMMIT;",
            )?;
        }
        Ok(())
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
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-he3683455757b owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */
                "INSERT INTO observer_cursors(observer, resource, cursor, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(observer) DO UPDATE SET
                     resource=excluded.resource, cursor=excluded.cursor,
                     updated_at_unix_ms=excluded.updated_at_unix_ms",
                params![observer, resource, cursor, super::now_ms() as i64],
            )?;
        // Under the connection, as forget remembers an absent observer, so a forget that found
        // no row cannot remember it after this row is written.
        self.absent.lock().unwrap_or_else(PoisonError::into_inner).remove(observer);
        Ok(())
    }

    /// Drop `observer`'s row, once this member no longer polls it: it stopped, or another member
    /// declares it. Polling here again reads on from the observer's claim. Every pass asks for
    /// each such observer, so one already gone costs nothing after the first.
    pub fn forget(&self, observer: &str) -> Result<()> {
        if self.absent.lock().unwrap_or_else(PoisonError::into_inner).contains(observer) {
            return Ok(());
        }
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
        let mirrored = {
            let mut statement = connection.prepare_cached(
                "SELECT repository FROM mirror_repositories WHERE observer=?1",
            )?;
            statement.query_map([observer], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?
        };
        for repository in mirrored {
            forget_repository(&connection, &repository)?;
        }
        self.absent.lock().unwrap_or_else(PoisonError::into_inner).insert(observer.to_owned());
        Ok(())
    }

    /// Whether `repository`'s mirror was filled from GitHub and has followed every look since:
    /// only then does it answer reads. A new, moved, restored or set-aside mirror is not, and
    /// neither is one a look found behind the poll cursor, as after a rollback or a crash.
    pub fn filled(&self, repository: &str) -> Result<bool> {
        Ok(self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .query_row(
                "SELECT filled_at_unix_ms IS NOT NULL FROM mirror_repositories WHERE repository=?1",
                [repository],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Record whether `repository`'s look is in compact mode, and whether that switched it: the
    /// first look of a process logs its mode, and each later change logs again.
    pub fn note_compact(&self, repository: &str, compact: bool) -> bool {
        self.compact.lock().unwrap_or_else(PoisonError::into_inner)
            .insert(repository.to_owned(), compact) != Some(compact)
    }

    /// Whether a fill of `repository` may start now: always, unless the last one failed and its
    /// wait has not passed.
    pub fn refill_due(&self, repository: &str, now: u128) -> bool {
        self.refills.lock().unwrap_or_else(PoisonError::into_inner)
            .get(repository)
            .is_none_or(|(next, _)| now >= *next)
    }

    /// Record how a fill of `repository` ended: a failure waits a minute before the next, doubling
    /// up to an hour; a success clears the wait.
    pub fn refill_ended(&self, repository: &str, now: u128, succeeded: bool) {
        let mut refills = self.refills.lock().unwrap_or_else(PoisonError::into_inner);
        if succeeded {
            refills.remove(repository);
            return;
        }
        let wait = refills.get(repository).map_or(REFILL_RETRY_MS, |(_, wait)| (wait * 2).min(REFILL_RETRY_MAX_MS));
        refills.insert(repository.to_owned(), (now.saturating_add(wait), wait));
    }

    /// Whether `repository`'s mirror is filled and followed every look up to `cursor`, the cursor
    /// the next look starts from. A mirror whose write failed while the daemon kept running is
    /// behind that cursor. One the daemon stopped before writing has the same cursor as the
    /// plugin database and may be behind the graph: a look's prior then still takes every
    /// compact and delivery fact from the graph, which records each change to them.
    pub fn current(&self, repository: &str, cursor: Option<&str>) -> Result<bool> {
        let followed: Option<(Option<String>, bool)> = self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .query_row(
                "SELECT cursor_digest, filled_at_unix_ms IS NOT NULL FROM mirror_repositories WHERE repository=?1",
                [repository],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(followed.is_some_and(|(digest, filled)| filled && digest.as_deref() == Some(cursor_digest(cursor).as_str())))
    }

    /// Fill `repository`'s mirror from one look at GitHub that recorded nothing else: each item
    /// it saw replaces the mirror's row, without events, and the mirror then answers reads. The
    /// mirror must still be at the poll cursor the last [`Self::mirror`] recorded; otherwise
    /// nothing is filled and the next look tries again.
    pub fn fill(&self, repository: &str, observer: &str, locator: &str, cursor: Option<&str>, items: &[MirrorItem]) -> Result<bool> {
        self.absent.lock().unwrap_or_else(PoisonError::into_inner).remove(observer);
        let at = super::now_ms();
        let mut connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let transaction = connection.transaction()?;
        let followed: Option<Option<String>> = transaction
            .query_row("SELECT cursor_digest FROM mirror_repositories WHERE repository=?1", [repository], |row| row.get(0))
            .optional()?;
        if followed.flatten().as_deref() != Some(cursor_digest(cursor).as_str()) {
            return Ok(false);
        }
        upsert_repository(&transaction, repository, observer, locator)?;
        for item in items {
            let Some(number) = item_number(item) else { continue };
            write_item(&transaction, repository, number, item, at)?;
        }
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hfdf8ce3fb8e1 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "UPDATE mirror_repositories SET filled_at_unix_ms=?2 WHERE repository=?1",
            params![repository, at as i64],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Record the items one look at `repository` recorded, and the keyed events each change
    /// holds, in one transaction. `polled_from` is the cursor the look started from and `cursor`
    /// the one it ended at. A mirror that did not end at `polled_from`, because looks went by
    /// without it, holds stale rows: it is no longer filled and this look derives no event. An
    /// item the mirror did not hold is a baseline and holds no event either. Returns the
    /// events, oldest first, as the read API shows them.
    pub fn mirror(
        &self,
        repository: &str,
        observer: &str,
        locator: &str,
        polled_from: Option<&str>,
        cursor: Option<&str>,
        items: &[MirrorItem],
    ) -> Result<Vec<Value>> {
        self.mirror_checked(repository, observer, locator, polled_from, cursor, items, &|_| Ok(false))
    }

    /// [`Self::mirror`], where `in_graph` also says whether an event subject is already a
    /// `github.event` claim, as one another poller wrote, so a key is never handed out twice.
    #[allow(clippy::too_many_arguments)]
    pub fn mirror_checked(
        &self,
        repository: &str,
        observer: &str,
        locator: &str,
        polled_from: Option<&str>,
        cursor: Option<&str>,
        items: &[MirrorItem],
        in_graph: &dyn Fn(&str) -> Result<bool>,
    ) -> Result<Vec<Value>> {
        self.absent.lock().unwrap_or_else(PoisonError::into_inner).remove(observer);
        let at = super::now_ms();
        let at_text = chrono::DateTime::from_timestamp_millis(at as i64)
            .unwrap_or_default()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let transaction = connection.transaction()?;
        let followed: Option<Option<String>> = transaction
            .query_row("SELECT cursor_digest FROM mirror_repositories WHERE repository=?1", [repository], |row| row.get(0))
            .optional()?;
        let current = followed.flatten().as_deref() == Some(cursor_digest(polled_from).as_str());
        upsert_repository(&transaction, repository, observer, locator)?;
        if !current {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h620c9051c434 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "UPDATE mirror_repositories SET filled_at_unix_ms=NULL WHERE repository=?1",
                [repository],
            )?;
        }
        let mut recorded = Vec::new();
        for item in items {
            let Some(number) = item_number(item) else { continue };
            let old: Option<Map<String, Value>> = if !current { None } else { transaction
                .query_row(
                    "SELECT facts FROM mirror_items WHERE repository=?1 AND number=?2",
                    params![repository, number as i64],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .and_then(|facts| serde_json::from_str(&facts).ok()) };
            for event in github_events::derive(old.as_ref(), &item.facts) {
                // Every key ever handed out stays taken, after its event ages out of the mirror,
                // so a repeated event never reuses the subject of an earlier one.
                let key = github_events::occurrence_key(&event.key, |key| {
                    let Some(subject) = fractal_schema::github_plugin::event_subject(&item.subject, key) else {
                        return Ok(false);
                    };
                    Ok(transaction
                        .query_row("SELECT 1 FROM mirror_event_keys WHERE subject=?1", [&subject], |_| Ok(()))
                        .optional()?
                        .is_some()
                        || in_graph(&subject)?)
                })?;
                let Some(subject) = fractal_schema::github_plugin::event_subject(&item.subject, &key) else {
                    continue;
                };
                let event = github_events::Event { key, ..event };
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-hec182a81f152 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mirror_event_keys(subject, repository) VALUES (?1, ?2)",
                    params![subject, repository],
                )?;
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-hdcadfb13979c owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mirror_events(subject, repository, number, key, event, head_sha, detail, at, recorded_at_unix_ms, sequence)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        subject, repository, number as i64, event.key, event.event, event.head_sha,
                        Value::Object(event.detail.clone()).to_string(), at_text, at as i64, recorded.len() as i64,
                    ],
                )?;
                recorded.push(github_events::view(&event, &item.subject, &subject, &at_text));
            }
            write_item(&transaction, repository, number, item, at)?;
        }
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h59e349f46b8d owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "UPDATE mirror_repositories SET mirrored_at_unix_ms=?2, cursor_digest=?3 WHERE repository=?1",
            params![repository, at as i64, cursor_digest(cursor)],
        )?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h9927b63f8690 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mirror_events WHERE repository=?1 AND recorded_at_unix_ms < ?2",
            params![repository, at.saturating_sub(EVENT_RETENTION_MS) as i64],
        )?;
        transaction.commit()?;
        Ok(recorded)
    }

    /// The repositories this member mirrors whose resource or locator is `owner/repo`, ignoring
    /// case: each one's resource, observer, locator and when it was last mirrored and filled.
    pub fn repositories(&self, locator: &str) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let mut statement = connection.prepare_cached(
            "SELECT repository, observer, locator, mirrored_at_unix_ms, filled_at_unix_ms
             FROM mirror_repositories WHERE lower(locator)=lower(?1) ORDER BY repository",
        )?;
        let rows = statement.query_map([locator], |row| {
            Ok(json!({
                "resource": row.get::<_, String>(0)?,
                "observer": row.get::<_, String>(1)?,
                "locator": row.get::<_, String>(2)?,
                "mirrored_at_unix_ms": row.get::<_, Option<i64>>(3)?,
                "filled_at_unix_ms": row.get::<_, Option<i64>>(4)?,
            }))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The pull requests of `repository`, newest number first: `state` `open`, `closed` or all,
    /// at most `limit` (bounded by [`MAX_READ_ROWS`]). Each row is the compact summary.
    pub fn pulls(&self, repository: &str, state: Option<&str>, limit: usize) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let mut statement = connection.prepare_cached(
            "SELECT subject, facts, claim, observed_at_unix_ms FROM mirror_items INDEXED BY mirror_items_state
             WHERE repository=?1 AND item_kind='vcs.pull-request' AND (?2 IS NULL OR state=?2)
             ORDER BY number DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![repository, state, limit.min(MAX_READ_ROWS) as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, i64>(3)?)),
        )?;
        let mut pulls = Vec::new();
        for row in rows {
            let (subject, facts, claim, observed_at) = row?;
            let facts: Map<String, Value> = serde_json::from_str(&facts)?;
            pulls.push(summary(&subject, facts, claim, Some(observed_at)));
        }
        Ok(pulls)
    }

    /// One item of `repository` with its full facts, or `None` when the mirror does not hold it.
    pub fn item(&self, repository: &str, number: u64) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let row = connection
            .query_row(
                "SELECT subject, item_kind, facts, claim, observed_at_unix_ms FROM mirror_items
                 WHERE repository=?1 AND number=?2",
                params![repository, number as i64],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?, row.get::<_, i64>(4)?)),
            )
            .optional()?;
        let Some((subject, kind, facts, claim, observed_at)) = row else {
            return Ok(None);
        };
        Ok(Some(json!({
            "subject": subject,
            "kind": kind,
            "number": number,
            "facts": serde_json::from_str::<Value>(&facts)?,
            "claim": claim,
            "observed_at_unix_ms": observed_at,
        })))
    }

    /// Every head `repository`'s pull request `number` has had, newest first, each with the
    /// checks last seen on it, and its current required checks. `None` when the mirror does not
    /// hold the pull request.
    pub fn checks(&self, repository: &str, number: u64) -> Result<Option<Value>> {
        let Some(item) = self.item(repository, number)? else {
            return Ok(None);
        };
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let mut statement = connection.prepare_cached(
            "SELECT head_sha, first_seen_unix_ms, last_seen_unix_ms, checks_state, checks, required_checks
             FROM mirror_heads WHERE repository=?1 AND number=?2
             ORDER BY last_seen_unix_ms DESC, first_seen_unix_ms DESC LIMIT ?3",
        )?;
        let heads = statement
            .query_map(params![repository, number as i64, MAX_READ_ROWS as i64], |row| {
                let parse = |text: Option<String>| text.and_then(|text| serde_json::from_str::<Value>(&text).ok()).unwrap_or(Value::Null);
                Ok(json!({
                    "head_sha": row.get::<_, String>(0)?,
                    "first_seen_unix_ms": row.get::<_, i64>(1)?,
                    "last_seen_unix_ms": row.get::<_, i64>(2)?,
                    "checks_state": row.get::<_, Option<String>>(3)?,
                    "checks": parse(row.get(4)?),
                    "required_checks": parse(row.get(5)?),
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let facts = &item["facts"];
        Ok(Some(json!({
            "subject": item["subject"],
            "number": number,
            "head_sha": facts.get("head_sha"),
            "checks_state": facts.get("checks_state"),
            "checks": facts.get("checks"),
            "required_checks": facts.get("required_checks"),
            "heads": heads,
            "observed_at_unix_ms": item["observed_at_unix_ms"],
        })))
    }

    /// The events the mirror keeps for `repository`, or for its item `number`, newest first.
    pub fn events(&self, repository: &str, number: Option<u64>, limit: usize) -> Result<Vec<Value>> {
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let mut statement = connection.prepare_cached(
            "SELECT subject, number, key, event, head_sha, detail, at FROM mirror_events
             WHERE repository=?1 AND (?2 IS NULL OR number=?2)
             ORDER BY recorded_at_unix_ms DESC, sequence DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![repository, number.map(|number| number as i64), limit.min(MAX_READ_ROWS) as i64],
            |row| Ok(json!({
                "subject": row.get::<_, String>(0)?,
                "number": row.get::<_, i64>(1)?,
                "key": row.get::<_, String>(2)?,
                "event": row.get::<_, String>(3)?,
                "head_sha": row.get::<_, Option<String>>(4)?,
                "detail": serde_json::from_str::<Value>(&row.get::<_, String>(5)?).unwrap_or(Value::Null),
                "at": row.get::<_, String>(6)?,
            })),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
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

fn shared_handles() -> &'static Mutex<HashMap<PathBuf, Arc<GithubPluginStore>>> {
    static HANDLES: OnceLock<Mutex<HashMap<PathBuf, Arc<GithubPluginStore>>>> = OnceLock::new();
    HANDLES.get_or_init(Mutex::default)
}

/// What the mirror keeps of a poll cursor to tell whether it followed every look.
fn cursor_digest(cursor: Option<&str>) -> String {
    use sha2::Digest as _;
    match cursor {
        None => "none".into(),
        Some(cursor) => hex::encode(sha2::Sha256::digest(cursor.as_bytes())),
    }
}

/// The number an item resource names, `.../pull-request/N` or `.../issue/N`.
fn item_number(item: &MirrorItem) -> Option<u64> {
    item.subject.rsplit('/').next().and_then(|number| number.parse().ok())
}

fn upsert_repository(connection: &Connection, repository: &str, observer: &str, locator: &str) -> Result<()> {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h42a085359417 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mirror_repositories(repository, observer, locator) VALUES (?1, ?2, ?3)
         ON CONFLICT(repository) DO UPDATE SET observer=excluded.observer, locator=excluded.locator",
        params![repository, observer, locator],
    )?;
    Ok(())
}

fn write_item(connection: &Connection, repository: &str, number: u64, item: &MirrorItem, at: u128) -> Result<()> {
    let state = if item.facts.get("state").and_then(Value::as_str) == Some("open") { "open" } else { "closed" };
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h371a84dadb26 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mirror_items(repository, number, item_kind, subject, state, facts, claim, observed_at_unix_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(repository, number) DO UPDATE SET item_kind=excluded.item_kind, subject=excluded.subject,
             state=excluded.state, facts=excluded.facts, claim=COALESCE(excluded.claim, mirror_items.claim),
             observed_at_unix_ms=excluded.observed_at_unix_ms",
        params![
            repository, number as i64, item.item_kind, item.subject, state,
            Value::Object(item.facts.clone()).to_string(), item.claim, at as i64,
        ],
    )?;
    if let Some(head) = item.facts.get("head_sha").and_then(Value::as_str) {
        let text = |name: &str| item.facts.get(name).filter(|value| !value.is_null()).map(Value::to_string);
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h97dc82980e2a owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mirror_heads(repository, number, head_sha, first_seen_unix_ms, last_seen_unix_ms, checks_state, checks, required_checks)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7)
             ON CONFLICT(repository, number, head_sha) DO UPDATE SET last_seen_unix_ms=excluded.last_seen_unix_ms,
                 checks_state=excluded.checks_state, checks=excluded.checks, required_checks=excluded.required_checks",
            params![
                repository, number as i64, head, at as i64,
                item.facts.get("checks_state").and_then(Value::as_str), text("checks"), text("required_checks"),
            ],
        )?;
    }
    Ok(())
}

/// Drop a repository's mirror. Its handed-out event keys stay, so polling it again never reuses
/// one: from step 3b each is a graph subject written once.
fn forget_repository(connection: &Connection, repository: &str) -> Result<()> {
    for table in ["mirror_items", "mirror_heads", "mirror_events", "mirror_repositories"] {
        connection.execute(
            &format!(/* sqlite-large-value: allow-for-now lv-h02b64a65ba01 owner=fractal_reconcile::reconcile::github_plugin_store TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {table} WHERE repository=?1"),
            [repository],
        )?;
    }
    Ok(())
}

/// One pull request as a list shows it. `observed_at` is when the mirror took it, when known.
pub fn summary(subject: &str, facts: Map<String, Value>, claim: Option<String>, observed_at: Option<i64>) -> Value {
    let mut row = Map::new();
    row.insert("subject".into(), subject.into());
    for name in [
        "number", "title", "url", "author", "state", "merged", "draft", "head_sha", "branch", "base_branch",
        "checks_state", "review_decision", "merge_queue",
    ] {
        if let Some(value) = facts.get(name).filter(|value| !value.is_null()) {
            row.insert(name.into(), value.clone());
        }
    }
    if let Some(state) = facts.get("required_checks").and_then(|checks| checks.get("state")) {
        row.insert("required_checks_state".into(), state.clone());
    }
    row.insert("claim".into(), claim.map_or(Value::Null, Value::String));
    row.insert("observed_at_unix_ms".into(), observed_at.into());
    Value::Object(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(number: u64, facts: Value, claim: &str) -> MirrorItem {
        MirrorItem {
            subject: format!("resource/github/acme/garden/pull-request/{number}"),
            item_kind: "vcs.pull-request".into(),
            facts: facts.as_object().unwrap().clone(),
            claim: Some(claim.into()),
        }
    }

    const REPOSITORY: &str = "resource/github/acme/garden";
    const OBSERVER: &str = "observer/github/acme/garden";

    /// A poller's looks at acme/garden, each from the cursor the one before ended at.
    struct Poller<'a> {
        store: &'a GithubPluginStore,
        at: std::cell::Cell<u32>,
    }

    impl<'a> Poller<'a> {
        fn new(store: &'a GithubPluginStore) -> Self {
            Self { store, at: std::cell::Cell::new(0) }
        }

        fn cursor(&self) -> String {
            format!("cursor-{}", self.at.get())
        }

        fn look(&self, items: &[MirrorItem]) -> Vec<Value> {
            std::thread::sleep(std::time::Duration::from_millis(2));
            let from = self.cursor();
            self.at.set(self.at.get() + 1);
            self.store.mirror(REPOSITORY, OBSERVER, "acme/garden", Some(&from), Some(&self.cursor()), items).unwrap()
        }
    }

    #[test]
    fn the_mirror_keeps_each_item_its_heads_and_the_events_of_each_change() {
        let store = GithubPluginStore::memory();
        let poller = Poller::new(&store);
        let opened = json!({"number": 7, "title": "Garden paths", "state": "open", "draft": false,
            "head_sha": "a", "checks_state": "pending", "required_checks": {"state": "pending"},
            "checks": [{"name": "build", "status": "in_progress"}]});
        assert!(poller.look(&[item(7, opened, "c1")]).is_empty(), "an item new to the mirror is a baseline");
        let pushed = json!({"number": 7, "title": "Garden paths", "state": "open", "draft": false,
            "head_sha": "b", "checks_state": "failure", "required_checks": {"state": "fail", "failed": ["build"]},
            "checks": [{"name": "build", "status": "completed", "conclusion": "failure"}]});
        let events = poller.look(&[item(7, pushed.clone(), "c2")]);
        let subjects: Vec<_> = events.iter().map(|event| event["subject"].as_str().unwrap()).collect();
        assert_eq!(subjects, [
            "github-event/github/acme/garden/pull-request/7/head/b",
            "github-event/github/acme/garden/pull-request/7/checks/b/fail",
        ]);
        poller.look(&[item(8, json!({"number": 8, "state": "closed", "head_sha": "z"}), "c3")]);

        let pulls = store.pulls(REPOSITORY, Some("open"), 10).unwrap();
        assert_eq!(pulls.len(), 1);
        assert_eq!(pulls[0]["number"], 7);
        assert_eq!(pulls[0]["required_checks_state"], "fail");
        assert_eq!(pulls[0]["claim"], "c2");
        assert_eq!(store.pulls(REPOSITORY, None, 10).unwrap().len(), 2);
        assert_eq!(store.item(REPOSITORY, 7).unwrap().unwrap()["facts"], pushed);
        assert!(store.item(REPOSITORY, 9).unwrap().is_none());
        let checks = store.checks(REPOSITORY, 7).unwrap().unwrap();
        let heads: Vec<_> = checks["heads"].as_array().unwrap().iter().map(|head| head["head_sha"].as_str().unwrap()).collect();
        assert_eq!(heads, ["b", "a"]);
        assert_eq!(checks["heads"][1]["checks"][0]["status"], "in_progress");
        assert_eq!(store.events(REPOSITORY, Some(7), 10).unwrap().len(), 2);
        assert!(store.events(REPOSITORY, Some(8), 10).unwrap().is_empty());
        assert_eq!(store.repositories("ACME/Garden").unwrap()[0]["observer"], OBSERVER);

        store.forget(OBSERVER).unwrap();
        assert!(store.pulls(REPOSITORY, None, 10).unwrap().is_empty());
        assert!(store.events(REPOSITORY, None, 10).unwrap().is_empty());
        assert!(store.repositories("acme/garden").unwrap().is_empty());
        // Mirrored again after it was forgotten, it is forgotten again.
        poller.look(&[item(7, json!({"state": "open"}), "c4")]);
        store.forget(OBSERVER).unwrap();
        assert!(store.repositories("acme/garden").unwrap().is_empty());
    }

    /// A repeated event never reuses an earlier one's key, even after that event aged out of
    /// the mirror: from step 3b each key is a graph subject written once.
    #[test]
    fn a_repeated_event_keeps_a_new_key_after_the_first_ages_out() {
        let store = GithubPluginStore::memory();
        let poller = Poller::new(&store);
        let look = |state: &str, claim: &str| poller.look(&[item(7, json!({"state": state}), claim)]);
        look("open", "c1");
        assert_eq!(look("closed", "c2")[0]["key"], "closed");
        store.connection.lock().unwrap().execute("UPDATE mirror_events SET recorded_at_unix_ms=0", []).unwrap();
        assert_eq!(look("open", "c3")[0]["key"], "reopened");
        assert!(store.events(REPOSITORY, Some(7), 10).unwrap().iter().all(|event| event["key"] != "closed"), "aged out");
        assert_eq!(look("closed", "c4")[0]["subject"], "github-event/github/acme/garden/pull-request/7/closed/2");
        // Forgotten and polled again, the repository starts over from a baseline, and a repeat
        // still takes a new key.
        store.forget(OBSERVER).unwrap();
        look("open", "c5");
        assert_eq!(look("closed", "c6")[0]["key"], "closed/3");
    }

    /// A mirror answers only once a look at GitHub filled it, and only while it follows every look:
    /// one a look finds behind the cursor, as after a rollback, is unfilled and derives no event
    /// from its stale rows.
    #[test]
    fn a_mirror_is_filled_from_github_and_unfilled_when_it_falls_behind() {
        let store = GithubPluginStore::memory();
        let poller = Poller::new(&store);
        poller.look(&[item(7, json!({"state": "open", "head_sha": "a"}), "c1")]);
        assert!(!store.filled(REPOSITORY).unwrap(), "a new mirror is not filled");
        assert!(!store.fill(REPOSITORY, OBSERVER, "acme/garden", Some("elsewhere"), &[]).unwrap(),
            "a fill from another cursor than the mirror followed fills nothing");
        let mut full = item(9, json!({"state": "open", "title": "Orchard", "head_sha": "q"}), "unused");
        full.claim = None;
        let mut refilled_seven = item(7, json!({"state": "open", "head_sha": "a"}), "unused");
        refilled_seven.claim = None;
        assert!(store.fill(REPOSITORY, OBSERVER, "acme/garden", Some(&poller.cursor()), &[full, refilled_seven]).unwrap());
        assert_eq!(store.item(REPOSITORY, 7).unwrap().unwrap()["claim"], "c1", "a fill keeps the recording claim");
        assert!(store.filled(REPOSITORY).unwrap());
        assert_eq!(store.item(REPOSITORY, 9).unwrap().unwrap()["facts"]["title"], "Orchard");
        assert!(store.events(REPOSITORY, None, 10).unwrap().is_empty(), "a fill derives no event");
        assert_eq!(poller.look(&[item(7, json!({"state": "closed"}), "c2")])[0]["event"], "closed");
        assert!(store.filled(REPOSITORY).unwrap());

        // Looks went by without the mirror, as under an older build after a rollback.
        poller.at.set(poller.at.get() + 3);
        assert!(poller.look(&[item(7, json!({"state": "open"}), "c3")]).is_empty(), "stale rows derive nothing");
        assert!(!store.filled(REPOSITORY).unwrap());
        assert!(store.fill(REPOSITORY, OBSERVER, "acme/garden", Some(&poller.cursor()), &[]).unwrap());
        assert!(store.filled(REPOSITORY).unwrap());
    }

    /// A step 1 database, cursors only, opens with its cursors and gains the mirror, and its
    /// `user_version` stays the one a step 1 build accepts, so rolling back keeps the cursors.
    #[test]
    fn a_cursor_only_database_gains_the_mirror_and_stays_readable_by_older_builds() {
        let state = tempfile::tempdir().unwrap();
        let directory = state.path().join("plugins/github");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("github.sqlite3");
        Connection::open(&path).unwrap().execute_batch(
            "CREATE TABLE observer_cursors(observer TEXT PRIMARY KEY, resource TEXT NOT NULL, cursor TEXT,
                 updated_at_unix_ms INTEGER NOT NULL) WITHOUT ROWID;
             INSERT INTO observer_cursors VALUES ('observer/github/acme/garden', 'resource/github/acme/garden', 'c1', 1);
             PRAGMA user_version = 1;",
        ).unwrap();
        let store = GithubPluginStore::open(state.path()).unwrap();
        assert_eq!(store.cursor(OBSERVER).unwrap(), Some(Some("c1".into())));
        Poller::new(&store).look(&[item(7, json!({"state": "open"}), "c1")]);
        drop(store);
        let older = Connection::open(&path).unwrap();
        let version: i64 = older.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION, "a step 1 build reads up to this version");
        let cursor: String = older.query_row("SELECT cursor FROM observer_cursors", [], |row| row.get(0)).unwrap();
        assert_eq!(cursor, "c1");
        drop(older);
        assert_eq!(GithubPluginStore::open(state.path()).unwrap().pulls(REPOSITORY, None, 5).unwrap().len(), 1);
    }

    #[test]
    fn a_newer_mirror_is_set_aside() {
        let state = tempfile::tempdir().unwrap();
        let store = GithubPluginStore::open(state.path()).unwrap();
        store.set_cursor(OBSERVER, REPOSITORY, Some("c1")).unwrap();
        drop(store);
        Connection::open(state.path().join("plugins/github/github.sqlite3")).unwrap()
            .execute("UPDATE plugin_meta SET value='2' WHERE key='mirror_version'", []).unwrap();
        assert_eq!(GithubPluginStore::open(state.path()).unwrap().cursor(OBSERVER).unwrap(), None);
    }

    #[test]
    fn the_github_manifest_admits_compact_state_and_keyed_events_only() {
        let facts = json!({"state": "open", "head_sha": "a", "title": "Garden paths"});
        let compact = fractal_schema::github_plugin::compact_item_state(
            "vcs.pull-request", OBSERVER, facts.as_object().unwrap(),
        ).unwrap();
        MANIFEST.admit("resource/github/acme/garden/pull-request/7", "vcs.item.state", &compact.into_iter().collect()).unwrap();
        let snapshot = std::collections::BTreeMap::from([
            ("kind".to_owned(), json!("vcs.pull-request")), ("facts".to_owned(), facts),
        ]);
        assert!(MANIFEST.admit("resource/github/acme/garden/pull-request/7", "resource.observed", &snapshot).is_err());
    }

    #[test]
    fn a_failed_fill_waits_longer_each_time_and_a_success_clears_the_wait() {
        let store = GithubPluginStore::memory();
        assert!(store.refill_due(REPOSITORY, 0));
        store.refill_ended(REPOSITORY, 0, false);
        assert!(!store.refill_due(REPOSITORY, REFILL_RETRY_MS - 1));
        assert!(store.refill_due(REPOSITORY, REFILL_RETRY_MS));
        store.refill_ended(REPOSITORY, REFILL_RETRY_MS, false);
        assert!(!store.refill_due(REPOSITORY, 3 * REFILL_RETRY_MS - 1));
        for _ in 0..10 {
            store.refill_ended(REPOSITORY, 0, false);
        }
        assert!(store.refill_due(REPOSITORY, REFILL_RETRY_MAX_MS), "the wait stops at an hour");
        store.refill_ended(REPOSITORY, 0, true);
        assert!(store.refill_due(REPOSITORY, 0));
    }

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

    /// Once an observer is forgotten, forgetting it again touches no table, even one that can no
    /// longer be read; a cursor written for it makes the next forget ask again.
    #[test]
    fn forgetting_an_observer_twice_reads_once() {
        let store = GithubPluginStore::memory();
        store.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", Some("c1")).unwrap();
        store.forget("observer/github/acme/garden").unwrap();
        store.fail_reads(true);
        store.forget("observer/github/acme/garden").unwrap();
        store.fail_reads(false);
        store.set_cursor("observer/github/acme/garden", "resource/github/acme/garden", Some("c2")).unwrap();
        store.fail_reads(true);
        assert!(store.forget("observer/github/acme/garden").is_err(), "a written cursor is forgotten again");
        store.fail_reads(false);
        store.forget("observer/github/acme/garden").unwrap();
        assert_eq!(store.cursor("observer/github/acme/garden").unwrap(), None);
    }

    /// A cursor written while a forget that found no row still holds the database is forgotten
    /// by the next forget: the observer is not left remembered as absent with a row.
    #[test]
    fn a_cursor_written_during_a_forget_is_forgotten_later() {
        let store = GithubPluginStore::memory();
        let observer = "observer/github/acme/garden";
        std::thread::scope(|scope| {
            // Between its SELECT, which found no row, and remembering the observer as absent, a
            // forget holds the connection.
            let connection = store.connection.lock().unwrap();
            let writer = scope.spawn(|| {
                store.set_cursor(observer, "resource/github/acme/garden", Some("c1")).unwrap()
            });
            std::thread::sleep(std::time::Duration::from_millis(100));
            store.absent.lock().unwrap().insert(observer.to_owned());
            drop(connection);
            writer.join().unwrap();
        });
        assert_eq!(store.cursor(observer).unwrap(), Some(Some("c1".into())));
        store.forget(observer).unwrap();
        assert_eq!(store.cursor(observer).unwrap(), None, "the forget after the write drops its row");
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
