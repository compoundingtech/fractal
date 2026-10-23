//! Read-only legacy closure capture. The spawn lock remains owned by the returned proof.
use super::*;
#[cfg(target_os = "linux")]
use anyhow::ensure;

/// Stable physical facts, not an authorization token. Only a held
/// [`LegacyStartClosureProof`] binds these facts to the runtime spawn fence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyStartClosureEvidence {
    pub version: u8,
    pub runtime_id: String,
    pub operation_id: String,
    pub runtime_root: PathBuf,
    pub owner_uid: u32,
    pub row_status: String,
    pub row_pid: Option<u32>,
    pub row_created_at: Option<String>,
    pub row_tags: BTreeMap<String, String>,
    pub row_operation_tag: Option<String>,
    pub row_fractal_operation_tag: Option<String>,
    pub operation_owner_absent: bool,
    pub manager_enumeration_complete: bool,
    pub matching_units: Vec<String>,
    pub recorded_scopes: Vec<String>,
    pub recorded_scopes_not_loaded: bool,
    pub process_enumeration_complete: bool,
    pub operation_pids: Vec<u32>,
}

/// The private file owns the flock until the Store writer has consumed the evidence.
#[derive(Debug)]
pub struct LegacyStartClosureProof {
    pub evidence: LegacyStartClosureEvidence,
    _spawn_lock: File,
}

#[cfg(target_os = "linux")]
impl PtyRuntime {
    fn legacy_operation_owner_absent(&self, id: &str, operation: &str) -> Result<bool> {
        let digest = Sha256::digest(operation.as_bytes());
        // Even an interrupted native prepare (without owner.json yet) is not
        // legacy evidence. Refuse every operation-specific producer entry.
        let directory = self.spawn_state_path(id, "producers")
            .join(format!("{digest:x}"));
        match std::fs::symlink_metadata(&directory) {
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error).context("inspect operation producer directory"),
        }
    }
}

