//! Commits that redo themselves after a crash, made without waiting on the disk.
use super::*;

/// Run `work`, whose commits redo themselves after a crash (giving free pages back, retiring
/// closed mail, moving blob bytes), without waiting on the disk at commit, then restore the connection's settings.
/// On a busy host the waits took most of a 100 ms hold:
/// - synchronous=NORMAL skips the WAL fsync. In WAL mode it is corruption-safe; the writer's
///   next FULL commit fsyncs the WAL and these frames with it, and a crash before then loses
///   only work the next pass does again.
/// - wal_autocheckpoint=0 leaves copying the WAL into the database (and its fsync) to the
///   daemon's checkpoint thread, which runs off the writer queue every few seconds.
///
/// Neither can change inside a transaction, so `work` begins and commits its own. Both are put
/// back before this returns, on success, on an error and on a panic alike, so the writer's next
/// ordinary commit runs at the member's own level.
pub fn committing_without_fsync<T>(connection: &Connection, work: impl FnOnce() -> Result<T>) -> Result<T> {
    let mut restore = RestoreDurability {
        connection,
        level: connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?,
        automatic: connection.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))?,
        armed: true,
    };
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "wal_autocheckpoint", 0)?;
    let done = work();
    // An error restoring wins over the work's result: the writer must not stay at NORMAL.
    restore.now()?;
    done
}

/// Puts a connection's synchronous level and checkpoint threshold back.
struct RestoreDurability<'a> {
    connection: &'a Connection,
    level: i64,
    automatic: i64,
    armed: bool,
}

impl RestoreDurability<'_> {
    fn now(&mut self) -> Result<()> {
        self.armed = false;
        self.connection.pragma_update(None, "wal_autocheckpoint", self.automatic)?;
        self.connection.pragma_update(None, "synchronous", self.level)?;
        Ok(())
    }
}

impl Drop for RestoreDurability<'_> {
    // Only on an early return or a panic; the ordinary path restores with `now`.
    fn drop(&mut self) {
        if self.armed {
            let _ = self.now();
        }
    }
}

