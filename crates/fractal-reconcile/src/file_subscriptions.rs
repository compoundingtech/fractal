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
type Roots = BTreeMap<PathBuf, (bool, usize)>;
type Users = Arc<Mutex<BTreeMap<u64, (Paths, Callback)>>>;
type Stamp = (u64, u64, i64, i64);
pub(super) struct Pool(Mutex<State>);
struct State {
    watcher: notify::RecommendedWatcher,
    users: Users,
    invalid: Arc<AtomicBool>,
    physical: BTreeMap<PathBuf, (bool, usize, Stamp)>,
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
    std::fs::metadata(path).map(|m| (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()))
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
        if let Err(error) = state.rebuild() {
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
        let _ = self.rebuild();
    }
    fn rebuild(&mut self) -> Result<()> {
        let desired = roots(&self.users);
        let invalid = self.invalid.swap(false, Ordering::AcqRel);
        let mut retired = false;
        let watcher = &mut self.watcher;
        self.physical.retain(|path, (recursive, _, identity)| {
            let keep = !invalid
                && stamp(path).is_ok_and(|current| *identity == current)
                && desired
                    .get(path)
                    .is_some_and(|(next, _)| *recursive == *next);
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
        for (path, &(recursive, users)) in &desired {
            if let Some(existing) = self.physical.get_mut(path) {
                existing.1 = users;
                continue;
            }
            let identity = stamp(path)?;
            self.watcher.watch(
                path,
                if recursive {
                    notify::RecursiveMode::Recursive
                } else {
                    notify::RecursiveMode::NonRecursive
                },
            )?;
            anyhow::ensure!(
                stamp(path)? == identity,
                "directory replaced while subscribing"
            );
            self.physical
                .insert(path.clone(), (recursive, users, identity));
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
            .any(|(root, (covers, _))| *covers && path.starts_with(root))
        {
            roots.insert(path, (recursive, 0));
        }
    }
    for (root, (recursive, count)) in &mut roots {
        *count = users
            .values()
            .filter(|(paths, _)| {
                paths
                    .iter()
                    .any(|(path, _)| path == root || *recursive && path.starts_with(root))
            })
            .count();
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
        let mut leases = Vec::new();
        for _ in 0..2 {
            leases.push(
                pool.subscribe(vec![(parent.clone(), false)], Arc::new(|_| {}))
                    .unwrap(),
            );
        }
        assert_eq!(pool.0.lock().unwrap().physical[&parent].1, 2);
        let path = parent.join(vec!["nested"; 24].join("/"));
        std::fs::create_dir_all(&path).unwrap();
        let paths = path
            .ancestors()
            .take(25)
            .map(|path| (path.to_owned(), true))
            .collect();
        leases.push(pool.subscribe(paths, Arc::new(|_| {})).unwrap());
        assert_eq!(pool.0.lock().unwrap().physical.len(), 1);
        assert_eq!(pool.0.lock().unwrap().physical[&parent].1, 3);
        let weak = Arc::downgrade(&pool);
        drop(pool);
        drop(leases);
        assert!(weak.upgrade().is_none());
    }
}
