//! One writer-owned mutation dispatcher. Row callbacks never execute SQL. Publication runs
//! after transaction resolution and before batched acknowledgements. Returned loans publish
//! before the guard returns its connection, alongside existing committed-write observers.
use anyhow::{Result, ensure};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Callbacks belong to the writer: observers must not replace them or panic. Row callbacks
/// only mark dirty state. Publication may read SQLite and must expose read errors to consumers.
pub trait WriterObserver: Send + Sync {
    /// Supported physical rowid tables. Schema replacement is checked before publication.
    fn tables(&self) -> &'static [(&'static str, &'static str)];
    fn mutated(&self, database: &str, table: &str);
    /// Physical row identity, delivered without executing SQL inside SQLite's hook.
    fn row_mutated(&self, action: rusqlite::hooks::Action, database: &str, table: &str, rowid: i64) {
        let _ = (action, rowid);
        self.mutated(database, table);
    }
    fn unobserved(&self, database: &str, table: &str);
    fn failed(&self, error: Arc<str>);
    /// Called only after native hook coverage and all watched physical tables validate.
    fn coverage_verified(&self) {}
    /// Resolution while physical coverage is disabled. Consumers may conservatively wake
    /// readers, but must not publish a selectively invalidated/cached answer.
    fn coverage_unavailable(&self) {}
    fn published(&self, connection: &Connection);
}

pub(crate) struct MutationState {
    observer: Arc<dyn WriterObserver>,
    schema_dirty: AtomicBool,
    invalid_schema: AtomicBool,
    schema: Mutex<Vec<(String, String, String, String)>>,
    schema_generations: Mutex<BTreeMap<String, i64>>,
    replacements: Mutex<BTreeSet<(String, String)>>,
    opaque_schema: AtomicBool,
    replacing_rows: AtomicBool,
    replacing_authorizer: AtomicBool,
    missed_rows: AtomicBool,
    missed_authorizer: AtomicBool,
    ledger: Mutex<super::commit_scope::Ledger>,
    resolution: Arc<Resolution>,
    replacing_scope: AtomicBool,
    missed_scope: AtomicBool,
    unknown_commit: AtomicBool,
    main_schema_generation: Mutex<i64>,
}

struct HookLease {
    state: Arc<MutationState>,
    rows: bool,
}
impl Drop for HookLease {
    fn drop(&mut self) {
        // rusqlite drops the registered closure when a caller replaces a hook. Expected
        // dispatcher swaps suppress this hint; external replacements retain it until the
        // authoritative transaction/loan boundary, even if a scope restores the row hook.
        if self.rows {
            if !self.state.replacing_rows.load(Ordering::Relaxed) {
                self.state.missed_rows.store(true, Ordering::Relaxed);
            }
        } else if !self.state.replacing_authorizer.load(Ordering::Relaxed) {
            self.state.missed_authorizer.store(true, Ordering::Relaxed);
        }
    }
}

struct ScopeLease(Arc<MutationState>);
impl Drop for ScopeLease {
    fn drop(&mut self) {
        if !self.0.replacing_scope.load(Ordering::Relaxed) {
            self.0.missed_scope.store(true, Ordering::Relaxed);
        }
    }
}

