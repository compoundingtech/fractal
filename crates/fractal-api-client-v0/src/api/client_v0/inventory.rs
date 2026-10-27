//! The part of `api::client_v0::inventory` that moved up from fractal-api-core.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::LimitsStore;
use crate::SeatsStore;
use super::*;
use std::path::PathBuf;
pub use fractal_api_core::api::client_v0::inventory::*;
pub use fractal_api_core::api::client_v0::inventory::{items, next_expiry, usage_value, validate};

pub fn is_inventory(collection: &str) -> bool {
    matches!(
        collection,
        "clients" | "usage" | "machines" | "devices" | "sessions" | "terminals"
    )
}

/// Watch native inventory files while a session list is held. Missing roots are watched at
/// their nearest existing ancestor, without recursively watching an unrelated home tree.
pub struct NativeWatch {
    watcher: notify::RecommendedWatcher,
    pub processes_in_read: Arc<Mutex<Vec<u32>>>,
    #[cfg(target_os = "linux")]
    processes: BTreeMap<u32, tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>>,
    home: PathBuf,
    paths: BTreeMap<PathBuf, notify::RecursiveMode>,
    events: tokio::sync::watch::Receiver<Result<(), String>>,
}

impl NativeWatch {
    pub fn open(home: &Path) -> anyhow::Result<Self> {
        let (sender, events) = tokio::sync::watch::channel(Ok(()));
        let roots = native_roots(home);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            match event {
                Ok(event) => {
                    if matches!(event.kind, notify::EventKind::Access(_)) {
                        return;
                    }
                    // Ancestors admit newly created provider roots; other home files do not wake us.
                    if !event.paths.iter().any(|path| {
                        roots
                            .iter()
                            .any(|root| path.starts_with(root) || root.starts_with(path))
                    }) {
                        return;
                    }
                    // A later event must not erase a source failure before the socket sees it.
                    sender.send_if_modified(|held| held.is_ok());
                }
                Err(error) => {
                    let _ = sender.send_replace(Err(error.to_string()));
                }
            }
        })?;
        let mut watch = Self {
            watcher,
            processes_in_read: Arc::new(Mutex::new(Vec::new())),
            #[cfg(target_os = "linux")]
            processes: BTreeMap::new(),
            home: home.to_owned(),
            paths: BTreeMap::new(),
            events,
        };
        watch.refresh()?;
        Ok(watch)
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        use notify::Watcher as _;
        let mut paths = BTreeMap::new();
        for root in self.roots() {
            let mut path = root.clone();
            while !path.is_dir() {
                path = path
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("native session home has no existing ancestor"))?
                    .to_owned();
            }
            let mode = if path == root {
                notify::RecursiveMode::Recursive
            } else {
                notify::RecursiveMode::NonRecursive
            };
            paths
                .entry(path)
                .and_modify(|held| {
                    if mode == notify::RecursiveMode::Recursive {
                        *held = mode;
                    }
                })
                .or_insert(mode);
        }
        for (path, mode) in &paths {
            if self.paths.get(path) != Some(mode) {
                self.watcher.watch(path, *mode)?;
            }
        }
        for path in self.paths.keys().filter(|path| !paths.contains_key(*path)) {
            self.watcher.unwatch(path)?;
        }
        self.paths = paths;
        Ok(())
    }

    pub fn observe_processes(&mut self) -> anyhow::Result<bool> {
        let mut exited = false;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::FromRawFd as _;
            let pids = self
                .processes_in_read
                .lock()
                .expect("native process coverage")
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            self.processes.retain(|pid, _| pids.contains(pid));
            for pid in pids {
                if self.processes.contains_key(&pid) {
                    continue;
                }
                let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
                if fd < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ESRCH) {
                        exited = true;
                        continue;
                    }
                    return Err(error.into());
                }
                let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
                self.processes
                    .insert(pid, tokio::io::unix::AsyncFd::new(fd)?);
            }
        }
        Ok(exited)
    }

    pub async fn changed(&mut self) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        let mut exits = self
            .processes
            .iter()
            .map(|(pid, fd)| async move { fd.readable().await.map(|_| *pid) })
            .collect::<futures_util::stream::FuturesUnordered<_>>();
        #[cfg(target_os = "linux")]
        let has_exits = !exits.is_empty();
        #[cfg(not(target_os = "linux"))]
        let (mut exits, has_exits) = (
            futures_util::stream::pending::<std::io::Result<u32>>(),
            false,
        );
        let exit = tokio::select! {
            event = self.events.changed() => {
                event?;
                self.events.borrow_and_update().clone().map_err(anyhow::Error::msg)?;
                None
            }
            pid = exits.next(), if has_exits => Some(pid.expect("process exit watcher")?),
        };
        drop(exits);
        #[cfg(target_os = "linux")]
        if let Some(pid) = exit {
            self.processes.remove(&pid);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = exit;
        self.refresh()
    }

    fn roots(&self) -> Vec<PathBuf> {
        native_roots(&self.home)
    }
}

fn native_roots(home: &Path) -> Vec<PathBuf> {
    [
        ".codex/sessions",
        ".claude/projects",
        ".claude/sessions",
        ".pi/agent/sessions",
        ".omp/agent/sessions",
        ".oh-omp/agent/sessions",
        ".local/share/opencode",
    ]
    .iter()
    .map(|suffix| home.join(suffix))
    .collect()
}
