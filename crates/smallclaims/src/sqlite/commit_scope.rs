//! Transaction-resolved mutation routing for the writer's native hook dispatcher.
use rusqlite::{Connection, ffi};
use std::{borrow::Cow, collections::BTreeSet, ffi::{CStr, c_char, c_int, c_uint, c_void}, sync::{Mutex, atomic::{AtomicU64, Ordering}}};

#[derive(Default)]
pub(super) struct Changes {
    tables: BTreeSet<String>,
    pub(super) schema: bool,
    pub(super) unknown: bool,
}
impl Changes {
    pub(super) fn changed(&self) -> bool { self.schema || self.unknown || !self.tables.is_empty() }
    pub(super) fn matches(&self, tables: &[&str]) -> bool {
        self.schema || self.unknown || tables.iter().any(|table| self.tables.iter().any(|changed| changed.eq_ignore_ascii_case(table)))
    }
    fn merge(&mut self, other: Self) {
        self.tables.extend(other.tables);
        self.schema |= other.schema;
        self.unknown |= other.unknown;
    }
}

enum SavepointAction {
    Begin { name: String, transaction: bool },
    RollbackTo(usize),
    Release { index: usize, transaction: bool },
}

struct SuspendedStatement {
    key: usize,
    rows: Changes,
    changes_at_start: i64,
    action: Option<SavepointAction>,
}

struct DeferredProfile {
    rows: Changes,
    changes_at_start: i64,
}

#[derive(Default)]
pub(super) struct Ledger {
    pending: Changes,
    tentative: Option<Changes>,
    committed: Changes,
    savepoints: Vec<(String, Changes)>,
    statement: Option<usize>,
    statement_rows: Changes,
    changes_at_start: i64,
    savepoint_action: Option<SavepointAction>,
    transaction_savepoint: bool,
    suspended: Vec<SuspendedStatement>,
    deferred_profile: Option<DeferredProfile>,
}
impl Ledger {
    pub(super) fn row(&mut self, database: &str, table: &str) {
        if database != "main" { return; }
        let changes = if self.statement.is_some() {
            &mut self.statement_rows
        } else {
            self.pending.unknown = true;
            &mut self.pending
        };
        if !changes.tables.contains(table) {
            changes.tables.insert(table.to_owned());
        }
    }
    pub(super) fn committing(&mut self) {
        if self.statement.is_none() && self.deferred_profile.is_none() {
            // A replaced trace callback hid transaction-control/DDL execution. The row
            // and commit hooks still fence this actual write, but cannot route it selectively.
            self.pending.unknown = true;
        }
        self.tentative = Some(std::mem::take(&mut self.pending));
        // The hook precedes the actual commit. Keep savepoint state until success is known.
    }
    pub(super) fn rollback(&mut self) {
        self.pending = Changes::default();
        self.tentative = None;
        self.savepoints.clear();
        self.statement_rows = Changes::default();
        self.deferred_profile = None;
        self.savepoint_action = None;
        self.transaction_savepoint = false;
        for statement in &mut self.suspended {
            statement.rows = Changes::default();
            statement.action = None;
        }
        ROLLBACKS.fetch_add(1, Ordering::Relaxed);
    }
    fn confirm(&mut self, autocommit: bool) {
        if let Some(changes) = self.tentative.take() {
            if autocommit {
                let mut changes = changes;
                for (_, parent) in self.savepoints.drain(..) {
                    changes.merge(parent);
                }
                self.committed.merge(changes);
                self.transaction_savepoint = false;
            } else {
                // A busy/failed COMMIT left the transaction alive.
                self.pending.merge(changes);
            }
        }
    }
    pub(super) fn take(&mut self, connection: &Connection, unknown: bool) -> Changes {
        // SAFETY: this is the exclusively lent, live writer connection. Unlike the legacy
        // counter, the 64-bit API matches the baseline used by the native trace callback.
        let total_changes = unsafe { ffi::sqlite3_total_changes64(connection.handle()) };
        self.resolve_deferred(total_changes);
        let autocommit = connection.is_autocommit();
        self.confirm(autocommit);
        if autocommit {
            // Prepared/no-op DDL can run without ever opening a write transaction.
            self.pending = Changes::default();
        }
        let mut changes = std::mem::take(&mut self.committed);
        changes.unknown |= unknown;
        changes
    }
    fn suspend(&mut self) -> Option<SuspendedStatement> {
        self.statement.take().map(|key| SuspendedStatement {
            key,
            rows: std::mem::take(&mut self.statement_rows),
            changes_at_start: self.changes_at_start,
            action: self.savepoint_action.take(),
        })
    }

    fn restore(&mut self, statement: SuspendedStatement) {
        self.statement = Some(statement.key);
        self.statement_rows = statement.rows;
        self.changes_at_start = statement.changes_at_start;
        self.savepoint_action = statement.action;
    }

    fn resolve_deferred(&mut self, total_changes: i64) {
        if let Some(profile) = self.deferred_profile.take()
            && total_changes > profile.changes_at_start
        {
            if let Some(tentative) = &mut self.tentative {
                tentative.merge(profile.rows);
            } else {
                self.pending.merge(profile.rows);
            }
        }
    }

