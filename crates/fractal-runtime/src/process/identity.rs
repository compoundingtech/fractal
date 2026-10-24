//! Translate caller-namespace PIDs through kernel-owned pidfds before reading procfs.
use std::fs;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

use anyhow::{Context as _, Result, ensure};

/// The runtime cannot bind a caller-namespace PID to exact procfs evidence.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProcessIdentityUnavailable {
    pub pid: u32,
    pub stage: &'static str,
    pub errno: Option<i32>,
}

impl std::fmt::Display for ProcessIdentityUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "process identity unavailable (pid={}, stage={}, errno={:?})", self.pid, self.stage, self.errno)
    }
}
impl std::error::Error for ProcessIdentityUnavailable {}

fn refusal(pid: u32, stage: &'static str, error: anyhow::Error) -> anyhow::Error {
    let errno = error.downcast_ref::<std::io::Error>().and_then(std::io::Error::raw_os_error);
    error.context(ProcessIdentityUnavailable { pid, stage, errno })
}

fn exited(pid: u32) -> anyhow::Error {
    refusal(pid, "pidfd-exited", std::io::Error::from_raw_os_error(libc::ESRCH).into())
}

pub(super) fn is_absent(error: &anyhow::Error) -> Option<bool> {
    error.downcast_ref::<ProcessIdentityUnavailable>().map(|error| {
        matches!(error.stage, "pidfd-open" | "pidfd-exited") && error.errno == Some(libc::ESRCH)
    })
}

fn namespace_ids<'a>(status: &'a str, field: &str) -> Result<impl Iterator<Item = &'a str>> {
    Ok(status.lines().find_map(|line| line.strip_prefix(field))
        .with_context(|| format!("missing {field} namespace identifiers"))?.split_whitespace())
}

fn namespace_depth() -> Result<usize> {
    let pid = std::process::id();
    let status = fs::read_to_string("/proc/self/status")
        .map_err(|error| refusal(pid, "caller-status", error.into()))?;
    let ids = namespace_ids(&status, "NSpid:")
        .and_then(|ids| ids.map(str::parse::<u32>).collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into))
        .map_err(|error| refusal(pid, "caller-namespace", error))?;
    ensure!(ids.last() == Some(&pid), ProcessIdentityUnavailable { pid, stage: "caller-namespace", errno: None });
    Ok(ids.len() - 1)
}

fn mapped_pid(info: &str, pid: u32, depth: usize) -> Result<u32> {
    let visible: i64 = info.lines().find_map(|line| line.strip_prefix("Pid:"))
        .context("pidfd info lacks Pid")?.trim().parse()?;
    if visible == -1 { return Err(exited(pid)); }
    let visible = u32::try_from(visible).context("pidfd Pid is outside the PID range")?;
    ensure!(visible > 0, "pidfd Pid is zero");
    let ids = namespace_ids(info, "NSpid:")?
        .map(str::parse::<u32>).collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(ids.first() == Some(&visible) && ids.get(depth) == Some(&pid),
        "pidfd namespace identifiers do not resolve caller PID {pid}");
    Ok(visible)
}

struct PinnedProcess {
    fd: OwnedFd,
    pid: u32,
    proc_pid: u32,
    depth: usize,
}

impl PinnedProcess {
    fn open(pid: u32) -> Result<Self> {
        Self::open_at_depth(pid, namespace_depth()?)
    }

    fn open_at_depth(pid: u32, depth: usize) -> Result<Self> {
        // SAFETY: pidfd_open creates a new descriptor for the exact process
        // denoted in the caller's PID namespace; zero flags are required.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
        if fd < 0 {
            return Err(refusal(pid, "pidfd-open", std::io::Error::last_os_error().into()));
        }
        // SAFETY: the successful syscall returned a fresh owned descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        let mut process = Self { fd, pid, proc_pid: 0, depth };
        process.proc_pid = process.visible_pid()?;
        Ok(process)
    }

    fn visible_pid(&self) -> Result<u32> {
        let info = fs::read_to_string(format!("/proc/self/fdinfo/{}", self.fd.as_raw_fd()))
            .map_err(|error| refusal(self.pid, "pidfd-info", error.into()))?;
        mapped_pid(&info, self.pid, self.depth).map_err(|error| {
            if error.downcast_ref::<ProcessIdentityUnavailable>().is_some() { error }
            else { refusal(self.pid, "pidfd-namespace", error) }
        })
    }

