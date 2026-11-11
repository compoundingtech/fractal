use std::{
    fs::File,
    io::{self, Read, Write},
    os::{fd::{AsRawFd, FromRawFd}, unix::process::CommandExt},
    process::{Child, Command},
    thread::{self, JoinHandle},
};

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
    gate: Option<File>,
    worker: Option<JoinHandle<io::Result<Child>>>,
    // Positive evidence, not absence of gate/worker: release consumes those too.
    aborted_before_release: bool,
}

impl GatedChild {
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        let (gate_read, gate_write) = pipe()?;
        let (mut pid_read, pid_write) = pipe()?;
        let parent_gate = gate_write.as_raw_fd();
        let parent_pid = pid_read.as_raw_fd();
        // Command::spawn waits for exec. A worker lets the parent configure J
        // while the forked child is still blocked in pre_exec.
        let worker = thread::spawn(move || {
            unsafe {
                command.pre_exec(move || {
                    libc::close(parent_gate);
                    libc::close(parent_pid);
                    let pid = libc::getpid().to_ne_bytes();
                    if libc::write(pid_write.as_raw_fd(), pid.as_ptr().cast(), pid.len())
                        != pid.len() as isize
                    {
                        libc::_exit(125);
                    }
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
        let mut bytes = [0; size_of::<libc::pid_t>()];
        if let Err(error) = pid_read.read_exact(&mut bytes) {
            drop(gate_write);
            let _ = worker.join();
            return Err(error);
        }
        Ok(Self {
            pid: libc::pid_t::from_ne_bytes(bytes) as u32,
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

    pub fn release(&mut self) -> io::Result<Child> {
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
        let before_release = self.is_gated();
        // Still an unreaped child: PID cannot have been recycled.
        unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL); }
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
        child.abort().unwrap();
        assert!(!child.is_gated());
        assert!(child.is_aborted_before_release());
        child.abort().unwrap();
        assert!(child.is_aborted_before_release());
        assert!(child.release().is_err());
    }
}
