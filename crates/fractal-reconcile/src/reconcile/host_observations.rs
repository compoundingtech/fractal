//! Physical evidence retained only while its native watch and safety fence remain current.
//! Graph decisions, admission and destructive-operation fences stay with their existing owners.
//!
//! The messages-shard wake adapter below remains graph-side: it observes the full
//! shard, including remote recipients needed by mission decisions. Its durable
//! local lifecycle/read receipts retain their existing custody and cursor; this
//! is not an own-seat subscription or a light-node mail transport.
use super::*;

#[derive(Default)]
pub(super) struct HostObservations {
    revision: Arc<std::sync::atomic::AtomicU64>,
    // Registry identity/coverage changes rearm watches; ordinary metadata changes
    // invalidate evidence without tearing down otherwise valid native coverage.
    coverage_revision: Arc<std::sync::atomic::AtomicU64>,
    state: Mutex<ObservationState>,
    mail_revision: Arc<std::sync::atomic::AtomicU64>,
    mail_observer: std::sync::OnceLock<smallclaims::sqlite::CommitObserver>,
    mail_seen: Mutex<Option<u64>>,
}

#[derive(Default)]
struct ObservationState {
    watch: Option<(Arc<file_subscriptions::Pool>, file_subscriptions::Lease, u64)>,
    snapshot: Option<(u64, Arc<PtyRegistrySnapshot>)>,
    exec_revision: Option<u64>,
}

impl HostObservations {
    pub(super) fn invalidate(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(super) fn observe_execs<R: RuntimeControl>(&self, runtime: &R,
        incremental: &crate::incremental::Incremental, safety: bool) {
        let revision = self.revision.load(Ordering::Acquire);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if runtime.observation_paths().is_some() && !safety
            && state.exec_revision == Some(revision) {
            return;
        }
        state.exec_revision = Some(revision);
        drop(state);
        incremental.observe_execs(|runtime_id| {
            let started = crate::incremental::thread_cpu();
            let observation = runtime.observe_exec(runtime_id);
            crate::performance::record_evaluation("host-executor-process", true,
                crate::incremental::thread_cpu().saturating_sub(started));
            observation.ok().flatten().map(|observation| observation.status)
        });
    }

    fn watch(&self, paths: &[PathBuf], notify: &Arc<Notify>) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let coverage = self.coverage_revision.load(Ordering::Acquire);
        if state.watch.as_ref().is_some_and(|(_, _, installed)| *installed == coverage) {
            return Ok(());
        }
        let pool = file_subscriptions::Pool::new()?;
        // Watch an existing ancestor: registries can be created or replaced after startup.
        let targets = paths.iter().map(|path| {
            let ancestor = path.ancestors().find(|path| path.is_dir())
                .context("physical observation has no existing watch ancestor")?;
            let root = file_subscriptions::canonicalize(ancestor)?;
            Ok(root.join(path.strip_prefix(ancestor)?))
        }).collect::<Result<Vec<_>>>()?;
        let roots = targets.iter().map(|path| {
            let ancestor = path.ancestors().find(|path| path.is_dir())
                .context("physical observation has no watch root")?;
            Ok((ancestor.to_owned(), ancestor != path.as_path()))
        }).collect::<Result<Vec<_>>>()?;
        let revision = self.revision.clone();
        let coverage_revision = self.coverage_revision.clone();
        let watched_targets = targets.clone();
        let notify = notify.clone();
        let lease = pool.subscribe_input(roots, Arc::new(move |event| {
            targets.iter().any(|target| registry_event_relevant(event, target))
        }), Arc::new(move |notification| {
            if notification.coverage_changed || notification.event.need_rescan()
                || notification.event.paths.iter().any(|path| {
                    watched_targets.iter().any(|root| root.starts_with(path))
                })
            {
                coverage_revision.fetch_add(1, Ordering::AcqRel);
            }
            revision.fetch_add(1, Ordering::AcqRel);
            notify.notify_one();
        }))?;
        // Retain the pre-subscription epoch: a concurrent replacement must remain
        // dirty even if it races acquisition. Failed acquisition leaves the old
        // epoch dirty too, so no later pass can reuse unobserved evidence.
        state.watch = Some((pool, lease, coverage));
        Ok(())
    }