    fn started(&mut self, key: usize, sql: &str, autocommit: bool, readonly: bool, total_changes: i64, active_write: bool) {
        self.resolve_deferred(total_changes);
        self.confirm(autocommit);
        if self.statement == Some(key) { return; }
        // A live RETURNING statement or a reentrant SQL function can run other SQL
        // before PROFILE. Preserve its row hints while tracking the intervening command.
        if let Some(statement) = self.suspend() {
            self.suspended.push(statement);
        }
        self.statement = Some(key);
        self.changes_at_start = total_changes;
        self.statement_rows = Changes::default();
        // SQLite omits system tables from preupdate callbacks. Explicit sequence writes
        // still change the claims index authority; resolve this hint like ordinary rows,
        // so zero-row statements and transaction/savepoint rollbacks remain no-ops.
        if !readonly && writes_main_sequence(sql) {
            self.row("main", "sqlite_sequence");
            self.row("main", "claims");
        }
        self.savepoint_action = None;
        let mut words = Tokens(sql);
        let Some(first) = words.next() else { return; };
        if first.eq_ignore_ascii_case("SAVEPOINT") {
            if !active_write && let Some(name) = words.next() {
                self.savepoint_action = Some(SavepointAction::Begin { name: name.into_owned(), transaction: autocommit });
            }
        } else if first.eq_ignore_ascii_case("ROLLBACK") {
            let mut next = words.next();
            if next.as_deref().is_some_and(|word| word.eq_ignore_ascii_case("TRANSACTION")) { next = words.next(); }
            if next.as_deref().is_some_and(|word| word.eq_ignore_ascii_case("TO")) {
                let mut name = words.next();
                if name.as_deref().is_some_and(|word| word.eq_ignore_ascii_case("SAVEPOINT")) { name = words.next(); }
                if let Some(name) = name && let Some(index) = self.savepoints.iter().rposition(|(saved, _)| saved.eq_ignore_ascii_case(&name)) {
                    self.savepoint_action = Some(SavepointAction::RollbackTo(index));
                }
            }
        } else if first.eq_ignore_ascii_case("RELEASE") {
            let mut name = words.next();
            if name.as_deref().is_some_and(|word| word.eq_ignore_ascii_case("SAVEPOINT")) { name = words.next(); }
            if !active_write && let Some(name) = name && let Some(index) = self.savepoints.iter().rposition(|(saved, _)| saved.eq_ignore_ascii_case(&name)) {
                self.savepoint_action = Some(SavepointAction::Release { index, transaction: index == 0 && self.transaction_savepoint });
            }
        } else if readonly {
            // Transaction-control statements are readonly too, so process them first.
        } else if ["CREATE", "ALTER", "DROP", "REINDEX", "VACUUM"].iter().any(|keyword| first.eq_ignore_ascii_case(keyword)) {
            self.pending.schema = true;
        } else if first.eq_ignore_ascii_case("PRAGMA") && words.any(|word| word.split(['.', '=', '(']).any(|part| part.eq_ignore_ascii_case("schema_version") || part.eq_ignore_ascii_case("writable_schema"))) {
            self.pending.unknown = true;
        }
    }

    fn finished(&mut self, key: usize, total_changes: i64, autocommit: bool, resetting: bool) {
        // Reset/finalize callbacks can follow one another without a new TRACE_STMT.
        // The preceding reset has now completed and advanced its counter: resolve its
        // hints before another still-busy PROFILE replaces the single deferred slot.
        self.resolve_deferred(total_changes);
        if self.statement != Some(key) {
            let Some(index) = self.suspended.iter().position(|statement| statement.key == key) else { return; };
            let statement = self.suspended.remove(index);
            if let Some(current) = self.suspend() {
                self.suspended.push(current);
            }
            self.restore(statement);
        }
        // Savepoint SQL may fail without rolling back its transaction. In particular,
        // deferred foreign keys can reject RELEASE of a transaction savepoint. Keep that
        // frame until the native commit is confirmed; autocommit=false means it survived.
        match self.savepoint_action.take() {
            Some(SavepointAction::Begin { name, transaction }) if !autocommit => {
                self.savepoints.push((name, std::mem::take(&mut self.pending)));
                self.transaction_savepoint |= transaction;
            }
            Some(SavepointAction::RollbackTo(index)) if !autocommit => {
                self.pending = Changes::default();
                self.savepoints.truncate(index + 1);
                // ROLLBACK TO also undoes writes of a still-live RETURNING statement.
                // Such a writer could not predate this savepoint: SQLite rejects opening
                // savepoints while a writable statement is active. Its delayed PROFILE
                // must not resurrect hints after another statement advances total_changes.
                self.statement_rows = Changes::default();
                for statement in &mut self.suspended {
                    statement.rows = Changes::default();
                }
            }
            Some(SavepointAction::Release { index, transaction: false }) if !autocommit => {
                for (_, parent) in self.savepoints.drain(index..) {
                    self.pending.merge(parent);
                }
            }
            Some(SavepointAction::Release { transaction: true, .. }) if autocommit && self.tentative.is_none() => {
                // An empty transaction savepoint may close without a native commit hook.
                self.savepoints.clear();
                self.pending = Changes::default();
                self.transaction_savepoint = false;
            }
            _ => {}
        }
        // total_changes excludes ordinary statement-ABORT writes, unlike preupdate hints.
        // A failing trigger can advance it before its outer statement rolls back: in that
        // case retaining a hint is deliberately conservative, never an authorization.
        let rows = std::mem::take(&mut self.statement_rows);
        if resetting {
            // sqlite3_reset/finalize invoke PROFILE before VdbeReset halts a live
            // RETURNING statement. Its successful writes and commit hook can therefore
            // occur after PROFILE; resolve the delta at the next SQL/writer boundary.
            self.deferred_profile = Some(DeferredProfile { rows, changes_at_start: self.changes_at_start });
        } else if total_changes > self.changes_at_start {
            if let Some(tentative) = &mut self.tentative {
                tentative.merge(rows);
            } else {
                self.pending.merge(rows);
            }
        }
        self.statement = None;
        self.confirm(autocommit);
        if let Some(statement) = self.suspended.pop() {
            self.restore(statement);
        }
    }
}

