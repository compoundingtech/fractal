//! Scratch for test stores, inherited from a tmpfs run or selected by the memory_tempdir rule.

use std::path::Path;

pub(super) fn directory() -> std::io::Result<tempfile::TempDir> {
    let temporary = std::env::temp_dir();
    // The runner owns this root, including files left by a process that exits
    // without dropping its store. Keep an already-memory-backed run together.
    if on_tmpfs(&temporary) {
        return tempfile::Builder::new()
            .prefix("graph-test-")
            .tempdir_in(temporary);
    }
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

#[cfg(target_os = "linux")]
fn on_tmpfs(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: path is NUL terminated and stat is valid for the statfs write.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: a successful statfs initialized stat.
    unsafe { stat.assume_init() }.f_type == libc::TMPFS_MAGIC
}

#[cfg(not(target_os = "linux"))]
fn on_tmpfs(_: &Path) -> bool {
    false
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

    #[cfg(target_os = "linux")]
    #[test]
    fn a_tmpfs_run_reclaims_its_stores_after_a_process_exits() {
        const CHILD: &str = "SMALLCLAIMS_TMPDIR_EXIT_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let run = std::fs::canonicalize(std::env::temp_dir()).unwrap();
            let store = Store::open_memory("scratch-process-exit", Arc::new(Plain)).unwrap();
            assert!(
                store.path.starts_with(&run),
                "store must be owned by the run: {}",
                store.path.display()
            );
            std::fs::write(run.join("store-path"), store.path.to_str().unwrap()).unwrap();
            // Model a timeout or process exit that never runs the store's destructors.
            std::process::exit(0);
        }
        let temporary = std::env::temp_dir();
        let base = if super::on_tmpfs(&temporary) {
            temporary
        } else {
            std::path::PathBuf::from("/dev/shm")
        };
        let run = tempfile::Builder::new()
            .prefix("graph-run-")
            .tempdir_in(base)
            .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::temporary::tests::a_tmpfs_run_reclaims_its_stores_after_a_process_exits",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("TMPDIR", run.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = std::path::PathBuf::from(
            std::fs::read_to_string(run.path().join("store-path")).unwrap(),
        );
        assert!(
            path.exists(),
            "the child left its store without dropping it"
        );
        run.close().unwrap();
        assert!(
            !path.exists(),
            "run cleanup reclaims the exited process's store"
        );
    }

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
