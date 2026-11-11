//! INACTIVE: discovery/fencing only. No boot hook, launch, release, re-pin or keeper
//! creation. Activation must authenticate recovery and own polling kept jobs.
use crate::job_launch::{ExitUsage, cgroup, keeper::{KeeperManager, populated, slice_name, validate_path}};
use anyhow::{Context, Result, ensure};
use fractal_jobs::{host_authority::JobHostBinding, launch::{LaunchIntent, cgroup_name, keeper_name},
    recovery::{ProcessIdentity, Released}};
use std::{fs, io::ErrorKind, os::unix::fs::MetadataExt, path::{Path, PathBuf}};

#[derive(Clone, Debug)]
pub struct LocatedJob {
    pub directory: PathBuf,
    pub hierarchy: String,
}
/// Only observations and whole-J stop/removal: deliberately no launch capability.
pub trait RecoveryEffects {
    fn locate(&mut self, binding: &JobHostBinding, intent: &LaunchIntent, heavy_slice: &str) -> Result<Option<LocatedJob>>;
    fn holder_matches(&mut self, job: &LocatedJob, released: &Released) -> Result<bool>;
    fn populated(&mut self, job: &LocatedJob) -> Result<bool>;
    /// Uniform kill->empty->sample->remove, retaining usage on every resolution.
    fn finish(&mut self, job: &LocatedJob, host: &str) -> Result<ExitUsage>;
}

pub struct CgroupRecovery<'a> {
    pub manager: &'a mut dyn KeeperManager,
    pub cgroup_root: &'a Path,
    /// /proc in production; explicit root permits fixture process identities.
    pub proc_root: &'a Path,
}

/// Read while the released child is still unreaped, including a fast-exit zombie.
/// Persist with the release fact. A PID alone cannot establish restart identity.
pub fn holder_identity(proc_root: &Path, pid: u32) -> Result<ProcessIdentity> {
    let boot_id = fs::read_to_string(proc_root.join("sys/kernel/random/boot_id"))?.trim().to_owned();
    ensure!(!boot_id.is_empty(), "missing kernel boot identity");
    let stat = fs::read_to_string(proc_root.join(pid.to_string()).join("stat"))?;
    // comm may contain whitespace and ')'; fields after its final ')' start at 3.
    let fields = stat.rsplit_once(')').context("invalid process stat")?.1;
    let start_ticks = fields.split_whitespace().nth(19).context("missing process start ticks")?.parse()?;
    Ok(ProcessIdentity { boot_id, start_ticks })
}

