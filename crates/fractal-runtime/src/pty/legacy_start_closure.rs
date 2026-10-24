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
    #[serde(default)]
    pub excluded_processes: Vec<ExcludedProcess>,
}

/// Diagnostic-only exclusions; process/session churn must not change repair approval.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExcludedProcess {
    pub pid: u32,
    pub comm: String,
    pub cgroup: String,
    pub reason: String,
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
            let (operation_pids, excluded_processes) = operation_processes(owner_uid, operation)?;
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
                excluded_processes,
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
fn operation_processes(owner_uid: u32, operation: &str) -> Result<(Vec<u32>, Vec<ExcludedProcess>)> {
    operation_processes_in(Path::new("/proc"), owner_uid, operation)
}

#[cfg(target_os = "linux")]
fn operation_processes_in(proc_root: &Path, owner_uid: u32, operation: &str) -> Result<(Vec<u32>, Vec<ExcludedProcess>)> {
    use std::os::unix::fs::MetadataExt as _;
    let mut matching = BTreeSet::new();
    let mut excluded = Vec::new();
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
        match owner_process_has_operation(&path, operation) {
            Ok(true) => { matching.insert(pid); }
            Ok(false) => {}
            Err(error) if error.downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied) => {
                excluded.push(exclude_unreadable_process(&path, proc_root, owner_uid, operation, pid)?);
            }
            Err(error) => return Err(error),
        }
    }
    excluded.sort_by_key(|process| process.pid);
    Ok((matching.into_iter().collect(), excluded))
}

#[cfg(target_os = "linux")]
fn operation_created_ms(operation: &str) -> Option<u64> {
    let bytes = operation.as_bytes();
    if bytes.len() != 36 || bytes[14] != b'7'
        || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b' | b'A' | b'B')
        || bytes.iter().enumerate().any(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) { *byte != b'-' } else { !byte.is_ascii_hexdigit() }
        })
    {
        return None;
    }
    Some((u64::from_str_radix(&operation[..8], 16).ok()? << 16)
        | u64::from_str_radix(&operation[9..13], 16).ok()?)
}