// Inspect only the top-level DML target, never string literals, comments or read sources.
// CTE bodies are parenthesized; SQLite does not allow data-changing CTE bodies.
fn writes_main_sequence(sql: &str) -> bool {
    let mut tokens = Tokens(sql);
    let mut depth = 0usize;
    while let Some(token) = tokens.next() {
        if token == "(" { depth += 1; continue; }
        if token == ")" { depth = depth.saturating_sub(1); continue; }
        if depth != 0 || matches!(token, Cow::Owned(_)) { continue; }
        let update = token.eq_ignore_ascii_case("UPDATE");
        let insert = token.eq_ignore_ascii_case("INSERT") || token.eq_ignore_ascii_case("REPLACE");
        let delete = token.eq_ignore_ascii_case("DELETE");
        if !update && !insert && !delete { continue; }
        let Some(mut target) = tokens.next() else { return false; };
        if target.eq_ignore_ascii_case("OR") {
            if tokens.next().is_none() { return false; }
            let Some(next) = tokens.next() else { return false; };
            target = next;
        }
        if (insert && target.eq_ignore_ascii_case("INTO")) || (delete && target.eq_ignore_ascii_case("FROM")) {
            let Some(next) = tokens.next() else { return false; };
            target = next;
        }
        if tokens.next().as_deref() == Some(".") {
            return target.eq_ignore_ascii_case("main")
                && tokens.next().is_some_and(|table| table.eq_ignore_ascii_case("sqlite_sequence"));
        }
        return target.eq_ignore_ascii_case("sqlite_sequence");
    }
    false
}

// SQLite accepts quoted identifiers, doubled quotes and comments between tokens.
// Punctuation stays separate for qualified DML targets and parenthesized CTE bodies.
struct Tokens<'a>(&'a str);
impl<'a> Iterator for Tokens<'a> {
    type Item = Cow<'a, str>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.0 = self.0.trim_start();
            if let Some(rest) = self.0.strip_prefix("--") {
                self.0 = rest.split_once('\n').map_or("", |(_, rest)| rest);
            } else if let Some(rest) = self.0.strip_prefix("/*") {
                self.0 = rest.split_once("*/").map_or("", |(_, rest)| rest);
            } else { break; }
        }
        let first = self.0.chars().next()?;
        if matches!(first, '(' | ')' | ',' | '.') {
            let (token, rest) = self.0.split_at(1);
            self.0 = rest;
            return Some(Cow::Borrowed(token));
        }
        if matches!(first, '\'' | '"' | '`' | '[') {
            let close = if first == '[' { ']' } else { first };
            let mut output = String::new();
            let mut chars = self.0[first.len_utf8()..].char_indices().peekable();
            while let Some((index, ch)) = chars.next() {
                if ch == close {
                    if close != ']' && chars.peek().is_some_and(|(_, ch)| *ch == close) {
                        chars.next(); output.push(close); continue;
                    }
                    self.0 = &self.0[first.len_utf8() + index + ch.len_utf8()..];
                    return Some(Cow::Owned(output));
                }
                output.push(ch);
            }
            self.0 = "";
            Some(Cow::Owned(output))
        } else {
            let end = self.0.char_indices().find_map(|(index, ch)| {
                (ch.is_whitespace() || matches!(ch, ';' | '\'' | '"' | '`' | '[' | '(' | ')' | ',' | '.')
                    || self.0[index..].starts_with("/*") || self.0[index..].starts_with("--"))
                    .then_some(index)
            }).unwrap_or(self.0.len());
            if end == 0 { self.0 = &self.0[1..]; return self.next(); }
            let word = Cow::Borrowed(&self.0[..end]);
            self.0 = &self.0[end..];
            Some(word)
        }
    }
}

// The bundled library enables this stable SQLite ABI via LIBSQLITE3_FLAGS. Declaring
// only the callback registration avoids bindgen/libclang and does not expose row values.
type Preupdate = unsafe extern "C" fn(*mut c_void, *mut ffi::sqlite3, c_int, *const c_char, *const c_char, i64, i64);
unsafe extern "C" {
    fn sqlite3_preupdate_hook(database: *mut ffi::sqlite3, callback: Option<Preupdate>, context: *mut c_void) -> *mut c_void;
}
pub(super) fn preupdate_available() -> bool {
    // SAFETY: the compile-option API borrows a static NUL-terminated string.
    unsafe { ffi::sqlite3_compileoption_used(c"ENABLE_PREUPDATE_HOOK".as_ptr()) != 0 }
}
pub(super) fn install_preupdate(connection: &Connection, ledger: &Mutex<Ledger>) -> bool {
    // SAFETY: the dispatcher retains the stable boxed/Arc-owned ledger through connection
    // closure. SQLite invokes this synchronously, never after closing the connection.
    // Row ids of WITHOUT ROWID tables are unspecified and deliberately never inspected.
    unsafe {
        let context = std::ptr::from_ref(ledger).cast_mut().cast();
        sqlite3_preupdate_hook(connection.handle(), Some(preupdated), context) == context
    }
}
unsafe extern "C" fn preupdated(context: *mut c_void, _: *mut ffi::sqlite3, _: c_int, database: *const c_char, table: *const c_char, _: i64, _: i64) {
    let _ = std::panic::catch_unwind(|| unsafe {
        // SAFETY: SQLite supplies valid UTF-8 names for this synchronous callback; no
        // borrowed SQLite data is retained and no SQL executes from a row callback.
        let ledger = &*context.cast::<Mutex<Ledger>>();
        let database = CStr::from_ptr(database).to_string_lossy();
        let table = CStr::from_ptr(table).to_string_lossy();
        ledger.lock().unwrap_or_else(std::sync::PoisonError::into_inner).row(&database, &table);
    });
}

