//! Blob bytes in their own file, `<store>.blobs.<ext>` beside the store.
//!
//! The `blobs` table keeps every row: its hash and size are what claims, documents and pages
//! join on, and what `documents.hash` references. Only the bytes move. Once a member has run a
//! build for `[store] blob_move_after` (three days unless set), a paced pass copies each row's
//! bytes into the blob file, commits that durably, and only then empties the row's own bytes.
//! A reader takes a row's bytes from the row, or from the blob file once they moved
//! ([`bytes!`], [`Store::blob_bytes`]). A blob of zero bytes keeps its own.
//!
//! The blob file is attached to every connection to the store as `blob_file`. It holds only
//! copies of the store's rows: the live store deletes a `blobs` row only in the one-time staging
//! of uploads (`separate_staged_blobs`), which runs at a store's first open with that build, long
//! before any byte can move, and the deletes a checkpoint proof makes run on a copy of the store
//! with a copy of its blob file ([`copy_beside`]). The bytes of a blob are also in the envelope that brought it, so moving them
//! changes nothing a peer sees; blobs left the projection digest with this change, since an
//! emptied row would read differently.
//!
//! A copy of the store needs its blob file beside it: a copy made without one reads every moved
//! blob as missing.
//!
//! An older build reads the `blobs` table alone and would serve an emptied row as zero bytes.
//! So the first emptying moves the store to [`STORAGE_VERSION`], which an older build refuses to
//! open; [`Store::restore_blob_bytes_step`] copies every byte back and lowers it again.
use super::*;
use std::time::{Duration, Instant};

/// The schema name the blob file is attached under on every connection to a store.
pub const SCHEMA: &str = "blob_file";

/// The storage version of a store some of whose blob bytes live only in its blob file.
pub const STORAGE_VERSION: u32 = 25;

/// `meta`: when this member first ran a build that moves blob bytes (unix ms).
pub const SINCE: &str = "blob_file_since";
/// `meta`: the last `blobs` rowid whose bytes the pass has dealt with.
pub const CURSOR: &str = "blob_file_moved_through";
/// `meta`: the storage version the store had before the first emptying raised it.
pub const FENCED_FROM: &str = "blob_file_fenced_from";
/// The blob file's own `meta`: the last `blobs` rowid whose bytes the blob file holds. It lives
/// in the blob file, so it commits with the copies it counts.
pub const COPIED: &str = "copied_through";

/// Rows one step considers.
const PAGE: i64 = 64;

