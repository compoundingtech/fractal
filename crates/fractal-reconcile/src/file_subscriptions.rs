//! Logical leases share one backend and reference-counted physical directory subscriptions.
use anyhow::Result;
use notify::Watcher as _;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};
type Callback = Arc<dyn Fn(notify::Event) + Send + Sync>;
type Roots = BTreeMap<PathBuf, (bool, usize)>;
pub(super) struct Pool(Mutex<State>);
struct State {
    watcher: notify::RecommendedWatcher,
    callbacks: Arc<Mutex<BTreeMap<u64, Callback>>>,
    users: BTreeMap<u64, Vec<(PathBuf, bool)>>,
    physical: Roots,
    next: u64,
}
pub(super) struct Lease {
    pool: Arc<Pool>,
    id: u64,
}
impl Pool {
    pub(super) fn new() -> Result<Arc<Self>> {
        let callbacks = Arc::new(Mutex::new(BTreeMap::<u64, Callback>::new()));
        let dispatch = callbacks.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                if !event.need_rescan() && matches!(event.kind, notify::EventKind::Access(_)) {
                    return;
                }
                let callbacks: Vec<_> = dispatch
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .values()
                    .cloned()
                    .collect();
                for callback in callbacks {
                    callback(event.clone());
                }
            }
        })?;
        Ok(Arc::new(Self(Mutex::new(State {
            watcher,
            callbacks,
            users: BTreeMap::new(),
            physical: BTreeMap::new(),
            next: 0,
        }))))
    }
    pub(super) fn subscribe(
        self: &Arc<Self>,
        paths: Vec<(PathBuf, bool)>,
        callback: Callback,
    ) -> Result<Lease> {
        let paths = paths
            .into_iter()
            .map(|(path, recursive)| Ok((path.canonicalize()?, recursive)))
            .collect::<Result<_>>()?;
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = state.next;
        state.next += 1;
        state
            .callbacks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, callback);
        state.users.insert(id, paths);
        if let Err(error) = state.rebuild() {
            state.users.remove(&id);
            state
                .callbacks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            let _ = state.rebuild();
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
        let mut state = self.pool.0.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .callbacks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
        state.users.remove(&self.id);
        let _ = state.rebuild();
    }
}
impl State {
    fn rebuild(&mut self) -> Result<()> {
        let desired = roots(&self.users);
        let retired: Vec<_> = self
            .physical
            .iter()
            .filter(|(path, (recursive, _))| {
                desired.get(*path).is_none_or(|(next, _)| recursive != next)
            })
            .map(|(path, _)| path.clone())
            .collect();
        for path in &retired {
            // Remove overlapping old backend watches before installing recursive coverage.
            let _ = self.watcher.unwatch(path);
            self.physical.remove(path);
        }
        if !retired.is_empty() {
            // Any transition gap must invalidate earlier captures, including when installation fails.
            let callbacks: Vec<_> = self
                .callbacks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .cloned()
                .collect();
            for callback in callbacks {
                callback(notify::Event::new(notify::EventKind::Modify(
                    notify::event::ModifyKind::Any,
                )));
            }
        }
        for (path, &(recursive, _)) in &desired {
            if !self.physical.contains_key(path) {
                self.watcher.watch(
                    path,
                    if recursive {
                        notify::RecursiveMode::Recursive
                    } else {
                        notify::RecursiveMode::NonRecursive
                    },
                )?;
                self.physical.insert(path.clone(), desired[path]);
            }
        }
        self.physical = desired;
        Ok(())
    }
}
fn roots(users: &BTreeMap<u64, Vec<(PathBuf, bool)>>) -> Roots {
    let mut requested = BTreeMap::new();
    for (path, recursive) in users.values().flatten() {
        *requested.entry(path.clone()).or_insert(false) |= *recursive;
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
    for paths in users.values() {
        let used: BTreeSet<_> = paths
            .iter()
            .filter_map(|(path, _)| {
                roots
                    .iter()
                    .find(|(root, (recursive, _))| {
                        path == *root || *recursive && path.starts_with(root)
                    })
                    .map(|(root, _)| root.clone())
            })
            .collect();
        for root in used {
            roots.get_mut(&root).unwrap().1 += 1;
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
        let first = pool
            .subscribe(vec![(parent.clone(), false)], Arc::new(|_| {}))
            .unwrap();
        let second = pool
            .subscribe(vec![(parent.clone(), false)], Arc::new(|_| {}))
            .unwrap();
        assert_eq!(pool.0.lock().unwrap().physical[&parent], (false, 2));
        let mut path = parent.clone();
        let mut paths = vec![(parent.clone(), true)];
        for _ in 0..24 {
            path.push("nested");
            std::fs::create_dir(&path).unwrap();
            paths.push((path.clone(), true));
        }
        let tree = pool.subscribe(paths, Arc::new(|_| {})).unwrap();
        assert_eq!(
            pool.0.lock().unwrap().physical,
            BTreeMap::from([(parent.clone(), (true, 3))])
        );
        drop(first);
        drop(second);
        assert_eq!(pool.0.lock().unwrap().physical.len(), 1);
        let weak = Arc::downgrade(&pool);
        drop(pool);
        assert!(weak.upgrade().is_some());
        drop(tree);
        assert!(
            weak.upgrade().is_none(),
            "last logical user releases the backend"
        );
    }
}
