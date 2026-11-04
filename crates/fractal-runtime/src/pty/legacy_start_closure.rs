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

/// Nonsecret metadata for a process whose absence could not be proved.
/// Permission errors remain refusals, never evidence that a producer is absent.
#[derive(Debug)]
pub struct LegacyStartProofUnavailable {
    pub stage: &'static str,
    pub pid: Option<u32>,
    pub unit: Option<String>,
    pub errno: Option<i32>,
}

impl std::fmt::Display for LegacyStartProofUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "legacy start proof unavailable at {} (pid {:?}, unit {:?})", self.stage, self.pid, self.unit)
    }
}

impl std::error::Error for LegacyStartProofUnavailable {}

/// A complete process inventory cannot be established from this procfs mount.
#[derive(Debug)]
pub struct ProcfsHidesProcesses {
    pub detail: &'static str,
    pub mount_options: Option<String>,
}

impl std::fmt::Display for ProcfsHidesProcesses {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "procfs visibility unavailable: {} ({:?})", self.detail, self.mount_options)
    }
}

impl std::error::Error for ProcfsHidesProcesses {}

#[cfg(target_os = "linux")]
fn procfs_visibility_error(detail: &'static str) -> ProcfsHidesProcesses {
    ProcfsHidesProcesses { detail, mount_options: None }
}

