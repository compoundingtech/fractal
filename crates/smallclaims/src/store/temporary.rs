//! Scratch for test stores, following fractal-secrets' memory_tempdir selection rule.

use std::path::Path;

pub(super) fn directory() -> std::io::Result<tempfile::TempDir> {
    let shm = Path::new("/dev/shm");
    let executable = !std::fs::read_to_string("/proc/mounts").is_ok_and(|mounts| {
        mounts.lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.nth(1).is_some_and(|point| Path::new(point) == shm)
                && fields
                    .nth(1)
                    .is_some_and(|options| options.split(',').any(|option| option == "noexec"))
        })
    });
    if shm.is_dir() && executable && available_bytes(shm).is_some_and(|bytes| bytes >= 2 << 30) {
        tempfile::Builder::new()
            .prefix("graph-test-")
            .tempdir_in(shm)
    } else {
        tempfile::Builder::new().prefix("graph-test-").tempdir()
    }
}

fn available_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt as _;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL terminated and stat is valid for the statvfs write.
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: a successful statvfs initialized stat.
    let stat = unsafe { stat.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(test)]
mod tests {
    use super::super::{Store, runtime::Plain};
    use std::sync::Arc;

    #[test]
    fn temporary_store_directory_lives_until_the_last_reader_closes() {
        let (path, pool) = {
            let store = Store::open_memory("scratch-lifetime", Arc::new(Plain)).unwrap();
            assert!(!store.shared_memory);
            (store.path.clone(), Arc::new(store.readers))
        };
        let directory = path.parent().unwrap();
        let retained_pool = pool.clone();
        drop(pool);
        // Keep no connection: the pool must retain scratch for a later lazy open.
        retained_pool.idle.lock().unwrap().clear();
        assert!(path.exists(), "a pool owner keeps lazy readers usable");
        let mode: String = retained_pool
            .get()
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        // Connections retained through the public idle pool can outlive the pool itself.
        let retained_connections = retained_pool.idle.clone();
        drop(retained_pool);
        assert!(
            path.exists(),
            "a retained SQLite connection keeps the directory"
        );
        drop(retained_connections);
        assert!(
            !directory.exists(),
            "the last connection releases all scratch files"
        );
    }

    #[test]
    fn dropping_a_temporary_store_removes_its_directory() {
        let store = Store::open_memory("scratch-drop", Arc::new(Plain)).unwrap();
        let directory = store.path.parent().unwrap().to_path_buf();
        assert!(directory.exists());
        drop(store);
        assert!(!directory.exists());
    }
}
