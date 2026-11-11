//! Explicit launch-owned watcher; no startup or production caller.
use super::{keeper::{KeeperManager, KeeperScope, replenish_keeper}, keeper_systemd::SystemdKeeper};
use anyhow::{Context, Result, bail, ensure};
use std::{fs::File, io::{Read, Seek}, os::{fd::{AsRawFd, FromRawFd, OwnedFd}, unix::net::UnixStream}, process::Child, thread::{self, JoinHandle}};

/// Explicit, joined keeper supervision; construction is never automatic.
pub struct KeeperWatcher {
    cancel: Option<UnixStream>,
    finished: UnixStream,
    thread: Option<JoinHandle<Result<()>>>,
}
impl KeeperWatcher {
    pub fn start(manager: SystemdKeeper, scope: KeeperScope) -> Result<Self> {
        Self::spawn(manager, scope)
    }

    fn spawn(mut manager: impl KeeperManager + Send + 'static, scope: KeeperScope) -> Result<Self> {
        // Arm before inspecting population: death between startup and poll is not lost.
        let mut events = File::open(scope.directory.join("idle/cgroup.events"))?;
        let (cancel, cancellation) = UnixStream::pair()?;
        let (finished, completion) = UnixStream::pair()?;
        let thread = thread::Builder::new().name("keeper-watch".into()).spawn(move || {
            // Closing completion wakes the owner's poll on every return, including panic.
            let _completion = completion;
            let mut contents = String::new();
            loop {
                let mut descriptors = [
                    libc::pollfd { fd: cancellation.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                    libc::pollfd { fd: events.as_raw_fd(), events: libc::POLLPRI | libc::POLLERR, revents: 0 },
                ];
                // Check cancellation even when the keeper is already empty.
                poll(&mut descriptors[..1], 0)?;
                if descriptors[0].revents != 0 { return Ok(()); }
                events.rewind()?;
                contents.clear();
                events.read_to_string(&mut contents)?;
                match contents.lines().find_map(|line| line.strip_prefix("populated ")) {
                    Some("0") => { replenish_keeper(&mut manager, &scope)?; continue; }
                    Some("1") => {},
                    _ => bail!("keeper population is unknown"),
                }
                poll(&mut descriptors, -1)?;
                if descriptors[0].revents != 0 { return Ok(()); }
                ensure!(descriptors[1].revents & (libc::POLLNVAL | libc::POLLHUP) == 0,
                    "keeper event descriptor closed");
            }
        })?;
        Ok(Self { cancel: Some(cancel), finished, thread: Some(thread) })
    }

    /// Observe watcher failure while the holder is still running, not only at teardown.
    pub fn wait_for_exit(&mut self, holder: &mut Child) -> Result<()> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, holder.id(), 0_u32) };
        ensure!(fd >= 0, "open holder pidfd: {}", std::io::Error::last_os_error());
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        let mut descriptors = [
            libc::pollfd { fd: self.finished.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        poll(&mut descriptors, -1)?;
        if descriptors[0].revents != 0 {
            self.join()?;
            bail!("keeper watcher stopped before holder exit");
        }
        self.stop()?;
        let _status = holder.wait()?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        // EOF wakes poll immediately; no numeric timeout or periodic wakeup.
        self.cancel.take();
        self.join()
    }

    fn join(&mut self) -> Result<()> {
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| anyhow::anyhow!("keeper watcher panicked"))?
                .context("keeper watcher failed")?;
        }
        Ok(())
    }
}
impl Drop for KeeperWatcher {
    fn drop(&mut self) { let _result = self.stop(); }
}

fn poll(descriptors: &mut [libc::pollfd], timeout: i32) -> Result<()> {
    loop {
        // SAFETY: the slice contains valid storage and all owners outlive this wait.
        if unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as libc::nfds_t, timeout) } >= 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted { return Err(error.into()); }
    }
}

#[cfg(test)]
#[path = "keeper_watcher/tests.rs"]
mod tests;