pub(super) static INDEX_REFRESHES: AtomicU64 = AtomicU64::new(0);
pub(super) static GLOBAL_EVALUATIONS: AtomicU64 = AtomicU64::new(0);
pub(super) static SCOPED_EVALUATIONS: AtomicU64 = AtomicU64::new(0);
static BOUNDARIES: AtomicU64 = AtomicU64::new(0);
static NOOPS: AtomicU64 = AtomicU64::new(0);
static ROLLBACKS: AtomicU64 = AtomicU64::new(0);
#[derive(serde::Serialize)]
pub struct WriterObserverStats {
    pub boundaries: u64,
    pub noops: u64,
    pub rollbacks: u64,
    pub index_refreshes: u64,
    pub global_evaluations: u64,
    pub scoped_evaluations: u64,
}
pub fn writer_observer_stats() -> WriterObserverStats {
    WriterObserverStats {
        boundaries: BOUNDARIES.load(Ordering::Relaxed), noops: NOOPS.load(Ordering::Relaxed),
        rollbacks: ROLLBACKS.load(Ordering::Relaxed), index_refreshes: INDEX_REFRESHES.load(Ordering::Relaxed),
        global_evaluations: GLOBAL_EVALUATIONS.load(Ordering::Relaxed), scoped_evaluations: SCOPED_EVALUATIONS.load(Ordering::Relaxed),
    }
}
pub(super) fn boundary(changes: &Changes) {
    BOUNDARIES.fetch_add(1, Ordering::Relaxed);
    if !changes.changed() { NOOPS.fetch_add(1, Ordering::Relaxed); }
}