impl PtyRuntime {
    /// Positively prove an old, unowned start is physically closed. No process or
    /// scope is stopped. Unknown/unreadable native state fails closed. Call again
    /// on apply (after dropping a dry-run proof) and compare the stable evidence;
    /// retain the apply proof through the Store transaction.
    pub fn legacy_start_closure(
        &self,
        id: &str,
        operation: &str,
    ) -> Result<Option<LegacyStartClosureProof>> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (id, operation);
            Ok(None)
        }
        #[cfg(target_os = "linux")]
        {
            if crate::isolation_mode() != crate::Isolation::Scope || operation.is_empty() {
                return Ok(None);
            }
            let Some(lock) = self.try_spawn_lock(id)? else {
                return Ok(None);
            };
            let Some(row) = self.snapshot()?.into_iter().find(|row| row.name == id) else {
                return Ok(None);
            };
            if row.status != "vanished" || row.pid.is_some() {
                return Ok(None);
            }
            if !self.legacy_operation_owner_absent(id, operation)? {
                return Ok(None);
            }
            let mut recorded_scopes = BTreeSet::new();
            match std::fs::read_to_string(self.spawn_state_path(id, "scope")) {
                Ok(scope) => {
                    ensure!(!scope.trim().is_empty(), "recorded scope is empty");
                    recorded_scopes.insert(scope.trim().to_owned());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("read recorded legacy scope"),
            }
            if let Some(scope) = row.tags.get("st3.scope-unit") {
                recorded_scopes.insert(scope.clone());
            }
            let matching_units = matching_units(id, &manager_units()?)?;
            if !matching_units.is_empty() {
                return Ok(None);
            }
            for scope in &recorded_scopes {
                if !scope_not_loaded(scope)? {
                    return Ok(None);
                }
            }
            let owner_uid = unsafe { libc::geteuid() };
            use std::os::unix::fs::MetadataExt as _;
            let runtime_root = self.root.canonicalize().context("resolve legacy PTY root")?;
            ensure!(std::fs::metadata(&runtime_root)?.uid() == owner_uid,
                "legacy PTY root is owned by another host user");
            let operation_pids = operation_processes(owner_uid, operation)?;
            let evidence = LegacyStartClosureEvidence {
                version: 1,
                runtime_id: id.into(),
                operation_id: operation.into(),
                runtime_root,
                owner_uid,
                row_status: row.status,
                row_pid: row.pid,
                row_operation_tag: row.tags.get("st3.start-operation").cloned(),
                row_fractal_operation_tag: row.tags.get("fractal.start-operation").cloned(),
                row_created_at: row.created_at,
                row_tags: row.tags,
                operation_owner_absent: true,
                manager_enumeration_complete: true,
                matching_units,
                recorded_scopes: recorded_scopes.into_iter().collect(),
                recorded_scopes_not_loaded: true,
                process_enumeration_complete: true,
                operation_pids,
            };
            Ok(positive(&evidence).then_some(LegacyStartClosureProof {
                evidence,
                _spawn_lock: lock,
            }))
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn positive(facts: &LegacyStartClosureEvidence) -> bool {
    facts.version == 1 && !facts.operation_id.is_empty()
        && facts.row_status == "vanished" && facts.row_pid.is_none()
        && facts.operation_owner_absent && facts.manager_enumeration_complete
        && facts.matching_units.is_empty() && facts.recorded_scopes_not_loaded
        && facts.process_enumeration_complete && facts.operation_pids.is_empty()
}

#[cfg(target_os = "linux")]
#[derive(Deserialize)]
struct ManagerUnit {
    unit: String,
    load: String,
    active: String,
    sub: String,
}

#[cfg(target_os = "linux")]
fn manager_units() -> Result<Vec<ManagerUnit>> {
    let mut command = crate::isolate::systemctl();
    command.args(["--user", "list-units", "--all", "--full", "--no-pager", "--output=json"]);
    let output = output_within(command, PTY_COMMAND_TIMEOUT)?;
    ensure!(output.status.success(), "legacy scope manager enumeration failed");
    serde_json::from_slice(&output.stdout).context("read complete legacy scope enumeration")
}

#[cfg(target_os = "linux")]
fn matching_units(id: &str, units: &[ManagerUnit]) -> Result<Vec<String>> {
    let safe = crate::isolate::sanitize(id);
    let prefixes = [format!("st3-{safe}-"), format!("st3-work-{safe}-"),
        format!("st3-pty-server-{safe}-")];
    let mut matching = BTreeSet::new();
    for unit in units {
        ensure!(!unit.unit.is_empty() && !unit.load.is_empty()
            && !unit.active.is_empty() && !unit.sub.is_empty(), "incomplete manager unit");
        if prefixes.iter().any(|prefix| unit.unit.starts_with(prefix)) {
            matching.insert(unit.unit.clone());
        }
    }
    Ok(matching.into_iter().collect())
}

#[cfg(target_os = "linux")]
fn scope_not_loaded(scope: &str) -> Result<bool> {
    ensure!(scope.ends_with(".scope") && scope.len() <= 256
        && scope.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
        "invalid recorded legacy scope");
    let mut command = crate::isolate::systemctl();
    command.args(["--user", "show", "--property=LoadState", "--", scope]);
    let output = output_within(command, PTY_COMMAND_TIMEOUT)?;
    ensure!(output.status.success(), "recorded scope manager query failed");
    let text = std::str::from_utf8(&output.stdout)?;
    match text.trim() {
        "LoadState=not-found" => Ok(true),
        "LoadState=loaded" => Ok(false),
        _ => anyhow::bail!("recorded scope load state is unknown"),
    }
}

#[cfg(target_os = "linux")]
fn operation_processes(owner_uid: u32, operation: &str) -> Result<Vec<u32>> {
    use std::os::unix::fs::MetadataExt as _;
    let mut matching = BTreeSet::new();
    for entry in std::fs::read_dir("/proc").context("enumerate legacy operation processes")? {
        let entry = entry?;
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        // Pin the proc directory: a recycled PID must never substitute a different
        // process between ownership, environment and exit checks.
        let directory = match File::open(entry.path()) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("open proc process directory"),
        };
        let path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let status = match std::fs::read_to_string(path.join("status")) {
            Ok(status) => status,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("read process ownership"),
        };
        let uids = status.lines().find_map(|line| line.strip_prefix("Uid:"))
            .context("process ownership is missing")?.split_whitespace()
            .map(str::parse::<u32>).collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(uids.len() == 4, "process ownership is incomplete");
        if !uids.contains(&owner_uid) && directory.metadata()?.uid() != owner_uid {
            continue;
        }
        if owner_process_has_operation(&path, operation)? {
            matching.insert(pid);
        }
    }
    Ok(matching.into_iter().collect())
}

#[cfg(target_os = "linux")]
fn owner_process_has_operation(path: &Path, operation: &str) -> Result<bool> {
    match std::fs::read(path.join("environ")) {
        Ok(environment) => Ok(has_operation(&environment, operation) && !process_exited(path)?),
        Err(error) => {
            if process_exited(path)? {
                Ok(false)
            } else {
                Err(error).context("read owner process environment")
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn process_exited(directory: &Path) -> Result<bool> {
    match std::fs::read_to_string(directory.join("stat")) {
        Ok(stat) => {
            let (_, tail) = stat.rsplit_once(") ").context("malformed process stat")?;
            let state = tail.split_whitespace().next().context("missing process state")?;
            Ok(matches!(state, "Z" | "X" | "x"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error).context("verify owner process exit"),
    }
}

#[cfg(any(target_os = "linux", test))]
fn has_operation(environment: &[u8], operation: &str) -> bool {
    environment.split(|byte| *byte == 0).any(|entry| {
        [b"ST3_START_OPERATION=".as_slice(), b"FRACTAL_START_OPERATION=".as_slice()]
            .iter().any(|prefix| entry.strip_prefix(*prefix) == Some(operation.as_bytes()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed() -> LegacyStartClosureEvidence {
        LegacyStartClosureEvidence {
            version: 1, runtime_id: "work".into(), operation_id: "operation".into(),
            runtime_root: PathBuf::from("/runtime/pty"),
            owner_uid: 1000, row_status: "vanished".into(), row_pid: None,
            row_created_at: Some("2026-10-23T00:00:00Z".into()),
            row_tags: BTreeMap::from([("st3.start-operation".into(), "older-operation".into())]),
            row_operation_tag: Some("older-operation".into()), row_fractal_operation_tag: None,
            operation_owner_absent: true, manager_enumeration_complete: true,
            matching_units: vec![], recorded_scopes: vec!["st3-work-1.scope".into()],
            recorded_scopes_not_loaded: true, process_enumeration_complete: true,
            operation_pids: vec![],
        }
    }

    #[test]
    fn positive_requires_every_physical_fact_not_the_operation_tag() {
        let facts = closed();
        assert!(positive(&facts));
        let encoded = serde_json::to_vec(&facts).unwrap();
        assert_eq!(facts, serde_json::from_slice::<LegacyStartClosureEvidence>(&encoded).unwrap());
        assert_eq!(encoded, serde_json::to_vec(&closed()).unwrap());
        let changes: &[fn(&mut LegacyStartClosureEvidence)] = &[
            |f| f.version = 0, |f| f.operation_id.clear(), |f| f.row_status = "unknown".into(),
            |f| f.row_status = "running".into(), |f| f.row_status = "exited".into(),
            |f| f.row_pid = Some(1), |f| f.operation_owner_absent = false,
            |f| f.manager_enumeration_complete = false,
            |f| f.matching_units.push("st3-work-1.scope".into()),
            |f| f.recorded_scopes_not_loaded = false,
            |f| f.process_enumeration_complete = false, |f| f.operation_pids.push(1),
        ];
        for change in changes {
            let mut missing = facts.clone();
            change(&mut missing);
            assert!(!positive(&missing), "{missing:?}");
        }
    }

    #[test]
    fn operation_environment_match_is_exact_and_supports_both_legacy_names() {
        assert!(has_operation(b"ST3_START_OPERATION=op\0", "op"));
        assert!(has_operation(b"FRACTAL_START_OPERATION=op\0", "op"));
        assert!(!has_operation(b"ST3_START_OPERATION=op-other\0", "op"));
        assert!(!has_operation(b"OTHER_ST3_START_OPERATION=op\0", "op"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn complete_manager_parser_refuses_missing_fields_and_matches_all_runtime_units() {
        assert!(serde_json::from_str::<Vec<ManagerUnit>>(r#"[{"unit":"st3-work-1.scope"}]"#).is_err());
        let units = ["st3-work-1.scope", "st3-pty-server-work-2.scope", "st3-work-work-3.scope", "unrelated.scope"]
            .map(|unit| ManagerUnit { unit: unit.into(), load: "loaded".into(), active: "inactive".into(), sub: "dead".into() });
        assert_eq!(matching_units("work", &units).unwrap().len(), 3);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_proc_scan_finds_current_process_exact_label() {
        let variable = "FRACTAL_START_OPERATION";
        let operation = "fractal-runtime-legacy-proof-native-test";
        let mut child = Command::new("sleep").arg("60").env(variable, operation).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = loop {
            match operation_processes(unsafe { libc::geteuid() }, operation) {
                Ok(pids) if !pids.contains(&child.id()) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => break result,
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        assert!(result.unwrap().contains(&child.id()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unreadable_environment_requires_positive_exit_evidence() {
        let proc = tempfile::tempdir().unwrap();
        std::fs::write(proc.path().join("stat"), "123 (producer) S 1 2 3").unwrap();
        assert!(owner_process_has_operation(proc.path(), "op").is_err());
        std::fs::write(proc.path().join("stat"), "123 (producer) Z 1 2 3").unwrap();
        assert!(!owner_process_has_operation(proc.path(), "op").unwrap());
        std::fs::write(proc.path().join("stat"), "unreadable evidence").unwrap();
        assert!(owner_process_has_operation(proc.path(), "op").is_err());
        std::fs::remove_file(proc.path().join("stat")).unwrap();
        assert!(!owner_process_has_operation(proc.path(), "op").unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_operation_owner_layout_excludes_legacy_proof() {
        let root = tempfile::tempdir().unwrap();
        let runtime = PtyRuntime::new(root.path().join("pty"));
        assert!(runtime.legacy_operation_owner_absent("work", "operation").unwrap());
        let digest = Sha256::digest(b"operation");
        let directory = runtime.spawn_state_path("work", "producers").join(format!("{digest:x}"));
        std::fs::create_dir_all(&directory).unwrap();
        assert!(!runtime.legacy_operation_owner_absent("work", "operation").unwrap());
        std::fs::write(directory.join("owner.next"), "{}").unwrap();
        assert!(!runtime.legacy_operation_owner_absent("work", "operation").unwrap());
        std::fs::write(directory.join("owner.json"), "{}").unwrap();
        assert!(!runtime.legacy_operation_owner_absent("work", "operation").unwrap());
        assert!(runtime.legacy_operation_owner_absent("other-work", "operation").unwrap());
        assert!(runtime.legacy_operation_owner_absent("work", "other-operation").unwrap());
    }

    #[test]
    fn row_incarnation_and_full_tags_change_stable_evidence() {
        let before = closed();
        let mut replacement = before.clone();
        replacement.row_created_at = Some("2026-10-23T00:01:00Z".into());
        assert_ne!(serde_json::to_vec(&before).unwrap(), serde_json::to_vec(&replacement).unwrap());
        let mut retagged = before.clone();
        retagged.row_tags.insert("another-tag".into(), "changed".into());
        assert_ne!(serde_json::to_vec(&before).unwrap(), serde_json::to_vec(&retagged).unwrap());
    }

    #[test]
    fn proof_retains_spawn_lock_until_dropped() {
        let root = tempfile::tempdir().unwrap();
        let runtime = PtyRuntime::new(root.path().join("pty"));
        let lock = runtime.try_spawn_lock("work").unwrap().unwrap();
        let proof = LegacyStartClosureProof { evidence: closed(), _spawn_lock: lock };
        assert!(runtime.try_spawn_lock("work").unwrap().is_none());
        drop(proof);
        assert!(runtime.try_spawn_lock("work").unwrap().is_some());
    }
}