    fn snapshot<R: RuntimeControl>(&self, runtime: &R, notify: &Arc<Notify>, safety: bool)
        -> Result<Arc<PtyRegistrySnapshot>>
    {
        let Some(paths) = runtime.observation_paths() else {
            // A backend without a watch never claims retained evidence is current.
            return sample_ptys(runtime);
        };
        self.watch(paths, notify)?;
        let revision = self.revision.load(Ordering::Acquire);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !safety && let Some((seen, snapshot)) = &state.snapshot && *seen == revision {
            return Ok(snapshot.clone());
        }
        // Capture the revision before sampling; a concurrent event remains dirty.
        state.snapshot = None;
        let snapshot = sample_ptys(runtime)?;
        state.snapshot = Some((revision, snapshot.clone()));
        Ok(snapshot)
    }
}
fn registry_event_relevant(event: &notify::Event, root: &Path) -> bool {
    if event.need_rescan() { return true; }
    if !FileWatch::relevant(event, root, root, None) { return false; }
    if event.paths.is_empty() { return true; }
    event.paths.iter().any(|path| {
        // PTY output/activity/event logs are not process or binding evidence.
        // Both native registries publish their current generation as root-level JSON.
        path == root || root.starts_with(path)
            || path.parent() == Some(root) && path.extension().is_some_and(|extension| {
                matches!(extension.to_str(), Some("json" | "pid" | "sock"))
            })
    })
}

fn sample_ptys<R: RuntimeControl>(runtime: &R) -> Result<Arc<PtyRegistrySnapshot>> {
    let started = crate::incremental::thread_cpu();
    let result = runtime.snapshot_ptys_with_provenance().map(Arc::new);
    crate::performance::record_evaluation("host-executor-observation", true,
        crate::incremental::thread_cpu().saturating_sub(started));
    result
}


impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn host_pty_snapshot(&self) -> Result<Arc<PtyRegistrySnapshot>> {
        let safety = !self.skip_unneeded
            || self.incremental.take_full_pass("host-observations", now_ms());
        self.host_observations.snapshot(self.runtime.as_ref(), &self.notify, safety)
    }
    /// Mark the items that read a stream item's mailbox or subject, as the change feed marks them
    /// for a graph `message.sent` or lifecycle claim: the feed reads only the graph's file, and
    /// mail routed to the messages shard (4c-2b) would otherwise reach these items only at the
    /// next full pass. The first call only finds where the shard stands, as the first pass reads
    /// every item.
    pub(super) fn observe_stream(&self) -> Result<()> {
        let Some(shard) = crate::store::messages_shard::shard(&self.store) else {
            return Ok(());
        };
        self.host_observations.mail_observer.get_or_init(|| {
            let revision = self.host_observations.mail_revision.clone();
            shard.observe_tables(&["stream_messages", "stream_lifecycle"], move |_| {
                revision.fetch_add(1, Ordering::AcqRel);
            })
        });
        let revision = self.host_observations.mail_revision.load(Ordering::Acquire);
        let mut seen = self.host_observations.mail_seen.lock().unwrap_or_else(PoisonError::into_inner);
        if *seen == Some(revision) { return Ok(()); }
        let started = crate::incremental::thread_cpu();
        let result = self.read_mail_observations();
        crate::performance::record_evaluation("graph-mail-observation", true,
            crate::incremental::thread_cpu().saturating_sub(started));
        if result.is_ok() { *seen = Some(revision); }
        result
    }

    fn read_mail_observations(&self) -> Result<()> {
        use crate::store::messages_shard::mailbox_reads;
        let mut seen = self.stream_seen.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(mut after) = *seen else {
            *seen = Some(mailbox_reads::position(&self.store)?);
            return Ok(());
        };
        loop {
            let (next, changed, more) = mailbox_reads::wake_batch(&self.store, after, 1024)?;
            for (subject, recipient) in changed {
                if !recipient.is_empty() {
                    self.incremental.touch(&format!("mailbox:{recipient}"));
                }
                self.incremental.touch_subject(&subject);
            }
            after = next;
            *seen = Some(after);
            if !more {
                return Ok(());
            }
        }
    }
    pub(super) fn ensure_file_watch(&self, subject: &str, path: &Path) -> Result<()> {
        self.file_watchers_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(subject.into());
        let mut watchers = self
            .file_watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let target = std::path::absolute(path)?;
        // Watching the file itself attaches to its inode and misses later replacements.
        // Descend from a missing target's nearest existing parent on each observation.
        let directory = target
            .parent()
            .into_iter()
            .flat_map(Path::ancestors)
            .find(|candidate| candidate.is_dir())
            .context("file watch has no existing parent directory")?;
        let next_component = target
            .strip_prefix(directory)?
            .components()
            .next()
            .context("file watch has no target component")?;
        // Backends report canonical directories (notably /private/tmp on macOS).
        // Keep the lexical target as identity, but compare events in their namespace.
        let directory = file_subscriptions::canonicalize(directory)?;
        let relevant = directory.join(next_component);
        // A file symlink can point outside the lexical parent. Observe its referent's
        // parent too, so both writes and inode replacements preserve file-gate behavior.
        let referent = file_subscriptions::canonicalize(&target).ok().filter(|path| *path != relevant);
        let recursive = target.is_dir();
        if watchers.get(subject).is_some_and(|watch| {
            watch.target == target
                && watch.directory == directory
                && watch.referent == referent
                && watch.recursive == recursive
                && !watch.dirty.load(std::sync::atomic::Ordering::Acquire)
        }) {
            return Ok(());
        }
        let watched_directory = directory.clone();
        let watched_referent = referent.clone();
        let notify = self.notify.clone();
        let file_changes = self.file_changes.clone();
        let observations = self.file_observations.clone();
        let incremental = self.incremental.clone();
        let revision = self.file_watch_revision.clone();
        let watched_subject = subject.to_owned();
        if self.file_subscriptions.get().is_none() {
            let _ = self.file_subscriptions.set(file_subscriptions::Pool::new()?);
        }
        let pool = self.file_subscriptions.get().expect("initialized subscriptions");
        let mut paths = vec![(directory.clone(), false)];
        if recursive { paths.push((target.clone(), true)); }
        if let Some(parent) = referent.as_ref().and_then(|path| path.parent()) {
            paths.push((parent.to_owned(), false));
        }
        let dirty = Arc::new(AtomicBool::new(false));
        let changed_watch = dirty.clone();
        let subscription = pool.subscribe_input(paths, Arc::new(move |event| {
            FileWatch::relevant(event, &relevant, &watched_directory, watched_referent.as_deref())
        }), Arc::new(move |notification| {
                    let event = notification.event;
                    revision.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    if event.need_rescan() || matches!(event.kind, notify::EventKind::Remove(_)
                        | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))) {
                        changed_watch.store(true, std::sync::atomic::Ordering::Release);
                    }
                    observations
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&watched_subject);
                    // A file change is not a claim: mark what read the file.
                    file_changes.invalidate(&incremental, &watched_subject,
                        event.need_rescan() && !notification.coverage_changed, &notify);
            }))?;
        watchers.insert(subject.into(), FileWatch {
            _subscription: subscription, target, directory, referent, recursive, dirty,
        });
        // A refreshed subscription may now refer to a different file at the same stamp.
        self.file_observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(subject);
        Ok(())
    }

    pub(super) fn release_unused_file_watchers(&self) {
        let used = std::mem::take(
            &mut *self
                .file_watchers_used
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        self.file_watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|subject, _| used.contains(subject));
        self.file_observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|subject, _| used.contains(subject));
    }
}
fn observed_pty_status(observation: &fractal_runtime::PtyObservation) -> String {
    if observation.status == "running"
        && observation
            .pid
            .is_some_and(|pid| !local_process_is_alive(pid))
    {
        return "vanished".into();
    }
    observation.status.clone()
}