pub(super) fn install_trace(connection: &Connection, ledger: &Mutex<Ledger>) {
    // SAFETY: MutationState owns this mutex and is retained by the connection's installed
    // hooks until SQLite closes. The callback never executes SQL or retains SQLite pointers.
    unsafe {
        ffi::sqlite3_trace_v2(connection.handle(), (ffi::SQLITE_TRACE_STMT | ffi::SQLITE_TRACE_PROFILE) as c_uint,
            Some(traced), std::ptr::from_ref(ledger).cast_mut().cast());
    }
}
unsafe extern "C" fn traced(event: c_uint, context: *mut c_void, statement: *mut c_void, detail: *mut c_void) -> c_int {
    // SAFETY: SQLite supplies live pointers for the duration of this synchronous callback.
    // Never unwind across the C boundary, including on a poisoned observer mutex.
    let _ = std::panic::catch_unwind(|| unsafe {
        #[cfg(any(test, feature = "test-support"))]
        super::work::traced(event, std::ptr::null_mut(), statement, detail);
        let statement = statement.cast::<ffi::sqlite3_stmt>();
        let sql = ffi::sqlite3_sql(statement);
        if sql.is_null() { return; }
        let sql = CStr::from_ptr(sql).to_string_lossy();
        let ledger = &*context.cast::<Mutex<Ledger>>();
        let mut ledger = ledger.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let database = ffi::sqlite3_db_handle(statement);
        if event == ffi::SQLITE_TRACE_STMT as c_uint {
            let control = Tokens(&sql).next().is_some_and(|word| word.eq_ignore_ascii_case("SAVEPOINT") || word.eq_ignore_ascii_case("RELEASE"));
            let mut active_write = false;
            if control {
                let mut other = ffi::sqlite3_next_stmt(database, std::ptr::null_mut());
                while !other.is_null() {
                    if other != statement && ffi::sqlite3_stmt_readonly(other) == 0 && ffi::sqlite3_stmt_busy(other) != 0 {
                        active_write = true;
                        break;
                    }
                    other = ffi::sqlite3_next_stmt(database, other);
                }
            }
            ledger.started(statement as usize, &sql, ffi::sqlite3_get_autocommit(database) != 0, ffi::sqlite3_stmt_readonly(statement) != 0, ffi::sqlite3_total_changes64(database), active_write);
            // Trigger subprogram traces share the outer statement pointer. SQLite exposes
            // their executed SQL through the "-- " detail, not sqlite3_sql(statement).
            let detail = CStr::from_ptr(detail.cast::<c_char>()).to_string_lossy();
            if detail.strip_prefix("-- ").is_some_and(writes_main_sequence) {
                ledger.row("main", "sqlite_sequence");
                ledger.row("main", "claims");
            }
        } else {
            // SQLite does not expose the current VDBE result code until after PROFILE.
            // Never inspect sqlite3_errcode here: it can still belong to an earlier SQL
            // error. Transaction and savepoint rollback are resolved by native hooks and
            // executed control statements, respectively.
            ledger.finished(statement as usize, ffi::sqlite3_total_changes64(database), ffi::sqlite3_get_autocommit(database) != 0, ffi::sqlite3_stmt_busy(statement) != 0);
            drop(ledger);
            let nanos = *detail.cast::<u64>();
            super::record_sqlite_time(&sql, std::time::Duration::from_nanos(nanos));
        }
    });
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::{CommitObserver, WriterConnection, work::SqliteWorkScope};
    use std::sync::Arc;

    fn writer() -> WriterConnection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE alpha(id INTEGER PRIMARY KEY);
             CREATE TABLE beta(id TEXT PRIMARY KEY) WITHOUT ROWID;"
        ).unwrap();
        WriterConnection::new(connection, Arc::new(AtomicU64::new(0)))
    }
    fn counter(writer: &WriterConnection, tables: Option<&'static [&'static str]>) -> (Arc<AtomicU64>, CommitObserver) {
        let count = Arc::new(AtomicU64::new(0));
        let observed = count.clone();
        let callback = move |_: &Connection| { observed.fetch_add(1, Ordering::Relaxed); };
        let handle = match tables {
            Some(tables) => writer.observe_tables(tables, callback),
            None => writer.observe_commits(callback),
        };
        (count, handle)
    }
    fn calls(count: &AtomicU64) -> u64 { count.load(Ordering::Relaxed) }

    #[test]
    fn sequence_only_writes_refresh_index_and_only_index_dependents() {
        let writer = writer();
        writer.write().execute_batch("INSERT INTO claims DEFAULT VALUES").unwrap();
        let (global, _global) = counter(&writer, None);
        let (claims, _claims) = counter(&writer, Some(&["claims"]));
        let (sequence, _sequence) = counter(&writer, Some(&["sqlite_sequence"]));
        let (alpha, _alpha) = counter(&writer, Some(&["alpha"]));
        for sql in [
            "UPDATE sqlite_sequence SET seq=seq+1 WHERE name='claims'",
            "UPDATE OR FAIL main.\"sqlite_sequence\" SET seq=seq+1 WHERE name='claims'",
            "WITH input(v) AS(SELECT 1)UPDATE [main].[sqlite_sequence] SET seq=seq+(SELECT v FROM input) WHERE name='claims'",
        ] {
            let before = writer.committed_index.load(Ordering::Acquire);
            writer.write().execute_batch(sql).unwrap();
            let connection = writer.write();
            assert_eq!(writer.committed_index.load(Ordering::Acquire), before + 1);
            assert_eq!(crate::store::current_index(&connection).unwrap(), before + 1);
        }
        assert_eq!((calls(&global), calls(&claims), calls(&sequence), calls(&alpha)), (3, 3, 3, 0));
        for sql in [
            "UPDATE sqlite_sequence SET seq=seq+1 WHERE name='missing'",
            "BEGIN; UPDATE sqlite_sequence SET seq=seq+1 WHERE name='claims'; ROLLBACK",
            "SAVEPOINT s; UPDATE sqlite_sequence SET seq=seq+1 WHERE name='claims'; ROLLBACK TO s; RELEASE s",
            "UPDATE alpha SET id=id WHERE id=(SELECT seq FROM sqlite_sequence WHERE name='claims')",
        ] {
            let connection = writer.write();
            connection.execute_batch(sql).unwrap();
            let work = SqliteWorkScope::start();
            drop(connection);
            assert_eq!(work.finish().statements, 0, "post-loan SQL after {sql}");
        }
        assert_eq!((calls(&global), calls(&claims), calls(&sequence), calls(&alpha)), (3, 3, 3, 0));
    }

    #[test]
    fn sequence_write_target_ignores_literals_comments_and_read_sources() {
        for sql in [
            "UPDATE alpha SET id='sqlite_sequence'",
            "UPDATE alpha SET id=(SELECT seq FROM sqlite_sequence)",
            "UPDATE /* sqlite_sequence */ alpha SET id=id",
            "UPDATE temp.sqlite_sequence SET seq=1",
            "WITH c AS(SELECT 'UPDATE sqlite_sequence') UPDATE alpha SET id=id",
            "WITH c AS(SELECT 1) INSERT INTO alpha SELECT seq FROM sqlite_sequence",
        ] {
            assert!(!writes_main_sequence(sql), "{sql}");
        }
        for sql in [
            "INSERT OR REPLACE INTO main.sqlite_sequence(name,seq) VALUES('claims',9)",
            "REPLACE INTO \"main\".\"sqlite_sequence\"(name,seq) VALUES('claims',9)",
            "DELETE FROM main.`sqlite_sequence` WHERE name='claims'",
        ] {
            assert!(writes_main_sequence(sql), "{sql}");
        }
    }

    #[test]
    fn cached_and_triggered_sequence_writes_preserve_claims_dependencies() {
        let writer = writer();
        writer.write().execute_batch(
            "INSERT INTO claims DEFAULT VALUES;
             CREATE TRIGGER advance_claims AFTER INSERT ON alpha BEGIN
               UPDATE sqlite_sequence SET seq=seq+1 WHERE name='claims';
             END;"
        ).unwrap();
        let (claims, _claims) = counter(&writer, Some(&["claims"]));
        {
            let connection = writer.write();
            let mut statement = connection.prepare_cached("UPDATE sqlite_sequence SET seq=seq+1 WHERE name='claims'").unwrap();
            statement.execute([]).unwrap();
            statement.execute([]).unwrap();
        }
        assert_eq!(calls(&claims), 1);
        writer.write().execute_batch("INSERT INTO alpha VALUES(1)").unwrap();
        assert_eq!(calls(&claims), 2);
        let connection = writer.write();
        assert_eq!(writer.committed_index.load(Ordering::Acquire), crate::store::current_index(&connection).unwrap());
    }

    #[test]
    fn no_op_read_only_zero_row_and_rollback_boundaries_run_no_refresh_sql() {
        let writer = writer();
        let (global, _global) = counter(&writer, None);
        let (alpha, _alpha) = counter(&writer, Some(&["alpha"]));
        for sql in [
            "",
            "SELECT 1",
            "BEGIN; SELECT 1; COMMIT",
            "UPDATE alpha SET id=id WHERE id=999",
            "BEGIN; INSERT INTO alpha VALUES(1); ROLLBACK",
            "SAVEPOINT s; INSERT INTO alpha VALUES(1); ROLLBACK TO s; RELEASE s",
        ] {
            let connection = writer.write();
            connection.execute_batch(sql).unwrap();
            let work = SqliteWorkScope::start();
            drop(connection);
            assert_eq!(work.finish().statements, 0, "post-loan SQL after {sql}");
            assert_eq!(calls(&global), 0, "global evaluation after {sql}");
            assert_eq!(calls(&alpha), 0, "dependent evaluation after {sql}");
        }
    }

    #[test]
    fn committed_tables_survive_later_rollback_without_waking_other_dependencies() {
        let writer = writer();
        let (global, _global) = counter(&writer, None);
        let (alpha, _alpha) = counter(&writer, Some(&["alpha"]));
        let (beta, _beta) = counter(&writer, Some(&["beta"]));
        let (both, _both) = counter(&writer, Some(&["alpha", "beta"]));
        writer.write().execute_batch(
            "BEGIN; INSERT INTO alpha VALUES(1); COMMIT;
             BEGIN; INSERT INTO beta VALUES('rolled-back'); ROLLBACK;
             BEGIN; INSERT INTO alpha VALUES(2); COMMIT;"
        ).unwrap();
        assert_eq!((calls(&global), calls(&alpha), calls(&beta), calls(&both)), (1, 1, 0, 1));
        writer.write().execute_batch("INSERT INTO beta VALUES('committed')").unwrap();
        assert_eq!((calls(&global), calls(&alpha), calls(&beta), calls(&both)), (2, 1, 1, 2));
    }

    #[test]
    fn nested_quoted_cached_savepoints_discard_only_the_rolled_back_level() {
        let writer = writer();
        let (alpha, _alpha) = counter(&writer, Some(&["alpha"]));
        let (beta, _beta) = counter(&writer, Some(&["beta"]));
        {
            let connection = writer.write();
            let mut rollback = connection.prepare_cached("ROLLBACK /* scope */ TO SAVEPOINT \"inner space\"").unwrap();
            let mut insert_beta = connection.prepare_cached("INSERT INTO beta VALUES(?1)").unwrap();
            for value in ["first", "second"] {
                connection.execute_batch("SAVEPOINT 'outer'; INSERT OR IGNORE INTO alpha VALUES(1); SAVEPOINT \"inner space\"").unwrap();
                insert_beta.execute([value]).unwrap();
                rollback.execute([]).unwrap();
                connection.execute_batch("RELEASE \"inner space\"; RELEASE 'outer'").unwrap();
            }
        }
        assert_eq!((calls(&alpha), calls(&beta)), (1, 0));
    }

    #[test]
    fn ddl_commits_route_even_with_zero_total_changes_and_rollback_does_not() {
        let writer = writer();
        let (global, _global) = counter(&writer, None);
        let (alpha, _alpha) = counter(&writer, Some(&["alpha"]));
        {
            let connection = writer.write();
            let before = connection.total_changes();
            connection.execute_batch("CREATE INDEX alpha_id ON alpha(id)").unwrap();
            assert_eq!(connection.total_changes(), before);
        }
        assert_eq!((calls(&global), calls(&alpha)), (1, 1));
        writer.write().execute_batch("BEGIN; DROP INDEX alpha_id; ROLLBACK").unwrap();
        assert_eq!((calls(&global), calls(&alpha)), (1, 1));
        writer.write().execute_batch("CREATE INDEX IF NOT EXISTS alpha_id ON alpha(id)").unwrap();
        assert_eq!((calls(&global), calls(&alpha)), (1, 1));
        writer.write().execute_batch("CREATE TEMP TABLE scratch(id INTEGER)").unwrap();
        assert_eq!((calls(&global), calls(&alpha)), (1, 1), "temp schema is not main authority");
    }

    #[test]
    fn replacement_of_each_native_coverage_hook_invalidates_dependencies_fail_closed() {
        for hook in ["row", "authorizer", "commit", "rollback", "preupdate"] {
            let writer = writer();
            let (alpha, _alpha) = counter(&writer, Some(&["alpha"]));
            let (beta, _beta) = counter(&writer, Some(&["beta"]));
            {
                let connection = writer.write();
                match hook {
                    "row" => connection.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>),
                    "authorizer" => connection.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>),
                    "commit" => connection.commit_hook(None::<fn() -> bool>),
                    "rollback" => connection.rollback_hook(None::<fn()>),
                    "preupdate" => unsafe {
                        // SAFETY: deliberately remove the callback on this exclusively lent
                        // connection; the boundary must detect and restore its ownership.
                        sqlite3_preupdate_hook(connection.handle(), None, std::ptr::null_mut());
                    },
                    _ => unreachable!(),
                }
            }
            assert_eq!((calls(&alpha), calls(&beta)), (1, 1), "{hook}");
            writer.write().execute_batch("INSERT INTO beta VALUES('restored')").unwrap();
            assert_eq!((calls(&alpha), calls(&beta)), (1, 2), "{hook}");
        }
    }
}