    fn verify(&self) -> Result<()> {
        ensure!(self.visible_pid()? == self.proc_pid,
            ProcessIdentityUnavailable { pid: self.pid, stage: "pidfd-namespace-changed", errno: None });
        Ok(())
    }
}

pub(super) fn process_identity(pid: u32) -> Result<(char, u64)> {
    let process = PinnedProcess::open(pid)?;
    let stat = fs::read_to_string(format!("/proc/{}/stat", process.proc_pid));
    // Recheck the same kernel handle after the stat read. If its process was
    // reaped, fdinfo reports Pid:-1: a reused numeric PID cannot supply evidence.
    process.verify()?;
    let stat = stat.map_err(|error| refusal(pid, "proc-stat", error.into()))?;
    let identity = (|| {
        let end = stat.rfind(')').context("malformed process stat")?;
        let tail = stat.get(end + 2..).context("malformed process stat tail")?;
        let mut fields = tail.split_whitespace();
        let state = fields.next().and_then(|value| value.chars().next()).context("process stat lacks a state")?;
        let token = fields.nth(18).context("process stat lacks a start token")?.parse()?;
        Ok((state, token))
    })();
    identity.map_err(|error| refusal(pid, "proc-stat", error))
}

pub(super) fn group_has_live_member(group: u32) -> Result<bool> {
    let depth = namespace_depth()?;
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(proc_pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue; };
        let stat = match fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied) => continue,
            Err(error) => return Err(error.into()),
        };
        let Some((_, tail)) = stat.rsplit_once(") ") else { continue; };
        let mut fields = tail.split_whitespace();
        let state = fields.next();
        if state == Some("Z") { continue; }
        let matches = if depth == 0 {
            fields.nth(1).and_then(|field| field.parse::<u32>().ok()) == Some(group)
        } else {
            let status = match fs::read_to_string(entry.path().join("status")) {
                Ok(status) => status,
                Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied) => continue,
                Err(error) => return Err(refusal(group, "proc-group-status", error.into())),
            };
            let candidate_group = namespace_ids(&status, "NSpgid:")
                .and_then(|mut ids| ids.nth(depth).map(str::parse::<u32>).transpose().map_err(Into::into))
                .map_err(|error| refusal(group, "proc-group-namespace", error))?;
            if candidate_group != Some(group) { continue; }
            let candidate_pid = namespace_ids(&status, "NSpid:")
                .and_then(|mut ids| ids.nth(depth).map(str::parse::<u32>).transpose().map_err(Into::into))
                .map_err(|error| refusal(group, "proc-group-namespace", error))?;
            let Some(candidate_pid) = candidate_pid else { continue; };
            // Same-depth IDs in sibling namespaces can collide. Only a
            // caller-local pidfd that resolves back to this exact procfs task
            // proves the task belongs to our namespace or its descendants.
            let pinned = match PinnedProcess::open_at_depth(candidate_pid, depth) {
                Ok(pinned) => pinned,
                Err(error) if is_absent(&error) == Some(true) => continue,
                Err(error) => return Err(error),
            };
            if pinned.proc_pid != proc_pid { continue; }
            pinned.verify()?;
            true
        };
        if matches { return Ok(true); }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pidfd_mapping_uses_the_callers_namespace_depth() {
        let info = "Pid:\t6100\nNSpid:\t6100 42 7\n";
        assert_eq!(mapped_pid(info, 6100, 0).unwrap(), 6100);
        assert_eq!(mapped_pid(info, 42, 1).unwrap(), 6100);
        assert_eq!(mapped_pid(info, 7, 2).unwrap(), 6100);
        assert!(mapped_pid(info, 7, 1).is_err());
        assert!(mapped_pid("Pid: 6100\nNSpid: 6101 42\n", 42, 1).is_err());
        assert!(mapped_pid("Pid: 6100\n", 42, 1).is_err());
    }

    #[test]
    fn reaped_pidfd_cannot_bind_another_process() {
        let error = mapped_pid("Pid: -1\nNSpid: -1\n", 42, 0).unwrap_err();
        assert_eq!(is_absent(&error), Some(true));
        assert_eq!(error.downcast_ref::<ProcessIdentityUnavailable>().unwrap().stage, "pidfd-exited");
    }

    #[test]
    fn unavailable_procfs_is_not_positive_exit_evidence() {
        let error = refusal(42, "pidfd-info", std::io::Error::from_raw_os_error(libc::ENOENT).into());
        assert_eq!(is_absent(&error), Some(false));
    }
}
