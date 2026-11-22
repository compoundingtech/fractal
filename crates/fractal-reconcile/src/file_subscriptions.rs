use anyhow::Result;
use notify::{
    EventKind,
    RecursiveMode::{NonRecursive, Recursive},
    Watcher as _,
    event::ModifyKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};
pub(super) struct Notification<'a> {
    pub event: &'a notify::Event,
    pub coverage_changed: bool,
}
type Callback = Arc<dyn Fn(Notification<'_>) + Send + Sync>;
type Paths = Vec<(PathBuf, bool)>;
type Predicate = Arc<dyn Fn(&notify::Event) -> bool + Send + Sync>;
type Users = Arc<Mutex<Subscribers>>;
#[derive(Default)]
struct Subscribers {
    leases: BTreeMap<u64, (Paths, Callback)>,
    roots: BTreeMap<PathBuf, BTreeSet<u64>>,
    inputs: BTreeMap<u64, Predicate>,
}
impl Subscribers {
    fn insert(&mut self, id: u64, entry: (Paths, Callback)) {
        for (path, _) in &entry.0 {
            self.roots.entry(path.clone()).or_default().insert(id);
        }
        self.leases.insert(id, entry);
    }
    fn remove(&mut self, id: &u64) -> Option<(Paths, Callback)> {
        let entry = self.leases.remove(id)?;
        self.inputs.remove(id);
        for (path, _) in &entry.0 {
            if let Some(ids) = self.roots.get_mut(path) {
                ids.remove(id);
                if ids.is_empty() { self.roots.remove(path); }
            }
        }
        Some(entry)
    }
    #[cfg(test)]
    fn len(&self) -> usize { self.leases.len() }
    #[cfg(test)]
    fn is_empty(&self) -> bool { self.leases.is_empty() }

    fn callbacks(&self, event: &notify::Event, coverage: bool) -> Vec<Callback> {
        let mut ids = BTreeSet::new();
        if event.need_rescan() && !coverage || event.paths.is_empty() {
            ids.extend(self.leases.keys().copied());
        } else {
            for path in &event.paths {
                for ancestor in path.ancestors() {
                    if let Some(users) = self.roots.get(ancestor) {
                        ids.extend(users);
                    }
                }
                if coverage || matches!(event.kind, EventKind::Remove(_)
                    | EventKind::Modify(ModifyKind::Name(_))) {
                    for root in subtree(&self.roots, path) {
                        ids.extend(&self.roots[root]);
                    }
                }
            }
        }
        ids.into_iter().filter_map(|id| {
            let (paths, callback) = &self.leases[&id];
            let relevant = if coverage {
                paths.iter().any(|(root, _)| event.paths.iter().any(|path|
                    root.starts_with(path) || path.starts_with(root)))
            } else if let Some(predicate) = self.inputs.get(&id) {
                predicate(event)
            } else {
                event.need_rescan() || event.paths.is_empty() || paths.iter().any(|(root, recursive)|
                    event.paths.iter().any(|path| path == root
                        || (*recursive && path.starts_with(root))
                        || path.parent() == Some(root.as_path())
                        || (root.starts_with(path) && matches!(event.kind, EventKind::Remove(_)
                            | EventKind::Modify(ModifyKind::Name(_))))))
            };
            relevant.then(|| callback.clone())
        }).collect()
    }
}
type Stamp = (u64, u64);
type Probe = fn(&Path) -> std::io::Result<Stamp>;
pub(super) struct Pool(Mutex<State>);
struct State {
    watcher: notify::RecommendedWatcher,
    users: Users,
    generation: Arc<Mutex<BTreeMap<PathBuf, u64>>>,
    requested: BTreeMap<PathBuf, [isize; 2]>,
    physical: BTreeMap<PathBuf, (bool, Stamp, u64)>,
    next: u64,
}
pub(super) struct Lease {
    pool: Arc<Pool>,
    id: u64,
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
fn dispatch(users: &Users, event: &notify::Event, coverage: bool) -> usize {
    let callbacks = lock(users).callbacks(event, coverage);
    for callback in &callbacks {
        crate::incremental::file_watch_work::delivery(coverage);
        callback(Notification { event, coverage_changed: coverage });
    }
    callbacks.len()
}
#[cfg(test)]
thread_local! { static STATS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
fn stamp(path: &Path) -> std::io::Result<Stamp> {
    #[cfg(test)]
    STATS.set(STATS.get() + 1);
    std::fs::metadata(path).map(|m| (m.dev(), m.ino()))
}
fn subtree<'a, T>(
    map: &'a BTreeMap<PathBuf, T>,
    root: &'a Path,
) -> impl Iterator<Item = &'a PathBuf> {
    map.range(root.to_owned()..)
        .take_while(move |(p, _)| p.starts_with(root))
        .map(|(p, _)| p)
}
fn invalidate_coverage(generations: &Mutex<BTreeMap<PathBuf, u64>>, event: &notify::Event) {
    if !event.need_rescan() && !matches!(event.kind, EventKind::Remove(_)
        | EventKind::Modify(ModifyKind::Name(_))) {
        return;
    }
    let mut generations = lock(generations);
    if event.need_rescan() || event.paths.is_empty() {
        for generation in generations.values_mut() { *generation += 1; }
    } else {
        let mut roots = BTreeSet::new();
        for path in &event.paths {
            roots.extend(path.ancestors().filter(|root| generations.contains_key(*root)).map(Path::to_owned));
            roots.extend(subtree(&generations, path).cloned());
        }
        for root in roots { *generations.get_mut(&root).expect("indexed root") += 1; }
    }
}
impl Pool {
    pub(super) fn new() -> Result<Arc<Self>> {
        let users = Users::default();
        let subscribers = users.clone();
        let generation = Arc::new(Mutex::new(BTreeMap::new()));
        let lost = generation.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let event = event.unwrap_or_else(|_| {
                notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)
            });
            invalidate_coverage(&lost, &event);
            if !event.need_rescan() && matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            crate::incremental::file_watch_work::event();
            dispatch(&subscribers, &event, false);
        })?;
        Ok(Arc::new(Self(Mutex::new(State {
            watcher,
            users,
            generation,
            requested: BTreeMap::new(),
            physical: BTreeMap::new(),
            next: 0,
        }))))
    }
    pub(super) fn subscribe_input(
        self: &Arc<Self>, paths: Paths, predicate: Predicate, callback: Callback,
    ) -> Result<Lease> {
        self.acquire(paths, Some(predicate), callback)
    }
    #[cfg(test)]
    fn subscribe(self: &Arc<Self>, paths: Paths, callback: Callback) -> Result<Lease> {
        self.acquire(paths, None, callback)
    }
    fn acquire(self: &Arc<Self>, paths: Paths, predicate: Option<Predicate>, callback: Callback) -> Result<Lease> {
        let paths = paths
            .into_iter()
            .map(|(path, recursive)| Ok((path.canonicalize()?, recursive)))
            .collect::<Result<Paths>>()?;
        let mut state = lock(&self.0);
        let id = state.next;
        state.next += 1;
        lock(&state.users).insert(id, (paths.clone(), callback));
        if let Some(predicate) = predicate { lock(&state.users).inputs.insert(id, predicate); }
        if let Err(error) = state.update(&paths, true, stamp) {
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
        let removed = lock(&self.users).remove(&id);
        if let Some((paths, _)) = removed {
            let _ = self.update(&paths, false, stamp);
        }
    }
    fn install(&mut self, path: &Path, recursive: bool, post: Probe) -> Result<()> {
        let identity = stamp(path)?;
        let generation = *lock(&self.generation).entry(path.to_owned()).or_default();
        let mode = if recursive { Recursive } else { NonRecursive };
        if let Err(error) = self.watcher.watch(path, mode) {
            lock(&self.generation).remove(path);
            return Err(error.into());
        }
        if post(path).ok() != Some(identity) {
            let _ = self.watcher.unwatch(path);
            lock(&self.generation).remove(path);
            anyhow::bail!("directory replaced while subscribing");
        }
        self.physical
            .insert(path.to_owned(), (recursive, identity, generation));
        Ok(())
    }
    fn update(&mut self, paths: &Paths, add: bool, post: Probe) -> Result<()> {
        for (path, recursive) in paths {
            let counts = self.requested.entry(path.clone()).or_default();
            counts[usize::from(*recursive)] += if add { 1 } else { -1 };
            if *counts == [0, 0] {
                self.requested.remove(path);
            }
        }
        let roots: std::collections::BTreeSet<_> = paths
            .iter()
            .map(|(path, _)| {
                path.ancestors()
                    .filter(|p| self.requested.get(*p).is_some_and(|c| c[1] > 0))
                    .last()
                    .unwrap_or(path)
                    .to_owned()
            })
            .collect();
        for root in roots {
            let expand = !self.physical.contains_key(&root)
                || self.physical.get(&root).is_some_and(|p| p.0)
                    != self.requested.get(&root).is_some_and(|c| c[1] > 0);
            let candidates = if expand {
                subtree(&self.requested, &root)
                    .chain(subtree(&self.physical, &root))
                    .cloned()
                    .collect()
            } else {
                std::collections::BTreeSet::from([root])
            };
            let mut pending = Vec::new();
            for path in candidates {
                let generation = lock(&self.generation).get(&path).copied().unwrap_or_default();
                let desired = self.requested.get(&path).map(|c| c[1] > 0).filter(|_| {
                    !path
                        .ancestors()
                        .skip(1)
                        .any(|p| self.requested.get(p).is_some_and(|c| c[1] > 0))
                });
                if self
                    .physical
                    .get(&path)
                    .is_some_and(|&(mode, id, installed)| {
                        desired == Some(mode)
                            && installed == generation
                            && stamp(&path).ok() == Some(id)
                    })
                {
                    continue;
                }
                if self.physical.remove(&path).is_some() {
                    let _ = self.watcher.unwatch(&path);
                    if desired.is_none() { lock(&self.generation).remove(&path); }
                    // Watch replacement can lose events only in the replaced coverage.
                    // A scoped rescan asks those leases to reread, not every input.
                    let event = notify::Event::new(EventKind::Modify(ModifyKind::Any))
                        .add_path(path.clone()).set_flag(notify::event::Flag::Rescan);
                    dispatch(&self.users, &event, true);
                }
                if let Some(mode) = desired {
                    pending.push((path, mode));
                }
            }
            for (path, mode) in pending {
                if let Err(error) = self.install(&path, mode, post) {
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                        && !(add && paths.iter().any(|(requested, _)| requested == &path))
                    {
                        // Retain a missing old request for recreation without
                        // blocking acquisition of its existing ancestor.
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        if add {
            for (path, _) in paths {
                // Root collapse must not hide a vanished acquiring child.
                // Exact roots were already validated during maintenance.
                if !self.physical.contains_key(path) {
                    stamp(path)?;
                }
                anyhow::ensure!(
                    path.ancestors().any(|root| {
                        self.physical
                            .get(root)
                            .is_some_and(|(recursive, _, _)| root == path || *recursive)
                    }),
                    "acquiring path has no physical watch coverage"
                );
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn sixty_four_roots_dispatch_and_upgrade_only_intersecting_leases() {
        use std::sync::atomic::AtomicUsize;
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().canonicalize().unwrap();
        let paths: Vec<_> = (0..64).map(|index| parent.join(index.to_string())).collect();
        for path in &paths { std::fs::create_dir(path).unwrap(); }
        let pool = Pool::new().unwrap();
        let counts: Vec<_> = paths.iter().map(|_| Arc::new(AtomicUsize::new(0))).collect();
        let mut leases = Vec::new();
        let predicate = |directory: &Path| -> Predicate {
            let directory = directory.to_owned();
            let target = directory.join("input");
            Arc::new(move |event| super::super::FileWatch::relevant(event, &target, &directory, None))
        };
        for (path, count) in paths.iter().zip(&counts) {
            let count = count.clone();
            leases.push(pool.subscribe_input(vec![(path.clone(), false)], predicate(path), Arc::new(move |_| {
                count.fetch_add(1, Ordering::Relaxed);
            })).unwrap());
        }
        let upgraded = pool.subscribe_input(vec![(paths[0].clone(), true)], predicate(&paths[0]), Arc::new(|_| {})).unwrap();
        assert_eq!(counts[0].load(Ordering::Relaxed), 1, "one replaced root");
        assert!(counts[1..].iter().all(|count| count.load(Ordering::Relaxed) == 0));
        let users = lock(&pool.0).users.clone();
        let event = notify::Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(parent.join("unrelated")).add_path(paths[0].join("unrelated"));
        assert_eq!(dispatch(&users, &event, false), 0, "no callbacks for unrelated paths");
        let event = event.add_path(paths[0].join("input"));
        assert_eq!(dispatch(&users, &event, false), 2, "any matching path, each lease once");
        assert_eq!(counts[0].load(Ordering::Relaxed), 2);
        assert!(counts[1..].iter().all(|count| count.load(Ordering::Relaxed) == 0));

        let generations = lock(&pool.0).generation.clone();
        let event = notify::Event::new(EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::From)))
            .add_path(paths[0].clone());
        invalidate_coverage(&generations, &event);
        assert_eq!(lock(&generations)[&paths[0]], 1);
        assert!(paths[1..].iter().all(|path| lock(&generations)[path] == 0));
        assert_eq!(dispatch(&users, &event, false), 2);
        let event = notify::Event::new(EventKind::Remove(notify::event::RemoveKind::Folder))
            .add_path(parent.clone());
        assert_eq!(dispatch(&users, &event, false), 65, "removed ancestor reaches descendants");
        let event = notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        assert_eq!(dispatch(&users, &event, false), 65, "lost paths retain all-root fallback");
        drop((upgraded, leases));
    }
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
        assert_eq!(lock(&pool.0).physical.len(), 1);
        assert_eq!(lock(&lock(&pool.0).users).len(), 2);
        let path = parent.join(vec!["nested"; 24].join("/"));
        std::fs::create_dir_all(&path).unwrap();
        assert_eq!(stamp(&parent).unwrap(), lock(&pool.0).physical[&parent].1);
        let paths = path
            .ancestors()
            .take(25)
            .map(|p| (p.to_owned(), true))
            .collect();
        leases.push(subscribe(paths));
        assert_eq!(lock(&pool.0).physical.len(), 1);
        assert_eq!(lock(&lock(&pool.0).users).len(), 3);
        drop(leases.pop());
        assert!(!lock(&pool.0).physical[&parent].0);
        let failed = tempfile::tempdir().unwrap();
        let mut state = lock(&pool.0);
        assert!(state.install(failed.path(), false, |_| Ok((0, 0))).is_err());
        assert!(state.watcher.unwatch(failed.path()).is_err());
        let result = state.install(failed.path(), false, |_| {
            Err(std::io::ErrorKind::NotFound.into())
        });
        assert!(result.is_err());
        assert!(state.watcher.unwatch(failed.path()).is_err());
        assert_eq!(state.physical.len(), 1);
        drop(state);
        let weak = Arc::downgrade(&pool);
        drop(pool);
        drop(leases);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn acquisition_and_release_only_validate_touched_roots() {
        let root = tempfile::tempdir().unwrap();
        let pool = Pool::new().unwrap();
        let mut leases = Vec::new();
        for index in 0..64 {
            let path = root.path().join(index.to_string());
            std::fs::create_dir(&path).unwrap();
            STATS.set(0);
            leases.push(
                pool.subscribe(vec![(path.clone(), false)], Arc::new(|_| {}))
                    .unwrap(),
            );
            assert_eq!(STATS.get(), 2, "new root: before and after install only");
            STATS.set(0);
            let duplicate = pool
                .subscribe(vec![(path, false)], Arc::new(|_| {}))
                .unwrap();
            assert_eq!(STATS.get(), 1, "only the reused root is revalidated");
            STATS.set(0);
            drop(duplicate);
            assert_eq!(STATS.get(), 1);
        }
        for generation in lock(&lock(&pool.0).generation).values_mut() { *generation += 1; }
        STATS.set(0);
        let renewal = pool
            .subscribe(vec![(root.path().join("0"), false)], Arc::new(|_| {}))
            .unwrap();
        assert_eq!(
            STATS.get(),
            2,
            "lost coverage reinstalls only the touched root"
        );
        assert_eq!(
            lock(&pool.0).physical.values().filter(|p| p.2 == 0).count(),
            63
        );
        drop(renewal);
        while let Some(lease) = leases.pop() {
            STATS.set(0);
            drop(lease);
            assert_eq!(STATS.get(), 0, "unrelated roots are not revalidated");
        }
        assert!(lock(&pool.0).physical.is_empty());
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
            assert!(lock(&pool.0).physical.contains_key(&unrelated));

            // ensure_file_watch acquires this lease before dropping the old one.
            let replacement = pool
                .subscribe(vec![(parent.clone(), false)], Arc::new(|_| {}))
                .unwrap();
            assert!(wakeups.load(Ordering::Relaxed) > 0);
            assert!(!lock(&pool.0).physical.contains_key(&missing));
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
        let mut state = lock(&pool.0);
        let paths = vec![(path.clone(), false)];
        lock(&state.users).insert(u64::MAX, (paths.clone(), Arc::new(|_| {})));
        let error = state.update(&paths, true, stamp).unwrap_err();
        state.remove(u64::MAX);
        drop(state);
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
        let paths = vec![(child, false)];
        lock(&state.users).insert(u64::MAX, (paths.clone(), Arc::new(|_| {})));
        std::fs::remove_dir_all(&parent).unwrap();
        let error = state.update(&paths, true, stamp).unwrap_err();
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
            let _ = send.send(event.event.clone());
        })).unwrap();
        let mut state = lock(&pool.0);
        let paths = vec![(added.clone(), false), (existing.clone(), true), (failed.clone(), false)];
        lock(&state.users).insert(u64::MAX, (paths.clone(), Arc::new(|_| {})));
        assert!(state.update(&paths, true, |path| {
            if path.file_name().unwrap() == "z-fail" { Ok((0, 0)) } else { stamp(path) }
        }).is_err());
        assert!(state.physical.contains_key(&added));
        assert!(state.physical[&existing].0);
        assert!(receive.try_iter().any(|event| event.need_rescan() && event.paths == [existing.clone()]));
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