/// Native transaction-resolution consumers for one connection. The mutation dispatcher
/// owns the single commit/rollback hook slot and fans out to these subscribers, so
/// installing or reinstating it at a loan boundary cannot drop another writer-owned
/// transaction-resolution callback the way a raw slot swap would.
struct Resolution {
    commits: Mutex<Vec<Arc<dyn Fn() -> bool + Send + Sync>>>,
    rollbacks: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

const RESOLUTION: &std::ffi::CStr = c"st3.transaction-resolution";


unsafe extern "C" fn destroy_resolution(pointer: *mut std::ffi::c_void) {
    // SAFETY: SQLite releases exactly the strong reference transferred by ensure_resolution,
    // including allocation failure in sqlite3_set_clientdata.
    unsafe { drop(Arc::from_raw(pointer.cast::<Resolution>())); }
}

fn commit_resolved(resolution: &Resolution) -> bool {
    let commits = resolution.commits.lock().unwrap();
    // Any subscriber vetoing a commit converts COMMIT into a rollback, as SQLite's
    // native commit-hook contract requires.
    commits.iter().any(|commit| commit())
}

fn rollback_resolved(resolution: &Resolution) {
    let rollbacks = resolution.rollbacks.lock().unwrap();
    for rollback in rollbacks.iter() {
        rollback();
    }
}

fn ensure_resolution(connection: &Connection) -> Result<Arc<Resolution>> {
    // Check before either clientdata call, including bare connections whose work
    // subscribers are registered before the native writer observer is constructed.
    ensure!(
        rusqlite::version_number() >= 3_044_000,
        "writer transaction resolution requires SQLite 3.44.0 or newer for connection clientdata; found {}",
        rusqlite::version()
    );
    // SAFETY: this private name holds only the Arc<Resolution> transferred below. The
    // clientdata reference is not consumed, so the strong count is restored before use.
    unsafe {
        let installed = rusqlite::ffi::sqlite3_get_clientdata(
            connection.handle(), RESOLUTION.as_ptr()) as *const Resolution;
        if !installed.is_null() {
            Arc::increment_strong_count(installed);
            return Ok(Arc::from_raw(installed));
        }
    }
    let resolution = Arc::new(Resolution {
        commits: Mutex::new(Vec::new()),
        rollbacks: Mutex::new(Vec::new()),
    });
    // SAFETY: SQLite owns this reference and releases it with the matching destructor.
    let result = unsafe {
        rusqlite::ffi::sqlite3_set_clientdata(connection.handle(), RESOLUTION.as_ptr(),
            Arc::into_raw(resolution.clone()).cast_mut().cast(), Some(destroy_resolution))
    };
    if result != rusqlite::ffi::SQLITE_OK {
        return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(result), None).into());
    }
    // Own the slot until a mutation dispatcher takes it over. Bare replay/proof
    // connections never get one, and their registered work still resolves.
    let commits = resolution.clone();
    connection.commit_hook(Some(move || commit_resolved(&commits)));
    let rollbacks = resolution.clone();
    connection.rollback_hook(Some(move || rollback_resolved(&rollbacks)));
    Ok(resolution)
}

/// Register connection-local work for native transaction resolution. The mutation
/// dispatcher owns the single commit/rollback hook slot and fans out to every
/// registered consumer, so reinstalling that slot at a loan boundary cannot strand
/// transaction-local state the way replacing a raw hook silently would. `commit`
/// returning true converts COMMIT into a rollback, matching SQLite's contract.
pub fn observe_transaction_resolution(
    connection: &Connection,
    commit: impl Fn() -> bool + Send + Sync + 'static,
    rollback: impl Fn() + Send + Sync + 'static,
) -> Result<()> {
    let resolution = ensure_resolution(connection)?;
    resolution.commits.lock().unwrap().push(Arc::new(commit));
    resolution.rollbacks.lock().unwrap().push(Arc::new(rollback));
    Ok(())
}

/// Writers without physical-row consumers still own the native mutation dispatcher.
pub(crate) struct NoPhysicalObserver;
impl WriterObserver for NoPhysicalObserver {
    fn tables(&self) -> &'static [(&'static str, &'static str)] { &[] }
    fn mutated(&self, _: &str, _: &str) {}
    fn unobserved(&self, _: &str, _: &str) {}
    fn failed(&self, _: Arc<str>) {}
    fn published(&self, _: &Connection) {}
}

impl MutationState {
    pub(crate) fn new(
        connection: &Connection,
        observer: Arc<dyn WriterObserver>,
    ) -> Result<Arc<Self>> {
        ensure!(
            super::commit_scope::preupdate_available(),
            "writer mutation routing requires SQLite compiled with SQLITE_ENABLE_PREUPDATE_HOOK"
        );
        let schema = validate_tables(connection, observer.tables())?;
        let schema_generations = schema_generations(connection, observer.tables())?;
        let state = Arc::new(Self {
            observer,
            schema_dirty: AtomicBool::new(false),
            invalid_schema: AtomicBool::new(false),
            schema: Mutex::new(schema),
            schema_generations: Mutex::new(schema_generations),
            replacements: Mutex::new(BTreeSet::new()),
            opaque_schema: AtomicBool::new(false),
            replacing_rows: AtomicBool::new(false),
            replacing_authorizer: AtomicBool::new(false),
            missed_rows: AtomicBool::new(false),
            missed_authorizer: AtomicBool::new(false),
            ledger: Mutex::new(super::commit_scope::Ledger::default()),
            resolution: ensure_resolution(connection)?,
            replacing_scope: AtomicBool::new(false),
            missed_scope: AtomicBool::new(false),
            unknown_commit: AtomicBool::new(false),
            main_schema_generation: Mutex::new(connection.query_row("PRAGMA main.schema_version", [], |row| row.get(0))?),
        });
        install_rows(connection, Some(state.clone()));
        state.install_authorizer(connection);
        state.install_scope(connection);
        Ok(state)
    }