#[cfg(unix)]
fn local_process_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn local_process_is_alive(_pid: u32) -> bool {
    true
}

impl NativeRuntime {
    pub(super) fn snapshot_pty_registry(&self, include_provenance: bool) -> Result<PtyRegistrySnapshot> {
        let observations = self.pty()?.snapshot()?;
        // A PTY server started before st moved servers out of their harness's scope moves here.
        self.pty()?.protect_servers(&observations)?;
        let mut start_operations = HashMap::new();
        let observations = observations
            .into_iter()
            .map(|item| {
                let incarnation_id = match (&item.pid, &item.created_at) {
                    (Some(pid), Some(created)) => Some(format!("{pid}:{created}")),
                    _ => None,
                };
                let status = observed_pty_status(&item);
                if include_provenance
                    && let Some(incarnation) = &incarnation_id
                    && let Some(operation) = item.tags.get(start_jobs::OPERATION_TAG)
                {
                    start_operations.insert(
                        item.name.clone(),
                        (incarnation.clone(), operation.clone()),
                    );
                }
                RuntimeObservation {
                    runtime_id: item.name,
                    terminal: true,
                    status,
                    exit_code: item.exit_code,
                    incarnation_id,
                }
            })
            .collect();
        Ok(PtyRegistrySnapshot { observations, start_operations })
    }
}

