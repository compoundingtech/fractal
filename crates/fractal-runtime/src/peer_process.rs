//! Accept-time process witnesses for authorization, not socket lifetime witnesses.

use anyhow::Result;
#[cfg(target_os = "linux")]
use anyhow::Context as _;
use std::os::fd::OwnedFd;
#[cfg(target_os = "linux")]
use std::{os::fd::AsRawFd as _, sync::Arc};

/// Read a live Darwin process's parent PID, microsecond birth token and effective UID together.
/// This numeric PID observation must be revalidated before authorizing its generation.
#[cfg(target_os = "macos")]
pub fn macos_process_identity(pid: u32) -> Result<(u32, u64, u32)> {
    use anyhow::Context as _;
    let pid_arg = i32::try_from(pid)?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // proc_pidinfo initializes this fixed-size buffer; reject partial results before reading it.
    let count = unsafe {
        libc::proc_pidinfo(pid_arg, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size)
    };
    anyhow::ensure!(count == size, "Darwin process generation is unavailable");
    // A full result initializes proc_bsdinfo without requesting another user's task details.
    let info = unsafe { info.assume_init() };
    anyhow::ensure!(info.pbi_pid == pid && info.pbi_status != 5, "Darwin process is terminal");
    let birth = info.pbi_start_tvsec.checked_mul(1_000_000)
        .and_then(|v| v.checked_add(info.pbi_start_tvusec))
        .context("Darwin process birth overflow")?;
    Ok((info.pbi_ppid, birth, info.pbi_uid))
}

/// Observe ownership only when full BSD process metadata is inaccessible.
/// Short BSD info has no birth token and must never authorize a process generation.
#[cfg(target_os = "macos")]
pub fn macos_process_owner(pid: u32) -> Result<u32> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdshortinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdshortinfo>() as libc::c_int;
    // The kernel initializes this bounded buffer; accept only a complete result.
    let count = unsafe {
        libc::proc_pidinfo(i32::try_from(pid)?, libc::PROC_PIDT_SHORTBSDINFO, 0, info.as_mut_ptr().cast(), size)
    };
    anyhow::ensure!(count == size, "Darwin process ownership is unavailable");
    // A full result initializes proc_bsdshortinfo.
    let info = unsafe { info.assume_init() };
    anyhow::ensure!(info.pbsi_pid == pid, "Darwin process ownership PID mismatch");
    Ok(info.pbsi_uid)
}

