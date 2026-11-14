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
    // may restore queue rows that a preceding flush removed.
    let committed = pending.clone();
    connection.commit_hook(Some(move || { committed.store(0, Ordering::Relaxed); false }));
    connection.rollback_hook(Some(move || pending.store(0, Ordering::Relaxed)));
    Ok(())
}

fn pending(connection: &Connection, bit: u8) -> bool {
    // Bare replay/proof connections without capture conservatively flush normally.
    state(connection).is_none_or(|pending| pending.load(Ordering::Relaxed) & bit != 0)
}
pub(super) fn resources_pending(connection: &Connection) -> bool { pending(connection, RESOURCES) }
pub(super) fn custom_pending(connection: &Connection) -> bool { pending(connection, CUSTOM) }