impl RecoveryEffects for CgroupRecovery<'_> {
    fn locate(&mut self, binding: &JobHostBinding, intent: &LaunchIntent, heavy_slice: &str) -> Result<Option<LocatedJob>> {
        let heavy_slice = slice_name(Some(heavy_slice))?;
        let Some(keeper) = self.manager.describe(&keeper_name(&binding.execution_host))? else { return Ok(None); };
        validate_path(&keeper.directory)?;
        keeper.directory.strip_prefix(self.cgroup_root)?;
        match fs::symlink_metadata(&keeper.directory) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            result => ensure!(result?.is_dir(), "keeper cgroup is not a directory"),
        }
        ensure!(keeper.delegated && keeper.slice == heavy_slice, "keeper admission properties changed");
        ensure!(keeper.directory.join("idle").is_dir(), "keeper idle leaf missing");
        ensure!(fs::read_to_string(keeper.directory.join("cgroup.procs"))?.trim().is_empty(), "keeper root is populated");
        let controllers = fs::read_to_string(keeper.directory.join("cgroup.controllers"))?;
        ensure!(["memory", "pids"].iter().all(|c| controllers.split_whitespace().any(|v| v == *c)), "keeper controllers missing");
        let directory = keeper.directory.join(cgroup_name(&binding.job_id, intent));
        let hierarchy = Path::new("/").join(directory.strip_prefix(self.cgroup_root)?);
        validate_path(&directory)?;
        match fs::symlink_metadata(&directory) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            result => {
                let metadata = result?;
                ensure!(metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() }
                    && metadata.mode() & 0o200 != 0, "job cgroup is not owned and writable");
            }
        }
        Ok(Some(LocatedJob { directory, hierarchy: hierarchy.to_str().context("non-UTF8 cgroup identity")?.into() }))
    }
    fn holder_matches(&mut self, job: &LocatedJob, released: &Released) -> Result<bool> {
        let procs = match fs::read_to_string(job.directory.join("cgroup.procs")) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            value => value?,
        };
        if !procs.split_whitespace().any(|pid| pid.parse::<u32>() == Ok(released.holder_pid)) { return Ok(false); }
        match holder_identity(self.proc_root, released.holder_pid) {
            Ok(identity) => Ok(identity == released.identity),
            Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == ErrorKind::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn populated(&mut self, job: &LocatedJob) -> Result<bool> {
        match populated(&job.directory) {
            Err(error) if error.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == ErrorKind::NotFound)
                && fs::symlink_metadata(&job.directory)
                    .is_err_and(|e| e.kind() == ErrorKind::NotFound) => Ok(false),
            result => result,
        }
    }
    fn finish(&mut self, job: &LocatedJob, host: &str) -> Result<ExitUsage> {
        cgroup::stop_and_sample(&job.directory, host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_identity_handles_comm_delimiters_and_fences_pid_reuse() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("sys/kernel/random")).unwrap();
        fs::create_dir(tmp.path().join("42")).unwrap();
        fs::write(tmp.path().join("sys/kernel/random/boot_id"), "boot-one\n").unwrap();
        let stat = format!("42 (holder ) with space) S {} 900 0\n", vec!["0"; 18].join(" "));
        fs::write(tmp.path().join("42/stat"), stat).unwrap();
        let original = holder_identity(tmp.path(), 42).unwrap();
        assert_eq!(original.start_ticks, 900);
        fs::write(tmp.path().join("sys/kernel/random/boot_id"), "boot-two\n").unwrap();
        assert_ne!(holder_identity(tmp.path(), 42).unwrap(), original);
        fs::write(tmp.path().join("sys/kernel/random/boot_id"), "boot-one\n").unwrap();
        fs::write(tmp.path().join("42/stat"), format!("42 (reused) S {} 901 0\n", vec!["0"; 18].join(" "))).unwrap();
        assert_ne!(holder_identity(tmp.path(), 42).unwrap(), original);
    }
    struct ExistingKeeper { directory: PathBuf, slice: &'static str }
    impl KeeperManager for ExistingKeeper {
        fn describe(&mut self, name: &str) -> Result<Option<crate::job_launch::keeper::KeeperScope>> {
            assert_eq!(name, keeper_name("fixture-host"));
            Ok(self.directory.is_dir().then(|| crate::job_launch::keeper::KeeperScope {
                directory: self.directory.clone(), slice: self.slice.into(), delegated: true,
            }))
        }
        fn start(&mut self, _: &str, _: &str, _: u32) -> Result<()> { panic!("recovery must not create a scope") }
        fn spawn_idle(&mut self) -> Result<u32> { panic!("recovery must not create a process") }
        fn kill_idle(&mut self, _: u32) -> Result<()> { panic!("recovery must not stop the keeper") }
    }
    #[test]
    fn discovers_only_stable_j_and_never_recreates_missing_keeper_or_job() {
        let tmp = tempfile::tempdir().unwrap();
        let keeper = tmp.path().join("keeper");
        fs::create_dir_all(keeper.join("idle")).unwrap();
        fs::write(keeper.join("cgroup.procs"), "").unwrap();
        fs::write(keeper.join("cgroup.controllers"), "memory pids").unwrap();
        let binding = JobHostBinding { job_id: "job".into(), execution_host: "fixture-host".into(), authority_declaration: "admission".into() };
        let intent = LaunchIntent { generation: fractal_jobs::launch::ExecutionGeneration(std::num::NonZeroU64::new(7).unwrap()), outcome_file: "/receipt".into() };
        let directory = keeper.join(cgroup_name("job", &intent));
        fs::create_dir(&directory).unwrap();
        let mut manager = ExistingKeeper { directory: keeper.clone(), slice: "configured.slice" };
        let mut effects = CgroupRecovery { manager: &mut manager, cgroup_root: tmp.path(), proc_root: tmp.path() };
        let located = effects.locate(&binding, &intent, "configured.slice").unwrap().unwrap();
        assert_eq!(located.directory, directory);
        assert_eq!(located.hierarchy, format!("/keeper/{}", cgroup_name("job", &intent)));
        assert!(effects.locate(&binding, &intent, "").is_err());
        assert!(effects.locate(&binding, &intent, "other.slice").is_err());
        fs::remove_dir(&directory).unwrap();
        // Disappearance AFTER successful lookup is idempotent; no zero sample.
        assert!(!effects.populated(&located).unwrap());
        assert!(matches!(effects.finish(&located, "fixture-host").unwrap(),
            ExitUsage::CountersUnavailable(fractal_jobs::launch::CountersUnavailable::CgroupVanished)));
        // ENOENT for a control file is not proof that J itself disappeared.
        fs::create_dir(&directory).unwrap();
        assert!(effects.populated(&located).is_err());
        assert!(effects.finish(&located, "fixture-host").is_err());
        fs::remove_dir(&directory).unwrap();
        assert!(effects.locate(&binding, &intent, "configured.slice").unwrap().is_none());
        fs::remove_dir_all(&keeper).unwrap();
        assert!(effects.locate(&binding, &intent, "configured.slice").unwrap().is_none());
        assert!(!keeper.exists());
    }

    #[test]
    fn launch_and_recovery_share_slash_qualified_slice_configuration() {
        let configured = "app.slice/app-capped.slice/app-capped-heavy.slice";
        let tmp = tempfile::tempdir().unwrap();
        let keeper = tmp.path().join("keeper");
        fs::create_dir_all(keeper.join("idle")).unwrap();
        fs::write(keeper.join("idle/cgroup.events"), "populated 1\n").unwrap();
        fs::write(keeper.join("cgroup.procs"), "").unwrap();
        fs::write(keeper.join("cgroup.controllers"), "memory pids").unwrap();
        let mut manager = ExistingKeeper {
            directory: keeper.clone(), slice: "app-capped-heavy.slice",
        };
        let launched = crate::job_launch::keeper::ensure_keeper(
            &mut manager, "fixture-host", Some(configured)).unwrap();
        assert_eq!(launched.slice, "app-capped-heavy.slice");
        let binding = JobHostBinding {
            job_id: "job".into(), execution_host: "fixture-host".into(),
            authority_declaration: "admission".into(),
        };
        let intent = LaunchIntent {
            generation: fractal_jobs::launch::ExecutionGeneration(std::num::NonZeroU64::new(7).unwrap()),
            outcome_file: "/receipt".into(),
        };
        let directory = launched.directory.join(cgroup_name(&binding.job_id, &intent));
        fs::create_dir(&directory).unwrap();
        let mut recovery = CgroupRecovery {
            manager: &mut manager, cgroup_root: tmp.path(), proc_root: tmp.path(),
        };
        assert_eq!(recovery.locate(&binding, &intent, configured).unwrap().unwrap().directory,
            directory);
    }
}
