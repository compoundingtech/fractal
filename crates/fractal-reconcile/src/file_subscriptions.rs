use anyhow::Result;
use notify::{EventKind, Watcher as _, event::RemoveKind};
use std::{
    collections::BTreeMap,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};
type Callback = Arc<dyn Fn(notify::Event) + Send + Sync>;
type Paths = Vec<(PathBuf, bool)>;
type Roots = BTreeMap<PathBuf, bool>;
type Users = Arc<Mutex<BTreeMap<u64, (Paths, Callback)>>>;
type Stamp = (u64, u64);
pub(super) struct Pool(Mutex<State>);
struct State {
    watcher: notify::RecommendedWatcher,
    users: Users,
    invalid: Arc<AtomicBool>,
    physical: BTreeMap<PathBuf, (bool, Stamp)>,
    next: u64,
}
pub(super) struct Lease {
    pool: Arc<Pool>,
    id: u64,
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
fn callbacks(users: &Users) -> Vec<Callback> {
    lock(users).values().map(|(_, cb)| cb.clone()).collect()
}
fn stamp(path: &Path) -> std::io::Result<Stamp> {
    std::fs::metadata(path).map(|m| (m.dev(), m.ino()))
}
impl Pool {
    pub(super) fn new() -> Result<Arc<Self>> {
        let users = Users::default();
        let dispatch = users.clone();
        let invalid = Arc::new(AtomicBool::new(false));
        let lost = invalid.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let event = event.unwrap_or_else(|_| {
                notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)
            });
            if event.need_rescan() || matches!(event.kind, EventKind::Remove(RemoveKind::Folder)) {
                lost.store(true, Ordering::Release);
            }
            if !event.need_rescan() && matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            for callback in callbacks(&dispatch) {
                callback(event.clone());
            }
        })?;
        Ok(Arc::new(Self(Mutex::new(State {
            watcher,
            users,
            invalid,
            physical: BTreeMap::new(),
            next: 0,
        }))))
    }
    pub(super) fn subscribe(self: &Arc<Self>, paths: Paths, callback: Callback) -> Result<Lease> {
        let paths = paths
            .into_iter()
            .map(|(path, recursive)| Ok((path.canonicalize()?, recursive)))
            .collect::<Result<_>>()?;
        let mut state = lock(&self.0);
        let id = state.next;
        state.next += 1;
        lock(&state.users).insert(id, (paths, callback));
        if let Err(error) = state.rebuild(stamp, Some(id)) {
            state.remove(id);
            return Err(error);
        }
        Ok(Lease {
            pool: self.clone(),
            id,
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        lock(&self.pool.0).remove(self.id);
    }
}
impl State {
    fn remove(&mut self, id: u64) {
        lock(&self.users).remove(&id);
        let _ = self.rebuild(stamp, None);
    }
    fn rebuild(
        &mut self,
        post: fn(&Path) -> std::io::Result<Stamp>,
        acquiring: Option<u64>,
    ) -> Result<()> {
        let desired = roots(&self.users);
        let invalid = self.invalid.swap(false, Ordering::AcqRel);
        let mut retired = false;
        let watcher = &mut self.watcher;
        self.physical.retain(|path, (recursive, identity)| {
            let keep = !invalid
                && stamp(path).is_ok_and(|current| *identity == current)
                && desired.get(path).copied() == Some(*recursive);
            if !keep {
                let _ = watcher.unwatch(path);
                retired = true;
            }
            keep
        });
        if retired {
            for callback in callbacks(&self.users) {
                callback(notify::Event::new(EventKind::Modify(
                    notify::event::ModifyKind::Any,
                )));
            }
        }
        for (path, &recursive) in &desired {
            if self.physical.contains_key(path) {
                continue;
            }
            let identity = match stamp(path) {
                Ok(identity) => identity,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && !acquiring.is_some_and(|id| {
                            lock(&self.users)[&id].0.iter().any(|(p, _)| p == path)
                        }) =>
                {
                    // A retained lease can outlive its directory while its owner
                    // subscribes to an existing ancestor. Keep the logical request
                    // for recreation, but do not let it block other subscribers.
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            self.watcher.watch(
                path,
                if recursive {
                    notify::RecursiveMode::Recursive
                } else {
                    notify::RecursiveMode::NonRecursive
                },
            )?;
            if post(path).ok() != Some(identity) {
                let _ = self.watcher.unwatch(path);
                anyhow::bail!("directory replaced while subscribing");
            }
            self.physical.insert(path.clone(), (recursive, identity));
        }
        if let Some(id) = acquiring {
            for (path, _) in &lock(&self.users)[&id].0 {
                // Root collapse must not hide a vanished acquiring path behind
                // a missing recursive root retained by another subscriber.
                stamp(path)?;
                anyhow::ensure!(
                    path.ancestors().any(|root| {
                        self.physical
                            .get(root)
                            .is_some_and(|(recursive, _)| root == path || *recursive)
                    }),
                    "acquiring path has no physical watch coverage"
                );
            }
        }
        Ok(())
    }
}
fn roots(users: &Users) -> Roots {
    let users = lock(users);
    let mut requested = BTreeMap::new();
    for (paths, _) in users.values() {
        for (path, recursive) in paths {
            *requested.entry(path.clone()).or_insert(false) |= *recursive;
        }
    }
    let mut roots: Roots = BTreeMap::new();
    for (path, recursive) in requested {
        if !roots
            .iter()
            .any(|(root, covers)| *covers && path.starts_with(root))
        {
            roots.insert(path, recursive);
        }
    }
    roots
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscriptions_share_parents_and_collapse_deep_recursive_trees() {
        let root = tempfile::tempdir().unwrap();
        let pool = Pool::new().unwrap();
        let parent = root.path().canonicalize().unwrap();
        let subscribe = |paths| pool.subscribe(paths, Arc::new(|_| {})).unwrap();
        let mut leases = vec![
            subscribe(vec![(parent.clone(), false)]),
            subscribe(vec![(parent.clone(), false)]),
        ];
        assert_eq!(lock(&lock(&pool.0).users).len(), 2);
        assert_eq!(lock(&pool.0).physical.len(), 1);
        let path = parent.join(vec!["nested"; 24].join("/"));
        std::fs::create_dir_all(&path).unwrap();
        assert_eq!(stamp(&parent).unwrap(), lock(&pool.0).physical[&parent].1);
        let paths = path
            .ancestors()
            .take(25)
            .map(|path| (path.to_owned(), true))
            .collect();
        leases.push(subscribe(paths));
        assert_eq!(pool.0.lock().unwrap().physical.len(), 1);
        assert_eq!(lock(&lock(&pool.0).users).len(), 3);
        let failed = tempfile::tempdir().unwrap();
        let mut state = lock(&pool.0);
        lock(&state.users).insert(
            u64::MAX,
            (vec![(failed.path().to_owned(), false)], Arc::new(|_| {})),
        );
        assert!(state.rebuild(|_| Ok((0, 0)), None).is_err());
        assert!(state.watcher.unwatch(failed.path()).is_err());
        state.remove(u64::MAX);
        drop(state);
        let weak = Arc::downgrade(&pool);
        drop(pool);
        drop(leases);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn missing_root_does_not_block_acquisition_or_parent_replacement() {
        for recursive in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let parent = root.path().canonicalize().unwrap();
            let missing = parent.join("missing");
            let unrelated = parent.join("unrelated");
            std::fs::create_dir(&missing).unwrap();
            std::fs::create_dir(&unrelated).unwrap();
            let pool = Pool::new().unwrap();
            let wakeups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let notified = wakeups.clone();
            let stale = pool
                .subscribe(
                    vec![(missing.clone(), recursive)],
                    Arc::new(move |_| {
                        notified.fetch_add(1, Ordering::Relaxed);
                    }),
                )
                .unwrap();
            std::fs::remove_dir(&missing).unwrap();
            let independent = pool
                .subscribe(vec![(unrelated.clone(), false)], Arc::new(|_| {}))
                .unwrap();
            assert!(wakeups.load(Ordering::Relaxed) > 0);
            assert!(!lock(&pool.0).physical.contains_key(&missing));
            assert!(lock(&pool.0).physical.contains_key(&unrelated));

            // ensure_file_watch acquires this lease before dropping the old one.
            let replacement = pool
                .subscribe(vec![(parent.clone(), false)], Arc::new(|_| {}))
                .unwrap();
            assert_eq!(lock(&pool.0).physical.len(), 2);
            drop(stale);
            assert_eq!(lock(&pool.0).physical.len(), 2);
            assert!(pool
                .subscribe(vec![(missing.clone(), recursive)], Arc::new(|_| {}))
                .is_err());

            std::fs::create_dir(&missing).unwrap();
            let recreated = pool
                .subscribe(vec![(missing.clone(), recursive)], Arc::new(|_| {}))
                .unwrap();
            assert_eq!(lock(&pool.0).physical[&missing].1, stamp(&missing).unwrap());
            drop((recreated, replacement, independent));
            assert!(lock(&pool.0).physical.is_empty());
            assert!(lock(&lock(&pool.0).users).is_empty());
        }
    }

    #[test]
    fn recreated_root_survives_stale_lease_release_and_canonical_alias_changes() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().canonicalize().unwrap();
        let path = parent.join("watched");
        let retired = parent.join("retired");
        let alias = parent.join("alias");
        std::fs::create_dir(&path).unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        let pool = Pool::new().unwrap();
        let stale = pool
            .subscribe(vec![(alias.clone(), false)], Arc::new(|_| {}))
            .unwrap();
        let original = stamp(&path).unwrap();
        std::fs::rename(&path, &retired).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_ne!(original, stamp(&path).unwrap());
        let renewed = pool
            .subscribe(vec![(alias.clone(), false)], Arc::new(|_| {}))
            .unwrap();
        assert_eq!(lock(&pool.0).physical[&path].1, stamp(&path).unwrap());
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&retired, &alias).unwrap();
        let redirected = pool
            .subscribe(vec![(alias, false)], Arc::new(|_| {}))
            .unwrap();
        drop(stale);
        assert_eq!(lock(&pool.0).physical.len(), 2);
        assert_eq!(lock(&pool.0).physical[&path].1, stamp(&path).unwrap());
        assert_eq!(lock(&pool.0).physical[&retired].1, original);
        drop(renewed);
        assert!(!lock(&pool.0).physical.contains_key(&path));
        assert!(lock(&pool.0).physical.contains_key(&retired));
        drop(redirected);
        assert!(lock(&pool.0).physical.is_empty());
    }

    #[test]
    fn missing_acquiring_root_and_other_stamp_errors_are_not_ignored() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().canonicalize().unwrap();
        let path = parent.join("watched");
        std::fs::create_dir(&path).unwrap();
        let pool = Pool::new().unwrap();
        let lease = pool
            .subscribe(vec![(path.clone(), false)], Arc::new(|_| {}))
            .unwrap();
        std::fs::remove_dir(&path).unwrap();
        // Deterministically exercise removal after acquisition canonicalized its paths.
        let error = lock(&pool.0).rebuild(stamp, Some(lease.id)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
        std::os::unix::fs::symlink(&path, &path).unwrap();
        let expected = stamp(&path).unwrap_err();
        assert_ne!(expected.kind(), std::io::ErrorKind::NotFound);
        let error = pool
            .subscribe(vec![(parent, false)], Arc::new(|_| {}))
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().raw_os_error(),
            expected.raw_os_error()
        );
        assert_eq!(lock(&lock(&pool.0).users).len(), 1);
        drop(lease);
        assert!(lock(&pool.0).physical.is_empty());
    }

    #[test]
    fn collapsed_acquiring_child_cannot_succeed_after_ancestor_disappears() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().canonicalize().unwrap().join("ancestor");
        let child = parent.join("child");
        std::fs::create_dir_all(&child).unwrap();
        let pool = Pool::new().unwrap();
        let retained = pool
            .subscribe(vec![(parent.clone(), true)], Arc::new(|_| {}))
            .unwrap();
        let child = child.canonicalize().unwrap();
        let mut state = lock(&pool.0);
        lock(&state.users).insert(u64::MAX, (vec![(child, false)], Arc::new(|_| {})));
        std::fs::remove_dir_all(&parent).unwrap();
        let error = state.rebuild(stamp, Some(u64::MAX)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
        state.remove(u64::MAX);
        assert_eq!(lock(&state.users).len(), 1);
        assert!(state.physical.is_empty());
        drop(state);
        drop(retained);
    }

    #[test]
    fn partial_install_rollback_restores_existing_subscriber_notifications() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().canonicalize().unwrap();
        let added = parent.join("a-new");
        let existing = parent.join("m-existing");
        let failed = parent.join("z-fail");
        for path in [&added, &existing, &failed] {
            std::fs::create_dir(path).unwrap();
        }
        let pool = Pool::new().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let retained = pool.subscribe(vec![(existing.clone(), false)], Arc::new(move |event| {
            let _ = send.send(event);
        })).unwrap();
        let mut state = lock(&pool.0);
        lock(&state.users).insert(u64::MAX, (
            vec![(added.clone(), false), (existing.clone(), true), (failed.clone(), false)],
            Arc::new(|_| {}),
        ));
        assert!(state.rebuild(|path| {
            if path.file_name().unwrap() == "z-fail" { Ok((0, 0)) } else { stamp(path) }
        }, Some(u64::MAX)).is_err());
        assert!(state.physical.contains_key(&added));
        assert!(state.physical[&existing].0);
        assert!(receive.try_iter().any(|event| event.paths.is_empty()));
        state.remove(u64::MAX);
        assert_eq!(state.physical.len(), 1);
        assert!(!state.physical[&existing].0);
        assert!(state.watcher.unwatch(&added).is_err());
        assert!(state.watcher.unwatch(&failed).is_err());
        drop(state);
        while receive.try_recv().is_ok() {}
        let target = existing.join("after-rollback");
        std::fs::write(&target, "still watched").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let event = receive.recv_timeout(
                deadline.saturating_duration_since(std::time::Instant::now())
            ).expect("retained subscriber must receive mutations after rollback");
            if event.paths.contains(&target) { break; }
        }
        drop(retained);
        assert!(lock(&pool.0).physical.is_empty());
    }
}