    fn install_scope(self: &Arc<Self>, connection: &Connection) {
        self.replacing_scope.store(true, Ordering::Relaxed);
        super::commit_scope::install_preupdate(connection, &self.ledger);
        // Native slots resolve the ledger and the hub established at construction.
        // Hook reinstalls cannot drop earlier connection-local work subscribers.
        let commits = self.resolution.clone();
        let state = self.clone();
        let lease = ScopeLease(self.clone());
        connection.commit_hook(Some(move || {
            let _lease = &lease;
            state.ledger.lock().unwrap().committing();
            commit_resolved(&commits)
        }));
        let state = self.clone();
        let lease = ScopeLease(self.clone());
        let rollbacks = self.resolution.clone();
        connection.rollback_hook(Some(move || {
            let _lease = &lease;
            state.ledger.lock().unwrap().rollback();
            rollback_resolved(&rollbacks);
        }));
        super::commit_scope::install_trace(connection, &self.ledger);
        self.replacing_scope.store(false, Ordering::Relaxed);
    }

    pub(super) fn take_committed(&self, connection: &Connection) -> super::commit_scope::Changes {
        let mut changes = self.ledger.lock().unwrap().take(
            connection,
            self.unknown_commit.swap(false, Ordering::Relaxed),
        );
        if changes.schema {
            // DDL can execute without changing any schema (IF [NOT] EXISTS), or affect
            // only temp/attached databases. Read the main cookie only after executed DDL
            // reached a commit, never on ordinary no-op/read-only/unrelated writer loans.
            match connection.query_row("PRAGMA main.schema_version", [], |row| row.get::<_, i64>(0)) {
                Ok(current) => {
                    let mut previous = self.main_schema_generation.lock().unwrap();
                    changes.schema = current != *previous;
                    *previous = current;
                }
                Err(_) => changes.unknown = true,
            }
        }
        changes
    }