pub(super) fn observe_exec(runtime: &fractal_runtime::ExecRuntime, runtime_id: &str)
    -> Result<Option<RuntimeObservation>> {
        Ok(runtime.observe(runtime_id)?
            .map(|observation| match observation {
                fractal_runtime::ExecObservation::Running(generation) => RuntimeObservation {
                    runtime_id: runtime_id.into(),
                    terminal: false,
                    status: "running".into(),
                    exit_code: None,
                    incarnation_id: Some(generation.generation_id),
                },
                fractal_runtime::ExecObservation::Exited(generation) => RuntimeObservation {
                    runtime_id: runtime_id.into(),
                    terminal: false,
                    status: "exited".into(),
                    exit_code: generation.exit_code.map(i64::from),
                    incarnation_id: Some(generation.generation_id),
                },
                fractal_runtime::ExecObservation::Indeterminate(reason) => RuntimeObservation {
                    runtime_id: runtime_id.into(),
                    terminal: false,
                    status: "indeterminate".into(),
                    exit_code: None,
                    incarnation_id: Some(reason),
                },
            }))
    }
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn a_running_pty_with_a_dead_pid_is_observed_as_vanished() {
        let observation = fractal_runtime::PtyObservation {
            name: "stale-worker".into(),
            status: "running".into(),
            exit_code: None,
            pid: Some(i32::MAX as u32),
            created_at: Some("2026-09-22T12:00:00.000Z".into()),
            display_name: None,
            tags: BTreeMap::new(),
        };

        assert_eq!(observed_pty_status(&observation), "vanished");
    }

    #[cfg(unix)]
    #[test]
    fn a_running_pty_with_a_live_pid_remains_running() {
        let observation = fractal_runtime::PtyObservation {
            name: "live-worker".into(),
            status: "running".into(),
            exit_code: None,
            pid: Some(std::process::id()),
            created_at: Some("2026-09-22T12:00:00.000Z".into()),
            display_name: None,
            tags: BTreeMap::new(),
        };

        assert_eq!(observed_pty_status(&observation), "running");
    }

    #[test]
    fn registry_dispatch_ignores_output_activity_and_unrelated_fleet_files() {
        let root = Path::new("/example/physical-registry");
        let changed = |path: &str| notify::Event::new(
            notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(PathBuf::from(path));
        for path in [
            "/example/graph.sqlite3",
            "/example/physical-registry/example.events.jsonl",
            "/example/physical-registry/.activity/example.json",
        ] {
            assert!(!registry_event_relevant(&changed(path), root), "{path}");
        }
        assert!(registry_event_relevant(
            &changed("/example/physical-registry/example.json"), root));
        assert!(registry_event_relevant(
            &notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::Folder))
                .add_path(PathBuf::from("/example")), root));
        assert!(registry_event_relevant(
            &notify::Event::new(notify::EventKind::Other)
                .set_flag(notify::event::Flag::Rescan), root));
    }

    #[tokio::test]
    async fn replaced_or_recreated_registry_rearms_native_observation_coverage() {
        for rename in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let registry = temporary.path().join("registry");
            std::fs::create_dir(&registry).unwrap();
            let paths = vec![registry.clone()];
            let observations = HostObservations::default();
            let notify = Arc::new(Notify::new());
            observations.watch(&paths, &notify).unwrap();
            let original_pool = observations.state.lock().unwrap().watch.as_ref().unwrap().0.clone();
            observations.watch(&paths, &notify).unwrap();
            assert!(Arc::ptr_eq(&original_pool,
                &observations.state.lock().unwrap().watch.as_ref().unwrap().0),
                "unchanged coverage is retained without reacquisition");

            if rename {
                std::fs::rename(&registry, temporary.path().join("previous")).unwrap();
            } else {
                std::fs::remove_dir(&registry).unwrap();
            }
            tokio::time::timeout(Duration::from_secs(5), async {
                while observations.coverage_revision.load(Ordering::Acquire) == 0 {
                    notify.notified().await;
                }
            }).await.expect("native removal/replacement must invalidate coverage");

            // A pass while the root is absent must move coverage to its existing
            // ancestor, so recreation is observable without polling the filesystem.
            observations.watch(&paths, &notify).unwrap();
            drop(original_pool);
            let before_recreation = observations.coverage_revision.load(Ordering::Acquire);
            std::fs::create_dir(&registry).unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while observations.coverage_revision.load(Ordering::Acquire) == before_recreation {
                    notify.notified().await;
                }
            }).await.expect("ancestor coverage must observe registry recreation");
            observations.watch(&paths, &notify).unwrap();
            let before_metadata = observations.revision.load(Ordering::Acquire);
            std::fs::write(registry.join("example.json"), b"{}").unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while observations.revision.load(Ordering::Acquire) == before_metadata {
                    notify.notified().await;
                }
            }).await.expect("new registry metadata must invalidate physical observations");
        }
    }
}
