//! Directory admission shared by the identity layer, writer and reader connections.
//!
//! Linux opens through the retained directory descriptor. SQLite's Unix VFS may resolve
//! that spelling to a pathname; the immediate post-open identity check is therefore still
//! required. macOS uses the checked pathname: it has no proc-dirfd equivalent here, and
//! retains a pathname race after verification (including later SQLite sidecar opens).

use std::fs::{self, File};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use parking_lot::Mutex;

use anyhow::{Context as _, Result, ensure};
use rusqlite::{Connection, OpenFlags};

/// The database and the two SQLite-managed sidecars admitted as one store.
pub const STORE_DATABASE_FILES: [&str; 3] =
    ["claims.sqlite3", "claims.sqlite3-wal", "claims.sqlite3-shm"];

// NUL-terminated names derived at compile time, without allocating during fstatat.
const ENTRY_NAMES: [[u8; 32]; 3] = {
    let mut names = [[0; 32]; 3];
    let mut entry = 0;
    while entry < STORE_DATABASE_FILES.len() {
        let source = STORE_DATABASE_FILES[entry].as_bytes();
        let mut byte = 0;
        while byte < source.len() {
            names[entry][byte] = source[byte];
            byte += 1;
        }
        entry += 1;
    }
    names
};

/// A directory descriptor and its exclusive nonblocking advisory lock, retained for
/// every connection using this store. Directory aliases (including ancestor symlinks)
/// are permitted; the SQLite database/WAL/SHM entries must not be symlinks.
///
/// Linux retains `/proc/self/fd/<dirfd>` for reader/reopen/sidecar paths, even if
/// SQLite's VFS resolves its own filename. macOS uses a checked pathname: identity
/// checks run before and immediately after open, but cannot eliminate concurrent
/// pathname changes after those checks or during SQLite's later sidecar opens.
pub struct AdmittedStore {
    configured: PathBuf,
    directory: File,
    identity: (u64, u64),
    database: PathBuf,
    #[cfg(any(test, feature = "test-support"))]
    before_open: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl std::fmt::Debug for AdmittedStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AdmittedStore")
            .field("configured", &self.configured)
            .field("directory", &self.directory)
            .field("identity", &self.identity)
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

impl AdmittedStore {
    /// Admit an already-open directory. Reapplying flock on a cloned open file
    /// description is idempotent; no directory or lock file is created here.
    pub fn from_locked_directory(path: &Path, directory: File) -> Result<Arc<Self>> {
        let metadata = directory.metadata().context("stat admitted store directory")?;
        ensure!(metadata.is_dir(), "admitted store descriptor is not a directory");
        // SAFETY: directory remains open throughout this call. flock changes only the
        // advisory lock on this open file description, shared with any cloned handle.
        ensure!(unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "lock admitted store directory: {}", std::io::Error::last_os_error());
        let configured = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
        #[cfg(target_os = "linux")]
        let database = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(STORE_DATABASE_FILES[0]);
        #[cfg(not(target_os = "linux"))]
        let database = configured.join(STORE_DATABASE_FILES[0]);
        let admitted = Arc::new(Self {
            configured,
            directory,
            identity: (metadata.dev(), metadata.ino()),
            database,
            #[cfg(any(test, feature = "test-support"))]
            before_open: Mutex::new(None),
        });
        admitted.verify_directory()?;
        admitted.verify_entries()?;
        Ok(admitted)
    }

    pub fn verify_directory(&self) -> Result<()> {
        let current = fs::metadata(&self.configured)
            .with_context(|| format!("stat configured store directory {}", self.configured.display()))?;
        ensure!(current.is_dir() && (current.dev(), current.ino()) == self.identity,
            "configured store directory was replaced or its alias retargeted: {}", self.configured.display());
        Ok(())
    }

    pub fn database_path(&self) -> &Path { &self.database }

    fn entry_stat(&self, entry: usize) -> Result<Option<libc::stat>> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the descriptor and NUL-terminated name live through the call;
        // fstatat initializes stat on success. NOFOLLOW examines the entry itself.
        let result = unsafe { libc::fstatat(self.directory.as_raw_fd(), ENTRY_NAMES[entry].as_ptr().cast(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
        if result == 0 {
            // SAFETY: successful fstatat initialized the entire stat value.
            Ok(Some(unsafe { stat.assume_init() }))
        } else {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound { Ok(None) } else { Err(error.into()) }
        }
    }

    fn verify_entries(&self) -> Result<()> {
        for (entry, name) in STORE_DATABASE_FILES.iter().enumerate() {
            if let Some(stat) = self.entry_stat(entry)? {
                ensure!(stat.st_mode & libc::S_IFMT == libc::S_IFREG,
                    "store SQLite entry must be a regular file, not a symlink: {name}");
            }
        }
        Ok(())
    }

    /// Validate immediately before opening and immediately after SQLite returns,
    /// before any caller can configure pragmas, install schema or run migrations.
    pub fn open_connection(&self, flags: OpenFlags) -> Result<Connection> {
        self.open_connection_with_vfs(flags, None)
    }

    pub(crate) fn open_connection_with_vfs(&self, flags: OpenFlags, vfs: Option<&str>) -> Result<Connection> {
        self.verify_directory()?;
        self.verify_entries()?;
        let before = self.entry_stat(0)?;
        // An existing database disappearing must not be silently recreated by SQLite.
        let flags = if before.is_some() { flags & !OpenFlags::SQLITE_OPEN_CREATE } else { flags };
        #[cfg(any(test, feature = "test-support"))]
        {
            let hook = self.before_open.lock().take();
            if let Some(hook) = hook { hook(); }
        }
        let connection = match vfs {
            Some(vfs) => Connection::open_with_flags_and_vfs(&self.database, flags, vfs),
            None => Connection::open_with_flags(&self.database, flags),
        }.with_context(|| format!("open admitted database {}", self.database.display()))?;
        self.verify_directory()?;
        self.verify_entries()?;
        let entry = self.entry_stat(0)?.context("admitted database disappeared during open")?;
        if let Some(before) = before {
            ensure!(before.st_dev == entry.st_dev && before.st_ino == entry.st_ino,
                "admitted database was replaced during open");
        }
        let opened_path = crate::sqlite::opened_database_path(&connection, &self.database)?;
        let opened = fs::metadata(&opened_path).context("stat SQLite-opened database filename")?;
        #[cfg(target_os = "macos")]
        #[allow(clippy::cast_sign_loss)] // Match std's MetadataExt::dev representation.
        let entry_device = entry.st_dev as u64;
        #[cfg(not(target_os = "macos"))]
        let entry_device = entry.st_dev;
        ensure!(entry_device == opened.dev() && entry.st_ino == opened.ino(),
            "SQLite-opened database does not belong to the admitted directory");
        Ok(connection)
    }

    /// One-shot deterministic race seam, also usable through Store::open_admitted.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_before_open_hook_for_test(&self, hook: impl FnOnce() + Send + 'static) {
        *self.before_open.lock() = Some(Box::new(hook));
    }
}

#[cfg(test)]
mod tests;