const BLOB_FILE_TABLE: &str = /* sqlite-large-value: allow-for-now lv-hca561fb42720 owner=smallclaims::store::blob_file TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS blob_file.blobs (
    hash TEXT PRIMARY KEY, bytes BLOB NOT NULL, size INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS blob_file.meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)";

/// The bytes of the `blobs` row aliased `$alias`: its own, or the blob file's once they moved.
#[macro_export]
macro_rules! blob_bytes {
    ($alias:literal) => {
        concat!(
            "CASE WHEN ", $alias, ".size>0 AND length(", $alias, ".bytes)=0 THEN (SELECT moved.bytes FROM blob_file.blobs AS moved WHERE moved.hash=",
            $alias, ".hash) ELSE ", $alias, ".bytes END"
        )
    };
}
pub use crate::blob_bytes as bytes;

/// Where a store's blob file lives: a file beside it, or a shared in-memory database beside a
/// shared in-memory store, or a private one beside a private one.
pub fn location(database: &Path) -> String {
    let text = database.to_string_lossy();
    if let Some((name, query)) = text.strip_prefix("file:").and_then(|rest| rest.split_once('?')) {
        return format!("file:{name}-blobs?{query}");
    }
    if text.is_empty() || text == ":memory:" {
        return ":memory:".to_owned();
    }
    let stem = database.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match database.extension() {
        Some(extension) => format!("{stem}.blobs.{}", extension.to_string_lossy()),
        None => format!("{stem}.blobs"),
    };
    database.with_file_name(name).to_string_lossy().into_owned()
}

/// Attach the blob file to the store's writer, creating it with incremental auto-vacuum.
pub fn attach_writer(connection: &Connection, database: &Path) -> Result<()> {
    connection.execute(/* sqlite-large-value: allow-for-now lv-h2e95b3b64cc3 owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ "ATTACH DATABASE ?1 AS blob_file", [location(database)])?;
    // Only an empty file takes a vacuum mode, so set it before the first table.
    let fresh: bool =
        connection.query_row("SELECT NOT EXISTS(SELECT 1 FROM blob_file.sqlite_master)", [], |row| row.get(0))?;
    if fresh {
        connection.execute_batch("PRAGMA blob_file.auto_vacuum = INCREMENTAL")?;
    }
    connection.query_row("PRAGMA blob_file.journal_mode = WAL", [], |_| Ok(()))?;
    connection.pragma_update(Some(rusqlite::DatabaseName::Attached("blob_file")),
        "journal_size_limit", super::WAL_JOURNAL_SIZE_LIMIT)?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hed8f4c129a6a owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ &format!("PRAGMA blob_file.synchronous = FULL; {BLOB_FILE_TABLE};"))?;
    Ok(())
}

/// Attach the blob file to a reader. A store whose writer has not yet run this build has no blob
/// file, and none of its bytes moved: an empty stand-in answers the same.
pub fn attach_reader(connection: &Connection, database: &Path) -> Result<()> {
    let location = location(database);
    let present = location == ":memory:" || location.starts_with("file:") || Path::new(&location).exists();
    if present {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h86d6852c28d8 owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ "ATTACH DATABASE ?1 AS blob_file", [&location])?;
        if location != ":memory:" {
            return Ok(());
        }
    } else {
        connection.execute(/* sqlite-large-value: allow-for-now lv-hb2d3b2648e1d owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ "ATTACH DATABASE ':memory:' AS blob_file", [])?;
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hedea69cc751c owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ BLOB_FILE_TABLE)?;
    Ok(())
}

/// Copy the blob file of the store at `database` beside `copy`, a copy of the store just made,
/// so the copy reads every byte the store does. The blob file gains a row's bytes before the
/// store lets the row's own go, so a blob file copied after the store covers every row the store
/// copy holds empty.
pub fn copy_beside(database: &Path, shared_memory: bool, copy: &Path) -> Result<()> {
    let target = location(copy);
    match std::fs::remove_file(&target) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    // `VACUUM INTO` needs a connection that may write the copy; readers are query-only.
    let flags = if shared_memory {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let connection = Connection::open_with_flags(database, flags)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    attach_reader(&connection, database)?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-h1145b31da060 owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ "VACUUM blob_file INTO ?1", [&target])?;
    Ok(())
}

/// What one step of moving or restoring did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BlobBytes {
    /// Rows whose bytes moved (or came back).
    pub rows: usize,
    /// Rows whose bytes this step copied into the blob file, still also held by the row.
    pub copied: usize,
    /// Their bytes.
    pub bytes: u64,
    /// The longest of the step's writer holds.
    pub hold: Duration,
    /// Whether more rows remain.
    pub more: bool,
    /// Whether the pass is still inside the window after this build first ran.
    pub waiting: bool,
}

impl Store {
    /// The bytes of the blob `hash`, wherever they live: the graph, the blob file, or an upload
    /// no claim names yet.
    pub fn blob_bytes(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let connection = self.readers.get();
        connection
            .query_row(
                concat!(
                    "SELECT ", blob_bytes!("b"), " FROM blobs AS b WHERE b.hash=?1
                     UNION ALL SELECT bytes FROM local_blobs WHERE hash=?1 LIMIT 1"
                ),
                [hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Move blob bytes into the blob file, one bounded writer hold a step, once `move_after_ms`
    /// has passed since this member first ran a build that moves them. A step either copies the
    /// next rows' bytes into the blob file, or, once a checkpoint of the blob file (run off the
    /// writer) has made those copies durable, empties the rows' own bytes, raising the storage
    /// version on the first. Neither hold waits on the disk at commit. Call again while `more`.
    pub fn move_blob_bytes_step(&self, now_ms: u64, move_after_ms: u64, budget: Duration) -> Result<BlobBytes> {
        let since = self.blob_file_since(now_ms)?;
        if now_ms < since.saturating_add(move_after_ms) {
            return Ok(BlobBytes { waiting: true, ..BlobBytes::default() });
        }
        let (emptied, copied): (i64, i64) = self.readers.get().query_row(
            "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM main.meta WHERE key=?1), 0),
                    COALESCE((SELECT CAST(value AS INTEGER) FROM blob_file.meta WHERE key=?2), 0)",
            params![CURSOR, COPIED],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut done = BlobBytes { more: true, ..BlobBytes::default() };
        if copied > emptied {
            // A row lets go of its bytes only once their copy is on disk.
            if !self.blob_file_durable()? {
                return Ok(done);
            }
            let page: Vec<(i64, i64)> = self
                .readers
                .get()
                .prepare_cached("SELECT rowid, size FROM blobs WHERE rowid>?1 AND rowid<=?2 ORDER BY rowid LIMIT ?3")?
                .query_map(params![emptied, copied, PAGE], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let connection = self.connection.write_background();
            let started = Instant::now();
            super::durability::committing_without_fsync(&connection, || {
                let transaction = connection.unchecked_transaction()?;
                let mut through = page.last().map_or(copied, |(rowid, _)| *rowid);
                {
                    let mut emptying = transaction.prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-h9494108a8003 owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "UPDATE main.blobs SET bytes=x'' WHERE rowid=?1 AND size>0 AND length(bytes)>0
                         AND EXISTS(SELECT 1 FROM blob_file.blobs AS moved
                                    WHERE moved.hash=main.blobs.hash AND length(moved.bytes)=main.blobs.size)",
                    )?;
                    for (n, &(rowid, size)) in page.iter().enumerate() {
                        if n > 0 && started.elapsed() >= budget {
                            through = page[n - 1].0;
                            break;
                        }
                        if emptying.execute(/* sqlite-large-value: allow-for-now lv-h11e3ad89e964 owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ [rowid])? > 0 {
                            done.rows += 1;
                            done.bytes += u64::try_from(size).unwrap_or(0);
                        }
                    }
                }
                if done.rows > 0 {
                    let version: u32 = transaction.query_row("PRAGMA main.user_version", [], |row| row.get(0))?;
                    if version < STORAGE_VERSION {
                        transaction.execute(
                            /* sqlite-large-value: allow-for-now lv-h05943313efeb owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO main.meta(key, value) VALUES (?1, ?2)",
                            params![FENCED_FROM, version.to_string()],
                        )?;
                        transaction.pragma_update(Some(rusqlite::DatabaseName::Main), "user_version", STORAGE_VERSION)?;
                    }
                }
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-ha531b7e4f69f owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO main.meta(key, value) VALUES (?1, ?2)",
                    params![CURSOR, through.to_string()],
                )?;
                transaction.commit()?;
                Ok(())
            })?;
            done.hold = started.elapsed();
            return Ok(done);
        }
        // Copy the next rows. Rows already empty or of zero bytes copy nothing.
        let page: Vec<i64> = self
            .readers
            .get()
            .prepare_cached("SELECT rowid FROM blobs WHERE rowid>?1 ORDER BY rowid LIMIT ?2")?
            .query_map(params![copied, PAGE], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if page.is_empty() {
            done.more = false;
            return Ok(done);
        }
        let connection = self.connection.write_background();
        let started = Instant::now();
        super::durability::committing_without_fsync(&connection, || {
            let transaction = connection.unchecked_transaction()?;
            let mut through = *page.last().unwrap_or(&copied);
            {
                let mut copying = transaction.prepare_cached(
                    /* sqlite-large-value: allow-for-now lv-hc916442f4a28 owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO blob_file.blobs(hash, bytes, size)
                     SELECT hash, bytes, size FROM main.blobs WHERE rowid=?1 AND size>0 AND length(bytes)>0",
                )?;
                for (n, &rowid) in page.iter().enumerate() {
                    if n > 0 && started.elapsed() >= budget {
                        through = page[n - 1];
                        break;
                    }
                    done.copied += copying.execute(/* sqlite-large-value: allow-for-now lv-h4abc007bf491 owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ [rowid])?;
                }
            }
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h4efe9877a623 owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO blob_file.meta(key, value) VALUES (?1, ?2)",
                params![COPIED, through.to_string()],
            )?;
            transaction.commit()?;
            Ok(())
        })?;
        done.hold = started.elapsed();
        Ok(done)
    }

    /// Whether everything committed to the blob file is on disk: a PASSIVE checkpoint, from a
    /// connection of its own so the writer is not held, copied every frame of its WAL into the
    /// file and synced it. A private in-memory store's blob file has no disk to wait on.
    fn blob_file_durable(&self) -> Result<bool> {
        let location = location(&self.path);
        if location == ":memory:" || location.starts_with("file:") {
            return Ok(true);
        }
        let connection = Connection::open_with_flags(&location, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(Duration::ZERO)?;
        let (busy, frames, backfilled): (i64, i64, i64) =
            connection.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(busy == 0 && frames == backfilled)
    }

    /// Copy one bounded page of moved bytes back into the store ([`restore_page`]). Call again
    /// while `more`.
    pub fn restore_blob_bytes_step(&self, now_ms: u64) -> Result<BlobBytes> {
        restore_page(&self.connection.write(), now_ms)
    }

    /// When this member first ran a build that moves blob bytes, recording `now_ms` the first
    /// time it is asked.
    fn blob_file_since(&self, now_ms: u64) -> Result<u64> {
        let read = || -> Result<Option<u64>> {
            Ok(self
                .readers
                .get()
                .query_row("SELECT value FROM meta WHERE key=?1", [SINCE], |row| row.get::<_, String>(0))
                .optional()?
                .and_then(|value| value.parse().ok()))
        };
        if let Some(since) = read()? {
            return Ok(since);
        }
        self.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-hfffc65b29581 owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key, value) VALUES (?1, ?2)",
            params![SINCE, now_ms.to_string()],
        )?;
        Ok(read()?.unwrap_or(now_ms))
    }
}

/// Copy one bounded page of moved bytes back into the store on `connection`, its writer with the
/// blob file attached, for a rollback to a build that reads the `blobs` table alone. When none
/// remain, lower the storage version to what it was before the first emptying, and start the
/// window again so the pass waits before moving bytes anew. Call again while `more`.
pub fn restore_page(connection: &Connection, now_ms: u64) -> Result<BlobBytes> {
    let page: Vec<(i64, i64)> = connection
        .prepare_cached("SELECT rowid, size FROM main.blobs WHERE size>0 AND length(bytes)=0 LIMIT ?1")?
        .query_map([PAGE], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let started = Instant::now();
    let transaction = connection.unchecked_transaction()?;
    let mut done = BlobBytes::default();
    if page.is_empty() {
        let version: u32 = transaction.query_row("PRAGMA main.user_version", [], |row| row.get(0))?;
        if version == STORAGE_VERSION {
            let before: u32 = transaction
                .query_row("SELECT value FROM meta WHERE key=?1", [FENCED_FROM], |row| row.get::<_, String>(0))
                .optional()?
                .and_then(|value| value.parse().ok())
                .unwrap_or(CURRENT_SCHEMA_VERSION);
            transaction.pragma_update(Some(rusqlite::DatabaseName::Main), "user_version", before)?;
        }
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h4cd859f27b01 owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key IN (?1, ?2)", params![FENCED_FROM, CURSOR])?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h3b943e990626 owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
            params![SINCE, now_ms.to_string()],
        )?;
    } else {
        let mut restoring = transaction.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hbe4d59b219de owner=smallclaims::store::blob_file TODO=bound-or-reference-values-and-reclamation */ "UPDATE main.blobs SET bytes=(SELECT moved.bytes FROM blob_file.blobs AS moved WHERE moved.hash=main.blobs.hash)
             WHERE rowid=?1 AND EXISTS(SELECT 1 FROM blob_file.blobs AS moved
                                       WHERE moved.hash=main.blobs.hash AND length(moved.bytes)=main.blobs.size)",
        )?;
        for &(rowid, size) in &page {
            anyhow::ensure!(
                restoring.execute(/* sqlite-large-value: allow-for-now lv-h6fc82c88c317 owner=smallclaims::store::blob_file TODO=trace-statement-and-value-representation */ [rowid])? == 1,
                "blob row {rowid} has no bytes here or in the blob file; the store can't be restored"
            );
            done.rows += 1;
            done.bytes += u64::try_from(size).unwrap_or(0);
        }
        done.more = true;
    }
    transaction.commit()?;
    done.hold = started.elapsed();
    Ok(done)
}