#[cfg(target_os = "linux")]
fn outside_runtime_cgroup(cgroup: &str, uid: u32) -> bool {
    let user = format!("/user.slice/user-{uid}.slice/");
    let Some(relative) = cgroup.strip_prefix(&user) else { return false };
    relative == format!("user@{uid}.service/init.scope")
        || relative.strip_prefix("session-").and_then(|session| session.strip_suffix(".scope"))
            .is_some_and(|session| !session.is_empty() && session.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(target_os = "linux")]
fn exclude_unreadable_process(
    path: &Path, proc_root: &Path, uid: u32, operation: &str, pid: u32,
) -> Result<ExcludedProcess> {
    let stat = std::fs::read_to_string(path.join("stat")).context("read unreadable process starttime")?;
    let (_, tail) = stat.rsplit_once(") ").context("malformed unreadable process stat")?;
    let start_ticks: u64 = tail.split_whitespace().nth(19).context("missing process starttime")?.parse()?;
    let cgroups = std::fs::read_to_string(path.join("cgroup")).context("read unreadable process cgroup")?;
    let mut unified = cgroups.lines().filter_map(|line| line.strip_prefix("0::"));
    let cgroup = unified.next().context("missing unified process cgroup")?;
    ensure!(unified.next().is_none() && cgroup.starts_with('/'), "ambiguous unified process cgroup");
    let reason = if outside_runtime_cgroup(cgroup, uid) {
        "outside-runtime-cgroup"
    } else {
        let created_ms = operation_created_ms(operation).context("unreadable process needs UUIDv7 operation time")?;
        let boot = std::fs::read_to_string(proc_root.join("stat")).context("read boot time for process exclusion")?;
        let boot_seconds: u64 = boot.lines().find_map(|line| line.strip_prefix("btime "))
            .context("missing boot time")?.trim().parse()?;
        let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        ensure!(ticks_per_second > 0, "process clock tick rate unavailable");
        // btime has whole-second precision. Include its entire uncertainty
        // interval and round ticks up; near-boundary processes stay held.
        let latest_start_ms = u128::from(boot_seconds) * 1000 + 1000
            + (u128::from(start_ticks) * 1000).div_ceil(ticks_per_second as u128);
        ensure!(latest_start_ms < u128::from(created_ms),
            "read owner process environment: unreadable live process may belong to operation");
        "predates-operation"
    };
    let comm = std::fs::read_to_string(path.join("comm")).context("read excluded process comm")?;
    Ok(ExcludedProcess { pid, comm: comm.trim_end_matches('\n').into(), cgroup: cgroup.into(), reason: reason.into() })
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
            excluded_processes: vec![],
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
    fn native_proc_scan_finds_owned_child_exact_label() {
        let variable = "FRACTAL_START_OPERATION";
        let operation = "fractal-runtime-legacy-proof-native-test";
        let mut child = Command::new("sleep").arg("60").env(variable, operation).spawn().unwrap();
        // Limit enumeration to this test's child. Sandboxed CI can expose
        // unrelated same-UID processes whose environ is deliberately unreadable.
        let proc_root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            format!("/proc/{}", child.id()), proc_root.path().join(child.id().to_string()),
        ).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = loop {
            match operation_processes_in(proc_root.path(), unsafe { libc::geteuid() }, operation) {
                Ok((pids, _)) if !pids.contains(&child.id()) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => break result,
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        assert!(result.unwrap().0.contains(&child.id()));
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
    fn unreadable_process_exclusions_require_exact_cgroup_or_positive_age() {
        let root = tempfile::tempdir().unwrap();
        let process = root.path().join("123");
        std::fs::create_dir(&process).unwrap();
        std::fs::write(root.path().join("stat"), "btime 1000\n").unwrap();
        std::fs::write(process.join("comm"), "example-process\n").unwrap();
        // Fields 3..21 precede starttime (field 22).
        let stat = |ticks: u64| format!("123 (example-process) S {} {ticks}", "0 ".repeat(18));
        std::fs::write(process.join("stat"), stat(u64::MAX)).unwrap();
        let operation = "0000001e-8480-7000-8000-000000000000";
        assert_eq!(operation_created_ms(operation), Some(2_000_000));
        for group in [
            "/user.slice/user-1000.slice/user@1000.service/init.scope",
            "/user.slice/user-1000.slice/session-7.scope",
        ] {
            std::fs::write(process.join("cgroup"), format!("0::{group}\n")).unwrap();
            let evidence = exclude_unreadable_process(&process, root.path(), 1000, operation, 123).unwrap();
            assert_eq!(evidence.reason, "outside-runtime-cgroup");
            assert_eq!(evidence.cgroup, group);
            assert_eq!(evidence.comm, "example-process");
            assert_eq!(evidence.pid, 123);
        }
        for group in [
            "/user.slice/user-1000.slice/user@1000.service/app.slice/st3-work-example.scope",
            "/user.slice/user-1000.slice/user@1000.service/session-7.scope",
            "/user.slice/user-1000.slice/session-7.scope/child",
            "/user.slice/user-1001.slice/user@1001.service/init.scope",
        ] {
            std::fs::write(process.join("cgroup"), format!("0::{group}\n")).unwrap();
            assert!(exclude_unreadable_process(&process, root.path(), 1000, operation, 123).is_err());
        }
        std::fs::write(process.join("stat"), stat(0)).unwrap();
        let old = exclude_unreadable_process(&process, root.path(), 1000, operation, 123).unwrap();
        assert_eq!(old.reason, "predates-operation");
        assert!(exclude_unreadable_process(&process, root.path(), 1000, "not-a-uuid", 123).is_err());
        // btime truncation uncertainty must keep a process near the operation held.
        let boundary = "0000000f-443f-7000-8000-000000000000"; // 1_000_511 ms
        assert!(exclude_unreadable_process(&process, root.path(), 1000, boundary, 123).is_err());
        std::fs::write(process.join("cgroup"), "0::/unknown\n0::/other\n").unwrap();
        assert!(exclude_unreadable_process(&process, root.path(), 1000, operation, 123).is_err());
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
