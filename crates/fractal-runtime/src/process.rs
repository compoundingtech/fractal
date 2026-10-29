use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const SCHEMA: &str = "st.runtime.exec-generation.v1";

/// Positive producer evidence: this exact attempt returned before creating any child.
/// Absence of a generation record after a child was created is not this proof.
#[derive(Debug)]
pub struct StartRejected {
    pub operation: Option<String>,
}
impl std::fmt::Display for StartRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("launch rejected before producer creation")
    }
}
impl std::error::Error for StartRejected {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecGeneration {
    pub schema: String,
    pub pid: u32,
    pub created_at_unix_ms: u128,
    pub start_token: u64,
    pub generation_id: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub exit_signal: Option<i32>,
    #[serde(default)]
    pub group_exited: bool,
    #[serde(default)]
    pub isolation_mode: String,
    #[serde(default)]
    pub scope_unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_operation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecObservation {
    Running(ExecGeneration),
    Exited(ExecGeneration),
    Indeterminate(String),
}

#[derive(Clone, Debug)]
pub struct ExecRuntime {
    state_dir: PathBuf,
    log_dir: PathBuf,
    owned_children: Arc<Mutex<BTreeMap<u32, Child>>>,
}

impl ExecRuntime {
    pub fn new(state_dir: PathBuf, log_dir: PathBuf) -> Self {
        crate::warn_if_degraded("st3");
        Self {
            state_dir,
            log_dir,
            owned_children: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn spawn(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<ExecGeneration> {
        self.spawn_guarded(id, launch, cwd, env, None)
    }

    pub fn spawn_checked(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        guard: &dyn Fn() -> Result<()>,
    ) -> Result<ExecGeneration> {
        self.spawn_guarded(id, launch, cwd, env, Some(guard))
    }

    fn spawn_guarded(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        guard: Option<&dyn Fn() -> Result<()>>,
    ) -> Result<ExecGeneration> {
        let mut child_created = false;
        self.spawn_inner(id, launch, cwd, env, guard, &mut child_created)
            .map_err(|error| {
                if child_created {
                    error
                } else {
                    error.context(StartRejected {
                        operation: env.get("ST3_START_OPERATION").cloned(),
                    })
                }
            })
    }

    fn spawn_inner(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        guard: Option<&dyn Fn() -> Result<()>>,
        child_created: &mut bool,
    ) -> Result<ExecGeneration> {
        crate::procfs::require_matching_procfs()?;
        fs::create_dir_all(&self.state_dir)?;
        fs::create_dir_all(&self.log_dir)?;
        // What the previous generation left running ends before its replacement starts. Each
        // launch has a scope of its own, so this never reaches the replacement.
        if let Err(error) = self.end_leftovers(id) {
            eprintln!(
                "st3: WARN what the last generation of {id} left running did not end: {error:#}"
            );
        }
        self.rotate_generation(id)?;
        let log_path = self.log_path(id, false);
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&log_path)
            .with_context(|| format!("open exec log {}", log_path.display()))?;

        let (program, arguments): (OsString, Vec<OsString>) = match launch {
            crate::Launch::Shell(source) => (
                OsString::from("sh"),
                vec![OsString::from("-c"), OsString::from(source)],
            ),
            crate::Launch::Argv(argv) => {
                let (program, arguments) = argv
                    .split_first()
                    .context("an argv launch must contain a program")?;
                (
                    OsString::from(program),
                    arguments.iter().map(OsString::from).collect(),
                )
            }
        };
        // Builds and tests run below the live path; see `crate::priority`.
        let (program, arguments) = match crate::work_prefix(env).split_first() {
            Some((prefix, prefix_arguments)) => (
                OsString::from(prefix),
                prefix_arguments
                    .iter()
                    .map(OsString::from)
                    .chain(std::iter::once(program))
                    .chain(arguments)
                    .collect(),
            ),
            None => (program, arguments),
        };
        let unit = crate::scope_unit("st3", id);
        let argument_refs = arguments
            .iter()
            .map(OsString::as_os_str)
            .collect::<Vec<_>>();
        let mut command =
            crate::isolate::wrap_in(&unit, program.as_os_str(), &argument_refs, Some(env));
        command
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        if let Some(guard) = guard {
            guard()?;
        }
        let child = command.spawn().context("spawn detached exec")?;
        *child_created = true;
        let pid = child.id();
        let start_token = process_start_token(pid)?;
        let created_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let generation_id = generation_id(id, pid, start_token, created_at_unix_ms);
        let generation = ExecGeneration {
            schema: SCHEMA.into(),
            pid,
            created_at_unix_ms,
            start_token,
            generation_id,
            exit_code: None,
            exit_signal: None,
            group_exited: false,
            isolation_mode: isolation_name(crate::isolation_mode()).into(),
            scope_unit: (crate::isolation_mode() == crate::Isolation::Scope).then_some(unit),
            start_operation: env.get("ST3_START_OPERATION").cloned(),
        };
        self.write_generation(id, &generation)?;
        self.owned_children
            .lock()
            .expect("exec child mutex poisoned")
            .insert(pid, child);
        Ok(generation)
    }

    pub fn observe(&self, id: &str) -> Result<Option<ExecObservation>> {
        let path = self.record_path(id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut generation: ExecGeneration = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => return Ok(Some(ExecObservation::Indeterminate(error.to_string()))),
        };
        if generation.schema != SCHEMA {
            return Ok(Some(ExecObservation::Indeterminate(format!(
                "unsupported generation schema {}",
                generation.schema
            ))));
        }
        // A terminal group is durable evidence about this exact exec. In particular, do not
        // let an unrelated process that later reuses the PID make a completed generation look
        // live again on platforms whose best available start token is the PID itself.
        if generation.group_exited {
            return Ok(Some(ExecObservation::Exited(generation)));
        }
        if generation.exit_code.is_some() || generation.exit_signal.is_some() {
            return self.observe_exited_group(id, generation).map(Some);
        }
        let owned_exit = {
            let mut children = self
                .owned_children
                .lock()
                .expect("exec child mutex poisoned");
            let status = children
                .get_mut(&generation.pid)
                .map(Child::try_wait)
                .transpose()?
                .flatten();
            if status.is_some() {
                children.remove(&generation.pid);
            }
            status
        };
        if let Some(status) = owned_exit {
            generation.exit_code = status.code();
            generation.exit_signal = exit_signal(&status);
            self.write_generation(id, &generation)?;
            return self.observe_exited_group(id, generation).map(Some);
        }
        match process_identity(generation.pid) {
            Ok((state, token)) if token == generation.start_token && state != 'Z' => {
                Ok(Some(ExecObservation::Running(generation)))
            }
            Ok((_state, token)) if token == generation.start_token => {
                self.observe_exited_group(id, generation).map(Some)
            }
            Ok(_) => Ok(Some(ExecObservation::Exited(generation))),
            Err(error) if process_is_absent(&error) => {
                self.observe_exited_group(id, generation).map(Some)
            }
            Err(error) if error.is::<crate::ProcessNamespaceMismatch>() => Err(error),
            Err(error) => Ok(Some(ExecObservation::Indeterminate(error.to_string()))),
        }
    }

    fn observe_exited_group(
        &self,
        id: &str,
        mut generation: ExecGeneration,
    ) -> Result<ExecObservation> {
        // The leader can exit after observe's first status probe. Reap a waitable
        // child before persisting group completion, including while descendants live.
        // A zombie leader thread can still have live sibling threads: never block
        // waiting for the whole process to exit while holding the child-map mutex.
        if generation.exit_code.is_none() && generation.exit_signal.is_none() {
            let owned_exit = {
                let mut children = self
                    .owned_children
                    .lock()
                    .expect("exec child mutex poisoned");
                let status = children
                    .get_mut(&generation.pid)
                    .map(Child::try_wait)
                    .transpose()?;
                if matches!(status, Some(Some(_))) {
                    children.remove(&generation.pid);
                }
                status
            };
            match owned_exit {
                Some(Some(status)) => {
                    generation.exit_code = status.code();
                    generation.exit_signal = exit_signal(&status);
                    self.write_generation(id, &generation)?;
                }
                Some(None) => return Ok(ExecObservation::Running(generation)),
                None => {}
            }
        }
        match observe_exited_group(generation)? {
            ExecObservation::Exited(mut generation) => {
                generation.group_exited = true;
                self.write_generation(id, &generation)?;
                Ok(ExecObservation::Exited(generation))
            }
            observation => Ok(observation),
        }
    }

    pub fn stop(&self, id: &str) -> Result<()> {
        self.stop_if(id, None)
    }

    pub fn stop_if(&self, id: &str, expected_generation: Option<&str>) -> Result<()> {
        self.end_if(id, expected_generation, libc::SIGTERM)
    }

    /// Sends `signal`, SIGTERM or SIGKILL, to everything the exec started. Its work scope holds
    /// its process group and every process that left that group, so the scope gets the signal
    /// where there is one. Once the group has ended, what it left in its scope ends.
    fn end_if(&self, id: &str, expected_generation: Option<&str>, signal: i32) -> Result<()> {
        let generation = match self.observe(id)? {
            Some(ExecObservation::Running(generation)) => generation,
            Some(ExecObservation::Exited(generation)) => {
                if let Some(unit) = &generation.scope_unit {
                    let grace = if signal == libc::SIGKILL {
                        Duration::ZERO
                    } else {
                        crate::SCOPE_GRACE
                    };
                    crate::end_scope(unit, grace)?;
                }
                return Ok(());
            }
            Some(ExecObservation::Indeterminate(_)) | None => return Ok(()),
        };
        if expected_generation.is_some_and(|expected| expected != generation.generation_id) {
            anyhow::bail!("exec `{id}` changed incarnation before signal {signal}");
        }
        if let Some(unit) = &generation.scope_unit {
            match crate::signal_scope(unit, signal) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) => eprintln!(
                    "st3: WARN exec {id}'s scope missed signal {signal}, so only its group gets it: {error:#}"
                ),
            }
        }
        signal_group(id, generation.pid, signal)
    }