    fn install_authorizer(self: &Arc<Self>, connection: &Connection) {
        self.replacing_authorizer.store(true, Ordering::Relaxed);
        let schema = self.clone();
        let lease = HookLease {
            state: self.clone(),
            rows: false,
        };
        // SQLite authorizes DROP_TABLE immediately before DELETE on the same table. IGNORE
        // there cancels the DROP, unlike ordinary DELETE where it only disables truncation.
        let mut drop_delete: Option<(String, String)> = None;
        connection.authorizer(Some(move |context: AuthContext<'_>| {
            let _keep_lease = &lease;
            let database = context.database_name.unwrap_or("main");
            let watched =
                |database: &str, table: &str| schema.observer.tables().contains(&(database, table));
            // SQLITE_IGNORE for DELETE preserves deletion and disables only the truncate
            // optimisation. Every watched deletion therefore produces a row notification,
            // including cached full-table DELETE, without polling unrelated table changes.
            if let AuthAction::Delete { table_name } = context.action
                && watched(database, table_name)
            {
                if drop_delete
                    .take()
                    .is_some_and(|(db, table)| db == database && table == table_name)
                {
                    return Authorization::Allow;
                }
                return Authorization::Ignore;
            }
            if let AuthAction::DropTable { table_name } | AuthAction::DropTempTable { table_name } =
                context.action
                && watched(database, table_name)
            {
                drop_delete = Some((database.to_string(), table_name.to_string()));
                schema
                    .replacements
                    .lock()
                    .unwrap()
                    .insert((database.into(), table_name.into()));
            }
            let changed = match context.action {
                // The authorizer receives ALTER's old table name, so a replacement renamed
                // into a watched name cannot be detected by name filtering here. Check the
                // final watched schema after any DDL; unchanged unrelated schema does not
                // dirty observers or trigger their indexed reads.
                AuthAction::CreateIndex { .. }
                | AuthAction::CreateTable { .. }
                | AuthAction::CreateTrigger { .. }
                | AuthAction::CreateTempIndex { .. }
                | AuthAction::CreateTempTable { .. }
                | AuthAction::CreateTempTrigger { .. }
                | AuthAction::CreateVtable { .. }
                | AuthAction::DropIndex { .. }
                | AuthAction::DropTable { .. }
                | AuthAction::DropTrigger { .. }
                | AuthAction::DropTempIndex { .. }
                | AuthAction::DropTempTable { .. }
                | AuthAction::DropTempTrigger { .. }
                | AuthAction::DropVtable { .. }
                | AuthAction::CreateView { .. }
                | AuthAction::CreateTempView { .. }
                | AuthAction::DropView { .. }
                | AuthAction::DropTempView { .. }
                | AuthAction::AlterTable { .. }
                | AuthAction::Attach { .. }
                | AuthAction::Detach { .. } => true,
                AuthAction::Pragma {
                    pragma_name,
                    pragma_value: Some(_),
                } => {
                    pragma_name.eq_ignore_ascii_case("schema_version")
                        || pragma_name.eq_ignore_ascii_case("writable_schema")
                }
                _ => false,
            };
            if changed {
                schema.schema_dirty.store(true, Ordering::Relaxed);
            }
            if let AuthAction::Pragma {
                pragma_name,
                pragma_value: Some(_),
            } = context.action
                && (pragma_name.eq_ignore_ascii_case("schema_version")
                    || pragma_name.eq_ignore_ascii_case("writable_schema"))
            {
                // An explicit cookie reset or raw sqlite_schema edit can erase generation
                // evidence. Conservatively fence those authoritative schema boundaries.
                schema.opaque_schema.store(true, Ordering::Relaxed);
            }
            Authorization::Allow
        }));
        self.replacing_authorizer.store(false, Ordering::Relaxed);
    }

    pub(crate) fn reverify(self: &Arc<Self>, connection: &Connection) {
        self.schema_dirty.store(true, Ordering::Relaxed);
        self.resolved(connection);
    }

    pub(crate) fn resolved(self: &Arc<Self>, connection: &Connection) {
        if !connection.is_autocommit() {
            return;
        }
        // Raw preupdate replacement has no closure destructor. Registration returns the
        // previous userdata, allowing ownership attestation without executing any SQL.
        if !super::commit_scope::install_preupdate(connection, &self.ledger) {
            self.missed_scope.store(true, Ordering::Relaxed);
        }
        super::commit_scope::install_trace(connection, &self.ledger);
        let missed_rows = self.missed_rows.swap(false, Ordering::Relaxed);
        let missed_authorizer = self.missed_authorizer.swap(false, Ordering::Relaxed);
        let missed_scope = self.missed_scope.swap(false, Ordering::Relaxed);
        if missed_scope || missed_rows || missed_authorizer {
            // Lost native coverage is an explicit all-dependency invalidation, even when
            // the replacement hid every mutation or transaction-resolution callback.
            self.unknown_commit.store(true, Ordering::Relaxed);
            self.install_scope(connection);
        }
        if missed_rows || missed_authorizer {
            // This is an explicit missed-observation fence, not polling after unrelated SQL.
            // Hook replacement could hide receipt mutations or DDL, so rescan watched schema
            // and publish one exact-table dirty hint before the next queued job/ACK.
            install_rows(connection, Some(self.clone()));
            self.install_authorizer(connection);
            self.schema_dirty.store(true, Ordering::Relaxed);
        }
        if self.schema_dirty.swap(false, Ordering::Relaxed) {
            let opaque = self.opaque_schema.swap(false, Ordering::Relaxed);
            let replacements = std::mem::take(&mut *self.replacements.lock().unwrap());
            let validated = (|| -> Result<_> {
                Ok((
                    validate_tables(connection, self.observer.tables())?,
                    schema_generations(connection, self.observer.tables())?,
                ))
            })();
            // Authorizers run during preparation. Expire cached statements after a schema
            // boundary so a cached cookie-reset/raw-schema statement cannot hide a later
            // replacement from the new authoritative generation fence.
            self.install_authorizer(connection);
            match validated {
                Err(error) => {
                    self.invalid_schema.store(true, Ordering::Relaxed);
                    self.observer.failed(Arc::from(error.to_string()));
                    return;
                }
                Ok((current, generations)) => {
                    let recovered = self.invalid_schema.swap(false, Ordering::Relaxed);
                    let mut previous = self.schema.lock().unwrap();
                    let mut previous_generations = self.schema_generations.lock().unwrap();
                    for (database, table) in self.observer.tables() {
                        let entries = |schema: &Vec<(String, String, String, String)>| {
                            schema
                                .iter()
                                .filter(|(db, tbl, _, _)| db == database && tbl == table)
                                .cloned()
                                .collect::<Vec<_>>()
                        };
                        // DROP/recreate may reproduce identical SQL and root pages. A
                        // committed generation change plus a watched replacement intent
                        // preserves that signal; prepared-only/rolled-back DDL has no
                        // committed generation change. Ordinary DML never reads generations.
                        let replaced = replacements
                            .contains(&(database.to_string(), table.to_string()))
                            && previous_generations.get(*database) != generations.get(*database);
                        if recovered
                            || missed_rows
                            || missed_authorizer
                            || opaque
                            || replaced
                            || entries(&previous) != entries(&current)
                        {
                            self.observer.unobserved(database, table);
                        }
                    }
                    *previous = current;
                    *previous_generations = generations;
                    self.observer.coverage_verified();
                }
            }
        }
        // Do not retry metadata on ordinary writes while unsupported. Consumers still need
        // a conservative resolution signal until a supported schema is positively verified.
        if self.invalid_schema.load(Ordering::Relaxed) {
            self.observer.coverage_unavailable();
        } else {
            self.observer.published(connection);
        }
    }
}