#[cfg(test)]
mod statement_failure_tests {
    use super::*;
    use crate::sqlite::{WriterConnection, work::SqliteWorkScope};
    use std::sync::Arc;

    #[test]
    fn caught_statement_abort_then_commit_is_noop_but_partial_fail_publishes() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE source(id INTEGER PRIMARY KEY);"
        ).unwrap();
        let writer = WriterConnection::new(connection, Arc::new(AtomicU64::new(0)));
        let count = Arc::new(AtomicU64::new(0));
        let observed = count.clone();
        let _observer = writer.observe_tables(&["source"], move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        });
        {
            let connection = writer.write();
            connection.execute_batch("BEGIN").unwrap();
            let before = connection.total_changes();
            assert!(connection.execute_batch("INSERT INTO source VALUES(1),(1)").is_err());
            assert_eq!(connection.total_changes(), before);
            connection.execute_batch("COMMIT").unwrap();
            let work = SqliteWorkScope::start();
            drop(connection);
            assert_eq!(work.finish().statements, 0);
        }
        assert_eq!(count.load(Ordering::Relaxed), 0);
        {
            let connection = writer.write();
            connection.execute_batch("BEGIN").unwrap();
            assert!(connection.execute_batch("INSERT OR FAIL INTO source VALUES(1),(1)").is_err());
            connection.execute_batch("COMMIT").unwrap();
        }
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
}

