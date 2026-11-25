//! Shared task isolation for st2 and st3.
//!
//! A new process session survives a parent process exit. It does not escape a Linux service cgroup.
//!
//! Linux uses a transient systemd user scope when the user manager is available. The scope is a
//! sibling of the daemon service, so a daemon cgroup restart does not stop the task.
//!
//! Named instances keep their own runtime directory for tasks, but systemd clients use
//! `ST_INSTANCE_HOST_XDG_RUNTIME_DIR` to reach the host's user manager.
//!
//! Other Unix hosts use a detached process session. Linux uses the same fallback when its user
//! manager is unavailable, but reports that mode as degraded.
//!
//! The scope provides survival and descendant containment. The runtimes still own adoption,
//! signals, and teardown. Teardown ends with the scope: [`end_scope`] ends every process a task
//! started, including one that left its process tree or started a process group of its own.

use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

mod retirement;
pub mod darwin_slot;
pub use retirement::{CgroupEnding, ScopeRetirementProof, end_scope_with_proof};
pub(crate) use retirement::observe_closed_scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    Scope,
    DarwinSlot { slot: &'static str },
    Detached,
    DegradedDetached,
}

static MODE: OnceLock<Isolation> = OnceLock::new();
static SCOPE_EXPANSION_FLAG: OnceLock<bool> = OnceLock::new();
static WARNED: AtomicBool = AtomicBool::new(false);
static DARWIN_HELPER_AVAILABLE: OnceLock<bool> = OnceLock::new();
static SCOPE_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn mode() -> Isolation {
    *MODE.get_or_init(detect)
}

/// Select daemon isolation before constructing runtimes, using its captured login environment.
/// Other callers retain the existing ambient-environment default.
pub fn initialize_isolation(environment: &std::collections::BTreeMap<String, String>) -> Isolation {
    *MODE.get_or_init(|| detect_in(environment))
}

// --collect arrived in v236. Before v254, scope mode execs the original argv
// directly; percent and dollar escaping would change the task's literal bytes.
fn scope_expansion_flag(output: &[u8]) -> Option<bool> {
    let mut words = std::str::from_utf8(output).ok()?.split_ascii_whitespace();
    if words.next()? != "systemd" {
        return None;
    }
    let version = words.next()?.parse::<u32>().ok()?;
    (version >= 236).then_some(version >= 254)
}

// Only the systemd client uses the host runtime directory. The scoped task keeps the
// instance directory, just as other children of the daemon do.
fn host_runtime_dir(environment: &std::collections::BTreeMap<String, String>) -> Option<&str> {
    if environment
        .get("ST_INSTANCE")
        .is_some_and(|name| !name.is_empty())
    {
        environment
            .get("ST_INSTANCE_HOST_XDG_RUNTIME_DIR")
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    } else {
        environment.get("XDG_RUNTIME_DIR").map(String::as_str)
    }
}

fn systemd_scope_in(environment: &std::collections::BTreeMap<String, String>) -> Option<bool> {
    let runtime_dir = host_runtime_dir(environment)?;
    let systemd_run = crate::resolve_executable("systemd-run", environment).ok()?;
    let output = crate::systemd::output(Command::new(systemd_run)
        .arg("--version")
        .env_clear()
        .envs(environment)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .stderr(Stdio::null())).ok()?;
    if !output.status.success() {
        return None;
    }
    let expansion_flag = scope_expansion_flag(&output.stdout)?;
    let systemctl = crate::resolve_executable("systemctl", environment).ok()?;
    crate::systemd::output(Command::new(systemctl)
        .args(["--user", "show-environment"])
        .env_clear()
        .envs(environment)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null()))
        .ok()?.status.success().then_some(expansion_flag)
}

pub fn warn_if_degraded(product: &str) {
    if mode() == Isolation::DegradedDetached && !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "{product}: WARN systemd user scopes are unavailable; tasks lack cgroup isolation. A daemon restart can stop them. Enable a user systemd manager with linger."
        );
    }
    if cfg!(target_os = "macos") && !WARNED.swap(true, Ordering::Relaxed) {
        let available = *DARWIN_HELPER_AVAILABLE.get_or_init(|| {
            darwin_slot::Helper::detect_in(&std::env::vars().collect()).is_some()
        });
        if !available {
            eprintln!("{product}: WARN {}", darwin_slot::HELPER_UNAVAILABLE);
        }
    }
}

fn detect() -> Isolation {
    detect_in(&std::env::vars().collect())
}