    pub fn signal_if(
        &self,
        id: &str,
        expected_generation: Option<&str>,
        signal: i32,
    ) -> Result<()> {
        let Some(ExecObservation::Running(generation)) = self.observe(id)? else {
            return Ok(());
        };
        if expected_generation.is_some_and(|expected| expected != generation.generation_id) {
            anyhow::bail!("exec `{id}` changed incarnation before signal {signal}");
        }
        signal_group(id, generation.pid, signal)
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        self.kill_if(id, None)
    }

    pub fn kill_if(&self, id: &str, expected_generation: Option<&str>) -> Result<()> {
        self.end_if(id, expected_generation, libc::SIGKILL)
    }

    /// The work scope of an exec whose process group has ended. It can still hold a process that
    /// left the group, such as a build started in a session of its own.
    pub fn leftover_scope(&self, id: &str) -> Result<Option<String>> {
        Ok(match self.observe(id)? {
            Some(ExecObservation::Exited(generation)) => generation.scope_unit,
            _ => None,
        })
    }

    /// Ends what an ended exec left running in its work scope. A running exec keeps everything:
    /// [`Self::stop_if`] ends it.
    pub fn end_leftovers(&self, id: &str) -> Result<()> {
        if let Some(unit) = self.leftover_scope(id)? {
            crate::end_scope(&unit, crate::SCOPE_GRACE)?;
        }
        Ok(())
    }