#[cfg(target_os = "linux")]
fn validate_procfs_mount(mountinfo: std::io::Result<String>, mount_id: u64) -> std::result::Result<(), ProcfsHidesProcesses> {
    let mountinfo = mountinfo.map_err(|_| procfs_visibility_error("mountinfo-unreadable"))?;
    let mut selected = None;
    for line in mountinfo.lines() {
        let (mount, filesystem) = line.split_once(" - ")
            .ok_or_else(|| procfs_visibility_error("mountinfo-invalid"))?;
        let mut fields = mount.split_whitespace();
        let id = fields.next().and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| procfs_visibility_error("mountinfo-invalid"))?;
        let options = fields.nth(4).ok_or_else(|| procfs_visibility_error("mountinfo-invalid"))?;
        let mut filesystem = filesystem.split_whitespace();
        let kind = filesystem.next().ok_or_else(|| procfs_visibility_error("mountinfo-invalid"))?;
        filesystem.next().ok_or_else(|| procfs_visibility_error("mountinfo-invalid"))?;
        let super_options = filesystem.next().ok_or_else(|| procfs_visibility_error("mountinfo-invalid"))?;
        if id == mount_id && selected.replace((kind, options, super_options)).is_some() {
            return Err(procfs_visibility_error("mountinfo-invalid"));
        }
    }
    let (kind, options, super_options) = selected
        .ok_or_else(|| procfs_visibility_error("procfs-mount-not-found"))?;
    if kind != "proc" { return Err(procfs_visibility_error("procfs-mount-not-found")); }
    if options.split(',').chain(super_options.split(',')).any(|option| {
        option.strip_prefix("hidepid=").is_some_and(|value| !matches!(value, "0" | "off"))
            || option == "subset=pid"
    }) {
        return Err(ProcfsHidesProcesses {
            detail: "procfs-hides-processes",
            mount_options: Some(format!("{options};{super_options}")),
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn require_complete_procfs() -> std::result::Result<(), ProcfsHidesProcesses> {
    // Match the opened /proc directory's mount ID, not an arbitrary mountinfo
    // line named /proc: stacked or bind mounts can have the same mount point.
    let directory = File::open("/proc").map_err(|_| procfs_visibility_error("procfs-root-unreadable"))?;
    let fdinfo = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", directory.as_raw_fd()))
        .map_err(|_| procfs_visibility_error("procfs-mount-identity-unreadable"))?;
    let mount_id = fdinfo.lines().find_map(|line| line.strip_prefix("mnt_id:"))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .ok_or_else(|| procfs_visibility_error("procfs-mount-identity-invalid"))?;
    validate_procfs_mount(std::fs::read_to_string("/proc/self/mountinfo"), mount_id)
}

#[cfg(target_os = "linux")]
fn process_proof_error(stage: &'static str, pid: u32, error: anyhow::Error) -> anyhow::Error {
    LegacyStartProofUnavailable {
        stage,
        pid: Some(pid),
        unit: None,
        errno: error.downcast_ref::<std::io::Error>().and_then(std::io::Error::raw_os_error),
    }.into()
}

#[cfg(target_os = "linux")]
fn unit_proof_error(stage: &'static str, unit: &str, error: anyhow::Error) -> anyhow::Error {
    LegacyStartProofUnavailable {
        stage, pid: None, unit: Some(unit.into()),
        errno: error.downcast_ref::<std::io::Error>().and_then(std::io::Error::raw_os_error),
    }.into()
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
            ensure!(row.tags.get("st3.isolation").is_some_and(|mode| mode == "scope")
                && !recorded_scopes.is_empty(), unit_proof_error(
                    "scope-provenance", id, anyhow::anyhow!("legacy scope isolation is unproved")));
            let units = manager_units().map_err(|error| unit_proof_error("manager-enumeration", "user-manager", error))?;
            let matching_units = matching_units(id, &units)?;
            if !matching_units.is_empty() {
                return Ok(None);
            }
            for scope in &recorded_scopes {
                if !scope_not_loaded(scope).map_err(|error| unit_proof_error("recorded-scope", scope, error))? {
                    return Ok(None);
                }
            }
            let owner_uid = unsafe { libc::geteuid() };
            use std::os::unix::fs::MetadataExt as _;
            let runtime_root = self.root.canonicalize().context("resolve legacy PTY root")?;
            ensure!(std::fs::metadata(&runtime_root)?.uid() == owner_uid,
                "legacy PTY root is owned by another host user");
            let operation_pids = operation_processes(owner_uid, operation, id, &recorded_scopes)?;
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
    let mut matching = BTreeSet::new();
    for unit in units {
        ensure!(!unit.unit.is_empty() && !unit.load.is_empty()
            && !unit.active.is_empty() && !unit.sub.is_empty(),
            unit_proof_error("manager-unit", &unit.unit, anyhow::anyhow!("incomplete manager unit")));
        if runtime_unit_matches(&unit.unit, &safe) {
            matching.insert(unit.unit.clone());
        }
    }
    Ok(matching.into_iter().collect())
}

#[cfg(target_os = "linux")]
fn runtime_unit_matches(unit: &str, safe: &str) -> bool {
    if crate::priority::server_identity(unit) == Some(safe) {
        return true;
    }
    ["st3-work", "st3"].iter().any(|product| {
        crate::isolate::scope_identity(product, unit).is_some_and(|identity| {
            if identity == safe { return true; }
            // Operation-bound work scopes append a UUID to the runtime identity.
            let Some(operation) = identity.strip_prefix(safe).and_then(|tail| tail.strip_prefix('-')) else {
                return false;
            };
            operation.len() == 36 && operation.bytes().enumerate().all(|(i, byte)| {
                if matches!(i, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
            })
        })
    })
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
fn operation_processes(owner_uid: u32, operation: &str, id: &str, scopes: &BTreeSet<String>) -> Result<Vec<u32>> {
    operation_processes_in(Path::new("/proc"), owner_uid, operation, id, scopes)
}

#[cfg(target_os = "linux")]
fn operation_processes_in(proc_root: &Path, owner_uid: u32, operation: &str, id: &str, scopes: &BTreeSet<String>) -> Result<Vec<u32>> {
    operation_processes_with_visibility(proc_root, owner_uid, operation, id, scopes, require_complete_procfs())
}

#[cfg(target_os = "linux")]
fn operation_processes_with_visibility(
    proc_root: &Path, owner_uid: u32, operation: &str, id: &str, scopes: &BTreeSet<String>,
    visibility: std::result::Result<(), ProcfsHidesProcesses>,
) -> Result<Vec<u32>> {
    use std::os::unix::fs::MetadataExt as _;
    visibility?;
    let mut matching = BTreeSet::new();
    let safe = crate::isolate::sanitize(id);
    for entry in std::fs::read_dir(proc_root).context("enumerate legacy operation processes")? {
        let entry = entry?;
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        // Pin the proc directory: a recycled PID must never substitute a different
        // process between ownership, environment and exit checks.
        let directory = match File::open(entry.path()) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(process_proof_error("proc-directory", pid, error.into())),
        };
        let path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let status = match std::fs::read_to_string(path.join("status")) {
            Ok(status) => status,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(process_proof_error("proc-status", pid, error.into())),
        };
        let owned = (|| -> Result<bool> {
            let uids = status.lines().find_map(|line| line.strip_prefix("Uid:"))
                .context("process ownership is missing")?.split_whitespace()
                .map(str::parse::<u32>).collect::<std::result::Result<Vec<_>, _>>()?;
            ensure!(uids.len() == 4, "process ownership is incomplete");
            Ok(uids.contains(&owner_uid) || directory.metadata()?.uid() == owner_uid)
        })().map_err(|error| process_proof_error("proc-status", pid, error))?;
        if !owned {
            continue;
        }
        if !candidate_process(&path, &safe, scopes, pid)? { continue; }
        if owner_process_has_operation_in_complete_procfs(&path, operation, pid)? {
            matching.insert(pid);
        }
    }
    Ok(matching.into_iter().collect())
}

#[cfg(target_os = "linux")]
fn candidate_process(path: &Path, safe: &str, scopes: &BTreeSet<String>, pid: u32) -> Result<bool> {
    let cgroup = match std::fs::read_to_string(path.join("cgroup")) {
        Ok(cgroup) => cgroup,
        Err(error) => {
            if process_exited(path).map_err(|error| process_proof_error("proc-stat", pid, error))? {
                return Ok(false);
            }
            return Err(process_proof_error("proc-cgroup", pid, error.into()));
        }
    };
    // Scope isolation puts every launched producer and descendant in the work
    // scope (or the server's dedicated scope); fork/exec cannot escape it.
    // Before joining, the launcher holds the spawn lock we now own. Thus neither
    // an unrelated login/service cgroup nor a recycled PID can belong to this
    // start. Check every path component to include nested delegated cgroups.
    ensure!(cgroup.lines().any(|line| line.starts_with("0::/")),
        process_proof_error("proc-cgroup", pid, anyhow::anyhow!("missing unified process cgroup")));
    Ok(cgroup.lines().filter_map(|line| line.strip_prefix("0::"))
        .flat_map(|path| path.split('/'))
        .any(|unit| scopes.contains(unit) || runtime_unit_matches(unit, safe)))
}

#[cfg(all(target_os = "linux", test))]
fn owner_process_has_operation(path: &Path, operation: &str, pid: u32) -> Result<bool> {
    require_complete_procfs()?;
    owner_process_has_operation_in_complete_procfs(path, operation, pid)
}

#[cfg(target_os = "linux")]
fn owner_process_has_operation_in_complete_procfs(path: &Path, operation: &str, pid: u32) -> Result<bool> {
    match std::fs::read(path.join("environ")) {
        Ok(environment) => Ok(has_operation(&environment, operation)
            && !process_exited(path).map_err(|error| process_proof_error("proc-stat", pid, error))?),
        Err(error) => {
            if process_exited(path).map_err(|error| process_proof_error("proc-stat", pid, error))? {
                Ok(false)
            } else {
                Err(process_proof_error("proc-environ", pid, error.into()))
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

    #[cfg(target_os = "linux")]
    #[test]
    fn procfs_mount_visibility_refuses_every_hiding_option() {
        for option in ["hidepid=1", "hidepid=2", "hidepid=invisible", "hidepid=noaccess",
            "hidepid=ptraceable", "hidepid=unknown", "hidepid=", "subset=pid"]
        {
            for (mount, super_options) in [(format!("rw,{option}"), "rw".to_owned()),
                ("rw".to_owned(), format!("rw,{option}"))]
            {
                let error = validate_procfs_mount(Ok(format!(
                    "42 1 0:1 / /proc {mount} - proc proc {super_options}\n")), 42).unwrap_err();
                assert_eq!(error.detail, "procfs-hides-processes", "{option}");
                assert_eq!(error.mount_options, Some(format!("{mount};{super_options}")));
            }
        }
        for options in ["rw", "rw,hidepid=0", "rw,hidepid=off", "rw,subset=all"] {
            validate_procfs_mount(Ok(format!("42 1 0:1 / /proc rw - proc proc {options}\n")), 42).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn procfs_mount_visibility_refuses_unknown_mountinfo_and_matches_the_open_mount() {
        let unreadable = validate_procfs_mount(Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)), 42)
            .unwrap_err();
        assert_eq!(unreadable.detail, "mountinfo-unreadable");
        for (text, detail) in [
            ("not mountinfo", "mountinfo-invalid"),
            ("42 1 0:1 / /proc rw - proc", "mountinfo-invalid"),
            ("41 1 0:1 / /proc rw - proc proc rw", "procfs-mount-not-found"),
            ("42 1 0:1 / /proc rw - tmpfs tmpfs rw", "procfs-mount-not-found"),
        ] {
            let error = validate_procfs_mount(Ok(text.to_owned()), 42).unwrap_err();
            assert_eq!(error.detail, detail);
            assert!(error.mount_options.is_none());
        }
        let stacked = "41 1 0:1 / /proc rw - proc proc rw\n42 41 0:2 / /proc rw - proc proc rw,hidepid=invisible\n";
        validate_procfs_mount(Ok(stacked.into()), 41).unwrap();
        assert_eq!(validate_procfs_mount(Ok(stacked.into()), 42).unwrap_err().detail, "procfs-hides-processes");
    }

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
        let units = [crate::scope_unit("st3", "work"), crate::server_unit("work", 2),
            crate::scope_unit("st3-work", "work"), "unrelated.scope".into()]
            .map(|unit| ManagerUnit { unit, load: "loaded".into(), active: "inactive".into(), sub: "dead".into() });
        assert_eq!(matching_units("work", &units).unwrap().len(), 3);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn successor_seats_are_not_the_legacy_identity() {
        let operation = "01234567-89ab-7cde-8fab-0123456789ab";
        let units = [
            crate::scope_unit("st3", &format!("repo.worker-2-{operation}")),
            crate::server_unit("repo.worker-2", 42),
        ].map(|unit| ManagerUnit { unit, load: "loaded".into(), active: "active".into(), sub: "running".into() });
        assert!(matching_units("repo.worker", &units).unwrap().is_empty());
        assert_eq!(matching_units("repo.worker-2", &units).unwrap().len(), 2);
        assert!(runtime_unit_matches(&crate::scope_unit("st3", &format!("repo.worker-{operation}")), "repo.worker"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn server_prefixed_runtime_work_scope_is_still_a_candidate() {
        let id = "pty-server-repo.worker";
        let unit = crate::scope_unit("st3", id);
        let units = [ManagerUnit { unit: unit.clone(), load: "loaded".into(),
            active: "active".into(), sub: "running".into() }];
        assert_eq!(matching_units(id, &units).unwrap(), vec![unit.clone()]);
        let root = tempfile::tempdir().unwrap();
        let process = root.path().join("123");
        std::fs::create_dir(&process).unwrap();
        let uid = unsafe { libc::geteuid() };
        std::fs::write(process.join("status"), format!("Uid:\t{uid}\t{uid}\t{uid}\t{uid}\n")).unwrap();
        std::fs::write(process.join("stat"), "123 (producer) S 1 2 3").unwrap();
        std::fs::write(process.join("cgroup"), format!("0::/user.slice/{unit}\n")).unwrap();
        std::fs::write(process.join("environ"), b"FRACTAL_START_OPERATION=op\0").unwrap();
        let scopes = BTreeSet::from([crate::scope_unit("st3", id)]);
        assert!(!scopes.contains(&unit));
        let mut evidence = closed();
        evidence.operation_pids = operation_processes_with_visibility(root.path(), uid, "op", id, &scopes, Ok(())).unwrap();
        assert_eq!(evidence.operation_pids, vec![123]);
        assert!(!positive(&evidence));
        std::fs::remove_file(process.join("environ")).unwrap();
        std::fs::create_dir(process.join("environ")).unwrap();
        let error = operation_processes_with_visibility(root.path(), uid, "op", id, &scopes, Ok(())).unwrap_err();
        let refusal = error.downcast_ref::<LegacyStartProofUnavailable>().unwrap();
        assert_eq!((refusal.stage, refusal.pid), ("proc-environ", Some(123)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_candidate_cgroups_require_readable_environments() {
        let root = tempfile::tempdir().unwrap();
        let process = root.path().join("123");
        std::fs::create_dir(&process).unwrap();
        let uid = unsafe { libc::geteuid() };
        std::fs::write(process.join("status"), format!("Uid:\t{uid}\t{uid}\t{uid}\t{uid}\n")).unwrap();
        std::fs::write(process.join("stat"), "123 (sshd-session) S 1 2 3").unwrap();
        // A directory produces a deterministic unreadable-environ error even as root.
        std::fs::create_dir(process.join("environ")).unwrap();
        for unit in ["session-1.scope", "user@1000.service", "sd-pam.service",
            "st3-pty-server-repo.worker-2-42.scope"] {
            std::fs::write(process.join("cgroup"), format!("0::/user.slice/{unit}\n")).unwrap();
            assert!(operation_processes_with_visibility(root.path(), uid, "op", "repo.worker", &BTreeSet::new(), Ok(())).unwrap().is_empty());
        }
        for unit in [crate::scope_unit("st3", "repo.worker"),
            crate::server_unit("repo.worker", 42), "recorded.scope".into()] {
            std::fs::write(process.join("cgroup"), format!("0::/user.slice/{unit}/nested\n")).unwrap();
            let scopes = BTreeSet::from(["recorded.scope".into()]);
            let error = operation_processes_with_visibility(root.path(), uid, "op", "repo.worker", &scopes, Ok(())).unwrap_err();
            let refusal = error.downcast_ref::<LegacyStartProofUnavailable>().unwrap();
            assert_eq!(refusal.stage, "proc-environ");
            assert_eq!(refusal.pid, Some(123));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn hidden_procfs_refuses_before_scanning_candidates() {
        let visibility = validate_procfs_mount(Ok(
            "42 1 0:1 / /proc rw - proc proc rw,hidepid=invisible\n".into()), 42);
        let root = tempfile::tempdir().unwrap();
        let error = operation_processes_with_visibility(
            root.path(), 1000, "op", "repo.worker", &BTreeSet::new(), visibility).unwrap_err();
        let refusal = error.downcast_ref::<ProcfsHidesProcesses>().unwrap();
        assert_eq!(refusal.detail, "procfs-hides-processes");
        assert_eq!(refusal.mount_options.as_deref(), Some("rw;rw,hidepid=invisible"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_proc_scan_finds_owned_child_exact_label() {
        let variable = "FRACTAL_START_OPERATION";
        let operation = "fractal-runtime-legacy-proof-native-test";
        if let Err(expected) = require_complete_procfs() {
            let error = operation_processes(unsafe { libc::geteuid() }, operation, "work", &BTreeSet::new()).unwrap_err();
            let refusal = error.downcast_ref::<ProcfsHidesProcesses>().unwrap();
            assert_eq!(refusal.detail, expected.detail);
            assert_eq!(refusal.mount_options, expected.mount_options);
            eprintln!("native_proc_scan_finds_owned_child_exact_label: typed refusal instead of partial enumeration: {refusal:?}");
            return;
        }
        let mut child = Command::new("sleep").arg("60").env(variable, operation).spawn().unwrap();
        // Limit enumeration to this test's child. Sandboxed CI can expose
        // unrelated same-UID processes whose environ is deliberately unreadable.
        let proc_root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            format!("/proc/{}", child.id()), proc_root.path().join(child.id().to_string()),
        ).unwrap();
        let scopes = std::fs::read_to_string(format!("/proc/{}/cgroup", child.id())).unwrap()
            .lines().filter_map(|line| line.strip_prefix("0::"))
            .flat_map(|path| path.split('/')).map(str::to_owned).collect();
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = loop {
            match operation_processes_in(proc_root.path(), unsafe { libc::geteuid() }, operation, "work", &scopes) {
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
    fn non_dumpable_marked_owner_child_remains_a_typed_refusal() {
        const CHILD: &str = "FRACTAL_PROOF_NON_DUMPABLE_CHILD";
        const READER: &str = "FRACTAL_PROOF_UNPRIVILEGED_READER";
        const TEST: &str = "pty::legacy_start_closure::tests::non_dumpable_marked_owner_child_remains_a_typed_refusal";
        const OPERATION: &str = "legacy-proof-unreadable-producer";
        const CONTROL: &str = "FRACTAL_PROOF_DUMPABLE_CONTROL";
        const CONTROL_PID: &str = "FRACTAL_PROOF_CONTROL_PROCFS_PID";
        struct ControlChild(Option<std::process::Child>);
        impl Drop for ControlChild {
            fn drop(&mut self) {
                if let Some(mut child) = self.0.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        fn reader_diagnostics() -> String {
            let status = std::fs::read_to_string("/proc/self/status");
            let fields = status.as_ref().map(|text| text.lines()
                .filter(|line| line.starts_with("CapEff:") || line.starts_with("NSpid:"))
                .collect::<Vec<_>>());
            let mounts = std::fs::read_to_string("/proc/self/mountinfo");
            let proc_mounts = mounts.as_ref().map(|text| text.lines()
                .filter(|line| line.split_whitespace().nth(4) == Some("/proc"))
                .collect::<Vec<_>>());
            format!("reader_local_pid={} reader_procself={:?} reader_fields={fields:?} proc_mounts={proc_mounts:?}",
                std::process::id(), std::fs::read_link("/proc/self"))
        }
        fn process_diagnostics(pid: u32) -> String {
            let path = PathBuf::from(format!("/proc/{pid}"));
            let status = std::fs::read_to_string(path.join("status"));
            let fields = status.as_ref().map(|text| text.lines()
                .filter(|line| line.starts_with("Uid:") || line.starts_with("NSpid:"))
                .collect::<Vec<_>>());
            format!("procfs_pid={pid} exists={:?} metadata={:?} fields={fields:?}",
                path.try_exists(), std::fs::metadata(&path))
        }
        if std::env::var_os(CONTROL).is_some() {
            println!("proof-control-ready {}", std::fs::read_link("/proc/self")
                .expect("control readiness: /proc/self").display());
            use std::io::Read as _;
            std::io::stdin().read_exact(&mut [0]).expect("control hold: stdin");
            return;
        }
        if std::env::var_os(CHILD).is_some() {
            // SAFETY: these prctl operations affect only this dedicated child.
            unsafe {
                assert_eq!(libc::prctl(libc::PR_SET_NAME, c"systemd".as_ptr()), 0, "producer prctl(PR_SET_NAME): {}", std::io::Error::last_os_error());
                assert_eq!(libc::prctl(libc::PR_SET_DUMPABLE, 0), 0, "producer prctl(PR_SET_DUMPABLE=0): {}", std::io::Error::last_os_error());
            }
            // These calls happen after re-exec, in the producer itself.
            let dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE) };
            assert_eq!(dumpable, 0, "producer prctl(PR_GET_DUMPABLE): {}", std::io::Error::last_os_error());
            // procfs may be mounted from an ancestor PID namespace. Production
            // enumerates its PIDs, not the namespace-local Child::id values.
            let proc_pid = std::fs::read_link("/proc/self").expect("producer readiness: readlink /proc/self");
            let proc_pid: u32 = proc_pid.to_str().expect("producer readiness: UTF-8 /proc/self")
                .parse().expect("producer readiness: numeric /proc/self pid");
            let status = std::fs::read_to_string("/proc/self/status");
            let nspid = status.as_ref().map(|status| status.lines()
                .find_map(|line| line.strip_prefix("NSpid:")).unwrap_or("missing")
                .split_whitespace().collect::<Vec<_>>().join(":"))
                .unwrap_or_else(|error| format!("unavailable(errno={:?})", error.raw_os_error()));
            println!("proof-child-ready pid={} proc_pid={proc_pid} dumpable={dumpable} NSpid={nspid}", std::process::id());
            eprintln!("producer readiness diagnostics: proc_pid={proc_pid} NSpid={nspid} status_error={:?}; {}",
                status.as_ref().err(), reader_diagnostics());
            // Keep the producer alive until the parent completes its scan.
            // A fixed sleep can expire under a heavily loaded CI scheduler.
            use std::io::Read as _;
            let mut release = [0];
            std::io::stdin().read_exact(&mut release).expect("producer hold: read stdin pipe");
            return;
        }
        fn assert_denied(pid: u32, reported_pid: u32, owner_uid: u32) -> Result<()> {
            use std::os::unix::fs::MetadataExt as _;
            let path = PathBuf::from(format!("/proc/{pid}"));
            if let Err(expected) = require_complete_procfs() {
                let error = owner_process_has_operation(&path, OPERATION, pid).unwrap_err();
                let refusal = error.downcast_ref::<ProcfsHidesProcesses>()
                    .context("hidden procfs producer must return a typed refusal")?;
                assert_eq!(refusal.detail, expected.detail);
                assert_eq!(refusal.mount_options, expected.mount_options);
                let enumeration = operation_processes(owner_uid, OPERATION, "work", &BTreeSet::new()).unwrap_err();
                let enumeration = enumeration.downcast_ref::<ProcfsHidesProcesses>()
                    .context("hidden procfs enumeration must refuse, not return a partial inventory")?;
                assert_eq!(enumeration.detail, refusal.detail);
                assert_eq!(enumeration.mount_options, refusal.mount_options);
                eprintln!("{TEST}: typed ProcfsHidesProcesses refusal asserted for owner lookup AND enumeration: {refusal:?}; {}; producer=[{}]",
                    reader_diagnostics(), process_diagnostics(pid));
                return Ok(());
            }
            let reader = std::fs::read_to_string("/proc/self/status").context("reader diagnostics: read /proc/self/status")?;
            let caps = reader.lines().find(|line| line.starts_with("CapEff:")).unwrap_or("CapEff: missing");
            let namespace = std::fs::read_link("/proc/self/ns/user").context("reader diagnostics: readlink /proc/self/ns/user")?;
            let visibility = format!("{} producer=[{}] control=[{}]", reader_diagnostics(),
                process_diagnostics(pid), std::env::var(CONTROL_PID)
                    .ok().and_then(|value| value.parse().ok())
                    .map(process_diagnostics).unwrap_or_else(|| "control pid unavailable".to_owned()));
            let producer = std::fs::read_to_string(path.join("status"))
                .with_context(|| format!("owner inclusion: read /proc/{pid}/status; {visibility}"))?;
            let uids = producer.lines().find_map(|line| line.strip_prefix("Uid:"))
                .with_context(|| format!("owner inclusion: Uid field in /proc/{pid}/status missing"))?.split_whitespace()
                .map(str::parse::<u32>).collect::<std::result::Result<Vec<_>, _>>()
                .with_context(|| format!("owner inclusion: parse Uid in /proc/{pid}/status"))?;
            let metadata_uid = std::fs::metadata(&path).with_context(|| format!("owner inclusion: stat /proc/{pid}"))?.uid();
            let stat = std::fs::read_to_string(path.join("stat"))
                .with_context(|| format!("producer liveness: read /proc/{pid}/stat"))?;
            let state = stat.rsplit_once(") ").with_context(|| format!("producer liveness: malformed /proc/{pid}/stat"))?.1
                .split_whitespace().next().with_context(|| format!("producer liveness: state missing in /proc/{pid}/stat"))?;
            let errno = std::fs::read(path.join("environ")).err().and_then(|error| error.raw_os_error());
            let diagnostics = format!(
                "reader_pid={} uid={} euid={} {caps} userns={} producer_procfs_pid={pid} producer_namespace_pid={reported_pid} producer_dumpable=0 (post-exec readiness, unchanged while blocked on stdin) producer_state={state} environ_errno={errno:?} owner_uid={owner_uid} producer_uids={uids:?} metadata_uid={metadata_uid}",
                std::process::id(), unsafe { libc::getuid() }, unsafe { libc::geteuid() }, namespace.display(),
            );
            // Match production's owner test: non-dumpable procfs metadata may
            // be root-owned even though the status UIDs identify the owner.
            anyhow::ensure!(uids.contains(&owner_uid) || metadata_uid == owner_uid,
                "positive owner-pid inclusion failed: {diagnostics}");
            anyhow::ensure!(matches!(state, "S" | "R"), "producer is not live: {diagnostics}");
            let proc_root = tempfile::tempdir()?;
            std::os::unix::fs::symlink(&path, proc_root.path().join(pid.to_string()))?;
            let cgroup = std::fs::read_to_string(path.join("cgroup"))?;
            assert!(operation_processes_in(proc_root.path(), owner_uid, OPERATION,
                "unrelated.fixture", &BTreeSet::new())?.is_empty());
            let scopes = cgroup.lines().filter_map(|line| line.strip_prefix("0::"))
                .flat_map(|path| path.split('/')).map(str::to_owned).collect();
            let error = operation_processes_in(proc_root.path(), owner_uid, OPERATION, "work", &scopes)
                .err().with_context(|| format!("unreadable producer was incorrectly excluded: {diagnostics}"))?;
            let refusal = error.downcast_ref::<LegacyStartProofUnavailable>()
                .with_context(|| format!("production helper returned an untyped refusal: {diagnostics}"))?;
            assert_eq!(refusal.stage, "proc-environ", "{diagnostics}");
            assert_eq!(refusal.pid, Some(pid), "{diagnostics}");
            assert_eq!(refusal.errno, Some(libc::EACCES), "{diagnostics}");
            anyhow::ensure!(errno == Some(libc::EACCES), "positive EACCES read failed: {diagnostics}");
            eprintln!("{TEST}: positive owner-pid inclusion and EACCES: {diagnostics}");
            Ok(())
        }
        if let Ok(pid) = std::env::var(READER) {
            let mut fields = pid.split(':');
            let proc_pid = fields.next().expect("reader marker procfs pid").parse().expect("parse reader marker procfs pid");
            let reported_pid = fields.next().expect("reader marker namespace pid").parse().expect("parse reader marker namespace pid");
            let owner_uid = fields.next().expect("reader marker owner uid").parse().expect("parse reader marker owner uid");
            assert_denied(proc_pid, reported_pid, owner_uid).unwrap();
            eprintln!("{TEST}: EACCES assertion exercised by user-namespace reader");
            return;
        }
        let status = std::fs::read_to_string("/proc/self/status").expect("reader lane selection: read /proc/self/status");
        let effective = u64::from_str_radix(
            status.lines().find_map(|line| line.strip_prefix("CapEff:")).expect("reader lane selection: CapEff missing in /proc/self/status").trim(),
            16,
        ).expect("reader lane selection: parse CapEff in /proc/self/status");
        const CAP_SYS_PTRACE: u32 = 19;
        let executable = std::env::current_exe().expect("resolve fixture test binary using /proc/self/exe");
        let mut child = Command::new(&executable)
            .args(["--exact",
                "pty::legacy_start_closure::tests::non_dumpable_marked_owner_child_remains_a_typed_refusal",
                "--nocapture"])
            .env(CHILD, "1").env("ST3_START_OPERATION", OPERATION)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped()).spawn()
            .with_context(|| format!("spawn producer test binary {}", executable.display())).unwrap();
        let mut control = ControlChild(None);
        let mut control_pid = None;
        let mut readiness = String::new();
        let result = (|| -> Result<()> {
            use std::io::BufRead as _;
            let sibling = Command::new(&executable).args(["--exact", TEST, "--nocapture"])
                .env(CONTROL, "1").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).spawn().context("spawn dumpable control child")?;
            control.0 = Some(sibling);
            let output = control.0.as_mut().context("control child missing")?
                .stdout.take().context("control readiness: acquire stdout")?;
            for line in std::io::BufReader::new(output).lines() {
                let line = line.context("control readiness: read stdout")?;
                if let Some(pid) = line.strip_prefix("proof-control-ready ") {
                    control_pid = Some(pid.parse::<u32>().context("control readiness: parse procfs pid")?);
                    break;
                }
            }
            let sibling_pid = control_pid.context("control readiness: missing procfs pid")?;
            let output = child.stdout.take().context("producer readiness: acquire child stdout pipe")?;
            let mut producer_pid = None;
            for line in std::io::BufReader::new(output).lines() {
                let line = line.context("producer readiness: read stdout pipe")?;
                readiness.push_str(&line);
                readiness.push('\n');
                if let Some(fields) = line.strip_prefix("proof-child-ready ") {
                    let fields: Vec<_> = fields.split_whitespace().collect();
                    anyhow::ensure!(fields.len() == 4 && fields[0] == format!("pid={}", child.id()) && fields[2] == "dumpable=0",
                        "producer readiness: mismatched pid/dumpable: {line}; spawned pid={}", child.id());
                    producer_pid = Some(fields[1].strip_prefix("proc_pid=").context("producer readiness: proc_pid field missing")?
                        .parse::<u32>().context("producer readiness: parse procfs pid")?);
                    break;
                }
            }
            let producer_pid = producer_pid.with_context(|| format!("producer readiness: expected pid={} dumpable=0 from {}", child.id(), executable.display()))?;
            if require_complete_procfs().is_err() {
                anyhow::ensure!(child.try_wait().context("verify hidden producer wait status")?.is_none(),
                    "hidden producer exited before its refusal assertion");
                assert_denied(producer_pid, child.id(), unsafe { libc::geteuid() })?;
                return Ok(());
            }
            if effective & (1 << CAP_SYS_PTRACE) == 0 {
                assert_denied(producer_pid, child.id(), unsafe { libc::geteuid() })
                    .with_context(|| format!("direct reader control: {}", process_diagnostics(sibling_pid)))?;
                eprintln!("{TEST}: EACCES assertion exercised directly (CAP_SYS_PTRACE not effective)");
            } else {
                use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
                let uid = unsafe { libc::getuid() };
                let gid = unsafe { libc::getgid() };
                let uid_map = format!("{uid} {uid} 1");
                let gid_map = format!("{gid} {gid} 1");
                let mut command = Command::new(&executable);
                command.args(["--exact", TEST, "--nocapture"])
                    .env(READER, format!("{producer_pid}:{}:{uid}", child.id()))
                    .env(CONTROL_PID, sibling_pid.to_string());
                // SAFETY: pre_exec runs only in the isolated, single-threaded
                // post-fork reader. User-namespace unshare in a libtest worker
                // would fail EINVAL because its harness is multithreaded.
                // The closure uses only raw async-signal-safe syscalls and
                // preallocated map strings; no allocator or Rust file I/O.
                unsafe {
                    command.pre_exec(move || {
                        if libc::unshare(libc::CLONE_NEWUSER) != 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        fn write_map(path: &std::ffi::CStr, contents: &[u8]) -> std::io::Result<()> {
                            // SAFETY: valid C path and live slice; descriptor
                            // is used and closed only by this post-fork child.
                            unsafe {
                                fn failed(path: &std::ffi::CStr, stage: &[u8]) -> std::io::Error {
                                    // Raw stderr writes remain safe after fork;
                                    // retain the exact failing step and path.
                                    unsafe {
                                        libc::write(2, stage.as_ptr().cast(), stage.len());
                                        libc::write(2, path.as_ptr().cast(), path.to_bytes().len());
                                        libc::write(2, b"\n".as_ptr().cast(), 1);
                                    }
                                    std::io::Error::from_raw_os_error(libc::EIO)
                                }
                                let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
                                if fd < 0 { return Err(failed(path, b"fixture: open namespace map failed: ")); }
                                let written = libc::write(fd, contents.as_ptr().cast(), contents.len());
                                let closed = libc::close(fd);
                                if written != contents.len() as isize {
                                    return Err(failed(path, b"fixture: write namespace map failed: "));
                                }
                                if closed != 0 { return Err(failed(path, b"fixture: close namespace map failed: ")); }
                            }
                            Ok(())
                        }
                        // Preserve the owner's numeric identity. Mapping
                        // failures use EIO, never the unshare-only skip lane.
                        write_map(c"/proc/self/setgroups", b"deny")?;
                        write_map(c"/proc/self/uid_map", uid_map.as_bytes())?;
                        write_map(c"/proc/self/gid_map", gid_map.as_bytes())?;
                        Ok(())
                    });
                }
                let mut reader = match command.spawn() {
                    Ok(reader) => reader,
                    Err(error) if matches!(error.raw_os_error(), Some(libc::EPERM | libc::EINVAL | libc::ENOSPC)) => {
                        eprintln!("{TEST}: SKIP EACCES assertion: privileged reader; user namespace forbidden or unavailable ({error})");
                        return Ok(());
                    }
                    Err(error) => return Err(error).with_context(|| format!("spawn mapped user-namespace reader test binary {}", executable.display())),
                };
                let reader = reader.wait().with_context(|| format!("wait mapped reader test binary {}", executable.display()))?;
                if reader.signal() == Some(libc::SIGSYS) {
                    eprintln!("{TEST}: SKIP EACCES assertion: privileged reader; user namespace forbidden by sandbox (SIGSYS)");
                } else {
                    anyhow::ensure!(reader.success(), "isolated EACCES reader failed: {reader}");
                }
            }
            Ok(())
        })();
        let final_diagnostics = format!("fixture failure diagnostics: {}; producer readiness={readiness:?}; control={:?}",
            reader_diagnostics(), control_pid.map(process_diagnostics));
        child.kill().with_context(|| format!("cleanup: kill producer pid={}", child.id())).unwrap();
        child.wait().with_context(|| format!("cleanup: wait producer pid={}", child.id())).unwrap();
        if let Some(mut sibling) = control.0.take() {
            let killed = sibling.kill();
            let waited = sibling.wait();
            killed.context("cleanup: kill dumpable control").unwrap();
            waited.context("cleanup: wait dumpable control").unwrap();
        }
        result.context(final_diagnostics).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unreadable_environment_requires_positive_exit_evidence() {
        let proc = tempfile::tempdir().unwrap();
        std::fs::write(proc.path().join("stat"), "123 (producer) S 1 2 3").unwrap();
        let error = owner_process_has_operation_in_complete_procfs(proc.path(), "op", 123).unwrap_err();
        let refusal = error.downcast_ref::<LegacyStartProofUnavailable>().unwrap();
        assert_eq!(refusal.stage, "proc-environ");
        assert_eq!(refusal.pid, Some(123));
        assert_eq!(refusal.errno, Some(libc::ENOENT));
        std::fs::write(proc.path().join("stat"), "123 (producer) Z 1 2 3").unwrap();
        assert!(!owner_process_has_operation_in_complete_procfs(proc.path(), "op", 123).unwrap());
        std::fs::write(proc.path().join("stat"), "unreadable evidence").unwrap();
        assert!(owner_process_has_operation_in_complete_procfs(proc.path(), "op", 123).is_err());
        std::fs::remove_file(proc.path().join("stat")).unwrap();
        assert!(!owner_process_has_operation_in_complete_procfs(proc.path(), "op", 123).unwrap());
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
