//! `fractal store restore-blob-bytes`: put every moved blob byte back into the store, so the
//! previous release, which reads the `blobs` table alone, can open it again.
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Copy every byte back from the store's blob file, then lower its storage version to what it was
/// before the first move. The daemon must be stopped: the store's directory lock is taken.
pub fn restore(store: &Path, output: &mut impl Write) -> Result<()> {
    let store = store.canonicalize().context("resolve explicit existing store path")?;
    let parent = store.parent().context("store has no parent directory")?;
    let _directory_lock = crate::node_identity::lock_directory(parent)
        .context("stop the daemon before restoring blob bytes")?;
    let connection = Connection::open_with_flags(&store, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    smallclaims::store::blob_file::attach_writer(&connection, &store)?;
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
    let (mut rows, mut bytes) = (0, 0);
    loop {
        let page = smallclaims::store::blob_file::restore_page(&connection, now_ms)?;
        rows += page.rows;
        bytes += page.bytes;
        if !page.more {
            break;
        }
    }
    let version: u32 = connection.query_row("PRAGMA main.user_version", [], |row| row.get(0))?;
    let (busy, _, _): (i64, i64, i64) =
        connection.query_row("PRAGMA main.wal_checkpoint(TRUNCATE)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    anyhow::ensure!(busy == 0, "the store's checkpoint is busy; keep the daemon stopped");
    writeln!(
        output,
        "restored {rows} blobs ({bytes} bytes) into the store; its storage version is now {version}, which the previous release opens"
    )?;
    Ok(())
}
