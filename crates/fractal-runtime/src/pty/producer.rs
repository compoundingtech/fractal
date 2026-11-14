//! Durable ownership, not registry absence, closes a native PTY producer.
use super::*;
use std::io::Write as _;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProducerOwnership {
    version: u8,
    runtime_id: String,
    operation: String,
    scope: String,
    // Set false durably before each launch. Only a returned launcher acknowledges it.
    launcher_acknowledged: bool,
    publication_pending: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ServerOwnership {
    version: u8,
    unit: String,
    pid: u32,
    start_token: u64,
    boot_id: String,
}

// Existing sidecars carry scope ownership only. They may still be closed by
// scope evidence, but never by a PID observation without generation evidence.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ServerRecord {
    Identity(ServerOwnership),
    Scope(String),
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ClosureReason {
    ScopesClosed,
    RuntimeVanished,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProducerClosure<S> {
    version: u8,
    reason: ClosureReason,
    publication_pending: bool,
    scopes: S,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ClosureRecord {
    Current(ProducerClosure<Vec<crate::ScopeRetirementProof>>),
    Legacy(Vec<crate::ScopeRetirementProof>),
}

fn close_producer(
    directory: &Path, ownership: &mut ProducerOwnership,
    scopes: &[crate::ScopeRetirementProof], reason: ClosureReason,
) -> Result<()> {
    // The atomic closure record is the authority. A crash before the owner
    // update leaves a conservative pending owner and is repaired on the next pass.
    durable_json(&directory.join("closure.json"), &ProducerClosure {
        version: 1, reason, publication_pending: false, scopes,
    })?;
    ownership.publication_pending = false;
    durable_json(&directory.join("owner.json"), ownership)
}

fn durable_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("next");
    let mut file = File::create(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    File::open(path.parent().context("producer record has no parent")?)?.sync_all()?;
    Ok(())
}

impl PtyRuntime {
    fn producer_directory(&self, id: &str, operation: &str) -> PathBuf {
        let digest = Sha256::digest(operation.as_bytes());
        self.spawn_state_path(id, "producers").join(format!("{digest:x}"))
    }

    pub(super) fn prepare_producer(
        &self, id: &str, operation: Option<&String>, scope: &str,
    ) -> Result<Option<ProducerOwnership>> {
        let Some(operation) = operation.filter(|_| crate::isolation_mode() == crate::Isolation::Scope) else {
            return Ok(None);
        };
        let directory = self.producer_directory(id, operation);
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("owner.json");
        // Replaying the same operation must never overwrite ambiguous ownership.
        anyhow::ensure!(!path.try_exists()?, "PTY operation already has producer ownership");
        let ownership = ProducerOwnership {
            version: 1, runtime_id: id.into(), operation: operation.clone(), scope: scope.into(),
            launcher_acknowledged: true, publication_pending: true,
        };
        durable_json(&path, &ownership)?;
        File::open(directory.parent().context("producer directory has no parent")?)?.sync_all()?;
        Ok(Some(ownership))
    }

    pub(super) fn acknowledge_launcher(
        &self, ownership: &mut Option<ProducerOwnership>, acknowledged: bool,
    ) -> Result<()> {
        if let Some(ownership) = ownership {
            ownership.launcher_acknowledged = acknowledged;
            durable_json(&self.producer_directory(&ownership.runtime_id, &ownership.operation)
                .join("owner.json"), ownership)?;
        }
        Ok(())
    }

    /// Sidecars are immutable and independent of the worker's spawn lock.
    pub(super) fn record_server_ownership(&self, observation: &PtyObservation) -> Result<()> {
        let (Some(operation), Some(scope), Some(pid)) = (
            observation.tags.get("st3.start-operation"),
            observation.tags.get("st3.scope-unit"), observation.pid,
        ) else { return Ok(()); };
        let directory = self.producer_directory(&observation.name, operation);
        let owner = directory.join("owner.json");
        let ownership: ProducerOwnership = match File::open(&owner) {
            Ok(file) => serde_json::from_reader(file)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(ownership.version == 1 && ownership.runtime_id == observation.name
            && ownership.operation == *operation && ownership.scope == *scope,
            "PTY server ownership disagrees with its publication");
        let unit = crate::server_unit(&observation.name, pid);
        let path = directory.join(format!("server-{pid}.json"));
        let record = ServerOwnership {
            version: 1, unit, pid,
            start_token: crate::process_start_token(pid)?,
            boot_id: pty_core::registry::read_boot_id().context("server boot identity unavailable")?,
        };
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                serde_json::to_writer(&mut file, &record)?;
                file.sync_all()?;
                File::open(&directory)?.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let recorded: ServerRecord = serde_json::from_reader(File::open(&path)?)?;
                let recorded_unit = match &recorded {
                    ServerRecord::Identity(server) => &server.unit,
                    ServerRecord::Scope(unit) => unit,
                };
                anyhow::ensure!(*recorded_unit == record.unit, "PTY server scope ownership changed");
                if let ServerRecord::Identity(recorded) = &recorded {
                    anyhow::ensure!(recorded.version == record.version && recorded.pid == record.pid
                        && recorded.start_token == record.start_token && recorded.boot_id == record.boot_id,
                        "PTY server generation ownership changed");
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// Record every destination before any server can leave its work scope.
    pub fn protect_servers(&self, observations: &[PtyObservation]) -> Result<()> {
        for observation in observations {
            self.record_server_ownership(observation)?;
        }
        crate::protect_servers(observations);
        Ok(())
    }

    /// Legacy, detached and crash-before-ack launches deliberately have no proof.
    /// A free lock or absent scope alone can never authorize this transition.
    pub fn start_operation_closed(&self, id: &str, operation: &str) -> Result<bool> {
        if crate::isolation_mode() != crate::Isolation::Scope {
            return Ok(false);
        }
        let Some(_lock) = self.try_spawn_lock(id)? else { return Ok(false); };
        let directory = self.producer_directory(id, operation);
        let path = directory.join("owner.json");
        let mut ownership: ProducerOwnership = match File::open(&path) {
            Ok(file) => serde_json::from_reader(file)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(ownership.version == 1 && ownership.runtime_id == id
            && ownership.operation == operation, "PTY producer ownership changed");
        if !ownership.launcher_acknowledged { return Ok(false); }
        self.reconcile_producer(&directory, &mut ownership, crate::isolate::observe_closed_scope)
    }

    fn reconcile_producer(
        &self, directory: &Path, ownership: &mut ProducerOwnership,
        observe_scope: impl Fn(&str) -> Result<Option<crate::ScopeRetirementProof>>,
    ) -> Result<bool> {
        let Some(work) = observe_scope(&ownership.scope)? else { return Ok(false); };
        let mut proofs = vec![work];
        let mut vanished = false;
        // Validate every sidecar before writing anything. Work closure and the
        // spawn lock exclude a launcher still publishing another server.
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("non-UTF8 producer entry")?;
            if name == "owner.json" { continue; }
            if name == "closure.json" {
                let closure: ClosureRecord = serde_json::from_reader(File::open(entry.path())?)?;
                let scopes = match closure {
                    ClosureRecord::Current(closure) => {
                        anyhow::ensure!(closure.version == 1 && !closure.publication_pending,
                            "invalid producer closure");
                        closure.scopes
                    }
                    ClosureRecord::Legacy(scopes) => scopes,
                };
                anyhow::ensure!(!scopes.is_empty(), "empty producer closure");
                continue;
            }
            let pid: u32 = name.strip_prefix("server-")
                .and_then(|name| name.strip_suffix(".json"))
                .context("partial or unknown producer entry")?.parse()?;
            anyhow::ensure!(pid > 0, "invalid server PID");
            let record: ServerRecord = serde_json::from_reader(File::open(entry.path())?)?;
            let unit = match &record {
                ServerRecord::Identity(server) => {
                    anyhow::ensure!(server.version == 1 && server.pid == pid
                        && server.start_token > 0 && !server.boot_id.is_empty(),
                        "invalid server identity");
                    // A boot mismatch is deliberately unknown, not PID absence.
                    anyhow::ensure!(Some(&server.boot_id) == pty_core::registry::read_boot_id().as_ref(),
                        "server boot identity unavailable");
                    if crate::process_generation_alive(pid, server.start_token)? {
                        return Ok(false);
                    }
                    vanished = true;
                    &server.unit
                }
                ServerRecord::Scope(unit) => {
                    // Old sidecars cannot distinguish a reused PID. Refuse
                    // every present process and require scope proof as well.
                    #[cfg(target_os = "linux")]
                    match crate::process::process_identity(pid) {
                        Ok(_) => return Ok(false),
                        Err(error) if error.downcast_ref::<std::io::Error>()
                            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
                        Err(error) => return Err(error),
                    }
                    #[cfg(not(target_os = "linux"))]
                    return Ok(false);
                    #[cfg(target_os = "linux")]
                    unit
                }
            };
            anyhow::ensure!(*unit == crate::server_unit(&ownership.runtime_id, pid),
                "server scope identity changed");
            let Some(proof) = observe_scope(unit)? else { return Ok(false); };
            proofs.push(proof);
        }
        close_producer(directory, ownership, &proofs,
            if vanished { ClosureReason::RuntimeVanished } else { ClosureReason::ScopesClosed })?;
        Ok(true)
    }

    /// Explicit daemon-startup mutation, including already-applied starts that
    /// no longer enter pending-start cleanup. Never call from identityless reads.
    pub fn reconcile_producers_at_daemon_startup(&self) -> Result<()> {
        let roots = match std::fs::read_dir(self.spawn_state_directory()) {
            Ok(roots) => roots,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for root in roots {
            let root = root?;
            if !root.file_name().to_string_lossy().ends_with(".producers") { continue; }
            for entry in std::fs::read_dir(root.path())? {
                let directory = entry?.path();
                let result = (|| -> Result<()> {
                    let owner: ProducerOwnership =
                        serde_json::from_reader(File::open(directory.join("owner.json"))?)?;
                    anyhow::ensure!(directory == self.producer_directory(&owner.runtime_id, &owner.operation),
                        "producer directory identity changed");
                    if owner.publication_pending {
                        self.start_operation_closed(&owner.runtime_id, &owner.operation)?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    tracing::warn!(path = %directory.display(), error = %error,
                        "refusing ambiguous PTY producer reconciliation");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    fn startup_fixture(runtime: &PtyRuntime, operation: &str, pid: u32, token: u64) -> PathBuf {
        let directory = runtime.producer_directory("worker", operation);
        std::fs::create_dir_all(&directory).unwrap();
        durable_json(&directory.join("owner.json"), &ProducerOwnership {
            version: 1, runtime_id: "worker".into(), operation: operation.into(),
            scope: crate::scope_unit("st3", operation),
            launcher_acknowledged: true, publication_pending: true,
        }).unwrap();
        durable_json(&directory.join(format!("server-{pid}.json")), &ServerOwnership {
            version: 1, unit: crate::server_unit("worker", pid), pid, start_token: token,
            boot_id: pty_core::registry::read_boot_id().unwrap(),
        }).unwrap();
        directory
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn constructing_runtime_is_read_only_until_daemon_startup_hook() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("requires real systemd user scope support");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = PtyRuntime::new(root.path().join("registry"));
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        let token = crate::process_start_token(pid).unwrap();
        child.wait().unwrap();
        let directory = startup_fixture(&runtime, "vanished-start", pid, token);
        assert!(!directory.join("closure.json").exists());
        let original_owner = std::fs::read(directory.join("owner.json")).unwrap();
        let restarted = PtyRuntime::new(root.path().join("registry"));
        assert!(!directory.join("closure.json").exists(), "constructing a runtime must not finalize a producer");
        assert_eq!(std::fs::read(directory.join("owner.json")).unwrap(), original_owner);
        restarted.reconcile_producers_at_daemon_startup().unwrap();
        let closure: serde_json::Value = serde_json::from_reader(
            File::open(directory.join("closure.json")).expect("startup must finalize the vanished producer")).unwrap();
        assert_eq!(closure["reason"], "runtime-vanished");
        assert_eq!(closure["publication_pending"], false);
        let owner: ProducerOwnership =
            serde_json::from_reader(File::open(directory.join("owner.json")).unwrap()).unwrap();
        assert!(!owner.publication_pending);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_closes_only_the_vanished_generation_of_a_reused_session_pid() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("requires real systemd user scope support");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = PtyRuntime::new(root.path().join("registry"));
        let pid = std::process::id();
        let token = crate::process_start_token(pid).unwrap();
        assert!(token > 1);
        // The same session name and reused PID now belong to another start.
        let old = startup_fixture(&runtime, "older-start", pid, token - 1);
        let newer = startup_fixture(&runtime, "newer-start", pid, token);
        let newer_owner = std::fs::read(newer.join("owner.json")).unwrap();
        let newer_server = std::fs::read(newer.join(format!("server-{pid}.json"))).unwrap();
        let restarted = PtyRuntime::new(root.path().join("registry"));
        restarted.reconcile_producers_at_daemon_startup().unwrap();
        assert!(old.join("closure.json").exists(), "the old generation must be finalized");
        let owner: ProducerOwnership =
            serde_json::from_reader(File::open(old.join("owner.json")).unwrap()).unwrap();
        assert!(!owner.publication_pending);
        assert!(!newer.join("closure.json").exists(), "the live generation must remain open");
        assert_eq!(std::fs::read(newer.join("owner.json")).unwrap(), newer_owner);
        assert_eq!(std::fs::read(newer.join(format!("server-{pid}.json"))).unwrap(), newer_server);
        assert!(crate::process_generation_alive(pid, token).unwrap());
    }

    #[test]
    fn startup_refuses_partial_or_corrupt_ownership_without_effects() {
        for name in ["owner.json", "owner.next"] {
            let root = tempfile::tempdir().unwrap();
            let runtime = PtyRuntime::new(root.path().join("registry"));
            let directory = runtime.producer_directory("worker", "op");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(name), b"{").unwrap();
            let restarted = PtyRuntime::new(root.path().join("registry"));
            restarted.reconcile_producers_at_daemon_startup().unwrap();
            assert_eq!(std::fs::read(directory.join(name)).unwrap(), b"{");
            assert!(!directory.join("closure.json").exists());
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn vanished_producer_reconciliation_is_conservative_and_idempotent() {
        let mut child = Command::new("true").spawn().unwrap();
        let dead_pid = child.id();
        let dead_token = crate::process_start_token(dead_pid).unwrap();
        child.wait().unwrap();
        let live_pid = std::process::id();
        let live_token = crate::process_start_token(live_pid).unwrap();
        for (pid, token, invalid, closes) in [
            (dead_pid, dead_token, "", true),
            (live_pid, live_token, "", false),
            (live_pid, live_token + 1, "", true),
            (dead_pid, dead_token, "corrupt", false),
            (dead_pid, dead_token, "partial", false),
            (dead_pid, dead_token, "unknown-boot", false),
            (dead_pid, dead_token, "corrupt-closure", false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let runtime = PtyRuntime::new(root.path().join("registry"));
            let mut owner = ProducerOwnership {
                version: 1, runtime_id: "worker".into(), operation: "op".into(),
                scope: "work.scope".into(), launcher_acknowledged: true,
                publication_pending: true,
            };
            let directory = runtime.producer_directory("worker", "op");
            std::fs::create_dir_all(&directory).unwrap();
            durable_json(&directory.join("owner.json"), &owner).unwrap();
            let original = std::fs::read(directory.join("owner.json")).unwrap();
            let server = directory.join(format!("server-{pid}.json"));
            durable_json(&server, &ServerRecord::Identity(ServerOwnership {
                version: 1, unit: crate::server_unit("worker", pid), pid,
                start_token: token,
                boot_id: if invalid == "unknown-boot" { "unknown".into() }
                    else { pty_core::registry::read_boot_id().unwrap() },
            })).unwrap();
            if invalid == "corrupt" { std::fs::write(&server, b"{").unwrap(); }
            if invalid == "partial" {
                std::fs::write(directory.join("server-1.next"), b"{").unwrap();
            }
            if invalid == "corrupt-closure" {
                std::fs::write(directory.join("closure.json"), b"[{}]").unwrap();
            }
            let closed_scope = |unit: &str| Ok(Some(crate::ScopeRetirementProof {
                version: 1, unit: unit.into(), load_state: "not-found".into(),
                active_state: "inactive".into(), control_group: String::new(), cgroup: None,
            }));
            let result = runtime.reconcile_producer(&directory, &mut owner, closed_scope);
            assert_eq!(result.unwrap_or(false), closes, "{invalid}");
            assert_eq!(directory.join("closure.json").exists(), closes || invalid == "corrupt-closure", "{invalid}");
            if closes {
                let closure: serde_json::Value = serde_json::from_reader(
                    File::open(directory.join("closure.json")).unwrap()).unwrap();
                assert_eq!(closure["reason"], "runtime-vanished");
                assert_eq!(closure["publication_pending"], false);
                assert!(!owner.publication_pending);
                let persisted: ProducerOwnership = serde_json::from_reader(
                    File::open(directory.join("owner.json")).unwrap()).unwrap();
                assert!(!persisted.publication_pending);
                let before = std::fs::read(directory.join("closure.json")).unwrap();
                assert!(runtime.reconcile_producer(&directory, &mut owner, closed_scope).unwrap());
                assert_eq!(std::fs::read(directory.join("closure.json")).unwrap(), before);
            } else {
                assert_eq!(std::fs::read(directory.join("owner.json")).unwrap(), original);
                assert!(owner.publication_pending);
                if invalid == "corrupt-closure" {
                    assert_eq!(std::fs::read(directory.join("closure.json")).unwrap(), b"[{}]");
                }
            }
        }
    }

    #[test]
    fn legacy_absence_has_no_native_producer_closure_proof() {
        let root = tempfile::tempdir().unwrap();
        let runtime = PtyRuntime::new(root.path().join("registry"));
        assert!(!runtime.start_operation_closed("example-worker", "legacy-operation").unwrap());
    }

    #[test]
    fn scoped_producer_requires_durable_launcher_ack_and_free_spawn_lock() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("requires real systemd user scope support");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = PtyRuntime::new(root.path().join("registry"));
        let id = "example-worker";
        let operation = "example-operation".to_owned();
        let unit = crate::scope_unit("st3", "example-operation");
        let mut ownership = runtime.prepare_producer(id, Some(&operation), &unit).unwrap();
        let lock = runtime.acquire_spawn_lock(id).unwrap();
        assert!(!runtime.start_operation_closed(id, &operation).unwrap());
        runtime.acknowledge_launcher(&mut ownership, false).unwrap();
        drop(lock);
        // A missing scope and free spawn lock cannot close a crash-before-ack.
        let error = runtime.spawn_checked(
            id, &Launch::Argv(vec!["true".into()]), root.path(),
            &BTreeMap::from([("ST3_START_OPERATION".into(), operation.clone())]),
            None, &BTreeMap::from([("st3.start-operation".into(), operation.clone())]),
            None, &|| Ok(()),
        ).unwrap_err();
        assert!(error.downcast_ref::<crate::StartRejected>().is_none(),
            "refusing ambiguous ownership replay is not a native rejection witness");
        assert!(!runtime.start_operation_closed(id, &operation).unwrap());
        assert!(!runtime.start_operation_closed(id, "other-operation").unwrap());
        let record = runtime.producer_directory(id, &operation).join("owner.json");
        let pending: ProducerOwnership = serde_json::from_reader(File::open(&record).unwrap()).unwrap();
        assert!(pending.publication_pending);
        runtime.acknowledge_launcher(&mut ownership, true).unwrap();
        assert!(runtime.start_operation_closed(id, &operation).unwrap());
        let closed: ProducerOwnership = serde_json::from_reader(File::open(record).unwrap()).unwrap();
        assert!(!closed.publication_pending);
        assert!(runtime.producer_directory(id, &operation).join("closure.json").exists());
    }
}
