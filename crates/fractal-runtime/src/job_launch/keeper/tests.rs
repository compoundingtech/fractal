use super::*;
use crate::job_launch::{ExitUsage, cgroup::verify_dedicated, sample_exit};
use fractal_jobs::launch::CountersUnavailable;

struct Manager {
    root: PathBuf,
    starts: usize,
    spawned: u32,
    slice: String,
    delegated: bool,
}
fn group(directory: &Path) {
    fs::create_dir(directory).unwrap();
    for (file, value) in [
        ("cgroup.controllers", "memory pids io"),
        ("cgroup.subtree_control", ""), ("cgroup.procs", ""),
        ("cgroup.events", "populated 0\n"),
    ] { fs::write(directory.join(file), value).unwrap(); }
}
impl KeeperManager for Manager {
    fn describe(&mut self, _: &str) -> Result<Option<KeeperScope>> {
        Ok(self.root.is_dir().then(|| KeeperScope {
            directory: self.root.clone(), slice: self.slice.clone(), delegated: self.delegated,
        }))
    }
    fn start(&mut self, _: &str, slice: &str, pid: u32) -> Result<()> {
        self.starts += 1;
        self.slice = slice.into();
        group(&self.root);
        fs::write(self.root.join("cgroup.procs"), pid.to_string())?;
        Ok(())
    }
    fn spawn_idle(&mut self) -> Result<u32> { self.spawned += 1; Ok(self.spawned) }
    fn kill_idle(&mut self, _: u32) -> Result<()> { Ok(()) }
    fn mkdir(&mut self, directory: &Path) -> Result<()> { group(directory); Ok(()) }
    fn move_pid(&mut self, pid: u32, directory: &Path) -> Result<()> {
        fs::write(directory.join("cgroup.procs"), pid.to_string())?;
        fs::write(directory.join("cgroup.events"), "populated 1\n")?;
        fs::write(self.root.join("cgroup.procs"), "")?;
        Ok(())
    }
}
fn manager(root: PathBuf) -> Manager {
    Manager { root, starts: 0, spawned: 100, slice: String::new(), delegated: true }
}

#[test]
fn ensure_reuses_verified_keeper_and_requires_configured_controllers() {
    let temporary = tempfile::tempdir().unwrap();
    let mut manager = manager(temporary.path().join("keeper.scope"));
    assert!(ensure_keeper(&mut manager, "host", None).is_err());
    assert_eq!(manager.spawned, 100);
    let scope = ensure_keeper(&mut manager, "host", Some("app.slice/app-heavy.slice")).unwrap();
    assert!(populated(&scope.directory.join("idle")).unwrap());
    assert_eq!(fs::read_to_string(scope.directory.join("cgroup.procs")).unwrap(), "");
    let controllers = fs::read_to_string(scope.directory.join("cgroup.subtree_control")).unwrap();
    assert_eq!(controllers, "+memory +pids +io");
    ensure_keeper(&mut manager, "host", Some("app-heavy.slice")).unwrap();
    assert_eq!(manager.starts, 1);
    assert_eq!(manager.spawned, 101);
    assert!(ensure_keeper(&mut manager, "host", Some("other.slice")).is_err());
    manager.delegated = false;
    assert!(ensure_keeper(&mut manager, "host", Some("app-heavy.slice")).is_err());
    manager.delegated = true;
    fs::write(scope.directory.join("cgroup.controllers"), "memory").unwrap();
    assert!(ensure_keeper(&mut manager, "host", Some("app-heavy.slice")).is_err());
    assert_eq!(manager.starts, 1);
}

#[test]
fn keeper_death_during_job_is_repaired_without_relaunch_and_counters_are_never_zeroed() {
    for vanished in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let mut manager = manager(temporary.path().join("keeper.scope"));
        let scope = ensure_keeper(&mut manager, "host", Some("app-heavy.slice")).unwrap();
        let job = scope.directory.join("job-generation-1");
        group(&job);
        fs::write(job.join("cgroup.procs"), "42").unwrap();
        fs::write(job.join("cgroup.events"), "populated 1\n").unwrap();
        // Kill the fixture keeper while the holder remains in sibling J.
        fs::write(scope.directory.join("idle/cgroup.procs"), "").unwrap();
        fs::write(scope.directory.join("idle/cgroup.events"), "populated 0\n").unwrap();
        watch_keeper_once(&mut manager, &scope).unwrap();
        assert_eq!(manager.starts, 1);
        assert_eq!(manager.spawned, 102);
        assert!(populated(&scope.directory.join("idle")).unwrap());
        assert_eq!(fs::read_to_string(job.join("cgroup.procs")).unwrap(), "42");
        fs::write(job.join("cgroup.procs"), "").unwrap();
        fs::write(job.join("cgroup.events"), "populated 0\n").unwrap();
        fs::write(job.join("cpu.stat"), "usage_usec 29\n").unwrap();
        if vanished { fs::remove_dir_all(&job).unwrap(); }
        match sample_exit(&job, "host") {
            ExitUsage::Sample(sample) => assert_eq!(sample.cpu_usage_usec.reading.value, Some(29)),
            ExitUsage::CountersUnavailable(reason) => {
                assert!(vanished);
                assert_eq!(reason, CountersUnavailable::CgroupVanished);
            }
        }
        if !vanished { fs::remove_dir_all(&job).unwrap(); }
        ensure_keeper(&mut manager, "host", Some("app-heavy.slice")).unwrap();
        assert_eq!(manager.starts, 1); // next job reuses the repaired keeper
        fs::remove_dir_all(&scope.directory).unwrap();
        ensure_keeper(&mut manager, "host", Some("app-heavy.slice")).unwrap();
        assert_eq!(manager.starts, 2); // lost keeper can also be recreated
    }
}

#[test]
fn holder_is_sole_member_with_empty_subtree_and_no_populated_children() {
    let temporary = tempfile::tempdir().unwrap();
    let job = temporary.path().join("job-generation-1");
    group(&job);
    verify_dedicated(&job, None).unwrap();
    fs::write(job.join("cgroup.procs"), "42\n").unwrap();
    verify_dedicated(&job, Some(42)).unwrap();
    fs::write(job.join("cgroup.procs"), "42\n43\n").unwrap();
    assert!(verify_dedicated(&job, Some(42)).is_err());
    fs::write(job.join("cgroup.procs"), "42\n").unwrap();
    fs::write(job.join("cgroup.subtree_control"), "memory").unwrap();
    assert!(verify_dedicated(&job, Some(42)).is_err());
    fs::write(job.join("cgroup.subtree_control"), "").unwrap();
    let child = job.join("unexpected");
    group(&child);
    fs::write(child.join("cgroup.events"), "populated 1\n").unwrap();
    assert!(verify_dedicated(&job, Some(42)).is_err());
    for invalid in ["/", "/x/../job", "/x/session-1.scope/job", "/x/user@1.service"] {
        assert!(validate_path(Path::new(invalid)).is_err());
    }
    validate_path(Path::new("/user.slice/user@1.service/app.slice/keeper.scope/job-1")).unwrap();
}
