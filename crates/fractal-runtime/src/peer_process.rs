//! Accept-time process witnesses for authorization, not socket lifetime witnesses.

use anyhow::Result;
#[cfg(target_os = "linux")]
use anyhow::Context as _;
use std::os::fd::OwnedFd;
#[cfg(target_os = "linux")]
use std::{os::fd::AsRawFd as _, sync::Arc};

/// The original connector's process generation. A socket may outlive this process.
#[derive(Clone, Debug)]
pub struct PeerProcessIdentity {
    pid: u32,
    #[cfg(target_os = "linux")]
    start_token: u64,
    #[cfg(target_os = "linux")]
    pidfd: Arc<OwnedFd>,
}

impl PeerProcessIdentity {
    /// Capture immediately at accept using the socket's mandatory SO_PEERPIDFD witness.
    pub fn capture(pid: u32, pidfd: OwnedFd) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            // Establish kernel liveness before the first numeric PID lookup;
            // validate() checks it again after capturing the start token.
            let mut poll = libc::pollfd { fd: pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let result = unsafe { libc::poll(&mut poll, 1, 0) };
            anyhow::ensure!(result == 0 && poll.revents == 0,
                "socket connector pidfd is exited or invalid at accept");
            let (state, start_token) = crate::process::process_identity(pid)?;
            anyhow::ensure!(!matches!(state, 'Z' | 'X' | 'x'), "socket connector has exited");
            let identity = Self { pid, start_token, pidfd: Arc::new(pidfd) };
            identity.validate()?;
            Ok(identity)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (pid, pidfd);
            anyhow::bail!("socket connector generation witnesses are unavailable on this platform")
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Pin a process explicitly enrolled by the trusted launcher before listening.
    pub fn capture_pid(pid: u32) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::FromRawFd as _;
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            anyhow::ensure!(fd >= 0, "cannot pin enrolled worker: {}", std::io::Error::last_os_error());
            Self::capture(pid, unsafe { OwnedFd::from_raw_fd(fd as i32) })
        }
        #[cfg(not(target_os = "linux"))]
        { let _ = pid; anyhow::bail!("worker process witnesses unavailable") }
    }

    /// Refuse exited connectors and numeric PID reuse, even if their socket survives.
    pub fn validate(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            crate::procfs::require_matching_procfs()?;
            // A pidfd is readable after exit; invalid/other descriptor types must
            // never turn into a start-token-only authorization.
            let mut poll = libc::pollfd { fd: self.pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let result = unsafe { libc::poll(&mut poll, 1, 0) };
            if result < 0 {
                return Err(std::io::Error::last_os_error()).context("poll connector pidfd");
            }
            anyhow::ensure!(result == 0 && poll.revents == 0, "socket connector pidfd is exited or invalid");
            let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", self.pidfd.as_raw_fd()))?;
            let pid = info.lines().find_map(|line| line.strip_prefix("Pid:")).and_then(|pid| pid.trim().parse::<u32>().ok());
            anyhow::ensure!(pid == Some(self.pid), "socket connector pidfd does not identify the accepted peer");
            let (state, token) = crate::process::process_identity(self.pid)?;
            anyhow::ensure!(!matches!(state, 'Z' | 'X' | 'x') && token == self.start_token,
                "socket connector exited or its PID was reused");
            // Check again after procfs: exit/reuse during that read cannot replace
            // the exact kernel witness with a merely matching numeric candidate.
            let mut poll = libc::pollfd { fd: self.pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let result = unsafe { libc::poll(&mut poll, 1, 0) };
            anyhow::ensure!(result == 0 && poll.revents == 0, "socket connector exited during validation");
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("socket connector generation witnesses are unavailable on this platform")
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn pidfd(pid: u32) -> OwnedFd {
        use std::os::fd::FromRawFd as _;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
        // A successful pidfd_open transfers a fresh descriptor to this fixture.
        unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) }
    }

    #[test]
    fn peer_process_reused_pid_candidate_is_refused() {
        let mut identity = PeerProcessIdentity::capture(std::process::id(), pidfd(std::process::id())).unwrap();
        identity.validate().unwrap();
        // Deterministically substitute another generation for the same numeric PID.
        // This does not force real PID reuse or claim tick-level uniqueness.
        identity.start_token = identity.start_token.wrapping_add(1);
        assert!(identity.validate().is_err());
    }

    #[test]
    fn peer_process_waiting_child() {
        if std::env::var_os("FRACTAL_PEER_PROCESS_WAITING_CHILD").is_some() {
            use std::io::Read as _;
            std::io::stdin().read_exact(&mut [0_u8]).unwrap();
        }
    }

    #[test]
    fn exited_kernel_witness_rejects_a_live_replacement_candidate() {
        use std::io::Write as _;
        use std::process::{Command, Stdio};
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "peer_process::tests::peer_process_waiting_child"])
            .env("FRACTAL_PEER_PROCESS_WAITING_CHILD", "1")
            .stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let captured = PeerProcessIdentity::capture(child.id(), pidfd(child.id()));
        child.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(child.wait().unwrap().success());
        let mut identity = captured.unwrap();
        assert!(identity.validate().is_err());
        // Simulate the ancestry reader now finding a live reused-PID candidate.
        // Even its valid token cannot resurrect the old kernel witness.
        identity.pid = std::process::id();
        identity.start_token = crate::process_start_token(identity.pid).unwrap();
        assert!(identity.validate().is_err());
    }

    #[test]
    fn peer_process_non_pidfd_descriptor_is_refused() {
        let descriptor = std::fs::File::open("/dev/null").unwrap();
        assert!(PeerProcessIdentity::capture(std::process::id(), descriptor.into()).is_err());
    }
}
