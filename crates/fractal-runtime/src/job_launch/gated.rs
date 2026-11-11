use std::{
    fs::File,
    io::{self, Write},
    os::{fd::{AsRawFd, FromRawFd, OwnedFd}, unix::process::CommandExt},
    process::{Child, Command},
    thread::{self, JoinHandle},
};
mod pidfd;

fn pipe() -> io::Result<(File, File)> {
    let mut fds = [-1; 2];
    // SAFETY: fds is writable; both returned descriptors are owned exactly once.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}

pub struct GatedChild {
    pub pid: u32,
    pidfd: OwnedFd,
    attached: bool,
    release_fenced: bool,
    gate: Option<File>,
    worker: Option<JoinHandle<io::Result<Child>>>,
    // Positive evidence, not absence of gate/worker: release consumes those too.
    aborted_before_release: bool,
}

impl GatedChild {
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        let (gate_read, gate_write) = pipe()?;
        let (pid_read, pid_write) = pidfd::pair()?;
        let parent_gate = gate_write.as_raw_fd();
        let parent_pid = pid_read.as_raw_fd();
        // Command::spawn waits for exec. A worker lets the parent configure J
        // while the forked child is still blocked in pre_exec.
        let worker = thread::spawn(move || {
            unsafe {
                command.pre_exec(move || {
                    libc::close(parent_gate);
                    libc::close(parent_pid);
                    // Command::spawn can internally reap a failed pre_exec child.
                    // Transfer its self-pidfd BEFORE readiness, never reopen a
                    // published numeric PID in the parent after that race.
                    if !pidfd::publish(pid_write.as_raw_fd()) { libc::_exit(125); }
                    libc::close(pid_write.as_raw_fd());
                    let mut release = 0_u8;
                    // EOF, EINTR, short read or any non-release byte means ABORT.
                    // No allocations, locks, formatting or Rust destructors here.
                    if libc::read(gate_read.as_raw_fd(), (&mut release as *mut u8).cast(), 1) != 1
                        || release != 1
                    {
                        libc::_exit(125);
                    }
                    Ok(())
                });
            }
            command.spawn()
        });
        let (pid, pidfd) = match pidfd::receive(pid_read.as_raw_fd()) {
            Ok(identity) => identity,
            Err(error) => {
                drop(gate_write); // EOF makes the child fail closed without exec.
                if let Ok(Ok(mut child)) = worker.join() { let _ = child.wait(); }
                return Err(error);
            }
        };
        Ok(Self {
            pid, pidfd, attached: false, release_fenced: false,
            gate: Some(gate_write),
            worker: Some(worker),
            aborted_before_release: false,
        })
    }

    /// An exclusive borrower may abort only while release is still fenced.
    /// Inspecting this does not itself serialize callers; retain the borrow and
    /// the caller's generation mutation guard through abort and retirement.
    pub fn is_gated(&self) -> bool {
        self.gate.is_some() && self.worker.is_some()
    }

    /// Successful pre-release abort permanently disables release. Retained across
    /// retirement/journal failures so cancellation may retry cleanup, never exec.
    pub fn is_aborted_before_release(&self) -> bool {
        self.aborted_before_release
    }

    pub fn mark_attached(&mut self) { self.attached = true; }
    pub fn is_attached(&self) -> bool { self.attached }

    /// Retain the pipe while fencing release: EOF must not stop an attached child
    /// before the common cgroup retirement path has killed and sampled its tree.
    pub fn fence_attached_abort(&mut self) -> io::Result<()> {
        if !self.attached || !(self.is_gated() || self.aborted_before_release) {
            return Err(io::Error::other("attached launch fence unavailable"));
        }
        self.release_fenced = true;
        Ok(())
    }

    /// Only after successful whole-tree retirement. Never signals an attached PID.
    pub fn reap_after_retirement(&mut self) -> io::Result<()> {
        if !self.attached || !self.release_fenced {
            return Err(io::Error::other("attached retirement was not fenced"));
        }
        if !self.aborted_before_release { self.reap_abort()?; }
        Ok(())
    }

    pub fn release(&mut self) -> io::Result<Child> {
        if self.release_fenced { return Err(io::Error::other("holder release permanently fenced")); }
        self.gate.take().ok_or_else(|| io::Error::other("holder gate already consumed"))?
            .write_all(&[1])?;
        self.join()?
    }

    fn join(&mut self) -> io::Result<io::Result<Child>> {
        self.worker.take().ok_or_else(|| io::Error::other("holder worker already joined"))?
            .join().map_err(|_| io::Error::other("holder spawn worker panicked"))
    }

    pub fn abort(&mut self) -> io::Result<()> {
        if self.worker.is_none() {
            return Ok(());
        }
        if self.attached { return Err(io::Error::other("attached holder requires cgroup retirement")); }
        // No waitpid(-1) or SIGCHLD-ignore exists in this runtime. The daemon's
        // tile PID1 reaper is a separate process; std spawn reaping is fenced by
        // the child's transferred self-pidfd, not by a numeric-PID assumption.
        if unsafe { libc::syscall(libc::SYS_pidfd_send_signal, self.pidfd.as_raw_fd(),
            libc::SIGKILL, std::ptr::null::<libc::siginfo_t>(), 0_u32) } < 0
            && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        { return Err(io::Error::last_os_error()); }
        self.reap_abort()
    }

    fn reap_abort(&mut self) -> io::Result<()> {
        let before_release = self.is_gated();
        self.gate.take();
        if let Ok(mut child) = self.join()? {
            child.wait()?;
        }
        self.aborted_before_release = before_release;
        Ok(())
    }
}