fn detect_in(environment: &std::collections::BTreeMap<String, String>) -> Isolation {
    if cfg!(target_os = "macos") {
        let _ = DARWIN_HELPER_AVAILABLE.set(darwin_slot::Helper::detect_in(environment).is_some());
        // Capability detection does not claim custody before the finite launch protocol is wired.
        return Isolation::Detached;
    }
    if !cfg!(target_os = "linux") {
        return Isolation::Detached;
    }
    if let Some(expansion_flag) = systemd_scope_in(environment) {
        let _ = SCOPE_EXPANSION_FLAG.set(expansion_flag);
        Isolation::Scope
    } else {
        Isolation::DegradedDetached
    }
}

pub fn systemd_user_available() -> bool {
    cfg!(target_os = "linux") && systemd_scope_in(&std::env::vars().collect()).is_some()
}

pub fn scope_unit(product: &str, task_id: &str) -> String {
    scope_unit_name(product, task_id, std::process::id(), SCOPE_SEQ.fetch_add(1, Ordering::Relaxed))
}

fn scope_unit_name(product: &str, task_id: &str, pid: u32, sequence: u64) -> String {
    let safe_product = sanitize(product);
    let safe_task = sanitize(task_id);
    format!("{safe_product}-{safe_task}-{pid}-{sequence}.scope")
}

/// Inverse of `scope_unit`, splitting numeric suffixes from the right.
pub(crate) fn scope_identity<'a>(product: &str, unit: &'a str) -> Option<&'a str> {
    let body = unit.strip_suffix(".scope")?;
    let (body, sequence) = body.rsplit_once('-')?;
    let (body, pid) = body.rsplit_once('-')?;
    let identity = body.strip_prefix(product)?.strip_prefix('-')?;
    (scope_unit_name(product, identity, pid.parse().ok()?, sequence.parse().ok()?) == unit)
        .then_some(identity)
}

pub(crate) fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '.' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub fn wrap(unit: &str, program: &OsStr, arguments: &[&OsStr]) -> Command {
    wrap_in(unit, program, arguments, None, None, None)
}

/// Construct the launcher with its final environment before selecting the host systemd session.
/// A `lane` ([`crate::lane::prepare`]) places the scope in the seat's capped slice.
pub(crate) fn wrap_in(
    unit: &str,
    program: &OsStr,
    arguments: &[&OsStr],
    environment: Option<&std::collections::BTreeMap<String, String>>,
    lane: Option<&crate::lane::Lane>,
    unlaned_seat: Option<&crate::UnlanedSeatPlacement>,
) -> Command {
    wrap_for_mode(
        mode(),
        *SCOPE_EXPANSION_FLAG.get().unwrap_or(&true),
        unit,
        program,
        arguments,
        environment,
        ScopePlacement { lane, unlaned_seat },
    )
}

/// Whether the user manager is systemd 254 or newer, which also supports OOMPolicy on a scope.
pub(crate) fn systemd_254() -> bool {
    // Detection sets the flag, so it runs first.
    mode() == Isolation::Scope && *SCOPE_EXPANSION_FLAG.get().unwrap_or(&false)
}

#[derive(Default)]
struct ScopePlacement<'a> {
    lane: Option<&'a crate::lane::Lane>,
    unlaned_seat: Option<&'a crate::UnlanedSeatPlacement>,
}

fn wrap_for_mode(
    isolation: Isolation,
    expansion_flag: bool,
    unit: &str,
    program: &OsStr,
    arguments: &[&OsStr],
    environment: Option<&std::collections::BTreeMap<String, String>>,
    placement: ScopePlacement<'_>,
) -> Command {
    match isolation {
        Isolation::Scope => {
            begin_scope_join(unit);
            let mut command = Command::new("systemd-run");
            command
                .args([
                    "--user",
                    "--scope",
                    "--collect",
                    "--quiet",
                    "--no-ask-password",
                ])
                .arg(format!("--unit={unit}"))
                .arg("--description=Fractal seat");
            if expansion_flag {
                command.arg("--expand-environment=no");
            }
            if let Some(environment) = environment {
                command.env_clear().envs(environment);
            }
            if let Some(lane) = placement.lane {
                command.args(lane.scope_arguments());
                let seat = |key: &str| environment?.get(key).cloned();
                command.envs(crate::lane::environment(lane, seat, |key| std::env::var(key).ok()));
            } else if let Some(policy) = placement.unlaned_seat {
                policy.apply(&mut command);
            }
            instance_scope_environment(&mut command, environment);
            command.arg("--").arg(program).args(arguments);
            command
        }
        Isolation::DarwinSlot { .. } => unreachable!("Darwin slots require explicit finite authorization"),
        Isolation::Detached | Isolation::DegradedDetached => {
            // Darwin slots are selected per finite launch, never by ambient `mode()`.
            let mut command = Command::new(program);
            if let Some(environment) = environment {
                command.env_clear().envs(environment);
            }
            command.args(arguments);
            command
        }
    }
}

