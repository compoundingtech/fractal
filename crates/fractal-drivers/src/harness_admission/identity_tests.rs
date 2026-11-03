use super::*;
use std::cell::Cell;
use std::os::unix::fs::symlink;

thread_local! {
    pub(super) static HASHED_FILES: Cell<usize> = const { Cell::new(0) };
}

fn hashed_files() -> usize {
    HASHED_FILES.with(Cell::get)
}

#[test]
fn resolver_selects_runtime_and_preserves_admission_hash() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("producer");
    let first = directory.path().join("runtime-one");
    let second = directory.path().join("runtime-two");
    fs::write(&binary, "#!/usr/bin/env runtime\nproducer").unwrap();
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();
    let resolve = |name: &str| {
        assert_eq!(name, "runtime");
        Ok(first.clone())
    };
    let identity = executable_identity_with(&binary, &resolve).unwrap();
    let mut hash = Sha256::new();
    hash.update(b"st.installed-producer.executable-runtime-metadata.v2");
    hash_identity_file(&binary, &mut hash).unwrap();
    hash_identity_file(&first, &mut hash).unwrap();
    assert_eq!(identity, format!("{:x}", hash.finalize()));
    assert_ne!(
        identity,
        executable_identity_with(&binary, &|_| Ok(second.clone())).unwrap()
    );
    executable_identity_recorded(&binary, &resolve, || SystemTime::now() + Duration::from_secs(2)).unwrap();
    let before = hashed_files();
    assert_eq!(identity, executable_identity_with(&binary, &resolve).unwrap());
    assert_eq!(before, hashed_files(), "cache hit must not stream files");
}

#[test]
fn dependency_edits_retargets_and_metadata_invalidate_cache() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let binary = root.join("producer");
    let source = root.join("source");
    let replacement = root.join("replacement");
    let runtime = root.join("runtime");
    fs::write(&source, "#!/usr/bin/env runtime\nfirst").unwrap();
    fs::write(&replacement, "#!/usr/bin/env runtime\nother").unwrap();
    fs::write(&runtime, "runtime-one").unwrap();
    symlink(&source, &binary).unwrap();
    let identity = || executable_identity_with(&binary, &|_| Ok(runtime.clone())).unwrap();
    let seed = || {
        let value = executable_identity_recorded(&binary, &|_| Ok(runtime.clone()), || SystemTime::now() + Duration::from_secs(120)).unwrap();
        let before = hashed_files();
        assert_eq!(value, identity());
        assert_eq!(before, hashed_files(), "seed must produce a settled cache hit");
        value
    };
    for (index, (path, content)) in [
        (&source, "#!/usr/bin/env runtime\nother"),
        (&runtime, "runtime-two"),
        (&root.join("package.json"), "{}"),
        (&root.join("package-lock.json"), "one"),
        (&root.join("package-lock.json"), "two"),
    ].into_iter().enumerate() {
        let previous = seed();
        let before = hashed_files();
        fs::write(path, content).unwrap();
        File::open(path).unwrap().set_times(fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(10 + index as u64))).unwrap();
        let current = identity();
        assert_ne!(previous, current);
        assert!(hashed_files() > before);
    }
    let previous = seed();
    fs::remove_file(&binary).unwrap();
    symlink(&replacement, &binary).unwrap();
    let retargeted = identity();
    assert_ne!(previous, retargeted, "resolved path is part of identity");
    seed();
    let before = hashed_files();
    let original = fs::metadata(&replacement).unwrap().permissions().mode();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(original ^ 0o100)).unwrap();
    assert_ne!(retargeted, identity());
    assert!(hashed_files() > before);
    let previous = seed();
    let before = hashed_files();
    // Same bytes, size and mode still change ctime/mtime: recompute, retaining the digest.
    fs::write(&replacement, "#!/usr/bin/env runtime\nother").unwrap();
    File::open(&replacement).unwrap().set_times(fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(30))).unwrap();
    assert_eq!(previous, identity());
    assert!(hashed_files() > before);
}

#[test]
fn same_tick_same_size_writes_are_never_cached_as_settled() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("producer");
    let tick = SystemTime::now() + Duration::from_secs(60);
    let write = |contents| {
        fs::write(&binary, contents).unwrap();
        File::open(&binary).unwrap().set_times(fs::FileTimes::new().set_modified(tick)).unwrap();
    };
    write("first");
    let before = hashed_files();
    let first = executable_identity_recorded(&binary, &|_| unreachable!(), || {
        assert_eq!(hashed_files(), before, "cutoff must precede content reads");
        SystemTime::now()
    }).unwrap();
    let before = hashed_files();
    write("other");
    assert_eq!(fs::metadata(&binary).unwrap().modified().unwrap(), tick);
    assert_ne!(first, executable_identity(&binary).unwrap());
    assert!(hashed_files() > before);
    assert!(!IDENTITY_CACHE.lock().unwrap().iter().any(|e| e.dependencies.iter().any(|d| d.path == binary)));
}