impl Drop for GatedChild {
    fn drop(&mut self) {
        let _ = self.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exited(child: &GatedChild) -> bool {
        let mut event = libc::pollfd { fd: child.pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        assert!(unsafe { libc::poll(&mut event, 1, 0) } >= 0);
        event.revents & libc::POLLIN != 0
    }

    #[test]
    fn eof_at_gate_never_executes_holder() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("executed");
        let mut command = Command::new("touch");
        command.arg(&marker);
        let mut child = GatedChild::spawn(command).unwrap();
        // Simulate daemon death: its sole gate writer disappears, no release byte.
        child.gate.take();
        if let Ok(mut process) = child.join().unwrap() {
            let _ = process.wait().unwrap();
        }
        assert!(!marker.exists());
    }

    #[test]
    fn explicit_release_executes_holder() {
        let mut child = GatedChild::spawn(Command::new("true")).unwrap();
        assert!(child.is_gated());
        assert!(child.release().unwrap().wait().unwrap().success());
        assert!(!child.is_gated());
        child.abort().unwrap();
        assert!(!child.is_aborted_before_release());
    }

    #[test]
    #[ignore = "native Linux gate/fork test; run explicitly"]
    fn aborted_gate_cannot_be_released() {
        let mut child = GatedChild::spawn(Command::new("true")).unwrap();
        assert!(child.is_gated());
        assert!(!child.is_attached());
        assert!(!exited(&child));
        child.abort().unwrap();
        assert!(exited(&child)); // The transferred pidfd still names the reaped child.
        assert!(!child.is_gated());
        assert!(child.is_aborted_before_release());
        child.abort().unwrap();
        assert!(child.is_aborted_before_release());
        assert!(child.release().is_err());
    }

    #[test]
    #[ignore = "native Linux gate/fork test; run explicitly"]
    fn attached_abort_does_not_signal_before_retirement() {
        let mut child = GatedChild::spawn(Command::new("true")).unwrap();
        child.mark_attached();
        assert!(child.abort().is_err());
        child.fence_attached_abort().unwrap();
        assert!(child.release().is_err());
        assert!(!exited(&child));
        // Fixture supplies the completed-retirement boundary; the system fixture
        // separately checks cgroup.kill/timeout before this reap is permitted.
        child.reap_after_retirement().unwrap();
        assert!(exited(&child));
        assert!(child.is_aborted_before_release());
    }

    #[test]
    #[ignore = "native Linux pre_exec descriptor-failure test; run explicitly"]
    fn failed_pidfd_publication_never_executes_holder() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("executed");
        let mut command = Command::new("touch");
        command.arg(&marker);
        // Deliberately destroy the child's private publication endpoint first.
        unsafe { command.pre_exec(|| { libc::close_range(3, u32::MAX, 0); Ok(()) }); }
        assert!(GatedChild::spawn(command).is_err());
        assert!(!marker.exists());
    }
}
