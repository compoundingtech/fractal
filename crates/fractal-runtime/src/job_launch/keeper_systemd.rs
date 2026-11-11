use super::keeper::{KeeperManager, KeeperScope};
use anyhow::{Context, Result, ensure};
use fractal_jobs::launch::LaunchRefusal;
use std::{fs, path::PathBuf, process::Command};
pub struct SystemdKeeper {
    root: PathBuf,
    child: Option<u32>,
}
impl SystemdKeeper {
    pub fn new(cgroup_root: PathBuf) -> Self {
        Self { root: cgroup_root, child: None }
    }

    pub(super) fn hierarchy_path(&self, directory: &std::path::Path) -> Result<PathBuf> {
        Ok(std::path::Path::new("/").join(directory.strip_prefix(&self.root)?))
    }
}
impl Drop for SystemdKeeper {
    fn drop(&mut self) {
        if let Some(pid) = self.child.take() {
            // Reap eventually without tying keeper lifetime to this adapter.
            std::thread::spawn(move || {
                let mut status = 0;
                loop {
                    if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) } >= 0 {
                        break;
                    }
                    if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                        break;
                    }
                }
            });
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
        if let Some(pid) = self.child {
            let mut status = 0;
            ensure!(unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) }
                == pid as libc::pid_t, "previous keeper child has not exited");
            self.child = None;
        }
        // SAFETY: child performs ONLY async-signal-safe syscalls, never returns
        // into Rust, never execs, and holds no FD/pipe tying it to the daemon.
        let pid = unsafe {
            let pid = libc::fork();
            if pid == 0 {
                if libc::setsid() < 0 || libc::syscall(libc::SYS_close_range, 0_u32, u32::MAX, 0_u32) < 0 {
                    libc::_exit(125);
                }
                loop { libc::pause(); }
            }
            pid
        };
        ensure!(pid >= 0, "fork keeper: {}", std::io::Error::last_os_error());
        self.child = Some(pid as u32);
        Ok(pid as u32)
    }
    fn kill_idle(&mut self, pid: u32) -> Result<()> {
        ensure!(self.child == Some(pid), "keeper child is not owned by this adapter");
        // Only called for a child this manager just created and has not reaped.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
            let mut status = 0;
            ensure!(libc::waitpid(pid as libc::pid_t, &mut status, 0) >= 0, "reap keeper failed");
        }
        self.child = None;
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
}
