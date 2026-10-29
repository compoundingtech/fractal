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

    fn accepted_socket_identity(stream: &std::os::unix::net::UnixStream) -> PeerProcessIdentity {
        use std::os::fd::FromRawFd as _;

        let mut credentials = libc::ucred { pid: 0, uid: 0, gid: 0 };
        let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
        // Both options query the actual accepted socket, not the process that
        // happens to hold its other endpoint when the assertion runs.
        let result = unsafe {
            libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(), &mut length)
        };
        assert_eq!(result, 0, "SO_PEERCRED: {}", std::io::Error::last_os_error());
        assert_eq!(length as usize, std::mem::size_of_val(&credentials));
        let pid = u32::try_from(credentials.pid).unwrap();

        // Linux 6.5+, including systems whose libc headers do not name it yet.
        const SO_PEERPIDFD: libc::c_int = 77;
        let mut fd: libc::c_int = -1;
        let mut length = std::mem::size_of_val(&fd) as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, SO_PEERPIDFD,
                (&mut fd as *mut libc::c_int).cast(), &mut length)
        };
        assert_eq!(result, 0, "SO_PEERPIDFD requires Linux 6.5+: {}", std::io::Error::last_os_error());
        assert!(fd >= 0);
        // Successful SO_PEERPIDFD transfers a fresh descriptor to this fixture.
        let descriptor = unsafe { OwnedFd::from_raw_fd(fd) };
        assert_eq!(length as usize, std::mem::size_of_val(&fd));
        PeerProcessIdentity::capture(pid, descriptor).unwrap()
    }

    #[test]
    fn peer_process_inherited_socket_child() {
        use std::io::{Read as _, Write as _};
        use std::os::fd::FromRawFd as _;
        use std::os::unix::{net::UnixStream, process::CommandExt as _};
        use std::process::{Command, Stdio};

        const TEST: &str = "peer_process::tests::peer_process_inherited_socket_child";
        if let Some(fd) = std::env::var_os("FRACTAL_PEER_PROCESS_INHERITED_FD") {
            let fd = fd.to_str().unwrap().parse::<libc::c_int>().unwrap();
            // The connector transferred this descriptor across exec exclusively
            // to this subprocess; no other Rust owner exists here.
            let mut socket = unsafe { UnixStream::from_raw_fd(fd) };
            socket.write_all(&std::process::id().to_ne_bytes()).unwrap();
            let mut command = [0_u8];
            socket.read_exact(&mut command).unwrap();
            assert_eq!(command, *b"p");
            socket.write_all(b"alive").unwrap();
            socket.read_exact(&mut command).unwrap();
            assert_eq!(command, *b"x");
        } else if let Some(path) = std::env::var_os("FRACTAL_PEER_PROCESS_CONNECT_SOCKET") {
            let mut socket = UnixStream::connect(path).unwrap();
            let mut command = [0_u8];
            // Do not create the descendant until accept-time capture is complete.
            socket.read_exact(&mut command).unwrap();
            assert_eq!(command, *b"s");
            let fd = socket.as_raw_fd();
            let mut descendant = Command::new(std::env::current_exe().unwrap());
            descendant.args(["--exact", TEST])
                .env("FRACTAL_PEER_PROCESS_INHERITED_FD", fd.to_string())
                .stdin(Stdio::null()).stdout(Stdio::null());
            // SAFETY: only async-signal-safe fcntl calls run after fork. Changing
            // CLOEXEC here affects only the subprocess descriptor table.
            unsafe {
                descendant.pre_exec(move || {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let _descendant = descendant.spawn().unwrap();
            drop(socket);
            // Deliberately exit before the descendant. The parent test observes
            // our wait status and later waits for the descendant's pidfd exit.
            std::io::stdin().read_exact(&mut command).unwrap();
            assert_eq!(command, *b"x");
        }
    }

    #[test]
    fn inherited_socket_retains_original_connector_and_refuses_after_its_exit() {
        use std::io::{Read as _, Write as _};
        use std::os::unix::net::UnixListener;
        use std::process::{Command, Stdio};

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peer.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let mut connector = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "peer_process::tests::peer_process_inherited_socket_child"])
            .env("FRACTAL_PEER_PROCESS_CONNECT_SOCKET", &path)
            .stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let (mut socket, _) = listener.accept().unwrap();
        let identity = accepted_socket_identity(&socket);
        assert_eq!(identity.pid(), connector.id());
        identity.validate().unwrap();

        socket.write_all(b"s").unwrap();
        let mut descendant_pid = [0_u8; 4];
        socket.read_exact(&mut descendant_pid).unwrap();
        let descendant_pid = u32::from_ne_bytes(descendant_pid);
        assert_ne!(descendant_pid, connector.id());
        let descendant_identity = PeerProcessIdentity::capture(descendant_pid, pidfd(descendant_pid)).unwrap();
        // The descendant owns and writes the original socket, but neither a new
        // socket query nor relabeling its original kernel pidfd gives its identity.
        assert_eq!(accepted_socket_identity(&socket).pid(), connector.id());
        assert!(PeerProcessIdentity::capture(
            descendant_pid, identity.pidfd.as_ref().try_clone().unwrap(),
        ).is_err());
        identity.validate().unwrap();

        connector.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(connector.wait().unwrap().success());
        socket.write_all(b"p").unwrap();
        let mut reply = [0_u8; 5];
        socket.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"alive");
        descendant_identity.validate().unwrap();
        assert_eq!(identity.pid(), connector.id());
        assert!(identity.validate().is_err());

        socket.write_all(b"x").unwrap();
        // The connector cannot reap a descendant that deliberately outlives it.
        // Wait on the descendant's exact generation instead of a timed probe.
        let mut exited = libc::pollfd {
            fd: descendant_identity.pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut exited, 1, -1) }, 1);
        assert_ne!(exited.revents & libc::POLLIN, 0);
    }
}
