//! Inactive host/class keeper. Only the explicitly constructed launch adapter watches it.
//! The keeper lives in idle/, a sibling of every J; it has no daemon FD or lifetime tie.
use anyhow::{Context, Result, ensure};
use fractal_jobs::launch::{LaunchRefusal, keeper_name};
use std::{fs, path::{Path, PathBuf}, sync::Mutex};

// All adapters in this daemon share mutation ownership, including watcher repair
// and pre-job ensure. Never hold this lock while waiting for kernel events.
static KEEPER_MUTATION: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug)]
pub struct KeeperScope {
    pub directory: PathBuf,
    pub slice: String,
    pub delegated: bool,
}

/// Systemd and cgroupfs capability boundary. Fixtures emulate kernel-created
/// cgroup files in mkdir/move_pid, while all validation uses the real file readers.
pub trait KeeperManager {
    fn describe(&mut self, name: &str) -> Result<Option<KeeperScope>>;
    fn start(&mut self, name: &str, slice: &str, pid: u32) -> Result<()>;
    fn spawn_idle(&mut self) -> Result<u32>;
    fn kill_idle(&mut self, pid: u32) -> Result<()>;
    /// End numeric-PID placement ownership only after all verification succeeds.
    fn commit_idle(&mut self, _pid: u32) -> Result<()> { Ok(()) }
    fn mkdir(&mut self, directory: &Path) -> Result<()> { Ok(fs::create_dir(directory)?) }
    fn move_pid(&mut self, pid: u32, directory: &Path) -> Result<()> {
        Ok(fs::write(directory.join("cgroup.procs"), pid.to_string())?)
    }
}


/// Match B6: normal user@UID.service ancestry is allowed, but J itself may not
/// be a user-manager service, and session scopes may not occur anywhere above it.
pub fn validate_path(directory: &Path) -> Result<()> {
    let path = directory.to_str().context(LaunchRefusal::InvalidCgroupPath)?;
    ensure!(directory.is_absolute() && path != "/" && !path.contains("..")
        && !directory.components().any(|part| {
            let part = part.as_os_str().to_string_lossy();
            part.starts_with("session-") && part.ends_with(".scope")
        }) && !directory.file_name().is_some_and(|part| {
            let part = part.to_string_lossy();
            part.starts_with("user@") && part.ends_with(".service")
        }), LaunchRefusal::InvalidCgroupPath);
    Ok(())
}

pub fn populated(directory: &Path) -> Result<bool> {
    let events = fs::read_to_string(directory.join("cgroup.events"))?;
    match events.lines().find_map(|line| line.strip_prefix("populated ")) {
        Some("0") => Ok(false),
        Some("1") => Ok(true),
        _ => anyhow::bail!("cgroup population is unknown"),
    }
}

pub fn slice_name(configured: Option<&str>) -> Result<&str> {
    let slice = configured.filter(|s| !s.is_empty()).context(LaunchRefusal::MissingHeavySlice)?
        .rsplit('/').next().context(LaunchRefusal::MissingHeavySlice)?;
    ensure!(slice.ends_with(".slice") && slice.bytes().all(|b| b.is_ascii_alphanumeric()
        || matches!(b, b'-' | b'_' | b'.')), LaunchRefusal::KeeperUnavailable);
    Ok(slice)
}

/// Idempotent, but unreachable until explicit activation. Reuse only after
/// verifying manager identity, configured slice, delegation, controllers and idle.
pub fn ensure_keeper(
    manager: &mut dyn KeeperManager,
    host: &str,
    configured_slice: Option<&str>,
) -> Result<KeeperScope> {
    let _guard = KEEPER_MUTATION.lock().map_err(|_| anyhow::anyhow!("keeper mutation poisoned"))?;
    let slice = slice_name(configured_slice)?;
    let name = keeper_name(host);
    let mut fresh_pid = None;
    let scope = if let Some(scope) = manager.describe(&name)? {
        scope
    } else {
        let pid = manager.spawn_idle()?;
        if let Err(error) = manager.start(&name, slice, pid) {
            manager.kill_idle(pid)?;
            return Err(error.context(LaunchRefusal::KeeperUnavailable));
        }
        fresh_pid = Some(pid);
        match manager.describe(&name) {
            Ok(Some(scope)) => scope,
            result => {
                manager.kill_idle(pid)?;
                return Err(result.err().unwrap_or_else(|| anyhow::anyhow!("keeper scope missing"))
                    .context(LaunchRefusal::KeeperUnavailable));
            }
        }
    };
    let verified = (|| {
        validate_path(&scope.directory)?;
        ensure!(scope.delegated && scope.slice == slice, LaunchRefusal::KeeperUnavailable);
        let controllers = fs::read_to_string(scope.directory.join("cgroup.controllers"))?;
        let controllers = controllers.split_whitespace().collect::<Vec<_>>();
        ensure!(controllers.contains(&"memory") && controllers.contains(&"pids"),
            LaunchRefusal::KeeperUnavailable);
        let idle = scope.directory.join("idle");
        if !idle.is_dir() {
            ensure!(fresh_pid.is_some(), LaunchRefusal::KeeperUnavailable);
            manager.mkdir(&idle)?;
        }
        if let Some(pid) = fresh_pid {
            manager.move_pid(pid, &idle)?;
        }
        ensure!(fs::read_to_string(scope.directory.join("cgroup.procs"))?.trim().is_empty(),
            LaunchRefusal::KeeperUnavailable);
        // Only io is optional. Unit 12 does not set CPU properties.
        let enable = if controllers.contains(&"io") { "+memory +pids +io" } else { "+memory +pids" };
        fs::write(scope.directory.join("cgroup.subtree_control"), enable)?;
        let enabled = fs::read_to_string(scope.directory.join("cgroup.subtree_control"))?;
        ensure!(["memory", "pids"].iter().all(|needed| enabled.split_whitespace()
            .any(|actual| actual.trim_start_matches('+') == *needed)), LaunchRefusal::KeeperUnavailable);
        replenish_locked(manager, &scope)?;
        Ok(())
    })();
    if verified.is_err() && let Some(pid) = fresh_pid {
        manager.kill_idle(pid)?;
    }
    verified.context(LaunchRefusal::KeeperUnavailable)?;
    if let Some(pid) = fresh_pid { manager.commit_idle(pid)?; }
    Ok(scope)
}

/// Unreachable watcher repair step. An empty idle is replenished in the SAME
/// delegated scope; no new scope and no job relaunch. Called again before each job.
pub fn replenish_keeper(manager: &mut dyn KeeperManager, scope: &KeeperScope) -> Result<()> {
    let _guard = KEEPER_MUTATION.lock().map_err(|_| anyhow::anyhow!("keeper mutation poisoned"))?;
    replenish_locked(manager, scope)
}

fn replenish_locked(manager: &mut dyn KeeperManager, scope: &KeeperScope) -> Result<()> {
    let idle = scope.directory.join("idle");
    if !populated(&idle)? {
        let pid = manager.spawn_idle()?;
        let placed = (|| {
            manager.move_pid(pid, &idle)?;
            ensure!(populated(&idle)?, LaunchRefusal::KeeperUnavailable);
            Ok(())
        })();
        if let Err(error) = placed {
            manager.kill_idle(pid)?;
            return Err(error);
        }
        manager.commit_idle(pid)?;
    }
    Ok(())
}


#[cfg(test)]
mod tests;
