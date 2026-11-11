use super::{ExitUsage, gated::GatedChild, keeper::*, keeper_systemd::SystemdKeeper};
use super::keeper_watcher::KeeperWatcher;
use super::cgroup::{stop_and_sample, verify_dedicated};
use crate::job_scope;
use anyhow::{Context, Result, ensure};
use fractal_jobs::{launch::{LaunchIntent, LaunchOutcome, LaunchRefusal, cgroup_name}, scope::{JobScopePlan, observation::{RawScopeUsage, ScopeReadback}}};
use std::{collections::BTreeMap, fs, num::NonZeroU64, path::{Path, PathBuf}, process::{Child, Command, Stdio}, time::SystemTime};
pub struct JobLaunch<'a> {
    pub job_id: &'a str,
    pub intent: &'a LaunchIntent,
    pub plan: &'a JobScopePlan,
    pub heavy_slice: Option<&'a str>,
    pub host: &'a str,
    pub launcher: &'a [String],
    pub cwd: &'a Path,
    pub environment: &'a BTreeMap<String, String>,
    pub receipt_directory: Option<&'a Path>,
    pub deadline: Option<SystemTime>,
    pub max_hold_ceiling_seconds: Option<NonZeroU64>,
}


/// Must durably append before returning success. This is not an authority grant;
/// unit 4c-2 must supply admission before any caller is wired.
pub trait LaunchJournal {
    fn intent(&mut self, intent: &LaunchIntent) -> Result<()>;
    /// Persist the confirmed release and admitted host pin. Failure leaves an
    /// uncertain intent: the runtime stops J rather than allowing a retry.
    fn released(&mut self, intent: &LaunchIntent, holder_pid: u32) -> Result<()>;
    fn refusal(&mut self, intent: &LaunchIntent, refusal: LaunchRefusal) -> Result<()>;
}