fn schema_generations(
    connection: &Connection,
    tables: &[(&str, &str)],
) -> Result<BTreeMap<String, i64>> {
    let mut generations = BTreeMap::new();
    for (database, _) in tables {
        if generations.contains_key(*database) {
            continue;
        }
        let identifier = database.replace('"', "\"\"");
        let version = connection.query_row(
            /* sqlite-large-value: allow-for-now lv-h72783baac7c1 owner=smallclaims::sqlite::writer_observer TODO=trace-statement-and-value-representation */ &format!("PRAGMA \"{identifier}\".schema_version"),
            [],
            |row| row.get(0),
        )?;
        generations.insert(database.to_string(), version);
    }
    Ok(generations)
}

fn validate_tables(
    connection: &Connection,
    tables: &[(&str, &str)],
) -> Result<Vec<(String, String, String, String)>> {
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare("PRAGMA table_list")?;
    let actual = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut schema = Vec::new();
    for (database, table) in tables {
        ensure!(
            actual
                .iter()
                .any(|(db, name, kind, without_rowid)| db == database
                    && name == table
                    && kind == "table"
                    && !without_rowid),
            "writer observer requires a physical rowid table: {database}.{table}"
        );
        // Database names are identifiers supplied by the observer, never SQL fragments.
        let database_identifier = database.replace('"', "\"\"");
        let sql = format!(
            "SELECT name,coalesce(sql,'') FROM \"{database_identifier}\".sqlite_schema
             WHERE tbl_name=?1 ORDER BY name"
        );
        let rows = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h710267011525 owner=smallclaims::sqlite::writer_observer TODO=trace-statement-and-value-representation */ &sql)?
            .query_map([table], |row| {
                Ok((
                    database.to_string(),
                    table.to_string(),
                    row.get(0)?,
                    row.get(1)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        schema.extend(rows);
    }
    Ok(schema)
}

pub(crate) fn install_rows(connection: &Connection, state: Option<Arc<MutationState>>) {
    if state.is_none() {
        connection.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
        return;
    }
    if let Some(state) = &state {
        state.replacing_rows.store(true, Ordering::Relaxed);
    }
    let lease = state.as_ref().map(|state| HookLease {
        state: state.clone(),
        rows: true,
    });
    let installed = state.clone();
    connection.update_hook(Some(move |action, database: &str, table: &str, rowid| {
        let _keep_lease = &lease;
        if let Some(state) = &state {
            state.observer.row_mutated(action, database, table, rowid);
        }
    }));
    if let Some(state) = installed {
        state.replacing_rows.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::WriterConnection;
    use std::sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    };

    #[derive(Default)]
    struct Receipts {
        dirty: AtomicBool,
        queries: AtomicU64,
        rows: AtomicU64,
        hints: AtomicU64,
        deadline: Mutex<Option<i64>>,
        error: Mutex<Option<Arc<str>>>,
    }
    impl WriterObserver for Receipts {
        fn tables(&self) -> &'static [(&'static str, &'static str)] {
            &[("main", "receipts")]
        }
        fn mutated(&self, database: &str, table: &str) {
            if database == "main" && table == "receipts" {
                self.rows.fetch_add(1, Ordering::Relaxed);
                self.dirty.store(true, Ordering::Relaxed);
            }
        }
        fn unobserved(&self, database: &str, table: &str) {
            if database == "main" && table == "receipts" {
                self.hints.fetch_add(1, Ordering::Relaxed);
                self.dirty.store(true, Ordering::Relaxed);
            }
        }
        fn failed(&self, error: Arc<str>) {
            *self.error.lock().unwrap() = Some(error);
        }
        fn published(&self, connection: &Connection) {
            assert!(connection.is_autocommit());
            if self.dirty.swap(false, Ordering::Relaxed) {
                self.queries.fetch_add(1, Ordering::Relaxed);
                *self.deadline.lock().unwrap() = connection
                    .query_row(
                        "SELECT MIN(at) FROM receipts INDEXED BY receipts_at",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                *self.error.lock().unwrap() = None;
            }
        }
    }
    fn connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        crate::sqlite::observe(&mut connection);
        connection
            .execute_batch(
                "CREATE TABLE claims(store_index INTEGER PRIMARY KEY);
            CREATE TABLE receipts(id INTEGER PRIMARY KEY,at INTEGER NOT NULL);
            CREATE INDEX receipts_at ON receipts(at);
            CREATE TABLE unrelated(id INTEGER PRIMARY KEY);",
            )
            .unwrap();
        connection
    }
    fn writer() -> (WriterConnection, Arc<Receipts>) {
        let observer = Arc::new(Receipts::default());
        let writer = WriterConnection::new_with_observer(
            connection(),
            Arc::new(AtomicU64::new(0)),
            observer.clone(),
        )
        .unwrap();
        (writer, observer)
    }

    #[test]
    fn observer_publishes_after_commit_ack_and_rollback_and_returned_loan() {
        let (writer, observer) = writer();
        writer
            .batched(|tx| {
                tx.execute("INSERT INTO receipts VALUES(1,100)", [])?;
                assert_eq!(*observer.deadline.lock().unwrap(), None);
                Ok::<_, rusqlite::Error>(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(*observer.deadline.lock().unwrap(), Some(100));
        assert_eq!(
            writer
                .batched(|tx| {
                    tx.execute("UPDATE receipts SET at=50", []).unwrap();
                    Err::<(), _>("refused")
                })
                .unwrap(),
            Err("refused")
        );
        assert_eq!(*observer.deadline.lock().unwrap(), Some(100));
        {
            let held = writer.write();
            held.execute("INSERT INTO receipts VALUES(2,25)", [])
                .unwrap();
            assert_eq!(*observer.deadline.lock().unwrap(), Some(100));
        }
        drop(writer.write()); // Queue order guarantees the previous return was published.
        assert_eq!(*observer.deadline.lock().unwrap(), Some(25));
        assert_eq!(observer.queries.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn cached_truncate_delete_notifies_but_unrelated_mutations_do_not_read_deadlines() {
        let (writer, observer) = writer();
        for n in 0..2 {
            writer
                .batched(|tx| tx.execute("INSERT INTO receipts VALUES(1,100)", []))
                .unwrap()
                .unwrap();
            let before = observer.rows.load(Ordering::Relaxed);
            writer
                .batched(|tx| {
                    tx.prepare_cached("DELETE FROM receipts")
                        .unwrap()
                        .execute([])
                })
                .unwrap()
                .unwrap();
            assert_eq!(
                observer.rows.load(Ordering::Relaxed) - before,
                1,
                "cached pass {n}"
            );
            assert_eq!(*observer.deadline.lock().unwrap(), None);
        }
        let before = observer.queries.load(Ordering::Relaxed);
        writer
            .batched(|tx| {
                tx.execute_batch(
                    "INSERT INTO unrelated VALUES(1); DELETE FROM unrelated;
            CREATE TABLE unrelated_schema(id INTEGER PRIMARY KEY); INSERT INTO claims VALUES(1);",
                )
            })
            .unwrap()
            .unwrap();
        assert_eq!(observer.queries.load(Ordering::Relaxed), before);
    }

    #[test]
    fn unsupported_watched_schema_is_visible_and_does_not_poll_ordinary_claims() {
        let (writer, observer) = writer();
        writer
            .batched(|tx| {
                tx.execute_batch(
                    "DROP TABLE receipts;
            CREATE TABLE receipts(id INTEGER PRIMARY KEY,at INTEGER NOT NULL) WITHOUT ROWID;",
                )
            })
            .unwrap()
            .unwrap();
        assert!(
            observer
                .error
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("physical rowid table")
        );
        let before = observer.queries.load(Ordering::Relaxed);
        writer
            .batched(|tx| tx.execute("INSERT INTO claims VALUES(1)", []))
            .unwrap()
            .unwrap();
        assert_eq!(observer.queries.load(Ordering::Relaxed), before);
        writer
            .batched(|tx| {
                tx.execute_batch(
                    "DROP TABLE receipts;
            CREATE TABLE receipts(id INTEGER PRIMARY KEY,at INTEGER NOT NULL);
            CREATE INDEX receipts_at ON receipts(at);",
                )
            })
            .unwrap()
            .unwrap();
        assert!(observer.error.lock().unwrap().is_none());
        assert_eq!(observer.queries.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn rolled_back_schema_does_not_publish_error_and_renamed_replacement_recovers() {
        let (writer, observer) = writer();
        writer
            .batched(|tx| tx.execute("INSERT INTO receipts VALUES(1,100)", []))
            .unwrap()
            .unwrap();
        let before = observer.queries.load(Ordering::Relaxed);
        let outcome = writer
            .batched(|tx| {
                tx.execute_batch(
                    "DROP TABLE receipts;
                CREATE TABLE receipts(id INTEGER PRIMARY KEY,at INTEGER NOT NULL) WITHOUT ROWID;",
                )
                .unwrap();
                assert!(observer.error.lock().unwrap().is_none());
                Err::<(), _>("rollback DDL")
            })
            .unwrap();
        assert_eq!(outcome, Err("rollback DDL"));
        assert!(observer.error.lock().unwrap().is_none());
        assert_eq!(*observer.deadline.lock().unwrap(), Some(100));
        assert_eq!(observer.queries.load(Ordering::Relaxed), before);
        writer
            .batched(|tx| {
                tx.execute_batch(
                    "DROP TABLE receipts;
            CREATE TABLE replacement(id INTEGER PRIMARY KEY,at INTEGER NOT NULL);
            INSERT INTO replacement VALUES(2,50);",
                )
            })
            .unwrap()
            .unwrap();
        assert!(observer.error.lock().unwrap().is_some());
        writer
            .batched(|tx| {
                tx.execute_batch(
                    "ALTER TABLE replacement RENAME TO receipts;
                CREATE INDEX receipts_at ON receipts(at);",
                )
            })
            .unwrap()
            .unwrap();
        assert!(observer.error.lock().unwrap().is_none());
        assert_eq!(*observer.deadline.lock().unwrap(), Some(50));
        assert_eq!(observer.queries.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn committed_identical_table_replacement_clears_deadline_and_coalesces_missed_hooks() {
        let (writer, observer) = writer();
        writer
            .batched(|tx| tx.execute("INSERT INTO receipts VALUES(1,100)", []))
            .unwrap()
            .unwrap();
        assert_eq!(*observer.deadline.lock().unwrap(), Some(100));
        let before = observer.queries.load(Ordering::Relaxed);
        writer
            .batched(|tx| {
                tx.execute_batch(
                    "DROP TABLE receipts;
            CREATE TABLE receipts(id INTEGER PRIMARY KEY,at INTEGER NOT NULL);
            CREATE INDEX receipts_at ON receipts(at);",
                )
            })
            .unwrap()
            .unwrap();
        assert_eq!(*observer.deadline.lock().unwrap(), None);
        assert_eq!(observer.queries.load(Ordering::Relaxed), before + 1);
        assert_eq!(observer.hints.load(Ordering::Relaxed), 1);
        writer
            .batched(|tx| tx.execute("INSERT INTO receipts VALUES(2,200)", []))
            .unwrap()
            .unwrap();
        {
            let held = writer.write();
            // Both replacement and missed callbacks need refresh, but one hint suffices.
            held.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
            held.execute_batch(
                "DROP TABLE receipts;
                CREATE TABLE receipts(id INTEGER PRIMARY KEY,at INTEGER NOT NULL);
                CREATE INDEX receipts_at ON receipts(at);",
            )
            .unwrap();
        }
        drop(writer.write());
        assert_eq!(*observer.deadline.lock().unwrap(), None);
        assert_eq!(observer.hints.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn failed_outer_commit_and_unexecuted_ddl_publish_only_the_resolved_state() {
        let (writer, observer) = writer();
        {
            let held = writer.write();
            held.execute_batch("PRAGMA foreign_keys=ON;
                CREATE TABLE parent(id INTEGER PRIMARY KEY);
                CREATE TABLE child(id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);")
                .unwrap();
        }
        drop(writer.write());
        assert_eq!(observer.queries.load(Ordering::Relaxed), 0);
        writer
            .batched(|tx| {
                let _prepared = tx.prepare("DROP TABLE receipts").unwrap();
                Ok::<_, rusqlite::Error>(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(observer.queries.load(Ordering::Relaxed), 0);
        assert!(observer.error.lock().unwrap().is_none());
        let result = writer.batched(|tx| {
            tx.execute("INSERT INTO receipts VALUES(1,100)", [])?;
            tx.execute("INSERT INTO child VALUES(999)", [])?;
            assert_eq!(*observer.deadline.lock().unwrap(), None);
            Ok::<_, rusqlite::Error>(())
        });
        assert!(result.unwrap_err().contains("FOREIGN KEY"));
        assert_eq!(*observer.deadline.lock().unwrap(), None);
        assert!(observer.error.lock().unwrap().is_none());
        assert_eq!(observer.queries.load(Ordering::Relaxed), 1);
        writer
            .batched(|tx| tx.execute("INSERT INTO receipts VALUES(2,200)", []))
            .unwrap()
            .unwrap();
        assert_eq!(*observer.deadline.lock().unwrap(), Some(200));
    }

    #[test]
    fn replaced_borrower_callbacks_fence_missed_mutations_and_are_restored() {
        let (writer, observer) = writer();
        {
            let held = writer.write();
            held.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
            held.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
            held.execute("INSERT INTO receipts VALUES(1,100)", [])
                .unwrap();
            assert_eq!(observer.rows.load(Ordering::Relaxed), 0);
            assert_eq!(*observer.deadline.lock().unwrap(), None);
        }
        drop(writer.write());
        assert_eq!(*observer.deadline.lock().unwrap(), Some(100));
        assert_eq!(observer.queries.load(Ordering::Relaxed), 1);
        writer
            .batched(|tx| tx.execute("DELETE FROM receipts", []))
            .unwrap()
            .unwrap();
        assert_eq!(observer.rows.load(Ordering::Relaxed), 1);
        assert_eq!(*observer.deadline.lock().unwrap(), None);
        let before = observer.queries.load(Ordering::Relaxed);
        {
            writer
                .write()
                .execute("INSERT INTO unrelated VALUES(1)", [])
                .unwrap();
        }
        drop(writer.write());
        assert_eq!(observer.queries.load(Ordering::Relaxed), before);
    }

    #[test]
    fn scope_restoration_preserves_a_detected_missed_hook_without_dirtying_normal_swaps() {
        let mut connection = connection();
        let observer = Arc::new(Receipts::default());
        let state = MutationState::new(&connection, observer.clone()).unwrap();
        for replace in [false, true] {
            let tx = connection.transaction().unwrap();
            // This minimal fixture cannot create the full graph scope; dispatch swaps are
            // the same owned operation used by Scope begin/drop.
            install_rows(&tx, Some(state.clone()));
            if replace {
                tx.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
                tx.execute("INSERT INTO receipts VALUES(1,100)", [])
                    .unwrap();
            } else {
                tx.execute("INSERT INTO unrelated VALUES(1)", []).unwrap();
            }
            install_rows(&tx, Some(state.clone()));
            tx.commit().unwrap();
            state.resolved(&connection);
            assert_eq!(observer.queries.load(Ordering::Relaxed), u64::from(replace));
        }
        assert_eq!(*observer.deadline.lock().unwrap(), Some(100));
    }
}
