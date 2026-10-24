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