    /// [`Self::end_leftovers`] on a background worker. A scope already seen empty costs one read
    /// of the exec's record.
    pub fn end_leftovers_later(&self, id: &str) {
        let Ok(Some(unit)) = self.leftover_scope(id) else {
            return;
        };
        if crate::isolate::has_ended(&unit) {
            return;
        }
        let id = id.to_owned();
        crate::isolate::end_later(unit.clone(), move || {
            crate::end_scope(&unit, crate::SCOPE_GRACE)
                .map(|_| ())
                .with_context(|| format!("end what exec `{id}` left running"))
        });
    }

    pub fn read_log(&self, id: &str) -> Result<Option<String>> {
        let path = self.log_path(id, false);
        match fs::read_to_string(&path) {
            Ok(log) => Ok(Some(log)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read exec log {}", path.display())),
        }
    }

    pub fn read_log_bytes(&self, id: &str, previous: bool) -> Result<Option<Vec<u8>>> {
        let path = self.log_path(id, previous);
        match fs::read(&path) {
            Ok(log) => Ok(Some(log)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read exec log {}", path.display())),
        }
    }

    pub fn previous_generation(&self, id: &str) -> Result<Option<ExecGeneration>> {
        self.read_generation_path(&self.previous_record_path(id))
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        // The record is the last place that names the exec's work scope.
        self.end_leftovers(id)?;
        for path in [
            self.record_path(id),
            self.previous_record_path(id),
            self.log_path(id, false),
            self.log_path(id, true),
        ] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.state_dir.join(format!("{id}.json"))
    }

    fn previous_record_path(&self, id: &str) -> PathBuf {
        self.state_dir.join(format!("{id}.json.1"))
    }

    fn log_path(&self, id: &str, previous: bool) -> PathBuf {
        self.log_dir.join(if previous {
            format!("{id}.log.1")
        } else {
            format!("{id}.log")
        })
    }

    fn rotate_generation(&self, id: &str) -> Result<()> {
        rotate_file(&self.record_path(id), &self.previous_record_path(id))?;
        rotate_file(&self.log_path(id, false), &self.log_path(id, true))
    }

    fn read_generation_path(&self, path: &Path) -> Result<Option<ExecGeneration>> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(Into::into),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write_generation(&self, id: &str, generation: &ExecGeneration) -> Result<()> {
        let path = self.record_path(id);
        let temporary = path.with_extension(format!("json.tmp.{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, generation)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        Ok(())
    }
}

/// The wrapper can exit on SIGTERM while its children ignore that signal. Keep its group
/// running until those children stop, so reconciliation still escalates to SIGKILL after
/// a daemon restart. A reused leader PID never grants authority over its new group.
fn observe_exited_group(generation: ExecGeneration) -> Result<ExecObservation> {
    match process_identity(generation.pid) {
        Ok((state, token)) if token != generation.start_token || state != 'Z' => {
            return Ok(ExecObservation::Exited(generation));
        }
        Ok(_) => {}
        Err(error) if process_is_absent(&error) => {}
        Err(error) if error.is::<crate::ProcessNamespaceMismatch>() => return Err(error),
        Err(error) => return Ok(ExecObservation::Indeterminate(error.to_string())),
    }
    match process_group_is_live(generation.pid) {
        Ok(true) => Ok(ExecObservation::Running(generation)),
        Ok(false) => Ok(ExecObservation::Exited(generation)),
        Err(error) if error.is::<crate::ProcessNamespaceMismatch>() => Err(error),
        Err(error) => Ok(ExecObservation::Indeterminate(error.to_string())),
    }
}

fn signal_group(id: &str, group: u32, signal: i32) -> Result<()> {
    let result = unsafe { libc::kill(-(group as i32), signal) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error)
                .with_context(|| format!("signal {signal} to exec `{id}`'s process group"));
        }
    }
    Ok(())
}

fn process_group_is_live(group: u32) -> Result<bool> {
    crate::procfs::require_matching_procfs()?;
    if unsafe { libc::kill(-(group as i32), 0) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err(error.into());
    }
    #[cfg(target_os = "linux")]
    {
        // kill(0) includes zombies. Orphaned descendants may await reaping by the host's
        // init process; they can no longer execute work and must not hold cleanup open.
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            let stat = match fs::read_to_string(entry.path().join("stat")) {
                Ok(stat) => stat,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => continue,
                Err(error) => return Err(error.into()),
            };
            let Some((_, tail)) = stat.rsplit_once(") ") else {
                continue;
            };
            let mut fields = tail.split_whitespace();
            let state = fields.next();
            let process_group = fields.nth(1).and_then(|field| field.parse::<u32>().ok());
            if process_group == Some(group) {
                if state != Some("Z") {
                    return Ok(true);
                }
                // A zombie main thread may leave live worker threads behind. Field 20
                // counts those threads even though /proc lists only group leaders.
                let thread_count = tail
                    .split_whitespace()
                    .nth(17)
                    .context("process stat lacks a thread count")?
                    .parse::<u32>()?;
                if thread_count > 1 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    #[cfg(not(target_os = "linux"))]
    Ok(true)
}

fn process_is_absent(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
    })
}

fn rotate_file(current: &Path, previous: &Path) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    match fs::remove_file(previous) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match fs::rename(current, previous) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn isolation_name(mode: crate::Isolation) -> &'static str {
    match mode {
        crate::Isolation::Scope => "scope",
        crate::Isolation::Detached => "detached",
        crate::Isolation::DegradedDetached => "degraded-detached",
    }
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal()
}

#[cfg(target_os = "linux")]
pub fn process_start_token(pid: u32) -> Result<u64> {
    process_identity(pid).map(|(_, token)| token)
}

#[cfg(target_os = "linux")]
pub(crate) fn process_identity(pid: u32) -> Result<(char, u64)> {
    crate::procfs::require_matching_procfs()?;
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let end = stat.rfind(')').context("malformed process stat")?;
    let tail = stat.get(end + 2..).context("malformed process stat tail")?;
    let mut fields = tail.split_whitespace();
    let state = fields
        .next()
        .and_then(|value| value.chars().next())
        .context("process stat lacks a state")?;
    let token = fields
        .nth(18)
        .context("process stat lacks a start token")?
        .parse()?;
    Ok((state, token))
}

#[cfg(not(target_os = "linux"))]
pub fn process_start_token(pid: u32) -> Result<u64> {
    // This platform has no portable process start token. The runtime uses the
    // PID as its best available token, so a fast exit must not make spawn fail.
    Ok(pid as u64)
}

#[cfg(not(target_os = "linux"))]
fn process_identity(pid: u32) -> Result<(char, u64)> {
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        Ok(('R', pid as u64))
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

fn generation_id(id: &str, pid: u32, start_token: u64, created_at_unix_ms: u128) -> String {
    let mut hash = Sha256::new();
    hash.update(id.as_bytes());
    hash.update(pid.to_be_bytes());
    hash.update(start_token.to_be_bytes());
    hash.update(created_at_unix_ms.to_be_bytes());
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[cfg(target_os = "linux")]
    #[test]
    fn current_process_token_matches_proc_self() {
        let status = fs::read_to_string("/proc/self/status").unwrap();
        eprintln!(
            "process namespace probe: getpid={} proc_self={:?} {}",
            std::process::id(),
            fs::read_link("/proc/self").unwrap(),
            status
                .lines()
                .find(|line| line.starts_with("NSpid:"))
                .unwrap_or("NSpid: unavailable")
        );
        let stat = fs::read_to_string("/proc/self/stat").unwrap();
        let (_, tail) = stat.rsplit_once(") ").unwrap();
        let token = tail
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap();

        assert_eq!(process_start_token(std::process::id()).unwrap(), token);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_outer_procfs_refuses_launch_and_control_before_mutation() {
        const ROOT: &str = "FRACTAL_EXEC_INHERITED_PROCFS_TEST_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = PathBuf::from(root);
            let runtime = ExecRuntime::new(root.join("exec"), root.join("logs"));
            let error = runtime
                .spawn(
                    "refused",
                    &crate::Launch::Shell("touch child-created".into()),
                    &root,
                    &BTreeMap::new(),
                )
                .unwrap_err();
            assert!(error.downcast_ref::<StartRejected>().is_some());
            assert!(error.downcast_ref::<crate::ProcessNamespaceMismatch>().is_some());
            assert!(
                process_start_token(std::process::id())
                    .unwrap_err()
                    .downcast_ref::<crate::ProcessNamespaceMismatch>()
                    .is_some()
            );
            assert!(
                process_group_is_live(std::process::id())
                    .unwrap_err()
                    .downcast_ref::<crate::ProcessNamespaceMismatch>()
                    .is_some()
            );
            let pty = crate::PtyRuntime::new(root.join("pty"));
            let error = pty
                .spawn(
                    "refused",
                    &crate::Launch::Shell("touch pty-child-created".into()),
                    &root,
                    &BTreeMap::new(),
                    None,
                    &BTreeMap::new(),
                    None,
                )
                .unwrap_err();
            assert!(error.downcast_ref::<crate::ProcessNamespaceMismatch>().is_some());
            assert!(!root.join("pty").exists());
            assert!(
                pty.snapshot()
                    .unwrap_err()
                    .downcast_ref::<crate::ProcessNamespaceMismatch>()
                    .is_some()
            );
            assert!(!root.join("pty").exists());
            assert!(!root.join("pty-child-created").exists());
            assert!(!root.join("exec").exists());
            assert!(!root.join("logs").exists());
            assert!(!root.join("child-created").exists());
            let control = ExecRuntime::new(root.join("control"), root.join("control-logs"));
            fs::create_dir_all(&control.state_dir).unwrap();
            for exit_code in [None, Some(0)] {
                let generation = ExecGeneration {
                    schema: SCHEMA.into(),
                    pid: i32::MAX as u32,
                    created_at_unix_ms: 1,
                    start_token: 0,
                    generation_id: "namespace-control-refusal".into(),
                    exit_code,
                    exit_signal: None,
                    group_exited: false,
                    isolation_mode: "detached".into(),
                    scope_unit: None,
                    start_operation: None,
                };
                control.write_generation("control", &generation).unwrap();
                let before = fs::read(control.record_path("control")).unwrap();
                let error = control.observe("control").unwrap_err();
                assert!(error.is::<crate::ProcessNamespaceMismatch>());
                for (operation, result) in [
                    ("stop", control.stop("control")),
                    ("kill", control.kill("control")),
                    (
                        "signal",
                        control.signal_if("control", None, libc::SIGTERM),
                    ),
                ] {
                    let error = result.unwrap_err();
                    assert!(
                        error.is::<crate::ProcessNamespaceMismatch>(),
                        "{operation} lost the typed refusal: {error:#}"
                    );
                    eprintln!(
                        "namespace control refusal: {operation}, exit_code={exit_code:?}, typed={error}"
                    );
                }
                assert_eq!(fs::read(control.record_path("control")).unwrap(), before);
            }
            assert!(!root.join("control-logs").exists());
            fs::write(root.join("refusal-confirmed"), b"before-child").unwrap();
            return;
        }

        let root = tempfile::tempdir().unwrap();
        // Deliberately inherit procfs rather than mounting one for the new PID
        // namespace. Re-exec the real test binary so spawn runs in that namespace.
        let output = std::process::Command::new("unshare")
            .args(["--user", "--map-root-user", "--pid", "--fork"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::inherited_outer_procfs_refuses_launch_and_control_before_mutation",
                "--nocapture",
            ])
            .env(ROOT, root.path())
            .output()
            .expect("launch inherited-procfs namespace regression");
        // A dev box can forbid unprivileged user namespaces (Ubuntu's AppArmor sets
        // kernel.apparmor_restrict_unprivileged_userns=1), and then unshare fails before the
        // regression starts. Outside CI only, say so and record the skip in CI_TEST_ENV_SKIPS,
        // which st-test signs into the note. CI always runs it.
        let in_ci = std::env::var_os("CI_RUN_ID").is_some() || std::env::var_os("GITHUB_ACTIONS").is_some();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !in_ci
            && output.status.code() == Some(1)
            && stderr
                .lines()
                .any(|line| line.starts_with("unshare: ") && line.ends_with("Operation not permitted"))
        {
            eprintln!("skipped: no unprivileged userns on this host ({})", stderr.trim());
            if let Some(skips) = std::env::var_os("CI_TEST_ENV_SKIPS") {
                use std::io::Write;
                writeln!(
                    fs::OpenOptions::new().create(true).append(true).open(skips).unwrap(),
                    "fractal-runtime process::tests::inherited_outer_procfs_refuses_launch_and_control_before_mutation"
                )
                .unwrap();
            }
            return;
        }
        assert!(
            output.status.success(),
            "namespace regression failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!root.path().join("exec").exists());
        assert!(!root.path().join("logs").exists());
        assert!(!root.path().join("child-created").exists());
        assert!(!root.path().join("pty").exists());
        assert!(!root.path().join("pty-child-created").exists());
        assert_eq!(
            fs::read(root.path().join("refusal-confirmed")).unwrap(),
            b"before-child"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_start_token_survives_a_fast_process_exit() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();

        assert_eq!(process_start_token(pid).unwrap(), pid as u64);
    }

    #[test]
    fn an_absent_process_is_a_normal_exit_on_every_unix_platform() {
        let error = anyhow::Error::from(std::io::Error::from_raw_os_error(libc::ESRCH));

        assert!(process_is_absent(&error));
    }

    #[test]
    fn a_terminal_record_stays_exited_when_its_pid_is_live() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let pid = std::process::id();
        let generation = ExecGeneration {
            schema: SCHEMA.into(),
            pid,
            created_at_unix_ms: 1,
            start_token: process_start_token(pid).unwrap(),
            generation_id: "completed-generation".into(),
            exit_code: Some(0),
            exit_signal: None,
            group_exited: true,
            isolation_mode: "detached".into(),
            scope_unit: None,
            start_operation: None,
        };
        fs::create_dir_all(&runtime.state_dir).unwrap();
        runtime.write_generation("completed", &generation).unwrap();

        assert_eq!(
            runtime.observe("completed").unwrap(),
            Some(ExecObservation::Exited(generation))
        );
    }

    #[test]
    fn launch_operation_is_bound_to_the_persisted_exec_generation() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let mut environment = std::env::vars().collect::<BTreeMap<_, _>>();
        environment.insert("ST3_START_OPERATION".into(), "operation-one".into());
        let launched = runtime
            .spawn(
                "operation-proof",
                &crate::Launch::Argv(vec!["sh".into(), "-c".into(), "exit 0".into()]),
                root.path(),
                &environment,
            )
            .unwrap();
        assert_eq!(launched.start_operation.as_deref(), Some("operation-one"));
        let exited = wait_for_exit(&runtime, "operation-proof");
        assert_eq!(exited.generation_id, launched.generation_id);
        assert_eq!(exited.start_operation, launched.start_operation);
        let reopened = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert_eq!(
            reopened.observe("operation-proof").unwrap(),
            Some(ExecObservation::Exited(exited))
        );
    }

    fn wait_for_exit(runtime: &ExecRuntime, id: &str) -> ExecGeneration {
        for _ in 0..200 {
            match runtime.observe(id).unwrap() {
                Some(ExecObservation::Exited(generation)) => return generation,
                Some(ExecObservation::Running(_)) => thread::sleep(Duration::from_millis(10)),
                other => panic!("unexpected exec observation: {other:?}"),
            }
        }
        panic!("the exec did not exit");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reaps_an_owned_child_before_committing_its_terminal_record() {
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
        use std::os::unix::ffi::OsStrExt as _;

        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let release_path = root.path().join("release");
        let release_name = std::ffi::CString::new(release_path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(release_name.as_ptr(), 0o600) }, 0);
        let generation = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "sh".into(),
                    "-c".into(),
                    "read release < \"$1\"; exit 7".into(),
                    "sh".into(),
                    release_path.to_str().unwrap().into(),
                ]),
                root.path(),
                &std::env::vars().collect(),
            )
            .unwrap();
        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(release_path)
            .unwrap();
        // Hold the child alive through observe's first status probe, then make it exit
        // before the process-identity probe. The FIFO and pidfd make the race deterministic.
        assert!(
            runtime
                .owned_children
                .lock()
                .unwrap()
                .get_mut(&generation.pid)
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, generation.pid, 0) };
        assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
        let exited = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        writeln!(release, "exit").unwrap();
        drop(release);
        let mut event = libc::pollfd {
            fd: exited.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut event, 1, -1) }, 1);
        assert_ne!(event.revents & libc::POLLIN, 0);
        assert_eq!(process_identity(generation.pid).unwrap().0, 'Z');

        let observed = runtime.observe_exited_group("work", generation).unwrap();
        let ExecObservation::Exited(ended) = observed else {
            panic!("unexpected exec observation: {observed:?}");
        };
        assert_eq!(ended.exit_code, Some(7));
        assert_eq!(ended.exit_signal, None);
        assert!(ended.group_exited);
        assert!(runtime.owned_children.lock().unwrap().is_empty());
        let reopened = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert_eq!(
            reopened.observe("work").unwrap(),
            Some(ExecObservation::Exited(ended))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_owned_leader_exit_keeps_its_status_while_a_descendant_is_running() {
        use std::io::BufRead as _;
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
        use std::os::unix::ffi::OsStrExt as _;

        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let descendant_release = root.path().join("descendant-release");
        let descendant_pid = root.path().join("descendant-pid");
        let leader_release = root.path().join("leader-release");
        for path in [&descendant_release, &descendant_pid, &leader_release] {
            let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        let generation = runtime
                .spawn(
                    "work",
                    &crate::Launch::Argv(vec![
                        "sh".into(),
                        "-c".into(),
                        "sh -c 'read release < \"$1\"' sh \"$1\" & printf '%s\\n' \"$!\" > \"$2\"; read release < \"$3\"; exit 7".into(),
                        "sh".into(),
                        descendant_release.to_str().unwrap().into(),
                        descendant_pid.to_str().unwrap().into(),
                        leader_release.to_str().unwrap().into(),
                    ]),
                    root.path(),
                    &std::env::vars().collect(),
                )
                .unwrap();
        let mut pid = String::new();
        std::io::BufReader::new(fs::File::open(descendant_pid).unwrap())
            .read_line(&mut pid)
            .unwrap();
        let descendant = pid.trim().parse::<u32>().unwrap();
        let pidfd = |pid: u32| {
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
            unsafe { OwnedFd::from_raw_fd(fd as i32) }
        };
        let wait = |fd: &OwnedFd| {
            let mut event = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            assert_eq!(unsafe { libc::poll(&mut event, 1, -1) }, 1);
            assert_ne!(event.revents & libc::POLLIN, 0);
        };
        let leader_exited = pidfd(generation.pid);
        let descendant_exited = pidfd(descendant);
        // As in the no-descendant regression, the child exits between the first status
        // probe and the process-identity probe. The descendant stays blocked on its FIFO.
        assert!(
            runtime
                .owned_children
                .lock()
                .unwrap()
                .get_mut(&generation.pid)
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(leader_release)
            .unwrap();
        writeln!(release, "exit").unwrap();
        drop(release);
        wait(&leader_exited);
        assert_eq!(process_identity(generation.pid).unwrap().0, 'Z');

        let observed = runtime.observe_exited_group("work", generation).unwrap();
        let ExecObservation::Running(running) = observed else {
            panic!("the live descendant must keep its group running: {observed:?}");
        };
        assert_eq!(running.exit_code, Some(7));
        assert!(!running.group_exited);
        assert!(runtime.owned_children.lock().unwrap().is_empty());
        assert_eq!(
            runtime.observe("work").unwrap(),
            Some(ExecObservation::Running(running))
        );

        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(descendant_release)
            .unwrap();
        writeln!(release, "exit").unwrap();
        drop(release);
        wait(&descendant_exited);
        let Some(ExecObservation::Exited(ended)) = runtime.observe("work").unwrap() else {
            panic!("the group must stop running once its descendant exits");
        };
        assert_eq!(ended.exit_code, Some(7));
        assert!(ended.group_exited);
        let reopened = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert_eq!(
            reopened.observe("work").unwrap(),
            Some(ExecObservation::Exited(ended))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_owned_exec_with_a_zombie_leader_thread_is_running() {
        assert_zombie_leader_thread_is_running(false);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_adopted_exec_with_a_zombie_leader_thread_is_running() {
        assert_zombie_leader_thread_is_running(true);
    }

    #[cfg(target_os = "linux")]
    fn assert_zombie_leader_thread_is_running(adopted: bool) {
        use std::io::BufRead as _;
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
        use std::os::unix::ffi::OsStrExt as _;

        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let ready_path = root.path().join("ready");
        let release_path = root.path().join("release");
        for path in [&ready_path, &release_path] {
            let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        let ready = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&ready_path)
            .unwrap();
        // pthread_exit terminates only the main thread. The worker reports readiness
        // after the kernel exposes that thread as Z, then lives until explicitly released.
        let fixture = r#"
import ctypes, os, sys, threading
pid = os.getpid()
def worker():
    while open(f'/proc/{pid}/stat').read().rsplit(') ', 1)[1].split()[0] != 'Z':
        os.sched_yield()
    with open(sys.argv[1], 'w') as ready:
        ready.write('ready\n')
    with open(sys.argv[2]) as release:
        release.readline()
    os._exit(7)
threading.Thread(target=worker).start()
libc = ctypes.CDLL(None)
libc.pthread_exit.argtypes = [ctypes.c_void_p]
libc.pthread_exit.restype = None
libc.pthread_exit(None)
"#;
        let generation = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "python3".into(),
                    "-c".into(),
                    fixture.into(),
                    ready_path.to_str().unwrap().into(),
                    release_path.to_str().unwrap().into(),
                ]),
                root.path(),
                &std::env::vars().collect(),
            )
            .unwrap();
        // A pidfd confines cleanup to this process even after its PID is reaped/reused.
        // It must also bypass observe, which blocks in the broken implementation.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, generation.pid, 0) };
        assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
        struct Cleanup(ExecRuntime, u32, OwnedFd);
        impl Cleanup {
            fn kill(&self) -> std::io::Result<()> {
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        self.2.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                };
                if result == 0 {
                    return Ok(());
                }
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.kill();
                if let Some(mut child) = self.0.owned_children.lock().unwrap().remove(&self.1) {
                    let _ = child.wait();
                }
            }
        }
        let cleanup = Cleanup(runtime.clone(), generation.pid, unsafe {
            OwnedFd::from_raw_fd(fd as i32)
        });
        let mut event = libc::pollfd {
            fd: ready.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(
            unsafe { libc::poll(&mut event, 1, 10_000) },
            1,
            "worker never became ready"
        );
        let mut line = String::new();
        std::io::BufReader::new(ready).read_line(&mut line).unwrap();
        assert_eq!(line, "ready\n");
        assert_eq!(process_identity(generation.pid).unwrap().0, 'Z');
        assert!(
            runtime
                .owned_children
                .lock()
                .unwrap()
                .get_mut(&generation.pid)
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none(),
            "the process must not yet be waitable"
        );
        let observing = if adopted {
            ExecRuntime::new(root.path().join("exec"), root.path().join("logs"))
        } else {
            runtime.clone()
        };
        let (send, receive) = std::sync::mpsc::channel();
        let observer = thread::spawn(move || {
            send.send(observing.observe("work")).unwrap();
        });
        let result = receive.recv_timeout(Duration::from_secs(10));
        if result.is_err() {
            cleanup.kill().unwrap();
            observer.join().unwrap();
            panic!("observe blocked on a zombie leader thread with a live worker");
        }
        observer.join().unwrap();
        assert_eq!(
            result.unwrap().unwrap(),
            Some(ExecObservation::Running(generation.clone()))
        );
        assert_eq!(
            runtime.observe("work").unwrap(),
            Some(ExecObservation::Running(generation))
        );
        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(release_path)
            .unwrap();
        writeln!(release, "exit").unwrap();
        drop(release);
        let ended = wait_for_exit(&runtime, "work");
        assert_eq!(ended.exit_code, Some(7));
        assert!(ended.group_exited);
    }

    #[test]
    fn retains_the_current_and_previous_log_generations() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        let cwd = root.path();
        let first = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "sh".into(),
                    "-c".into(),
                    "printf first; exit 3".into(),
                ]),
                cwd,
                &environment,
            )
            .unwrap();
        let first_exit = wait_for_exit(&runtime, "work");
        assert_eq!(first_exit.exit_code, Some(3));

        let second = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec!["sh".into(), "-c".into(), "printf second".into()]),
                cwd,
                &environment,
            )
            .unwrap();
        let second_exit = wait_for_exit(&runtime, "work");

        assert_ne!(first.generation_id, second.generation_id);
        assert_eq!(second_exit.exit_code, Some(0));
        assert_eq!(
            runtime.previous_generation("work").unwrap().unwrap(),
            first_exit
        );
        assert_eq!(
            runtime.read_log_bytes("work", true).unwrap().unwrap(),
            b"first"
        );
        assert_eq!(
            runtime.read_log_bytes("work", false).unwrap().unwrap(),
            b"second"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exited_wrapper_keeps_its_resistant_children_owned_after_adoption() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let generation = runtime.spawn(
            "work",
            &crate::Launch::Shell("sh -c 'trap \"\" TERM; echo $$ > child.pid; while :; do sleep 60; done' & wait".into()),
            root.path(), &std::env::vars().collect(),
        ).unwrap();
        struct Cleanup(i32);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    libc::kill(-self.0, libc::SIGKILL);
                }
            }
        }
        let _cleanup = Cleanup(generation.pid as i32);
        // The shell creates child.pid before it writes the pid: wait for the pid, not the file.
        let mut child = None;
        for _ in 0..200 {
            child = fs::read_to_string(root.path().join("child.pid"))
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            if child.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let child = child.expect("the child never wrote its pid");
        runtime
            .stop_if("work", Some(&generation.generation_id))
            .unwrap();
        // Reap the wrapper and retain its exit status while the child still runs.
        for _ in 0..200 {
            if let Some(ExecObservation::Running(observed)) = runtime.observe("work").unwrap()
                && observed.exit_signal.is_some()
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let adopted = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert!(matches!(
            adopted.observe("work").unwrap(),
            Some(ExecObservation::Running(_))
        ));
        assert!(adopted.kill_if("work", Some("another-generation")).is_err());
        adopted
            .kill_if("work", Some(&generation.generation_id))
            .unwrap();
        let exited = wait_for_exit(&adopted, "work");
        assert_eq!(exited.exit_signal, Some(libc::SIGTERM));
        assert!(process_identity(child).map_or(true, |(state, _)| state == 'Z'));
    }

    /// A launch that starts `orphan.pid`'s writer in a session of its own whose parent exits at
    /// once, waits for the pid, then runs `then`. The writer has left the exec's process group.
    #[cfg(target_os = "linux")]
    fn orphaning_launch(then: &str) -> crate::Launch {
        crate::Launch::Shell(format!(
            "(setsid sh -c 'echo $$ > orphan.pid; exec sleep 600' &); \
             while [ ! -s orphan.pid ]; do sleep 0.02; done; {then}"
        ))
    }

    #[cfg(target_os = "linux")]
    fn read_orphan(root: &Path) -> u32 {
        for _ in 0..500 {
            if let Some(pid) = fs::read_to_string(root.join("orphan.pid"))
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the exec never started its background process");
    }

    #[cfg(target_os = "linux")]
    fn assert_gone_soon(pid: u32) {
        for _ in 0..250 {
            if process_identity(pid).map_or(true, |(state, _)| state == 'Z') {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        panic!("background process {pid} outlived its exec");
    }

    /// Kills the test's exec when dropped, so a failing test leaves nothing behind.
    #[cfg(target_os = "linux")]
    struct KillOnDrop(ExecRuntime);

    #[cfg(target_os = "linux")]
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill("work");
        }
    }

    /// The environment a member needs to start in a systemd user scope, and no more.
    #[cfg(target_os = "linux")]
    fn scope_environment() -> BTreeMap<String, String> {
        ["PATH", "HOME", "XDG_RUNTIME_DIR"]
            .into_iter()
            .filter_map(|key| Some((key.to_owned(), std::env::var(key).ok()?)))
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stopping_an_exec_ends_what_it_left_running() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let generation = runtime
            .spawn(
                "work",
                &orphaning_launch("exec sleep 600"),
                root.path(),
                &scope_environment(),
            )
            .unwrap();
        let orphan = read_orphan(root.path());

        runtime
            .stop_if("work", Some(&generation.generation_id))
            .unwrap();

        wait_for_exit(&runtime, "work");
        assert_gone_soon(orphan);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_ended_execs_leftovers_end_and_its_replacement_keeps_running() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let ended = runtime
            .spawn(
                "work",
                &orphaning_launch("exit 0"),
                root.path(),
                &scope_environment(),
            )
            .unwrap();
        let orphan = read_orphan(root.path());
        wait_for_exit(&runtime, "work");

        let replacement = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec!["sleep".into(), "600".into()]),
                root.path(),
                &scope_environment(),
            )
            .unwrap();

        assert_gone_soon(orphan);
        // Each launch has a scope of its own. Ending the ended one again, as a reconcile pass
        // that read the record before the replacement started does, leaves the replacement.
        let ended_unit = ended.scope_unit.unwrap();
        assert_ne!(Some(&ended_unit), replacement.scope_unit.as_ref());
        assert!(crate::end_scope(&ended_unit, crate::SCOPE_GRACE).unwrap());
        runtime.end_leftovers("work").unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(matches!(
            runtime.observe("work").unwrap(),
            Some(ExecObservation::Running(_))
        ));
        runtime
            .kill_if("work", Some(&replacement.generation_id))
            .unwrap();
        wait_for_exit(&runtime, "work");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_ended_exec_ends_what_it_left_running() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        runtime
            .spawn(
                "work",
                &orphaning_launch("exit 0"),
                root.path(),
                &scope_environment(),
            )
            .unwrap();
        let orphan = read_orphan(root.path());
        wait_for_exit(&runtime, "work");

        // The reconciler asks this on each pass in which a stopped runtime is not running.
        runtime.end_leftovers("work").unwrap();

        assert_gone_soon(orphan);
    }

    /// What an exec's grandchild reports about its own priority: `sh -c nice` run by `sh`, so the
    /// value is the one a build the seat starts would inherit.
    #[cfg(target_os = "linux")]
    fn inherited_nice(id: &str, priority: Option<&str>) -> String {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let mut environment = scope_environment();
        if let Some(priority) = priority {
            environment.insert(crate::PRIORITY_ENV.into(), priority.into());
        }
        runtime
            .spawn(
                id,
                &crate::Launch::Shell("sh -c nice".into()),
                root.path(),
                &environment,
            )
            .unwrap();
        wait_for_exit(&runtime, id);
        runtime
            .read_log(id)
            .unwrap()
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exec_starts_below_the_daemon_unless_its_seat_asks_for_normal() {
        let base = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
        assert_eq!(
            inherited_nice("default", None),
            (base + 10).min(19).to_string()
        );
        assert_eq!(
            inherited_nice("explicit", Some("background")),
            (base + 10).min(19).to_string()
        );
        assert_eq!(inherited_nice("normal", Some("normal")), base.to_string());
    }
}