fn instance_scope_environment(
    command: &mut Command,
    environment: Option<&std::collections::BTreeMap<String, String>>,
) {
    let value = |key: &str| match environment {
        Some(environment) => environment.get(key).map(std::ffi::OsString::from),
        None => std::env::var_os(key),
    };
    if !value("ST_INSTANCE").is_some_and(|name| !name.is_empty()) {
        return;
    }
    // systemd-run applies --setenv only when it execs the task, after connecting
    // to the manager. The task therefore retains the instance runtime directory.
    if let Some(runtime_dir) = value("XDG_RUNTIME_DIR") {
        let mut assignment = std::ffi::OsString::from("--setenv=XDG_RUNTIME_DIR=");
        assignment.push(runtime_dir);
        command.arg(assignment);
    }
    match value("ST_INSTANCE_HOST_XDG_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        Some(host) => {
            command.env("XDG_RUNTIME_DIR", host);
        }
        None => {
            command.env_remove("XDG_RUNTIME_DIR");
        }
    }
}

pub(crate) fn systemctl() -> Command {
    let mut command = Command::new("systemctl");
    if std::env::var_os("ST_INSTANCE").is_some_and(|name| !name.is_empty()) {
        match std::env::var_os("ST_INSTANCE_HOST_XDG_RUNTIME_DIR").filter(|value| !value.is_empty())
        {
            Some(host) => {
                command.env("XDG_RUNTIME_DIR", host);
            }
            None => {
                command.env_remove("XDG_RUNTIME_DIR");
            }
        }
    }
    command
}

/// How long the processes left in a work scope get to exit on SIGTERM before SIGKILL.
pub const SCOPE_GRACE: Duration = Duration::from_secs(2);
/// How long a scope gets to empty after SIGKILL.
const SCOPE_KILL_WAIT: Duration = Duration::from_secs(2);

/// Work scopes this process has seen empty. A scope holds only what started in it, so an empty
/// one stays empty, and systemd collects it.
static ENDED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
// Keep the freshness boundary after joining: an observer may still hold an older Arc snapshot.
struct ScopeCreation {
    joining: bool,
    observed_after: Instant,
}

static CREATIONS: Mutex<std::collections::BTreeMap<String, ScopeCreation>> =
    Mutex::new(std::collections::BTreeMap::new());

fn begin_scope_join(unit: &str) {
    CREATIONS.lock().unwrap_or_else(PoisonError::into_inner).insert(unit.into(),
        ScopeCreation { joining: true, observed_after: Instant::now() });
    crate::systemd::invalidate();
}

pub(crate) fn verify_scope_absence(unit: &str, captured_at: Instant) -> Result<()> {
    if let Some(creation) = CREATIONS.lock().unwrap_or_else(PoisonError::into_inner).get(unit) {
        anyhow::ensure!(!creation.joining, "work scope creation has not completed");
        anyhow::ensure!(captured_at > creation.observed_after,
            "work scope absence predates its creation or join");
    }
    Ok(())
}

pub(crate) fn finish_scope_join(unit: &str) {
    let mut creations = CREATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(creation) = creations.get_mut(unit)
        && creation.joining
    {
        creation.observed_after = Instant::now();
        creation.joining = false;
        crate::systemd::invalidate();
    }
}


/// Whether `unit` is a work scope [`scope_unit`] named: never a PTY server's own scope, a
/// service, or the unit this process runs in.
fn is_work_scope(unit: &str) -> bool {
    unit.starts_with("st3-")
        && unit.ends_with(".scope")
        && !unit.starts_with("st3-pty-server-")
        && !unit.contains('/')
        && own_unit().as_deref() != Some(unit)
}

fn own_unit() -> Option<String> {
    let text = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = text.lines().find_map(|line| line.strip_prefix("0::"))?;
    Some(path.rsplit('/').next()?.to_owned())
}

/// Whether `pid` runs in the scope `unit`. `systemd-run --scope` moves itself into its scope when
/// the user manager starts it and only then execs the task; until then the scope is loaded and
/// empty, so a signal to it succeeds and reaches nothing. A process that cannot be read has not.
pub(crate) fn joined(unit: &str, pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup")).is_ok_and(|text| {
        text.lines()
            .filter_map(|line| line.strip_prefix("0::"))
            .any(|path| path.split('/').any(|part| part == unit))
    })
}

/// How long `systemd-run --scope` gets to join its scope. Its request to start the scope gives up
/// sooner, at the bus's 25s method timeout, but its wait for the start job to run has no limit.
pub(crate) const SCOPE_JOIN_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Join {
    Joined,
    Exited,
    TimedOut,
}