/// The original connector's process generation. A socket may outlive this process.
#[derive(Clone, Debug)]
pub struct PeerProcessIdentity {
    pid: u32,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    /// Accept-time generation token for diagnostic correlation, not fresh authority.
    pub fn diagnostic_start_token(&self) -> Option<u64> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        { Some(self.start_token) }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        { None }
    }

    /// Match an inspected generation to the accept-time witness, including liveness.
    pub fn validate_generation(&self, pid: u32, start_token: u64) -> Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            anyhow::ensure!(pid == self.pid && start_token == self.start_token,
                "ancestry connector differs from the accepted process generation");
            self.validate()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        { let _ = (pid, start_token); anyhow::bail!("process generation witnesses unavailable") }
    }

    /// Pin a same-UID ancestor of this accepted connector, revalidating the entire chain.
    ///
    /// This proves local process ownership only: callers must separately verify the native
    /// harness/session and authorize adoption. A PID or this witness grants no agent authority.
    /// The returned witness must be validated again when installing or using a binding.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn verify_ancestor(&self, ancestor: u32) -> Result<Self> {
        self.validate()?;
        let witness = verify_ancestor_chain(self.pid, ancestor, process_metadata, Self::capture_pid)?;
        self.validate()?;
        witness.validate()?;
        Ok(witness)
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
        #[cfg(target_os = "macos")]
        {
            let (_, start_token, _) = macos_process_identity(pid)?;
            let identity = Self { pid, start_token };
            identity.validate()?;
            Ok(identity)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
        #[cfg(target_os = "macos")]
        {
            let (_, token, _) = macos_process_identity(self.pid)?;
            anyhow::ensure!(token == self.start_token, "socket connector exited or its PID was reused");
            Ok(())
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        anyhow::bail!("socket connector generation witnesses are unavailable on this platform")
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessMetadata {
    parent: u32,
    birth: u64,
    uid: u32,
}

#[cfg(target_os = "macos")]
fn process_metadata(pid: u32) -> Result<ProcessMetadata> {
    let (parent, birth, uid) = macos_process_identity(pid)?;
    Ok(ProcessMetadata { parent, birth, uid })
}

#[cfg(target_os = "linux")]
fn process_metadata(pid: u32) -> Result<ProcessMetadata> {
    use anyhow::Context as _;
    crate::procfs::require_matching_procfs()?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let mut fields = stat.rsplit_once(") ").context("malformed ancestor stat")?.1.split_whitespace();
    let state = fields.next().context("missing ancestor state")?;
    anyhow::ensure!(!matches!(state, "Z" | "X" | "x"), "ancestor process has exited");
    let parent = fields.next().context("missing ancestor parent")?.parse()?;
    let birth = fields.nth(17).context("missing ancestor birth")?.parse()?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
    let uid = status.lines().find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .context("missing ancestor effective UID")?.parse()?;
    Ok(ProcessMetadata { parent, birth, uid })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_ancestor_chain<T>(
    connector: u32,
    ancestor: u32,
    mut inspect: impl FnMut(u32) -> Result<ProcessMetadata>,
    pin: impl FnOnce(u32) -> Result<T>,
) -> Result<T> {
    let first = inspect(connector)?;
    let mut chain = vec![(connector, first)];
    let mut current = first;
    let mut pid = connector;
    while pid != ancestor {
        anyhow::ensure!(current.parent > 1, "requested process is not a caller ancestor");
        pid = current.parent;
        anyhow::ensure!(!chain.iter().any(|(seen, _)| *seen == pid), "ancestor chain contains a cycle");
        current = inspect(pid)?;
        anyhow::ensure!(current.uid == first.uid, "ancestor chain crosses an effective UID boundary");
        chain.push((pid, current));
    }
    // Pin before the second read: a PID reused during pinning cannot replace the
    // observed ancestor. Parent links are rechecked as well as generations.
    let witness = pin(ancestor)?;
    for (pid, observed) in chain {
        anyhow::ensure!(inspect(pid)? == observed, "ancestor chain changed during verification");
    }
    Ok(witness)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod ancestor_tests {
    use super::*;

    fn fixture(pid: u32) -> Result<ProcessMetadata> {
        match pid {
            30 => Ok(ProcessMetadata { parent: 20, birth: 300, uid: 7 }),
            20 => Ok(ProcessMetadata { parent: 10, birth: 200, uid: 7 }),
            10 => Ok(ProcessMetadata { parent: 1, birth: 100, uid: 7 }),
            _ => anyhow::bail!("process unavailable"),
        }
    }

    #[test]
    fn adoption_ancestor_pins_only_a_same_uid_caller_ancestor() {
        assert_eq!(verify_ancestor_chain(30, 10, fixture, Ok).unwrap(), 10);
        assert_eq!(verify_ancestor_chain(30, 30, fixture, Ok).unwrap(), 30);
        assert!(verify_ancestor_chain(30, 40, fixture, |_| -> Result<()> {
            panic!("unrelated process must not be pinned")
        }).is_err());
    }

    #[test]
    fn adoption_ancestor_refuses_uid_transitions_cycles_and_missing_evidence() {
        for field in ["uid", "parent", "missing"] {
            assert!(verify_ancestor_chain(30, 10, |pid| {
                let mut metadata = fixture(pid)?;
                if pid == 20 {
                    match field {
                        "uid" => metadata.uid = 8,
                        "parent" => metadata.parent = 30,
                        _ => anyhow::bail!("process disappeared"),
                    }
                }
                Ok(metadata)
            }, |_| -> Result<()> { panic!("invalid ancestry must not be pinned") }).is_err());
        }
    }

    #[test]
    fn adoption_ancestor_revalidates_every_parent_generation_and_uid_after_pinning() {
        use std::cell::Cell;
        for changed_pid in [30, 20, 10] {
            for field in ["parent", "birth", "uid", "missing"] {
                let pinned = Cell::new(false);
                assert!(verify_ancestor_chain(30, 10, |pid| {
                    let mut metadata = fixture(pid)?;
                    if pinned.get() && pid == changed_pid {
                        match field {
                            "parent" => metadata.parent += 1,
                            "birth" => metadata.birth += 1,
                            "uid" => metadata.uid += 1,
                            _ => anyhow::bail!("process exited"),
                        }
                    }
                    Ok(metadata)
                }, |_| { pinned.set(true); Ok(()) }).is_err());
            }
        }
        assert!(verify_ancestor_chain(30, 10, fixture, |_| -> Result<()> {
            anyhow::bail!("cannot pin process")
        }).is_err());
    }

    #[test]
    fn adoption_ancestor_real_process_witness_expires_on_exit() {
        use std::process::{Command, Stdio};
        let mut child = Command::new("sh").args(["-c", "read line"])
            .stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let peer = PeerProcessIdentity::capture_pid(child.id()).unwrap();
        let own = peer.verify_ancestor(std::process::id());
        let unrelated = PeerProcessIdentity::capture_pid(std::process::id()).unwrap()
            .verify_ancestor(child.id());
        drop(child.stdin.take());
        child.wait().unwrap();
        own.unwrap().validate().unwrap();
        assert!(unrelated.is_err());
        assert!(peer.verify_ancestor(std::process::id()).is_err());
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

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn peer_process_changed_birth_token_is_refused() {
        let mut identity = PeerProcessIdentity::capture_pid(std::process::id()).unwrap();
        identity.validate().unwrap();
        // Substitute a different birth for this PID without relying on actual PID reuse.
        identity.start_token = identity.start_token.wrapping_add(1);
        assert!(identity.validate().is_err());
    }
}
