//! Private, conservative native fence. No SQL hooks, schema, metric labels or public errors.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::{BEFORE_VALIDATE};
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCut {
    connection: usize,
    pub file: String,
    pub changes: u64,
    schema: i64,
    temp_schema: i64,
    data: i64,
    user: i64,
}

impl NativeCut {
    fn read(connection: &Connection) -> Option<Self> {
        if !temporary_schema_safe(connection) {
            return None;
        }
        let changes = connection.total_changes();
        // rusqlite's bundled feature selects sqlite3_total_changes64. Never interpret a
        // negative/wrapped native signed counter as a reusable positive epoch.
        if changes > i64::MAX as u64 {
            return None;
        }
        let pragma = |sql| {
            connection
                .query_row(/* sqlite-large-value: allow-for-now lv-hbbfa338c1794 owner=fractal_store::store::resource_preparation TODO=trace-statement-and-value-representation */ sql, [], |row| row.get::<_, i64>(0))
                .ok()
        };
        Some(Self {
            // SAFETY: read the identity only while borrowing the live Connection exclusively
            // through its ordinary writer owner. Never dereference or export this address.
            connection: unsafe { connection.handle() } as usize,
            file: connection.path()?.to_owned(),
            changes,
            schema: pragma("PRAGMA main.schema_version")?,
            temp_schema: pragma("PRAGMA temp.schema_version")?,
            data: pragma("PRAGMA main.data_version")?,
            user: pragma("PRAGMA main.user_version")?,
        })
    }
}

fn temporary_schema_safe(connection: &Connection) -> bool {
    // Compare the actual catalog definitions, not trusted names. SQLite stores the CREATE
    // text beginning at the object name (TEMP / IF NOT EXISTS are not retained); normalize
    // whitespace only. A changed definition, trigger, view or index is ineligible.
    // https://www.sqlite.org/schematab.html and bundled sqlite3.c sqlite3EndTable.
    let check = || -> rusqlite::Result<bool> {
        let mut statement = connection.prepare("SELECT name, type, sql FROM temp.sqlite_schema")?;
        let mut rows = statement.query([])?;
        let mut replay = false;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let sql: Option<String> = row.get(2)?;
            let expected = match name.as_str() {
                "write_clock" => {
                    /* sqlite-large-value: allow-for-now lv-h914a6ef579dc owner=fractal_store::store::resource_preparation TODO=chunk-or-reference-variable-columns */ "CREATE TABLE write_clock(offset_ms INTEGER NOT NULL, at_ms INTEGER)"
                }
                "heal_replay_base_order" => {
                    replay = true;
                    /* sqlite-large-value: allow-for-now lv-h2cb49569e962 owner=fractal_store::store::resource_preparation TODO=chunk-or-reference-variable-columns */ "CREATE TABLE heal_replay_base_order( ordinal INTEGER PRIMARY KEY, claim_id TEXT NOT NULL)"
                }
                _ => return Ok(false),
            };
            if kind != "table"
                || sql
                    .as_deref()
                    .map(|sql| sql.split_ascii_whitespace().collect::<Vec<_>>().join(" "))
                    .as_deref()
                    != Some(expected)
            {
                return Ok(false);
            }
        }
        // Replay empties its private ID-order table before returning. Never read resource
        // inputs while an unexpected retained scratch state is present.
        if replay {
            return connection.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM temp.heal_replay_base_order LIMIT 1)",
                [],
                |row| row.get(0),
            );
        }
        Ok(true)
    };
    check().unwrap_or(false)
}

pub struct Fence {
    // Ownership identity only: the numeric claim frontier is NOT a currentness proof.
    pub incarnation: Arc<std::sync::atomic::AtomicU64>,
    owner: Arc<()>,
    path: std::path::PathBuf,
    shared_memory: bool,
}

pub struct Prepared<T> {
    owner: Arc<()>,
    pub cut: Option<NativeCut>,
    pub value: Option<T>,
}