/// Waits until `pid` has joined the scope `unit`, or `alive` says it has exited, for up to `limit`.
/// Until it joins, `systemd-run` is in the cgroup of the process that started it, and a kill of
/// that cgroup ends it.
pub(crate) fn await_join(
    unit: &str,
    pid: u32,
    limit: Duration,
    alive: impl Fn() -> Result<bool>,
) -> Result<Join> {
    let started = Instant::now();
    // A join usually takes a few milliseconds, so look often at first.
    let mut pause = Duration::from_millis(1);
    loop {
        if joined(unit, pid) {
            finish_scope_join(unit);
            return Ok(Join::Joined);
        }
        if !alive()? {
            finish_scope_join(unit);
            return Ok(Join::Exited);
        }
        if started.elapsed() >= limit {
            return Ok(Join::TimedOut);
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(10));
    }
}

/// Whether this process has seen the work scope `unit` empty.
pub(crate) fn has_ended(unit: &str) -> bool {
    ENDED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(unit)
}

pub(crate) fn mark_ended(unit: &str) {
    let mut ended = ENDED.lock().unwrap_or_else(PoisonError::into_inner);
    // Scope names identify a single generation. Forgetting terminal names makes historical
    // reconciliation query the manager forever; an empty scope can never gain new members.
    ended.insert(unit.to_owned());
}

/// Ends every process in the work scope `unit`: SIGTERM, up to `grace` for them to exit, then
/// SIGKILL. A scope that is not loaded has ended, since `--collect` removes an empty one.
///
/// Returns whether the scope is now empty. It is false, and nothing is signalled, on a host
/// without systemd user scopes or for a unit that is not a work scope.
#[tracing::instrument(
    skip_all,
    fields(
        caller = "end_scope",
        recorded_scope = unit,
        grace_ms = grace.as_millis() as u64,
        runtime_id = "unavailable",
        operation_id = "unavailable",
        expected_incarnation = "unavailable",
        observed_incarnation = "unavailable",
        registry_status = "unavailable",
        registry_pid = "unavailable",
        registry_created_at = "unavailable",
        registry_exit_code = "unavailable",
        tagged_scope = "unavailable",
        spawn_lock = "not_applicable"
    )
)]
pub fn end_scope(unit: &str, grace: Duration) -> Result<bool> {
    if darwin_slot::slot_for_unit(unit).is_some() {
        return Ok(end_scope_with_proof(unit, &|| Ok(()))?.is_some());
    }
    if mode() != Isolation::Scope || !is_work_scope(unit) {
        tracing::debug!(guard_decision = "skip", reason = "scope_mode_or_work_scope_guard", "scope teardown skipped");
        return Ok(false);
    }
    if has_ended(unit) {
        tracing::debug!(guard_decision = "skip", reason = "already_ended", "scope teardown skipped");
        return Ok(true);
    }
    let Some(cgroup) = control_group(unit)? else {
        tracing::debug!(guard_decision = "skip", reason = "scope_not_loaded", "scope teardown needs no signal");
        mark_ended(unit);
        return Ok(true);
    };
    if !grace.is_zero() && populated(&cgroup) {
        kill_unit(unit, libc::SIGTERM, ScopeSignalCaller::EndScope)?;
        wait_until_empty(&cgroup, grace);
    }
    if populated(&cgroup) {
        kill_unit(unit, libc::SIGKILL, ScopeSignalCaller::EndScope)?;
        anyhow::ensure!(
            wait_until_empty(&cgroup, SCOPE_KILL_WAIT),
            "processes in {unit} survived SIGKILL for {}s",
            SCOPE_KILL_WAIT.as_secs()
        );
    }
    mark_ended(unit);
    Ok(true)
}

type Job = Box<dyn FnOnce() -> Result<()> + Send>;

/// The jobs [`end_later`] queued, their keys, and whether a worker drains them.
struct Later {
    queue: VecDeque<(String, Job)>,
    keys: BTreeSet<String>,
    working: bool,
}

static LATER: Mutex<Later> = Mutex::new(Later {
    queue: VecDeque::new(),
    keys: BTreeSet::new(),
    working: false,
});

/// Runs `job`, which ends a scope, on one background worker, so a caller that may not wait, such
/// as a reconcile pass, never waits out a grace. A job already queued under `key` is not queued
/// twice.
pub(crate) fn end_later(key: String, job: impl FnOnce() -> Result<()> + Send + 'static) {
    let mut later = LATER.lock().unwrap_or_else(PoisonError::into_inner);
    if !later.keys.insert(key.clone()) {
        tracing::debug!(recorded_scope = %key, guard_decision = "skip", reason = "already_queued", "scope teardown queue skipped");
        return;
    }
    later.queue.push_back((key, Box::new(job)));
    if std::mem::replace(&mut later.working, true) {
        return;
    }
    drop(later);
    std::thread::spawn(|| {
        loop {
            let next = {
                let mut later = LATER.lock().unwrap_or_else(PoisonError::into_inner);
                let next = later.queue.pop_front();
                later.working = next.is_some();
                next
            };
            let Some((key, job)) = next else {
                return;
            };
            if let Err(error) = job() {
                eprintln!("fractal: WARN {error:#}");
            }
            LATER
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .keys
                .remove(&key);
        }
    });
}

