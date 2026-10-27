use super::*;

#[tokio::test(flavor = "current_thread")]
async fn slow_record_lock_does_not_block_unrelated_async_work() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(LOCK_NAME);
    let external = crate::flock::FileLock::hold_blocking(
        crate::flock::open(&path, crate::flock::Open::Create).unwrap(),
        crate::flock::Mode::Exclusive,
    ).unwrap();
    let admission = record_lock_admission(root.path()).await.unwrap();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (completed, mut completion) = tokio::sync::oneshot::channel();
    let operation = tokio::task::spawn_blocking(move || {
        let _admission = admission;
        entered.send(()).unwrap();
        let _held = lock_exclusive(&path).unwrap();
        completed.send(()).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
    let request = tokio::spawn(async { 42 });
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), request).await.unwrap().unwrap(), 42);
    assert!(matches!(completion.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
    drop(external);
    tokio::time::timeout(Duration::from_secs(2), operation).await.unwrap().unwrap();
    completion.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_aliases_share_admission_and_reuse_one_descriptor() {
    use std::os::fd::AsRawFd as _;
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let admission = record_lock_admission(&real).await.unwrap();
    let alias_entry = record_lock_entry(&alias.join(LOCK_NAME)).unwrap();
    assert!(Arc::ptr_eq(&admission._entry, &alias_entry));
    assert!(alias_entry.admission.try_lock().is_err());
    let real_lock = real.join(LOCK_NAME);
    let alias_lock = alias.join(LOCK_NAME);
    tokio::task::spawn_blocking(move || {
        let first = lock_exclusive(&real_lock).unwrap();
        let descriptor = first.held.as_ref().unwrap().file().as_raw_fd();
        drop(first);
        let second = lock_exclusive(&alias_lock).unwrap();
        assert_eq!(descriptor, second.held.as_ref().unwrap().file().as_raw_fd());
    }).await.unwrap();
    drop(admission);
    drop(alias_entry);
    let entry = record_lock_entry(&real.join(LOCK_NAME)).unwrap();
    assert_eq!(Arc::strong_count(&entry), 1);
}

#[test]
fn slow_initializer_leaves_cached_admission_responsive_and_keeps_one_race_winner() {
    let root = tempfile::tempdir().unwrap();
    let cached_dir = root.path().join("cached");
    let cached = record_lock_entry(&cached_dir.join(LOCK_NAME)).unwrap();
    let slow_path = root.path().join("slow").join(LOCK_NAME);
    let initializer_path = slow_path.clone();
    let (entered, waiting) = std::sync::mpsc::sync_channel(0);
    let (release, released) = std::sync::mpsc::sync_channel(0);
    let initializer = std::thread::spawn(move || {
        record_lock_entry_open_with(&initializer_path, crate::flock::Open::Create, |path, create| {
            entered.send(()).unwrap();
            released.recv().unwrap();
            crate::flock::open(path, create)
        }).unwrap()
    });
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    let (completed, completion) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let admission = runtime.block_on(record_lock_admission(&cached_dir)).unwrap();
        completed.send(Arc::ptr_eq(&cached, &admission._entry)).unwrap();
    });
    let responsive = completion.recv_timeout(Duration::from_secs(2));
    // Always release before asserting, so a regression cannot strand either worker.
    // A second synchronous initializer wins this key while the first is paused.
    let winner = if responsive.is_ok() {
        Some(record_lock_entry(&slow_path).unwrap())
    } else {
        None
    };
    release.send(()).unwrap();
    let initialized = initializer.join().unwrap();
    worker.join().unwrap();
    assert!(responsive.unwrap(), "cached admission must reuse its entry during a slow open");
    assert!(Arc::ptr_eq(&initialized, &winner.unwrap()), "racing opens must publish one entry");
}
