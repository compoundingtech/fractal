//! Metadata-only census of observed connections. The registration dies with the connection.
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use rusqlite::{Connection, functions::FunctionFlags};
use serde::Serialize;

static OPEN: LazyLock<Mutex<Vec<Weak<PathBuf>>>> = LazyLock::new(Default::default);
static ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Register a connection which does not use the SQL profiler.
pub fn register(connection: &Connection) {
    let Some(path) = connection.path().filter(|path| !path.is_empty()) else { return };
    let path = Arc::new(PathBuf::from(path));
    let weak = Arc::downgrade(&path);
    // SQLite owns the closure, so closing the connection releases this registration. No
    // update/commit/authorizer hook is occupied, and this function is never called by SQL.
    if let Err(error) = connection.create_scalar_function("st_storage_file_lifetime", 0,
        FunctionFlags::SQLITE_UTF8, move |_| { let _keep_alive = &path; Ok(0i64) }) {
        *ERROR.lock().unwrap_or_else(|error| error.into_inner()) = Some(error.to_string());
        return;
    }
    let mut open = OPEN.lock().unwrap_or_else(|error| error.into_inner());
    open.retain(|entry| entry.strong_count() != 0);
    open.push(weak);
}

#[derive(Serialize)]
pub struct FileSize {
    pub file: PathBuf,
    pub bytes: Option<u64>,
    pub error: Option<String>,
}

/// No directory enumeration, database open, SQL, page walk or recurring task. Missing WAL/SHM
/// means that sidecar is absent at sampling time, not an error or an inferred database size.
pub fn snapshot() -> anyhow::Result<Vec<FileSize>> {
    if let Some(error) = ERROR.lock().unwrap_or_else(|error| error.into_inner()).as_ref() {
        anyhow::bail!("SQLite file census registration failed: {error}");
    }
    let paths: BTreeSet<_> = OPEN.lock().unwrap_or_else(|error| error.into_inner())
        .iter().filter_map(Weak::upgrade).collect();
    let mut files = Vec::with_capacity(paths.len() * 3);
    for path in paths {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            let file = PathBuf::from(name);
            let (bytes, error) = match std::fs::metadata(&file) {
                Ok(metadata) => (Some(metadata.len()), None),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
                Err(error) => (None, Some(error.to_string())),
            };
            files.push(FileSize { file, bytes, error });
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_census_files_follow_connection_lifetime_and_deduplicate_readers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.db");
        let writer = Connection::open(&path).unwrap();
        register(&writer);
        let reader = Connection::open(&path).unwrap();
        register(&reader);
        writer.execute_batch("CREATE TABLE test(value INTEGER);").unwrap();
        let files = snapshot().unwrap();
        let sizes: Vec<_> = files.iter().filter(|file| file.file == path).collect();
        assert_eq!(sizes.len(), 1);
        assert_eq!(sizes[0].bytes, Some(std::fs::metadata(&path).unwrap().len()));
        drop(writer);
        assert!(snapshot().unwrap().iter().any(|file| file.file == path));
        drop(reader);
        assert!(!snapshot().unwrap().iter().any(|file| file.file == path));
    }
}