/// Side effects own ONLY this generation's J. Stop kills its gated child and J,
/// proves populated=0, samples usage, then removes J. Never stop the keeper.
pub trait JobLaunchEffects {
    fn create_group(&mut self, request: &JobLaunch<'_>) -> Result<PathBuf>;
    /// Kernel hierarchy identity, not the cgroupfs mount-prefixed pathname.
    fn hierarchy_path(&self, directory: &Path) -> Result<PathBuf>;
    fn gate_holder(&mut self, request: &JobLaunch<'_>) -> Result<()>;
    fn attach_holder(&mut self, directory: &Path) -> Result<()>;
    fn apply(&mut self, directory: &Path, plan: &JobScopePlan) -> Result<ScopeReadback> {
        Ok(job_scope::apply(directory, plan)?)
    }
    fn usage(&mut self, directory: &Path, host: &str) -> Result<RawScopeUsage> {
        Ok(job_scope::read_usage(directory, host)?)
    }
    fn release(&mut self) -> Result<u32>;
    /// Reap the holder, but NEVER use its exit status as its outcome.
    fn wait_for_exit(&mut self) -> Result<()>;
    fn stop_and_sample(&mut self, host: &str) -> Result<ExitUsage>;
}
pub struct JobLaunchResult {
    pub holder_pid: u32,
    pub outcome: LaunchOutcome,
    pub launch_usage: RawScopeUsage,
    pub exit_usage: ExitUsage,
}
/// No default configuration or caller. The keeper is shared; J is generation-local.
pub struct CgroupJobLaunch {
    manager: SystemdKeeper,
    daemon_identity: fractal_jobs::launch::DaemonIdentity,
    directory: Option<PathBuf>,
    gated: Option<GatedChild>,
    holder: Option<Child>,
    watcher: Option<KeeperWatcher>,
}
impl CgroupJobLaunch {
    /// The caller supplies the pinned member-key fingerprint; never derive it
    /// from a process, host alone, instance flag, or a fresh restart-local key.
    pub fn new(cgroup_root: PathBuf, daemon_identity: fractal_jobs::launch::DaemonIdentity) -> Self {
        Self { manager: SystemdKeeper::new(cgroup_root), daemon_identity,
            directory: None, gated: None, holder: None, watcher: None }
    }
}
impl JobLaunchEffects for CgroupJobLaunch {
    fn hierarchy_path(&self, directory: &Path) -> Result<PathBuf> {
        self.manager.hierarchy_path(directory)
    }
    fn create_group(&mut self, request: &JobLaunch<'_>) -> Result<PathBuf> {
        ensure!(self.directory.is_none() && self.gated.is_none(), "job adapter cannot be reused");
        let keeper = ensure_keeper(&mut self.manager, request.host, self.daemon_identity, request.heavy_slice)
            .context(LaunchRefusal::KeeperUnavailable)?;
        let directory = keeper.directory.join(cgroup_name(request.job_id, request.intent));
        validate_path(&directory)?;
        match fs::create_dir(&directory) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(LaunchRefusal::CgroupCollision.into());
            }
            Err(error) => return Err(error.into()),
        }
        self.directory = Some(directory.clone());
        self.watcher = Some(KeeperWatcher::start(self.manager.clone(), keeper)
            .context(LaunchRefusal::KeeperUnavailable)?);
        Ok(directory)
    }
    fn gate_holder(&mut self, request: &JobLaunch<'_>) -> Result<()> {
        crate::procfs::require_matching_procfs()?;
        self.gated = Some(GatedChild::spawn(holder_command(request, SystemTime::now())?)?);
        Ok(())
    }
    fn attach_holder(&mut self, directory: &Path) -> Result<()> {
        verify_dedicated(directory, None)?;
        let pid = self.gated.as_ref().context("holder is not gated")?.pid;
        self.manager.move_pid(pid, directory)?;
        self.gated.as_mut().context("holder is not gated")?.mark_attached();
        verify_dedicated(directory, Some(pid))
    }
    fn release(&mut self) -> Result<u32> {
        let gated = self.gated.as_mut().context("holder is not gated")?;
        let pid = gated.pid;
        self.holder = Some(gated.release()?);
        Ok(pid)
    }
    fn wait_for_exit(&mut self) -> Result<()> {
        self.watcher.as_mut().context("keeper is not watched")?
            .wait_for_exit(self.holder.as_mut().context("holder was not released")?)
    }
    fn stop_and_sample(&mut self, host: &str) -> Result<ExitUsage> {
        // Always clean J even if the watcher failed; never kill the shared keeper.
        let cleanup = (|| {
            if let Some(gated) = &mut self.gated {
                if gated.is_attached() {
                    if gated.is_gated() { gated.fence_attached_abort()?; }
                } else {
                    gated.abort()?;
                }
            }
            let usage = match &self.directory {
                Some(directory) => stop_and_sample(directory, host)?,
                None => ExitUsage::CountersUnavailable(
                    fractal_jobs::launch::CountersUnavailable::CgroupVanished),
            };
            if let Some(gated) = &mut self.gated
                && gated.is_attached() && gated.is_gated()
            { gated.reap_after_retirement()?; }
            Ok::<_, anyhow::Error>(usage)
        })();
        let watched = self.watcher.as_mut().map_or(Ok(()), KeeperWatcher::stop);
        let usage = cleanup?;
        if let Some(mut holder) = self.holder.take() { let _status = holder.wait()?; }
        if let Err(error) = watched {
            return Err(super::UncertainRelease { exit_usage: usage, release_error: error }.into());
        }
        Ok(usage)
    }
}

pub(super) fn max_hold_seconds(request: &JobLaunch<'_>, now: SystemTime) -> Result<NonZeroU64> {
    let deadline = request.deadline.context(LaunchRefusal::MissingDeadline)?;
    let ceiling = request.max_hold_ceiling_seconds.context(LaunchRefusal::MissingMaxHoldCeiling)?;
    let remaining = deadline.duration_since(now).ok()
        .and_then(|duration| NonZeroU64::new(duration.as_secs()))
        .context(LaunchRefusal::DeadlineExpired)?;
    Ok(remaining.min(ceiling))
}

