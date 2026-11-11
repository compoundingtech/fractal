//! Inactive host/class keeper. No startup, watcher or scheduler calls this module.
//! The keeper lives in idle/, a sibling of every J; it has no daemon FD or lifetime tie.
use anyhow::{Context, Result, ensure};
use fractal_jobs::launch::{LaunchRefusal, keeper_name};
use std::{fs, io::{Read, Seek}, os::fd::AsRawFd, path::{Path, PathBuf}};

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
        replenish_keeper(manager, &scope)?;
        Ok(())
    })();
    if verified.is_err() && let Some(pid) = fresh_pid {
        manager.kill_idle(pid)?;
    }
    verified.context(LaunchRefusal::KeeperUnavailable)?;
    Ok(scope)
}

/// Unreachable watcher repair step. An empty idle is replenished in the SAME
/// delegated scope; no new scope and no job relaunch. Called again before each job.
pub fn replenish_keeper(manager: &mut dyn KeeperManager, scope: &KeeperScope) -> Result<()> {
    let idle = scope.directory.join("idle");
    if !populated(&idle)? {
        let pid = manager.spawn_idle()?;
        if let Err(error) = manager.move_pid(pid, &idle) {
            manager.kill_idle(pid)?;
            return Err(error);
        }
        ensure!(populated(&idle)?, LaunchRefusal::KeeperUnavailable);
    }
    Ok(())
}

/// Wait for the next kernel cgroup.events notification, then immediately repair
/// a dead keeper. No production caller; activation must run this continuously.
/// Open/arm before inspecting population so a concurrent death is not lost.
pub fn watch_keeper_once(manager: &mut dyn KeeperManager, scope: &KeeperScope) -> Result<()> {
    let mut events = fs::File::open(scope.directory.join("idle/cgroup.events"))?;
    let mut contents = String::new();
    events.read_to_string(&mut contents)?;
    if contents.lines().any(|line| line == "populated 0") {
        return replenish_keeper(manager, scope);
    }
    let mut descriptor = libc::pollfd { fd: events.as_raw_fd(), events: libc::POLLPRI | libc::POLLERR, revents: 0 };
    loop {
        // SAFETY: one valid descriptor, infinite wait, no borrowed buffers.
        let result = unsafe { libc::poll(&mut descriptor, 1, -1) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted { continue; }
            return Err(error.into());
        }
        events.rewind()?;
        contents.clear();
        events.read_to_string(&mut contents)?;
        return replenish_keeper(manager, scope);
    }
}

#[cfg(test)]
mod tests;