/// Sends `signal` to every process in the work scope `unit`. Returns whether it was sent: it is
/// not where [`end_scope`] would do nothing, or once the scope has ended.
#[tracing::instrument(
    skip_all,
    fields(
        caller = "signal_scope",
        recorded_scope = unit,
        signal,
        runtime_id = "unavailable",
        operation_id = "unavailable",
        expected_incarnation = "unavailable",
        observed_incarnation = "unavailable",
        registry_status = "unavailable",
        registry_pid = "unavailable",
        registry_created_at = "unavailable",
        registry_exit_code = "unavailable",
        tagged_scope = "unavailable",
        spawn_lock = "not_applicable"
    )
)]
pub fn signal_scope(unit: &str, signal: i32) -> Result<bool> {
    if darwin_slot::slot_for_unit(unit).is_some() {
        return end_scope(unit, Duration::ZERO);
    }
    if mode() != Isolation::Scope || !is_work_scope(unit) || has_ended(unit) {
        tracing::debug!(recorded_scope = unit, signal, caller = "signal_scope", guard_decision = "skip", reason = "scope_mode_work_scope_or_ended_guard", "scope signal skipped");
        return Ok(false);
    }
    kill_unit(unit, signal, ScopeSignalCaller::SignalScope)
}

/// The cgroup directory of `unit`, or none once the unit is not loaded.
fn control_group(unit: &str) -> Result<Option<PathBuf>> {
    let snapshot = crate::systemd::snapshot_after_admission()?;
    let Some(observed) = snapshot.units.get(unit) else {
        verify_scope_absence(unit, snapshot.captured_at)?;
        return Ok(None);
    };
    if observed.load == "not-found" {
        anyhow::ensure!(observed.active == "inactive" && observed.sub == "dead"
            && observed.control_group.is_none() && observed.main_pid == 0,
            "unloaded work scope is not verified inactive");
        verify_scope_absence(unit, snapshot.captured_at)?;
        return Ok(None);
    }
    anyhow::ensure!(observed.load == "loaded", "work scope load state is unverified");
    let cgroup = observed.control_group.as_ref()
        .context("loaded work scope has no verified control group")?;
    anyhow::ensure!(cgroup.file_name() == Some(OsStr::new(unit)),
        "work scope has a different control group");
    finish_scope_join(unit);
    Ok(Some(cgroup.clone()))
}

