use super::{ExitUsage, sample_exit, gated::GatedChild, keeper::*, keeper_systemd::SystemdKeeper};
use super::cgroup::{kill_and_wait_empty, remove_empty_tree, verify_dedicated};
use crate::job_scope;
use anyhow::{Context, Result, ensure};
use fractal_jobs::{launch::{LaunchIntent, LaunchOutcome, LaunchRefusal, cgroup_name}, scope::{JobScopePlan, observation::{RawScopeUsage, ScopeReadback}}};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, process::{Child, Command, Stdio}};
pub struct JobLaunch<'a> {
    pub job_id: &'a str,
    pub intent: &'a LaunchIntent,
    pub plan: &'a JobScopePlan,
    pub heavy_slice: Option<&'a str>,
    pub host: &'a str,
    pub launcher: &'a [String],
    pub cwd: &'a Path,
    pub environment: &'a BTreeMap<String, String>,
}


/// Must durably append before returning success. This is not an authority grant;
/// unit 4c-2 must supply admission before any caller is wired.
pub trait LaunchJournal {
    fn intent(&mut self, intent: &LaunchIntent) -> Result<()>;
    fn refusal(&mut self, intent: &LaunchIntent, refusal: LaunchRefusal) -> Result<()>;
}

/// Side effects own ONLY this generation's J. Abort kills its gated child and J,
/// proves populated=0, and removes J before reporting success. Never stop the keeper.
pub trait JobLaunchEffects {
    fn create_group(&mut self, request: &JobLaunch<'_>) -> Result<PathBuf>;
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
    fn exit_usage(&mut self, directory: &Path, host: &str) -> ExitUsage {
        sample_exit(directory, host)
    }
    fn remove_group(&mut self) -> Result<()>;
    fn abort_and_confirm_empty(&mut self) -> Result<()>;
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
    directory: Option<PathBuf>,
    gated: Option<GatedChild>,
    holder: Option<Child>,
}
impl CgroupJobLaunch {
    pub fn new(cgroup_root: PathBuf) -> Self {
        Self { manager: SystemdKeeper::new(cgroup_root), directory: None, gated: None, holder: None }
    }
}
impl JobLaunchEffects for CgroupJobLaunch {
    fn create_group(&mut self, request: &JobLaunch<'_>) -> Result<PathBuf> {
        ensure!(self.directory.is_none() && self.gated.is_none(), "job adapter cannot be reused");
        let keeper = ensure_keeper(&mut self.manager, request.host, request.heavy_slice)
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
        Ok(directory)
    }
    fn gate_holder(&mut self, request: &JobLaunch<'_>) -> Result<()> {
        crate::procfs::require_matching_procfs()?;
        self.gated = Some(GatedChild::spawn(holder_command(request))?);
        Ok(())
    }
    fn attach_holder(&mut self, directory: &Path) -> Result<()> {
        verify_dedicated(directory, None)?;
        let pid = self.gated.as_ref().context("holder is not gated")?.pid;
        self.manager.move_pid(pid, directory)?;
        verify_dedicated(directory, Some(pid))
    }
    fn release(&mut self) -> Result<u32> {
        let gated = self.gated.as_mut().context("holder is not gated")?;
        let pid = gated.pid;
        self.holder = Some(gated.release()?);
        Ok(pid)
    }
    fn wait_for_exit(&mut self) -> Result<()> {
        let _status = self.holder.as_mut().context("holder was not released")?.wait()?;
        Ok(())
    }
    fn remove_group(&mut self) -> Result<()> {
        if let Some(directory) = &self.directory {
            if directory.exists() {
                ensure!(!populated(directory)?, "J is not empty");
                remove_empty_tree(directory)?;
            }
        }
        Ok(())
    }
    fn abort_and_confirm_empty(&mut self) -> Result<()> {
        if let Some(gated) = &mut self.gated { gated.abort()?; }
        if let Some(directory) = &self.directory {
            if directory.exists() { kill_and_wait_empty(directory)?; }
        }
        self.remove_group()
    }
}

fn holder_command(request: &JobLaunch<'_>) -> Command {
    let mut command = Command::new("gate-slot");
    command
        .args(["--class", "heavy", "--no-scope", "--job", request.job_id,
            "--outcome-file", &request.intent.outcome_file, "--"])
        .args(request.launcher)
        .current_dir(request.cwd)
        .env_clear()
        .envs(request.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_jobs::launch::ExecutionGeneration;
    use std::num::NonZeroU64;

    #[test]
    fn holder_argv_obeys_b6_without_a_nested_scope() {
        let intent = LaunchIntent {
            generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()),
            outcome_file: "/fixture/outcome-1".into(),
        };
        let plan = JobScopePlan::new(4096, 8192, 4, 4096, 8192, 4096).unwrap();
        let launcher = vec!["launcher".into(), "argument".into()];
        let environment = BTreeMap::new();
        let request = JobLaunch {
            job_id: "job/fixture", intent: &intent, plan: &plan,
            heavy_slice: Some("app-heavy.slice"), host: "fixture",
            launcher: &launcher, cwd: Path::new("/fixture"), environment: &environment,
        };
        let command = holder_command(&request);
        assert_eq!(command.get_program(), "gate-slot");
        assert_eq!(command.get_args().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>(),
            ["--class", "heavy", "--no-scope", "--job", "job/fixture",
             "--outcome-file", "/fixture/outcome-1", "--", "launcher", "argument"]);
    }
}
