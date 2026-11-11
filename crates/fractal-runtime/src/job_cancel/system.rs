//! INACTIVE: no production caller. Requires authenticated, generation-fenced input.
use super::StopEffects;
use crate::{job_launch::{cgroup, gated::GatedChild}, job_recovery::{LocatedJob, holder_identity}};
use fractal_jobs::{cancel::StopUncertain, launch::ExecutionGeneration, recovery::{RecoveryUsage, Released}};
use std::{fs, io, os::fd::{AsRawFd, FromRawFd, OwnedFd}, path::Path, sync::MutexGuard, time::{Duration, Instant}};

/// The caller must use this SAME generation mutex for every launch, release and
/// retirement mutation, and retain it through durable cancellation completion.
/// Holding an unrelated mutex is not authority. The generation must name the
/// authenticated LocatedJob and Released record; paths and proc_root must be
/// trusted, with proc_root in the pidfd syscall's PID namespace (normally /proc).
/// This is an inactive capability contract, not runtime admission or wiring.
pub struct CgroupStop<'a> {
    pub job: &'a LocatedJob,
    pub host: &'a str,
    pub proc_root: &'a Path,
    pub holder: StopHolder<'a>,
    pub generation_guard: MutexGuard<'a, ExecutionGeneration>,
    pub generation: ExecutionGeneration,
}

pub enum StopHolder<'a> {
    Running(&'a Released),
    /// Exclusive gate ownership OR retained proof of successful pre-release abort.
    /// None/released/uncertain consumption is a lost fence, not cleanup authority.
    Launching(Option<&'a mut GatedChild>),
}

impl StopEffects for CgroupStop<'_> {
    fn stop(&mut self, grace: Duration, escalation: Duration) -> Result<RecoveryUsage, StopUncertain> {
        if *self.generation_guard != self.generation {
            return Err(StopUncertain::LaunchFenceUnavailable);
        }
        match &mut self.holder {
            StopHolder::Launching(child) => {
                let child = child.as_deref_mut().ok_or(StopUncertain::LaunchFenceUnavailable)?;
                if child.is_attached() {
                    child.fence_attached_abort().map_err(|_| StopUncertain::LaunchFenceUnavailable)?;
                } else if !child.is_aborted_before_release() {
                    if !child.is_gated() { return Err(StopUncertain::LaunchFenceUnavailable); }
                    child.abort().map_err(|_| StopUncertain::LaunchFenceUnavailable)?;
                }
            }
            StopHolder::Running(released) => {
                if let Some(holder) = authenticated_holder(self.proc_root, self.job, released)? {
                    send_term(&holder)?;
                    wait_grace(&holder, grace)?;
                }
            }
        }
        // Natural holder exit never bypasses whole-tree kill, empty, sample, remove.
        let usage = cgroup::stop_and_sample_timed(&self.job.directory, self.host, escalation)?;
        if let StopHolder::Launching(Some(child)) = &mut self.holder
            && child.is_attached()
        {
            child.reap_after_retirement().map_err(|_| StopUncertain::LaunchFenceUnavailable)?;
        }
        Ok(usage)
    }
}

fn open_pidfd(pid: u32) -> io::Result<Option<OwnedFd>> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| io::Error::other("invalid holder pid"))?;
    if pid <= 0 { return Err(io::Error::other("invalid holder pid")); }
    // SAFETY: pidfd_open has no pointer arguments; flags=0 requests a process fd.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) { return Ok(None); }
        return Err(error);
    }
    // SAFETY: the successful syscall returned a new, uniquely owned descriptor.
    Ok(Some(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
}

fn exited(holder: &OwnedFd) -> io::Result<bool> {
    let mut descriptor = libc::pollfd { fd: holder.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: descriptor is writable storage for one pollfd.
    if unsafe { libc::poll(&mut descriptor, 1, 0) } < 0 { return Err(io::Error::last_os_error()); }
    if descriptor.revents & (libc::POLLNVAL | libc::POLLERR) != 0 {
        return Err(io::Error::other("holder pidfd poll failed"));
    }
    Ok(descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0)
}

fn authenticated_holder(proc_root: &Path, job: &LocatedJob, released: &Released) -> Result<Option<OwnedFd>, StopUncertain> {
    let Some(holder) = open_pidfd(released.holder_pid).map_err(|_| StopUncertain::HolderIdentity)?
        else { return Ok(None); };
    // Open FIRST. Rechecking after opening fences PID reuse between discovery and
    // acquisition; later reuse cannot retarget a signal sent to this descriptor.
    if exited(&holder).map_err(|_| StopUncertain::HolderIdentity)? { return Ok(None); }
    let identity = holder_identity(proc_root, released.holder_pid);
    let membership = fs::read_to_string(job.directory.join("cgroup.procs"));
    let matches = identity.is_ok_and(|identity| identity == released.identity)
        && membership.is_ok_and(|members| members.split_whitespace()
            .any(|pid| pid.parse::<u32>() == Ok(released.holder_pid)));
    // Disappearance during either observation is natural exit only if THIS pidfd
    // proves exit. IO errors and a live mismatched process are never proof.
    if exited(&holder).map_err(|_| StopUncertain::HolderIdentity)? { return Ok(None); }
    if !matches { return Err(StopUncertain::HolderIdentity); }
    Ok(Some(holder))
}

fn send_term(holder: &OwnedFd) -> Result<(), StopUncertain> {
    // SAFETY: the fd is owned and live; null siginfo requests the ordinary signal.
    if unsafe { libc::syscall(libc::SYS_pidfd_send_signal, holder.as_raw_fd(), libc::SIGTERM,
        std::ptr::null::<libc::siginfo_t>(), 0) } < 0
        && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return Err(StopUncertain::SignalFailed);
    }
    Ok(())
}

fn wait_grace(holder: &OwnedFd, grace: Duration) -> Result<(), StopUncertain> {
    let deadline = Instant::now().checked_add(grace).ok_or(StopUncertain::GraceFailed)?;
    loop {
        if exited(holder).map_err(|_| StopUncertain::GraceFailed)? { return Ok(()); }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() { return Ok(()); }
        let mut descriptor = libc::pollfd { fd: holder.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: descriptor is valid writable storage for one pollfd.
        if unsafe { libc::poll(&mut descriptor, 1, cgroup::poll_milliseconds(remaining)) } < 0
            && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
        { return Err(StopUncertain::GraceFailed); }
    }
}

#[cfg(test)]
mod tests;
