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

    /// Record the destination before any server can leave its operation-owned work scope.
    /// Sidecars are immutable and independent of the worker's spawn lock.
    pub fn protect_servers(&self, observations: &[PtyObservation]) -> Result<()> {
        for observation in observations {
            let (Some(operation), Some(scope), Some(pid)) = (
                observation.tags.get("st3.start-operation"),
                observation.tags.get("st3.scope-unit"), observation.pid,
            ) else { continue; };
            let directory = self.producer_directory(&observation.name, operation);
            let owner = directory.join("owner.json");
            let ownership: ProducerOwnership = match File::open(&owner) {
                Ok(file) => serde_json::from_reader(file)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            anyhow::ensure!(ownership.version == 1 && ownership.runtime_id == observation.name
                && ownership.operation == *operation && ownership.scope == *scope,
                "PTY server ownership disagrees with its publication");
            let unit = crate::server_unit(&observation.name, pid);
            let path = directory.join(format!("server-{pid}.json"));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    serde_json::to_writer(&mut file, &unit)?;
                    file.sync_all()?;
                    File::open(&directory)?.sync_all()?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let recorded: String = serde_json::from_reader(File::open(&path)?)?;
                    anyhow::ensure!(recorded == unit, "PTY server scope ownership changed");
                }
                Err(error) => return Err(error.into()),
            }
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
        let Some(work) = crate::isolate::observe_closed_scope(&ownership.scope)? else {
            return Ok(false);
        };
        let mut proofs = vec![work];
        // Read after work closure: any server that escaped that scope already has
        // its durable sidecar, because recording precedes migration.
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue; };
            if !name.starts_with("server-") || !name.ends_with(".json") { continue; }
            let unit: String = serde_json::from_reader(File::open(entry.path())?)?;
            let Some(proof) = crate::isolate::observe_closed_scope(&unit)? else {
                return Ok(false);
            };
            proofs.push(proof);
        }
        // Retire only this operation's publication record; never the runtime's
        // legacy pending fence, which might now belong to another operation.
        ownership.publication_pending = false;
        durable_json(&directory.join("closure.json"), &proofs)?;
        durable_json(&path, &ownership)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