#[cfg(test)]
mod savepoint_resolution_tests {
    use super::*;
    use crate::sqlite::{WriterConnection, work::SqliteWorkScope};
    use std::sync::Arc;

    fn writer() -> WriterConnection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE parent(id INTEGER PRIMARY KEY);
             CREATE TABLE child(parent_id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);
             CREATE TABLE alpha(id INTEGER PRIMARY KEY);"
        ).unwrap();
        WriterConnection::new(connection, Arc::new(AtomicU64::new(0)))
    }

    #[test]
    fn failed_transaction_savepoint_release_preserves_rollback_target() {
        let writer = writer();
        let globals = Arc::new(AtomicU64::new(0));
        let observed = globals.clone();
        let _global = writer.observe_commits(move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let dependents = Arc::new(AtomicU64::new(0));
        let observed = dependents.clone();
        let _dependent = writer.observe_tables(&["child"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let connection = writer.write();
        connection.execute_batch("SAVEPOINT s; INSERT INTO child VALUES(1)").unwrap();
        assert!(connection.execute_batch("RELEASE s").is_err());
        assert!(!connection.is_autocommit());
        connection.execute_batch("ROLLBACK TO s; RELEASE s").unwrap();
        assert!(connection.is_autocommit());
        assert_eq!(connection.query_row("SELECT count(*) FROM child", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
        let work = SqliteWorkScope::start();
        drop(connection);
        assert_eq!(work.finish().statements, 0);
        assert_eq!(globals.load(Ordering::Relaxed), 0);
        assert_eq!(dependents.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn adjacent_comments_are_sql_token_boundaries_for_savepoint_controls() {
        let writer = writer();
        let globals = Arc::new(AtomicU64::new(0));
        let observed = globals.clone();
        let _global = writer.observe_commits(move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let dependents = Arc::new(AtomicU64::new(0));
        let observed = dependents.clone();
        let _dependent = writer.observe_tables(&["alpha"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        for sql in [
            "SAVEPOINT s; INSERT INTO alpha VALUES(1); ROLLBACK/**/TO s; RELEASE s",
            "SAVEPOINT/**/s; INSERT INTO alpha VALUES(1); ROLLBACK/**/TO/**/s; RELEASE/**/s",
            "SAVEPOINT-- begin\ns; INSERT INTO alpha VALUES(1); ROLLBACK-- undo\nTO s; RELEASE-- finish\ns",
        ] {
            let connection = writer.write();
            connection.execute_batch(sql).unwrap();
            let work = SqliteWorkScope::start();
            drop(connection);
            assert_eq!(work.finish().statements, 0, "{sql}");
            assert_eq!(globals.load(Ordering::Relaxed), 0, "{sql}");
            assert_eq!(dependents.load(Ordering::Relaxed), 0, "{sql}");
        }
        // A commented RELEASE must also merge a retained nested level, not discard it.
        writer.write().execute_batch(
            "SAVEPOINT/**/outer; SAVEPOINT/**/inner;
             INSERT INTO alpha VALUES(1); RELEASE/**/inner; RELEASE/**/outer"
        ).unwrap();
        assert_eq!(globals.load(Ordering::Relaxed), 1);
        assert_eq!(dependents.load(Ordering::Relaxed), 1);
    }
}

#[cfg(test)]
mod busy_savepoint_tests {
    use super::*;
    use crate::sqlite::{WriterConnection, work::SqliteWorkScope};
    use std::sync::Arc;

    #[test]
    fn busy_nested_release_keeps_frames_and_preserves_the_active_statement_hints() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE alpha(id INTEGER PRIMARY KEY);"
        ).unwrap();
        let writer = WriterConnection::new(connection, Arc::new(AtomicU64::new(0)));
        let count = Arc::new(AtomicU64::new(0));
        let observed = count.clone();
        let _observer = writer.observe_tables(&["alpha"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        for rollback in [true, false] {
            let connection = writer.write();
            connection.execute_batch("SAVEPOINT outer; SAVEPOINT inner").unwrap();
            {
                let mut statement = connection.prepare("INSERT INTO alpha VALUES(1) RETURNING id").unwrap();
                let mut rows = statement.query([]).unwrap();
                assert!(rows.next().unwrap().is_some());
                assert!(connection.execute_batch("RELEASE inner").is_err());
                // Also keep the frame stack unchanged when opening a savepoint is busy.
                assert!(connection.execute_batch("SAVEPOINT rejected").is_err());
            }
            if rollback {
                connection.execute_batch("ROLLBACK TO inner").unwrap();
            }
            connection.execute_batch("RELEASE inner; RELEASE outer").unwrap();
            if rollback {
                let work = SqliteWorkScope::start();
                drop(connection);
                assert_eq!(work.finish().statements, 0);
                assert_eq!(count.load(Ordering::Relaxed), 0);
            } else {
                drop(connection);
                assert_eq!(count.load(Ordering::Relaxed), 1);
            }
        }
    }
}

#[cfg(test)]
mod suspended_rollback_tests {
    use super::*;
    use crate::sqlite::WriterConnection;
    use std::sync::Arc;

    #[test]
    fn rollback_to_discards_live_returning_hints_without_discarding_later_writes() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE alpha(id INTEGER PRIMARY KEY);
             CREATE TABLE beta(id INTEGER PRIMARY KEY);"
        ).unwrap();
        let writer = WriterConnection::new(connection, Arc::new(AtomicU64::new(0)));
        let alpha = Arc::new(AtomicU64::new(0));
        let observed = alpha.clone();
        let _alpha = writer.observe_tables(&["alpha"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let beta = Arc::new(AtomicU64::new(0));
        let observed = beta.clone();
        let _beta = writer.observe_tables(&["beta"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let globals = Arc::new(AtomicU64::new(0));
        let observed = globals.clone();
        let _global = writer.observe_commits(move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        {
            let connection = writer.write();
            connection.execute_batch("SAVEPOINT s").unwrap();
            {
                let mut statement = connection.prepare("INSERT INTO alpha VALUES(1) RETURNING id").unwrap();
                let mut rows = statement.query([]).unwrap();
                assert!(rows.next().unwrap().is_some());
                connection.execute_batch("ROLLBACK TO s; INSERT INTO beta VALUES(1)").unwrap();
                // Dropping the live statement runs its delayed PROFILE after beta's write.
            }
            connection.execute_batch("RELEASE s").unwrap();
            assert_eq!(connection.query_row("SELECT count(*) FROM alpha", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
            assert_eq!(connection.query_row("SELECT count(*) FROM beta", [], |row| row.get::<_, u64>(0)).unwrap(), 1);
        }
        assert_eq!(alpha.load(Ordering::Relaxed), 0);
        assert_eq!(beta.load(Ordering::Relaxed), 1);
        assert_eq!(globals.load(Ordering::Relaxed), 1);
    }
}

#[cfg(test)]
mod returning_reset_tests {
    use super::*;
    use crate::sqlite::WriterConnection;
    use std::sync::Arc;

    #[test]
    fn unfinished_autocommit_returning_publishes_scoped_writes_at_guard_boundary() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE alpha(id INTEGER PRIMARY KEY);
             CREATE TABLE beta(id INTEGER PRIMARY KEY);"
        ).unwrap();
        let writer = WriterConnection::new(connection, Arc::new(AtomicU64::new(0)));
        let alpha = Arc::new(AtomicU64::new(0));
        let observed = alpha.clone();
        let _alpha = writer.observe_tables(&["alpha"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let beta = Arc::new(AtomicU64::new(0));
        let observed = beta.clone();
        let _beta = writer.observe_tables(&["beta"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let globals = Arc::new(AtomicU64::new(0));
        let observed = globals.clone();
        let _global = writer.observe_commits(move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        {
            let connection = writer.write();
            {
                let mut statement = connection.prepare("INSERT INTO alpha VALUES(1),(2) RETURNING id").unwrap();
                let mut rows = statement.query([]).unwrap();
                assert!(rows.next().unwrap().is_some());
                // Do not drain RETURNING. Reset runs PROFILE before it updates the final
                // change count or invokes the native autocommit commit hook.
            }
            // No subsequent SQL: the guard boundary itself must resolve those writes.
        }
        assert_eq!(alpha.load(Ordering::Relaxed), 1);
        assert_eq!(beta.load(Ordering::Relaxed), 0);
        assert_eq!(globals.load(Ordering::Relaxed), 1);
        assert_eq!(writer.write().query_row("SELECT count(*) FROM alpha", [], |row| row.get::<_, u64>(0)).unwrap(), 2);
    }

    #[test]
    fn consecutive_returning_resets_preserve_every_committed_table() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE alpha(id INTEGER PRIMARY KEY);
             CREATE TABLE beta(id INTEGER PRIMARY KEY);"
        ).unwrap();
        let writer = WriterConnection::new(connection, Arc::new(AtomicU64::new(0)));
        let alpha = Arc::new(AtomicU64::new(0));
        let observed = alpha.clone();
        let _alpha = writer.observe_tables(&["alpha"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let beta = Arc::new(AtomicU64::new(0));
        let observed = beta.clone();
        let _beta = writer.observe_tables(&["beta"], move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        let globals = Arc::new(AtomicU64::new(0));
        let observed = globals.clone();
        let _global = writer.observe_commits(move |_| { observed.fetch_add(1, Ordering::Relaxed); });
        {
            let connection = writer.write();
            connection.execute_batch("BEGIN").unwrap();
            let mut alpha_statement = connection.prepare("INSERT INTO alpha VALUES(1) RETURNING id").unwrap();
            let mut alpha_rows = alpha_statement.query([]).unwrap();
            assert!(alpha_rows.next().unwrap().is_some());
            let mut beta_statement = connection.prepare("INSERT INTO beta VALUES(1) RETURNING id").unwrap();
            let mut beta_rows = beta_statement.query([]).unwrap();
            assert!(beta_rows.next().unwrap().is_some());
            drop(beta_rows);
            drop(beta_statement);
            // No SQL between the two resets: beta's deferred hint must survive alpha's
            // PROFILE even though alpha creates a second deferred completion.
            drop(alpha_rows);
            drop(alpha_statement);
            connection.execute_batch("COMMIT").unwrap();
        }
        assert_eq!(alpha.load(Ordering::Relaxed), 1);
        assert_eq!(beta.load(Ordering::Relaxed), 1);
        assert_eq!(globals.load(Ordering::Relaxed), 1);
        let connection = writer.write();
        assert_eq!(connection.query_row("SELECT count(*) FROM alpha", [], |row| row.get::<_, u64>(0)).unwrap(), 1);
        assert_eq!(connection.query_row("SELECT count(*) FROM beta", [], |row| row.get::<_, u64>(0)).unwrap(), 1);
    }
}
