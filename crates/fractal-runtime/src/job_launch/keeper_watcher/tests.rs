use super::*;
use std::{fs, path::PathBuf};

fn fixture(population: &str) -> (tempfile::TempDir, KeeperScope) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("idle")).unwrap();
    fs::write(root.path().join("idle/cgroup.events"), population).unwrap();
    let scope = KeeperScope { directory: root.path().to_owned(), slice: "fixture.slice".into(), delegated: true };
    (root, scope)
}

#[test]
fn cancellation_wakes_and_joins_a_blocked_watcher() {
    let (_root, scope) = fixture("populated 1\n");
    let mut watcher = KeeperWatcher::start(SystemdKeeper::new(PathBuf::new()), scope).unwrap();
    watcher.stop().unwrap();
    assert!(watcher.thread.is_none());
    watcher.stop().unwrap();
}

#[test]
fn watcher_errors_wake_the_owner_and_are_returned_on_join() {
    let (_root, scope) = fixture("populated unknown\n");
    let mut watcher = KeeperWatcher::start(SystemdKeeper::new(PathBuf::new()), scope).unwrap();
    let mut descriptor = [libc::pollfd {
        fd: watcher.finished.as_raw_fd(), events: libc::POLLIN, revents: 0,
    }];
    poll(&mut descriptor, -1).unwrap();
    let error = watcher.stop().unwrap_err();
    assert!(format!("{error:#}").contains("keeper population is unknown"));
    assert!(watcher.thread.is_none());
}

#[test]
fn dropping_a_watcher_joins_before_releasing_its_descriptors() {
    let (_root, scope) = fixture("populated 1\n");
    let watcher = KeeperWatcher::start(SystemdKeeper::new(PathBuf::new()), scope).unwrap();
    let completion = watcher.finished.try_clone().unwrap();
    drop(watcher);
    let mut descriptor = [libc::pollfd { fd: completion.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
    poll(&mut descriptor, 0).unwrap();
    assert_ne!(descriptor[0].revents & libc::POLLHUP, 0);
}