/// Whether the cgroup still holds a process. A cgroup that is gone holds none; one that cannot
/// be read counts as populated, so the caller never reports an end it did not see.
fn populated(cgroup: &Path) -> bool {
    match std::fs::read_to_string(cgroup.join("cgroup.events")) {
        Ok(events) => events.lines().any(|line| line.trim() == "populated 1"),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn wait_until_empty(cgroup: &Path, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if !populated(cgroup) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Debug, Clone, Copy)]
enum ScopeSignalCaller {
    EndScope,
    SignalScope,
    Retirement,
}

/// `systemctl kill` signals every process in the unit's cgroup. Returns false when the unit was
/// no longer loaded.
fn kill_unit(unit: &str, signal: i32, caller: ScopeSignalCaller) -> Result<bool> {
    let name = match signal {
        libc::SIGTERM => "SIGTERM".to_owned(),
        libc::SIGKILL => "SIGKILL".to_owned(),
        other => other.to_string(),
    };
    let mut command = systemctl();
    command
        .args(["--user", "kill"])
        .arg(format!("--signal={name}"))
        .arg(unit);
    tracing::info!(
        ?caller,
        reason = "signal_work_scope",
        recorded_scope = unit,
        signal,
        guard_decision = "allow",
        runtime_id = "unavailable",
        operation_id = "unavailable",
        expected_incarnation = "unavailable",
        observed_incarnation = "unavailable",
        registry_status = "unavailable",
        registry_pid = "unavailable",
        registry_created_at = "unavailable",
        registry_exit_code = "unavailable",
        tagged_scope = "unavailable",
        spawn_lock = "not_applicable",
        "signalling work scope"
    );
    let captured_at = Instant::now();
    let output = crate::systemd::output(&mut command)
        .with_context(|| format!("ask the systemd user manager to signal {unit}"))?;
    if output.status.success() {
        crate::systemd::invalidate();
        return Ok(true);
    }
    let error = String::from_utf8_lossy(&output.stderr);
    if error.contains("not loaded") {
        tracing::debug!(?caller, recorded_scope = unit, signal, guard_decision = "skip", reason = "scope_not_loaded", "scope signal found no loaded unit");
        if verify_scope_absence(unit, captured_at).is_ok() {
            mark_ended(unit);
        }
        return Ok(false);
    }
    anyhow::bail!(
        "the systemd user manager could not send {name} to {unit}: {}",
        error.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn unset_darwin_helper_retains_detached_isolation() {
        assert_eq!(detect_in(&std::collections::BTreeMap::new()), Isolation::Detached);
    }

    #[test]
    fn scope_version_selects_only_known_argv_semantics() {
        assert_eq!(scope_expansion_flag(b"systemd 249 (249.11)\n"), Some(false));
        assert_eq!(scope_expansion_flag(b"systemd 253\n"), Some(false));
        assert_eq!(scope_expansion_flag(b"systemd 254\n"), Some(true));
        assert_eq!(scope_expansion_flag(b"systemd 260\n"), Some(true));
        assert_eq!(scope_expansion_flag(b"systemd 235\n"), None);
        assert_eq!(scope_expansion_flag(b"unexpected 249\n"), None);
        assert_eq!(scope_expansion_flag(b"systemd unknown\n"), None);
    }

    #[test]
    fn modern_scope_preserves_raw_argv_with_expansion_disabled() {
        let arguments = [OsStr::new("%n:$HOME:${UNSET}:$$:%%"), OsStr::new("")];
        let command = wrap_for_mode(
            Isolation::Scope,
            true,
            "st-studio.scope",
            OsStr::new("provider"),
            &arguments,
            None,
            ScopePlacement::default(),
        );
        let actual = command.get_args().collect::<Vec<_>>();
        let separator = actual
            .iter()
            .position(|arg| *arg == OsStr::new("--"))
            .unwrap();
        assert_eq!(&actual[separator + 2..], &arguments);
        assert!(actual[..separator].contains(&OsStr::new("--expand-environment=no")));
    }

    #[test]
    fn legacy_scope_keeps_literal_percent_dollar_and_non_utf8_argv() {
        use std::os::unix::ffi::OsStringExt as _;
        let bytes = std::ffi::OsString::from_vec(b"%n:$HOME:${UNSET}:$$:%%:\xff".to_vec());
        let arguments = [bytes.as_os_str(), OsStr::new(""), OsStr::new("two words")];
        let program = OsStr::new("/tmp/provider%$literal");
        let command = wrap_for_mode(
            Isolation::Scope,
            false,
            "st-studio.scope",
            program,
            &arguments,
            None,
            ScopePlacement::default(),
        );
        let actual = command.get_args().collect::<Vec<_>>();
        let separator = actual
            .iter()
            .position(|arg| *arg == OsStr::new("--"))
            .unwrap();
        assert_eq!(actual[separator + 1], program);
        assert_eq!(&actual[separator + 2..], &arguments);
        assert!(!actual.contains(&OsStr::new("--expand-environment=no")));
        assert!(actual[..separator].contains(&OsStr::new("--description=Fractal seat")));
        assert!(actual.contains(&OsStr::new("--no-ask-password")));
        assert_eq!(command.get_program(), OsStr::new("systemd-run"));
    }

    #[test]
    fn instance_isolation_probe_uses_the_host_runtime_directory() {
        let root = tempfile::tempdir().unwrap();
        let environment = std::collections::BTreeMap::from([
            ("PATH".into(), root.path().display().to_string()),
            ("ST_INSTANCE".into(), "try".into()),
            ("XDG_RUNTIME_DIR".into(), "/tmp/st-instance/try/run".into()),
            (
                "ST_INSTANCE_HOST_XDG_RUNTIME_DIR".into(),
                "/run/user/1000".into(),
            ),
        ]);
        for program in ["systemd-run", "systemctl"] {
            crate::test_executable::fake_executable(
                root.path(),
                program,
                "#!/bin/sh\n[ \"$XDG_RUNTIME_DIR\" = /run/user/1000 ] || exit 1\nprintf 'systemd 260\\n'\n",
            );
        }
        assert_eq!(systemd_scope_in(&environment), Some(true));
        let mut without_instance = environment.clone();
        without_instance.remove("ST_INSTANCE");
        assert_eq!(systemd_scope_in(&without_instance), None);
        let mut without_host = environment;
        without_host.remove("ST_INSTANCE_HOST_XDG_RUNTIME_DIR");
        assert_eq!(systemd_scope_in(&without_host), None);
    }

    #[test]
    fn instance_scope_uses_the_host_session_but_preserves_the_task_runtime_directory() {
        let environment = std::collections::BTreeMap::from([
            ("ST_INSTANCE".into(), "try".into()),
            ("XDG_RUNTIME_DIR".into(), "/tmp/st-instance/try/run".into()),
            (
                "ST_INSTANCE_HOST_XDG_RUNTIME_DIR".into(),
                "/run/user/1000".into(),
            ),
        ]);
        let command = wrap_for_mode(
            Isolation::Scope,
            true,
            "st3-work.scope",
            OsStr::new("provider"),
            &[OsStr::new("%n:$HOME")],
            Some(&environment),
            ScopePlacement::default(),
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == "XDG_RUNTIME_DIR")
                .unwrap()
                .1,
            Some(OsStr::new("/run/user/1000")),
        );
        let arguments = command.get_args().collect::<Vec<_>>();
        let separator = arguments.iter().position(|arg| *arg == "--").unwrap();
        assert!(arguments[..separator].contains(&OsStr::new(
            "--setenv=XDG_RUNTIME_DIR=/tmp/st-instance/try/run"
        )));
        assert_eq!(
            &arguments[separator + 1..],
            &[OsStr::new("provider"), OsStr::new("%n:$HOME")]
        );
    }

    #[test]
    fn isolation_probe_resolves_tools_from_the_captured_path() {
        use crate::test_executable::fake_executable;
        let root = tempfile::tempdir().unwrap();
        let environment = std::collections::BTreeMap::from([
            ("PATH".into(), root.path().display().to_string()),
            ("XDG_RUNTIME_DIR".into(), root.path().display().to_string()),
            ("ORCHID_CONTROL".into(), "captured".into()),
        ]);
        assert_eq!(systemd_scope_in(&environment), None);
        for program in ["systemd-run", "systemctl"] {
            fake_executable(root.path(), program, "#!/bin/sh\n[ \"$ORCHID_CONTROL\" = captured ] || exit 1\nprintf 'systemd 249 (249.11)\\n'\n");
        }
        assert_eq!(systemd_scope_in(&environment), Some(false));
        fake_executable(
            root.path(),
            "systemd-run",
            "#!/bin/sh\n[ \"$ORCHID_CONTROL\" = captured ] || exit 1\nprintf 'systemd 260\\n'\n",
        );
        assert_eq!(systemd_scope_in(&environment), Some(true));
        fake_executable(root.path(), "systemctl", "#!/bin/sh\nexit 1\n");
        assert_eq!(systemd_scope_in(&environment), None);
    }

    #[test]
    fn scope_names_are_unique_and_safe() {
        let first = scope_unit("st3", "node/demo task");
        let second = scope_unit("st3", "node/demo task");
        assert!(first.starts_with("st3-node_demo_task-"));
        assert!(first.ends_with(".scope"));
        assert_ne!(first, second);
    }

    #[test]
    fn only_a_work_scope_st_named_is_ended() {
        assert!(is_work_scope(&scope_unit("st3", "fleet/demo worker")));
        assert!(!is_work_scope("st3-pty-server-fleet_demo_worker-42.scope"));
        assert!(!is_work_scope("st3.service"));
        assert!(!is_work_scope("app.slice"));
        assert!(!is_work_scope("st3-../app.slice/st3.scope"));
        if let Some(own) = own_unit() {
            assert!(!is_work_scope(&own));
        }
    }

    #[test]
    fn a_pending_scope_is_not_terminal_absence() {
        let unit = "st3-pending-join-absence.scope";
        begin_scope_join(unit);
        assert!(verify_scope_absence(unit, Instant::now()).is_err());
        assert!(!has_ended(unit));
        finish_scope_join(unit);
        assert!(verify_scope_absence(unit, Instant::now()).is_ok());
        assert!(!has_ended(unit));
    }

    #[test]
    fn a_held_snapshot_cannot_prove_absence_after_a_new_join() {
        let unit = "st3-held-snapshot-new-join.scope";
        let held = std::sync::Arc::new(crate::systemd::Snapshot {
            captured_at: Instant::now(),
            units: std::collections::BTreeMap::new(),
            service_identities: std::collections::BTreeMap::new(),
        });
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let reader_barrier = barrier.clone();
        let reader = std::thread::spawn(move || {
            reader_barrier.wait();
            reader_barrier.wait();
            assert!(!held.units.contains_key(unit));
            assert!(verify_scope_absence(unit, held.captured_at).is_err());
            assert!(!has_ended(unit));
        });
        barrier.wait();
        begin_scope_join(unit);
        finish_scope_join(unit);
        barrier.wait();
        reader.join().unwrap();
        assert!(verify_scope_absence(unit, Instant::now()).is_ok());
    }

    #[test]
    fn terminal_scope_memory_survives_large_histories() {
        for index in 0..5000 {
            mark_ended(&format!("st3-retained-history-{index}.scope"));
        }
        for index in 0..5000 {
            assert!(has_ended(&format!("st3-retained-history-{index}.scope")));
        }
    }

    #[test]
    fn unlaned_daemon_seat_placement_is_explicit_and_never_overrides_a_lane() {
        let policy = crate::seat_placement::tests::policy();
        let lane = crate::lane::Lane { slice: crate::lane::seat_slice("work") };
        let arguments = |lane: Option<&crate::lane::Lane>, policy: Option<&crate::UnlanedSeatPlacement>| {
            wrap_for_mode(Isolation::Scope, true, "st3-work.scope",
                OsStr::new("provider"), &[], None, ScopePlacement { lane, unlaned_seat: policy })
                .get_args().map(std::ffi::OsString::from).collect::<Vec<_>>()
        };
        assert_eq!(arguments(None, None), [
            "--user", "--scope", "--collect", "--quiet", "--no-ask-password",
            "--unit=st3-work.scope", "--description=Fractal seat", "--expand-environment=no",
            "--", "provider",
        ].map(std::ffi::OsString::from));
        assert_eq!(arguments(Some(&lane), Some(&policy)), arguments(Some(&lane), None));
        let placed = arguments(None, Some(&policy));
        assert_eq!(placed[8..], [
            "--slice=app-seats.slice", "-p", "MemoryMax=1073741824", "-p",
            "MemorySwapMax=0", "-p", "OOMPolicy=kill", "--", "provider",
        ].map(std::ffi::OsString::from));
        let detached = wrap_for_mode(Isolation::Detached, false, "unused.scope",
            OsStr::new("provider"), &[], None, ScopePlacement { lane: None, unlaned_seat: Some(&policy) });
        assert!(detached.get_args().next().is_none());
    }

    #[test]
    fn a_lane_caps_the_scope_and_keeps_its_builds_inside() {
        let environment = std::collections::BTreeMap::from([
            ("PATH".to_owned(), "/bin".to_owned()),
            ("XDG_RUNTIME_DIR".to_owned(), "/run/user/1000".to_owned()),
        ]);
        let lane = crate::lane::Lane { slice: crate::lane::seat_slice("work") };
        let command = wrap_for_mode(
            Isolation::Scope,
            true,
            "st3-work.scope",
            OsStr::new("provider"),
            &[OsStr::new("--flag")],
            Some(&environment),
            ScopePlacement { lane: Some(&lane), unlaned_seat: None },
        );
        let arguments = command.get_args().collect::<Vec<_>>();
        let separator = arguments.iter().position(|arg| *arg == OsStr::new("--")).unwrap();
        for property in lane.scope_arguments() {
            assert!(arguments[..separator].contains(&OsStr::new(&property)), "{property}");
        }
        assert_eq!(&arguments[separator + 1..], [OsStr::new("provider"), OsStr::new("--flag")]);
        let set = |name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value)
                .map(|value| value.to_string_lossy().into_owned())
        };
        assert_eq!(set("BUILDS_SLICE_ACTIVE").as_deref(), Some("1"));
        assert_eq!(set("FRACTAL_LANE_SLICE"), Some(lane.slice.clone()));
        assert!(set("SCCACHE_SERVER_UDS").unwrap().starts_with("/run/user/1000/sccache-lane-"));

        let detached = wrap_for_mode(
            Isolation::Detached,
            false,
            "unused.scope",
            OsStr::new("provider"),
            &[],
            Some(&environment),
            ScopePlacement { lane: Some(&lane), unlaned_seat: None },
        );
        assert!(detached.get_args().next().is_none());
        assert!(detached.get_envs().all(|(key, _)| key != "BUILDS_SLICE_ACTIVE"));
    }

    #[test]
    fn wrapper_matches_the_detected_mode() {
        let command = wrap(
            "st3-work.scope",
            OsStr::new("sh"),
            &[OsStr::new("-c"), OsStr::new("true")],
        );
        let arguments = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        match mode() {
            Isolation::Scope => {
                assert_eq!(command.get_program(), OsStr::new("systemd-run"));
                assert!(arguments.contains(&"--scope".to_owned()));
                assert!(arguments.contains(&"--unit=st3-work.scope".to_owned()));
                assert_eq!(
                    arguments.contains(&"--expand-environment=no".to_owned()),
                    *SCOPE_EXPANSION_FLAG.get().unwrap()
                );
            }
            Isolation::DarwinSlot { .. } => panic!("Darwin slots must not become ambient isolation"),
            Isolation::Detached | Isolation::DegradedDetached => {
                assert_eq!(command.get_program(), OsStr::new("sh"));
                assert_eq!(arguments, ["-c", "true"]);
            }
        }
    }
}
