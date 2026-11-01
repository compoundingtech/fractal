use super::*;
use crate::store::{Store, runtime::Plain};
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicBool, Ordering};

fn admit(path: &Path) -> Arc<AdmittedStore> {
    AdmittedStore::from_locked_directory(path, File::open(path).unwrap()).unwrap()
}

#[cfg(test)]
fn seed(path: &Path, value: &str) -> Vec<u8> {
    let connection = Connection::open(path.join("claims.sqlite3")).unwrap();
    connection.execute_batch("CREATE TABLE sentinel(value TEXT)").unwrap();
    connection.execute("INSERT INTO sentinel VALUES(?1)", [value]).unwrap();
    drop(connection);
    fs::read(path.join("claims.sqlite3")).unwrap()
}

#[test]
fn sqlite_vfs_opened_filename_for_admitted_directory() {
    let root = tempfile::tempdir().unwrap();
    let admitted = admit(root.path());
    let connection = admitted.open_connection(OpenFlags::default()).unwrap();
    let opened = crate::sqlite::opened_database_path(&connection, admitted.database_path()).unwrap();
    eprintln!("SQLite VFS requested={} opened={}", admitted.database_path().display(), opened.display());
    assert_eq!(fs::canonicalize(&opened).unwrap(), fs::canonicalize(admitted.database_path()).unwrap());
    let store = Store::open_admitted(admitted.clone(), "writer", Arc::new(Plain)).unwrap();
    assert_eq!(store.path, admitted.database_path());
    assert_eq!(store.readers.path, admitted.database_path());
    #[cfg(target_os = "linux")]
    assert!(store.path.starts_with("/proc/self/fd"));
}

#[test]
fn directory_swap_between_check_and_open_refuses_before_prepare_and_preserves_bytes() {
    let root = tempfile::tempdir().unwrap();
    let configured = root.path().join("store");
    let moved = root.path().join("moved");
    let replacement = root.path().join("replacement");
    fs::create_dir(&configured).unwrap();
    fs::create_dir(&replacement).unwrap();
    let original_bytes = seed(&configured, "original");
    let replacement_bytes = seed(&replacement, "replacement");
    let admitted = admit(&configured);
    let swap_from = configured.clone();
    let swap_to = moved.clone();
    let swap_replacement = replacement.clone();
    admitted.set_before_open_hook_for_test(move || {
        fs::rename(&swap_from, &swap_to).unwrap();
        fs::rename(&swap_replacement, &swap_from).unwrap();
    });
    let prepared = AtomicBool::new(false);
    let result = Store::open_with(admitted.database_path(), "writer".into(), Arc::new(Plain), Some(admitted.clone()), true,
        |_| { prepared.store(true, Ordering::Relaxed); });
    assert!(result.is_err());
    assert!(!prepared.load(Ordering::Relaxed));
    assert_eq!(fs::read(moved.join("claims.sqlite3")).unwrap(), original_bytes);
    assert_eq!(fs::read(configured.join("claims.sqlite3")).unwrap(), replacement_bytes);
    for directory in [&moved, &configured] {
        assert!(!directory.join("claims.sqlite3-wal").exists());
        assert!(!directory.join("claims.sqlite3-shm").exists());
    }
}

#[test]
fn directory_removal_between_check_and_open_never_recreates_it() {
    let root = tempfile::tempdir().unwrap();
    let configured = root.path().join("store");
    fs::create_dir(&configured).unwrap();
    let admitted = admit(&configured);
    let remove = configured.clone();
    admitted.set_before_open_hook_for_test(move || fs::remove_dir(remove).unwrap());
    assert!(Store::open_admitted(admitted, "writer", Arc::new(Plain)).is_err());
    assert!(!configured.exists());
}

#[test]
fn database_swap_or_removal_between_check_and_open_preserves_existing_bytes() {
    for replace in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let original = seed(root.path(), "original");
        let replacement = seed(other.path(), "replacement");
        let admitted = admit(root.path());
        let live = root.path().join("claims.sqlite3");
        let saved = root.path().join("saved.sqlite3");
        let incoming = other.path().join("claims.sqlite3");
        let hook_live = live.clone();
        let hook_saved = saved.clone();
        admitted.set_before_open_hook_for_test(move || {
            fs::rename(&hook_live, hook_saved).unwrap();
            if replace { fs::rename(incoming, hook_live).unwrap(); }
        });
        assert!(Store::open_admitted(admitted, "writer", Arc::new(Plain)).is_err());
        assert_eq!(fs::read(saved).unwrap(), original);
        if replace { assert_eq!(fs::read(live).unwrap(), replacement); }
        else { assert!(!live.exists()); }
        assert!(!root.path().join("claims.sqlite3-wal").exists());
        assert!(!root.path().join("claims.sqlite3-shm").exists());
    }
}

#[test]
fn sqlite_trio_symlinks_are_rejected_without_touching_targets() {
    for name in STORE_DATABASE_FILES {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("sentinel");
        fs::write(&target, b"unchanged").unwrap();
        symlink(&target, root.path().join(name)).unwrap();
        assert!(AdmittedStore::from_locked_directory(root.path(), File::open(root.path()).unwrap()).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    }
}

#[test]
fn directory_aliases_are_accepted_but_retargeting_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    let child = real.join("store");
    fs::create_dir_all(&child).unwrap();
    let alias = root.path().join("alias");
    symlink(&real, &alias).unwrap();
    let admitted = admit(&alias.join("store"));
    admitted.open_connection(OpenFlags::default()).unwrap();
    drop(admitted);
    let admitted = admit(&alias);
    admitted.open_connection(OpenFlags::default()).unwrap();
    let replacement = root.path().join("replacement");
    fs::create_dir(&replacement).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(&replacement, &alias).unwrap();
    assert!(admitted.verify_directory().is_err());
    assert!(admitted.open_connection(OpenFlags::default()).is_err());
    assert!(!replacement.join(STORE_DATABASE_FILES[0]).exists());
}

#[test]
fn cloned_description_relocks_idempotently_and_independent_description_is_excluded() {
    let root = tempfile::tempdir().unwrap();
    let directory = File::open(root.path()).unwrap();
    let admitted = AdmittedStore::from_locked_directory(root.path(), directory.try_clone().unwrap()).unwrap();
    let same_description = AdmittedStore::from_locked_directory(root.path(), directory).unwrap();
    assert!(AdmittedStore::from_locked_directory(root.path(), File::open(root.path()).unwrap()).is_err());
    drop(admitted);
    assert!(AdmittedStore::from_locked_directory(root.path(), File::open(root.path()).unwrap()).is_err());
    drop(same_description);
    admit(root.path());
}

#[test]
fn store_retains_the_directory_lock_after_the_admission_owner_drops() {
    let root = tempfile::tempdir().unwrap();
    let admitted = admit(root.path());
    let store = Store::open_admitted(admitted.clone(), "writer", Arc::new(Plain)).unwrap();
    drop(admitted);
    assert!(AdmittedStore::from_locked_directory(root.path(), File::open(root.path()).unwrap()).is_err());
    assert_eq!(store.readers.path, store.path);
    drop(store);
    admit(root.path());
}