fn holder_command(request: &JobLaunch<'_>, now: SystemTime) -> Result<Command> {
    let max_hold = max_hold_seconds(request, now)?.to_string();
    let mut command = Command::new("gate-slot");
    command
        .args(["--class", "heavy", "--no-scope", "--max-hold", &max_hold,
            "--job", request.job_id, "--outcome-file", &request.intent.outcome_file, "--"])
        .args(request.launcher)
        .current_dir(request.cwd)
        .env_clear()
        .envs(request.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_jobs::launch::ExecutionGeneration;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn holder_argv_obeys_b6_without_a_nested_scope() {
        let intent = LaunchIntent {
            generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()),
            outcome_file: "/fixture/outcome-1".into(),
        };
        let plan = JobScopePlan::new(4096, 8192, 4, 4096, 8192, 4096).unwrap();
        let launcher = vec!["launcher".into(), "argument".into()];
        let environment = BTreeMap::new();
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let request = JobLaunch {
            job_id: "job/fixture", intent: &intent, plan: &plan,
            heavy_slice: Some("app-heavy.slice"), host: "fixture",
            launcher: &launcher, cwd: Path::new("/fixture"), environment: &environment,
            receipt_directory: Some(Path::new("/fixture/receipts")),
            deadline: Some(now + Duration::from_secs(60)),
            max_hold_ceiling_seconds: NonZeroU64::new(90),
        };
        let command = holder_command(&request, now).unwrap();
        assert_eq!(command.get_program(), "gate-slot");
        assert_eq!(command.get_args().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>(),
            ["--class", "heavy", "--no-scope", "--max-hold", "60", "--job", "job/fixture",
             "--outcome-file", "/fixture/outcome-1", "--", "launcher", "argument"]);
    }

    #[test]
    fn max_hold_requires_configuration_and_floors_deadline_under_ceiling() {
        let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()),
            outcome_file: "/fixture/outcome".into() };
        let plan = JobScopePlan::new(4096, 8192, 4, 4096, 8192, 4096).unwrap();
        let environment = BTreeMap::new();
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let mut request = JobLaunch {
            job_id: "fixture", intent: &intent, plan: &plan, heavy_slice: Some("heavy.slice"),
            host: "fixture", launcher: &[], cwd: Path::new("/fixture"), environment: &environment,
            receipt_directory: Some(Path::new("/fixture/receipts")),
            deadline: None, max_hold_ceiling_seconds: None,
        };
        assert_eq!(max_hold_seconds(&request, now).unwrap_err().downcast::<LaunchRefusal>().unwrap(),
            LaunchRefusal::MissingDeadline);
        request.deadline = Some(now + Duration::from_millis(2999));
        assert_eq!(max_hold_seconds(&request, now).unwrap_err().downcast::<LaunchRefusal>().unwrap(),
            LaunchRefusal::MissingMaxHoldCeiling);
        request.max_hold_ceiling_seconds = NonZeroU64::new(10);
        assert_eq!(max_hold_seconds(&request, now).unwrap().get(), 2);
        request.max_hold_ceiling_seconds = NonZeroU64::new(1);
        assert_eq!(max_hold_seconds(&request, now).unwrap().get(), 1);
        for deadline in [now - Duration::from_secs(1), now, now + Duration::from_millis(999)] {
            request.deadline = Some(deadline);
            assert_eq!(max_hold_seconds(&request, now).unwrap_err().downcast::<LaunchRefusal>().unwrap(),
                LaunchRefusal::DeadlineExpired);
        }
    }
}

#[cfg(test)]
#[path = "driver_kernel_tests.rs"]
mod kernel_tests;