impl Fence {
    pub fn new(store: &Store) -> Self {
        Self {
            incarnation: store.committed_index.clone(),
            owner: Arc::new(()),
            path: store.path.clone(),
            shared_memory: store.shared_memory,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare<T>(
        &self,
        store: &Store,
        read: impl FnOnce(&Connection) -> T,
    ) -> Prepared<T> {
        self.prepare_with_actuals(store, |connection, _| read(connection))
    }

    pub fn prepare_with_actuals<T>(
        &self,
        store: &Store,
        read: impl FnOnce(&Connection, Option<u64>) -> T,
    ) -> Prepared<T> {
        let mut actual_serial = None;
        let mut prepared = Prepared {
            owner: self.owner.clone(),
            cut: None,
            value: None,
        };
        // Shared-cache snapshots hold table read locks until their transaction ends. The
        // frontier read locks sqlite_sequence and can reject an unrelated AUTOINCREMENT
        // publication with SQLITE_LOCKED. Keep shared-memory callers on the unchanged
        // canonical writer path; reject BEFORE acquiring even a reader or seed loan.
        // Do not join an older ambient snapshot or end an unregistered request transaction.
        if store.shared_memory
            || !Arc::ptr_eq(&self.incarnation, &store.committed_index)
            || store.path != self.path
            || store.readers.path != self.path
            || store.shared_memory != self.shared_memory
            || store.readers.has_request_reader()
            || !store.readers.get().is_autocommit()
        {
            return prepared;
        }
        {
            // Seed from the actual writer at autocommit. This short loan reads only native
            // counters/cookies, never history. Drop it BEFORE starting the fresh read cut.
            #[cfg(any(test, feature = "test-support"))]
            let queued = std::time::Instant::now();
            let connection = store.connection.write();
            #[cfg(any(test, feature = "test-support"))]
            let waited = queued.elapsed();
            #[cfg(any(test, feature = "test-support"))]
            let started = std::time::Instant::now();
            #[cfg(any(test, feature = "test-support"))]
            let work = smallclaims::sqlite::work::SqliteWorkScope::start();
            if connection.is_autocommit() {
                prepared.cut = NativeCut::read(&connection);
                actual_serial = store.smalltalk.observer_actuals.seed(
                    &connection, || connection.reverify_observer_coverage(),
                ).ok();
            }
            drop(connection);
            #[cfg(any(test, feature = "test-support"))]
            COUNTS.with(|counts| {
                counts
                    .borrow_mut()
                    .seed
                    .push((waited, started.elapsed(), work.finish()))
            });
        }
        if prepared.cut.as_ref().is_none_or(|cut| {
            // File identity also uses the retained Store incarnation and constructor pairing;
            // matching schema cookies alone never identifies a file.
            if self.shared_memory {
                !cut.file.is_empty()
            } else {
                cut.file.is_empty() || std::path::Path::new(&cut.file) != self.path
            }
        }) {
            return prepared;
        }
        #[cfg(any(test, feature = "test-support"))]
        let read_started = std::time::Instant::now();
        #[cfg(any(test, feature = "test-support"))]
        let read_work = smallclaims::sqlite::work::SqliteWorkScope::start();
        let value = store.read_snapshot(|_| {
            let connection = store.readers.get();
            if connection.path() != prepared.cut.as_ref().map(|cut| cut.file.as_str()) {
                return Ok(None);
            }
            let dirty: bool =
                connection.query_row("PRAGMA read_uncommitted", [], |row| row.get(0))?;
            if dirty {
                return Ok(None);
            }
            if !temporary_schema_safe(&connection) {
                return Ok(None);
            }
            let schema: i64 =
                connection.query_row("PRAGMA main.schema_version", [], |row| row.get(0))?;
            if prepared.cut.as_ref().is_none_or(|cut| cut.schema != schema) {
                return Ok(None);
            }
            Ok(Some(read(&connection, actual_serial)))
        });
        #[cfg(any(test, feature = "test-support"))]
        COUNTS.with(|counts| {
            counts
                .borrow_mut()
                .preparation
                .push((read_started.elapsed(), read_work.finish()))
        });
        // A failed/unusable cut is a stale attempt, not a new public error. Reader guards and
        // its pin have ended before queueing validation. The two-attempt caller owns fallback.
        prepared.value = value.ok().flatten();
        prepared
    }

    pub fn current<T>(
        &self,
        store: &Store,
        connection: &Connection,
        prepared: &Prepared<T>,
    ) -> bool {
        if !Arc::ptr_eq(&self.incarnation, &store.committed_index)
            || store.path != self.path
            || !Arc::ptr_eq(&self.owner, &prepared.owner)
            || prepared.value.is_none()
        {
            return false;
        }
        let Some(expected) = &prepared.cut else {
            return false;
        };
        // Native values from THIS transaction close the same-batch gap. No committed hint
        // participates: earlier job/trigger/repair/delete DML changes the counter immediately.
        NativeCut::read(connection).as_ref() == Some(expected)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub fast: usize,
    pub cached: usize,
    pub invalidated: usize,
    pub fallback: usize,
    pub seed: Vec<(
        std::time::Duration,
        std::time::Duration,
        smallclaims::sqlite::work::SqliteWork,
    )>,
    pub preparation: Vec<(std::time::Duration, smallclaims::sqlite::work::SqliteWork)>,
    pub until_callback: Vec<Option<std::time::Duration>>,
    pub acknowledged: Vec<std::time::Duration>,
    pub writer: Vec<(
        bool,
        std::time::Duration,
        smallclaims::sqlite::work::SqliteWork,
    )>,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static COUNTS: std::cell::RefCell<Counts> = std::cell::RefCell::new(Counts::default());
}

#[cfg(any(test, feature = "test-support"))]
pub fn take_counts() -> Counts {
    COUNTS.with(|counts| std::mem::take(&mut *counts.borrow_mut()))
}

#[cfg(any(test, feature = "test-support"))]
pub type ValidateHook = Box<dyn FnMut(&Store, usize)>;
#[cfg(any(test, feature = "test-support"))]
pub fn before_validate(store: &Store, pass: usize) {
    BEFORE_VALIDATE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(store, pass);
        }
    });
}

#[cfg(any(test, feature = "test-support"))]
pub struct WriterMeasure<'a> {
    sink: &'a Mutex<Option<(std::time::Duration, smallclaims::sqlite::work::SqliteWork)>>,
    start: std::time::Instant,
    work: Option<smallclaims::sqlite::work::SqliteWorkScope>,
}
#[cfg(any(test, feature = "test-support"))]
impl<'a> WriterMeasure<'a> {
    pub fn new(
        sink: &'a Mutex<Option<(std::time::Duration, smallclaims::sqlite::work::SqliteWork)>>,
    ) -> Self {
        Self {
            sink,
            start: std::time::Instant::now(),
            work: Some(smallclaims::sqlite::work::SqliteWorkScope::start()),
        }
    }
}
#[cfg(any(test, feature = "test-support"))]
impl Drop for WriterMeasure<'_> {
    fn drop(&mut self) {
        *self.sink.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((self.start.elapsed(), self.work.take().unwrap().finish()));
    }
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_seats;
