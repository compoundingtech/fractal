use super::keeper::{KeeperManager, KeeperScope};
use anyhow::{Context, Result, ensure};
use fractal_jobs::launch::LaunchRefusal;
use std::{collections::HashMap, fs, io::Read, os::fd::{FromRawFd, OwnedFd}, path::PathBuf, process::Command, sync::{Arc, Mutex}};
#[derive(Clone)]
pub struct SystemdKeeper {
    root: PathBuf,
    // Unreaped direct children fence numeric PIDs throughout placement.
    fresh: Arc<Mutex<HashMap<u32, FreshKeeper>>>,
}
impl SystemdKeeper {
    pub fn new(cgroup_root: PathBuf) -> Self {
        Self { root: cgroup_root, fresh: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub(super) fn hierarchy_path(&self, directory: &std::path::Path) -> Result<PathBuf> {
        Ok(std::path::Path::new("/").join(directory.strip_prefix(&self.root)?))
    }
}

struct FreshKeeper(libc::pid_t);
impl FreshKeeper {
    fn wait(&mut self) {
        while self.0 != 0 {
            let result = unsafe { libc::waitpid(self.0, std::ptr::null_mut(), 0) };
            if result == self.0 { self.0 = 0; }
            else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                // No other runtime code may reap this private direct child.
                self.0 = 0;
            }
        }
    }
}
impl Drop for FreshKeeper {
    fn drop(&mut self) {
        if self.0 != 0 {
            // This PID cannot be reused: we have never reaped our direct child.
            unsafe { libc::kill(self.0, libc::SIGKILL); }
            self.wait();
        }
    }
}
fn checked(output: std::process::Output) -> Result<String> {
    ensure!(output.status.success(), "systemd request failed: {}", String::from_utf8_lossy(&output.stderr));
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
fn properties(pid: u32, slice: &str, io: bool) -> Vec<String> {
    let mut values = vec![
        "PIDs".into(), "au".into(), "1".into(), pid.to_string(),
        "Delegate".into(), "b".into(), "true".into(),
        "Slice".into(), "s".into(), slice.into(),
        "MemoryAccounting".into(), "b".into(), "true".into(),
        "TasksAccounting".into(), "b".into(), "true".into(),
    ];
    if io { values.extend(["IOAccounting".into(), "b".into(), "true".into()]); }
    values
}
impl KeeperManager for SystemdKeeper {
    fn describe(&mut self, name: &str) -> Result<Option<KeeperScope>> {
        let output = Command::new("systemctl").args(["--user", "show",
            "--property=LoadState,ControlGroup,Slice,Delegate", name]).output()?;
        let text = String::from_utf8(output.stdout)?;
        let field = |key: &str| text.lines().find_map(|line| line.strip_prefix(key));
        if field("LoadState=") == Some("not-found") { return Ok(None); }
        ensure!(output.status.success() && field("LoadState=") == Some("loaded"), "keeper lookup failed");
        let relative = field("ControlGroup=").context("keeper has no cgroup")?;
        ensure!(relative.starts_with('/') && !relative.contains(".."), LaunchRefusal::InvalidCgroupPath);
        Ok(Some(KeeperScope {
            directory: self.root.join(relative.trim_start_matches('/')),
            slice: field("Slice=").context("keeper has no slice")?.into(),
            delegated: field("Delegate=") == Some("yes"),
        }))
    }
    fn start(&mut self, name: &str, slice: &str, pid: u32) -> Result<()> {
        let io = fs::read_to_string(self.root.join("cgroup.controllers"))?
            .split_whitespace().any(|controller| controller == "io");
        checked(Command::new("busctl").args(["--user", "call", "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1", "org.freedesktop.systemd1.Manager", "StartTransientUnit",
            "ssa(sv)a(sa(sv))", name, "fail", if io { "6" } else { "5" },
        ]).args(properties(pid, slice, io)).arg("0").output()?)?;
        checked(Command::new("systemctl").args(["--user", "start", name]).output()?)?;
        Ok(())
    }
    fn spawn_idle(&mut self) -> Result<u32> {
        let mut pipe = [-1; 2];
        // SAFETY: valid storage for two descriptors; both become owned below.
        ensure!(unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } == 0,
            "keeper pipe: {}", std::io::Error::last_os_error());
        let reader = unsafe { OwnedFd::from_raw_fd(pipe[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(pipe[1]) };
        // SAFETY: the child uses only async-signal-safe operations until pause;
        // never allocate, acquire locks, unwind, or return into Rust.
        let child = unsafe {
            // clone without sharing flags and with a null stack has fork
            // semantics on both supported Linux architectures (x86_64/aarch64).
            let pid = libc::syscall(libc::SYS_clone, libc::SIGCHLD as libc::c_ulong,
                0_usize, 0_usize, 0_usize, 0_usize) as libc::pid_t;
            if pid == 0 {
                libc::syscall(libc::SYS_close, pipe[0]);
                if libc::syscall(libc::SYS_prctl, libc::PR_SET_NAME,
                    c"fractal-keeper".as_ptr(), 0, 0, 0) < 0
                    || libc::syscall(libc::SYS_setsid) < 0 {
                    libc::syscall(libc::SYS_exit, 125);
                }
                // Preserve only the readiness writer. Never publish a PID until
                // BOTH close_range calls succeed; EOF alone cannot prove that.
                if (pipe[1] > 0 && libc::syscall(libc::SYS_close_range,
                    0_u32, pipe[1] as u32 - 1, 0_u32) < 0)
                    || libc::syscall(libc::SYS_close_range,
                        pipe[1] as u32 + 1, u32::MAX, 0_u32) < 0 {
                    libc::syscall(libc::SYS_exit, 125);
                }
                let keeper_pid = libc::syscall(libc::SYS_getpid) as u32;
                if libc::syscall(libc::SYS_write, pipe[1],
                    (&raw const keeper_pid).cast::<u8>(), size_of::<u32>()) != size_of::<u32>() as libc::c_long {
                    libc::syscall(libc::SYS_exit, 125);
                }
                libc::syscall(libc::SYS_close, pipe[1]);
                // pause is async-signal-safe; libc supplies the aarch64
                // implementation where Linux has no dedicated SYS_pause.
                loop { libc::pause(); }
            }
            pid
        };
        drop(writer);
        ensure!(child >= 0, "fork keeper: {}", std::io::Error::last_os_error());
        let keeper = FreshKeeper(child);
        let mut bytes = [0; size_of::<u32>()];
        let mut reader = fs::File::from(reader);
        let published = (|| -> std::io::Result<()> {
            reader.read_exact(&mut bytes)?;
            // EOF confirms the final readiness FD is closed before returning.
            let mut extra = [0];
            match reader.read_exact(&mut extra) {
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(()),
                Err(error) => Err(error),
                Ok(()) => Err(std::io::Error::other("extra keeper identity bytes")),
            }
        })();
        published.context("keeper failed before publishing its identity")?;
        ensure!(u32::from_ne_bytes(bytes) == child as u32, "keeper identity mismatch");
        self.fresh.lock().map_err(|_| anyhow::anyhow!("keeper ownership poisoned"))?
            .insert(child as u32, keeper);
        Ok(child as u32)
    }
    fn kill_idle(&mut self, pid: u32) -> Result<()> {
        let keeper = self.fresh.lock().map_err(|_| anyhow::anyhow!("keeper ownership poisoned"))?
            .remove(&pid).context("not an unpublished keeper")?;
        drop(keeper); // Kill and reap only while its numeric PID is still fenced.
        Ok(())
    }
    fn commit_idle(&mut self, pid: u32) -> Result<()> {
        let mut keeper = self.fresh.lock().map_err(|_| anyhow::anyhow!("keeper ownership poisoned"))?
            .remove(&pid).context("not an unpublished keeper")?;
        // After placement there are no further PID-based mutations. Only this
        // waiter may reap the child; daemon death reparents it without killing it.
        std::thread::Builder::new().name("fractal-keeper-reaper".into()).spawn(move || keeper.wait())?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_identity_strips_the_configured_mount_not_a_fixed_prefix() {
        let manager = SystemdKeeper::new(PathBuf::from("/configured/cgroupfs"));
        assert_eq!(
            manager.hierarchy_path(std::path::Path::new("/configured/cgroupfs/keeper/job")).unwrap(),
            PathBuf::from("/keeper/job"),
        );
        assert!(manager.hierarchy_path(std::path::Path::new("/another/mount/job")).is_err());
    }

    #[test]
    fn keeper_properties_have_no_daemon_lifetime_dependency() {
        let sent = properties(42, "app-capped-heavy.slice", true);
        assert!(!sent.iter().any(|s| s == "BindsTo" || s == "PartOf"));
        for property in ["Delegate", "MemoryAccounting", "TasksAccounting", "IOAccounting"] {
            assert!(sent.windows(3).any(|s| s == [property, "b", "true"]));
        }
        assert!(sent.windows(3).any(|s| s == ["Slice", "s", "app-capped-heavy.slice"]));
    }

    #[test]
    #[ignore = "real Linux fork/procfs; run in the explicit kernel pilot lane"]
    fn forked_keeper_is_named_detached_and_survives_adapter_drop() {
        struct Reap(OwnedFd);
        impl Drop for Reap {
            fn drop(&mut self) {
                use std::os::fd::AsRawFd;
                unsafe { libc::syscall(libc::SYS_pidfd_send_signal,
                    self.0.as_raw_fd(), libc::SIGKILL, 0_usize, 0_u32); }
            }
        }
        let mut manager = SystemdKeeper::new(PathBuf::new());
        let pid = manager.spawn_idle().unwrap();
        // Open while the direct child is unreaped, before placement commits.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) } as i32;
        assert!(fd >= 0);
        let _cleanup = Reap(unsafe { OwnedFd::from_raw_fd(fd) });
        assert_eq!(fs::read_to_string(format!("/proc/{pid}/comm")).unwrap(), "fractal-keeper\n");
        assert_eq!(unsafe { libc::getsid(pid as libc::pid_t) }, pid as libc::pid_t);
        assert_eq!(fs::read_dir(format!("/proc/{pid}/fd")).unwrap().count(), 0);
        manager.commit_idle(pid).unwrap();
        drop(manager);
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, 0);
    }

    #[test]
    #[ignore = "real Linux fork/procfs; run in the explicit kernel pilot lane"]
    fn dead_unpublished_keeper_remains_fenced_until_failed_placement_cleanup() {
        let mut manager = SystemdKeeper::new(PathBuf::new());
        let pid = manager.spawn_idle().unwrap();
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) }, 0);
        let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::waitid(libc::P_PID, pid, &mut status,
            libc::WEXITED | libc::WNOWAIT) }, 0);
        // Death after readiness does not allow PID reuse before placement or
        // cleanup. WNOWAIT observes termination without releasing ownership.
        assert!(manager.fresh.lock().unwrap().contains_key(&pid));
        manager.kill_idle(pid).unwrap();
        assert_eq!(unsafe { libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(),
            libc::WNOHANG) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
        assert!(manager.kill_idle(pid).is_err()); // Never signal a stale PID.
    }
}
