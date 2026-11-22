//! Connection-local transaction work hints, shared by finalizers without replacing hooks.
use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

const NAME: &std::ffi::CStr = c"fractal.transaction-work";
const RESOURCES: u8 = 1;
const CUSTOM: u8 = 2;

fn state(connection: &Connection) -> Option<&AtomicU8> {
    // SAFETY: this private name holds only the Arc<AtomicU8> transferred below.
    // SQLite owns it until connection close; this borrow cannot outlive the connection.
    unsafe {
        rusqlite::ffi::sqlite3_get_clientdata(connection.handle(), NAME.as_ptr())
            .cast::<AtomicU8>().as_ref()
    }
}

unsafe extern "C" fn destroy(pointer: *mut std::ffi::c_void) {
    // SAFETY: SQLite releases exactly the strong reference transferred by install,
    // including allocation failure in sqlite3_set_clientdata.
    unsafe { drop(Arc::from_raw(pointer.cast::<AtomicU8>())); }
}

pub(super) fn install(connection: &Connection) -> Result<()> {
    if state(connection).is_some() { return Ok(()); }
    let pending = Arc::new(AtomicU8::new(0));
    // SAFETY: SQLite owns this reference and releases it with the matching destructor.
    let result = unsafe {
        rusqlite::ffi::sqlite3_set_clientdata(connection.handle(), NAME.as_ptr(),
            Arc::into_raw(pending.clone()).cast_mut().cast(), Some(destroy))
    };
    if result != rusqlite::ffi::SQLITE_OK {
        return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(result), None).into());
    }
    for (name, bit) in [("resource_preparation_work_pending", RESOURCES), ("custom_work_pending", CUSTOM)] {
        let pending = pending.clone();
        connection.create_scalar_function(name, 0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
            move |_| { pending.fetch_or(bit, Ordering::Relaxed); Ok(0) })?;
    }
    // Keep hints raised until the OUTER transaction ends. Rolling back a savepoint
    // may restore queue rows that a preceding flush removed. Register with the
    // writer's transaction-resolution dispatcher instead of the raw hook slot: the
    // mutation dispatcher owns that slot and swaps it at construction and loan
    // boundaries, which silently drops a hook installed here.
    let committed = pending.clone();
    let rolled_back = pending.clone();
    smallclaims::sqlite::observe_transaction_resolution(
        connection,
        move || { committed.store(0, Ordering::Relaxed); false },
        move || rolled_back.store(0, Ordering::Relaxed),
    )?;
    Ok(())
}

fn pending(connection: &Connection, bit: u8) -> bool {
    // Bare replay/proof connections without capture conservatively flush normally.
    state(connection).is_none_or(|pending| pending.load(Ordering::Relaxed) & bit != 0)
}
pub(super) fn resources_pending(connection: &Connection) -> bool { pending(connection, RESOURCES) }
pub(super) fn custom_pending(connection: &Connection) -> bool { pending(connection, CUSTOM) }

#[cfg(test)]
mod tests {
    use super::*;
    use smallclaims::sqlite::{WriterConnection, WriterObserver};
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    #[derive(Default)]
    struct WorkObserver {
        mutations: AtomicUsize,
        faults: std::sync::Mutex<Vec<Arc<str>>>,
    }

    impl WriterObserver for WorkObserver {
        fn tables(&self) -> &'static [(&'static str, &'static str)] {
            &[("main", "work")]
        }
        fn mutated(&self, _: &str, _: &str) {
            self.mutations.fetch_add(1, Ordering::Relaxed);
        }
        fn unobserved(&self, _: &str, _: &str) {}
        fn failed(&self, error: Arc<str>) {
            self.faults.lock().unwrap().push(error);
        }
        fn published(&self, _: &Connection) {}
    }

    fn assert_work_clears_with_writer_observer(commit: bool) {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE work(id INTEGER PRIMARY KEY)").unwrap();
        // Match Store setup: work subscribers exist before the writer installs its
        // native hooks. A raw commit/rollback subscription would be replaced here.
        install(&connection).unwrap();
        let observer = Arc::new(WorkObserver::default());
        let writer = WriterConnection::new_with_observer(
            connection, Arc::new(AtomicU64::new(0)), observer.clone(),
        ).unwrap();

        // Separate loans also cover native hook reinstalls at writer boundaries.
        for _ in 0..2 {
            let mut connection = writer.write();
            assert!(!custom_pending(&connection));
            assert!(!resources_pending(&connection));
            let transaction = connection.transaction().unwrap();
            // A real write ensures both COMMIT and ROLLBACK invoke native hooks.
            transaction.execute("INSERT INTO work DEFAULT VALUES", []).unwrap();
            transaction.query_row(
                "SELECT custom_work_pending(), resource_preparation_work_pending()",
                [], |_| Ok(()),
            ).unwrap();
            assert!(custom_pending(&transaction));
            assert!(resources_pending(&transaction));
            if commit {
                transaction.commit().unwrap();
            } else {
                transaction.rollback().unwrap();
            }
            // Check before returning the loan: clearing belongs to transaction
            // resolution, not writer publication or the next finalizer.
            assert!(!custom_pending(&connection));
            assert!(!resources_pending(&connection));
        }
        assert_eq!(observer.mutations.load(Ordering::Relaxed), 2);
        assert!(observer.faults.lock().unwrap().is_empty());
        let connection = writer.write();
        let rows: i64 = connection.query_row("SELECT count(*) FROM work", [], |row| row.get(0)).unwrap();
        assert_eq!(rows, if commit { 2 } else { 0 });
    }

    #[test]
    fn commit_clears_both_work_hints_with_native_writer_observer() {
        assert_work_clears_with_writer_observer(true);
    }

    #[test]
    fn rollback_clears_both_work_hints_with_native_writer_observer() {
        assert_work_clears_with_writer_observer(false);
    }
}
