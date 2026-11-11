use super::{ExitUsage, JobLaunch, JobLaunchEffects, JobLaunchResult, LaunchJournal};
use crate::ExecRuntime;
use anyhow::{Result, ensure};
use fractal_jobs::{launch::{LaunchRefusal, require_caps}, recovery::Receipt};
use std::path::Path;

/// Release may have executed the payload. Cleanup succeeded, but the intent is
/// unresolved and must never be refused or retried. Retain its final counters.
#[derive(Debug)]
pub struct UncertainRelease {
    pub exit_usage: ExitUsage,
    pub release_error: anyhow::Error,
}
impl std::fmt::Display for UncertainRelease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "uncertain gate release: {}", self.release_error)
    }
}
impl std::error::Error for UncertainRelease {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.release_error.as_ref())
    }
}

impl ExecRuntime {
    /// INACTIVE, no production caller. Unit 8 must never release or relaunch an
    /// uncertain intent after restart. A reboot loses J; absent outcome is unknown
    /// after host loss, not failed-and-rerun. Proven released running J is kept.
    pub fn launch_job(
        &self,
        request: &JobLaunch<'_>,
        journal: &mut dyn LaunchJournal,
        effects: &mut dyn JobLaunchEffects,
    ) -> Result<JobLaunchResult> {
        journal.intent(request.intent)?;
        let prepared = (|| {
            ensure!(request.heavy_slice.is_some_and(|s| !s.is_empty()),
                LaunchRefusal::MissingHeavySlice);
            ensure!(!request.launcher.is_empty(), LaunchRefusal::Preparation);
            ensure!(Path::new(&request.intent.outcome_file).is_absolute(),
                LaunchRefusal::InvalidOutcomePath);
            ensure!(!Path::new(&request.intent.outcome_file).exists(), LaunchRefusal::Preparation);
            let directory = effects.create_group(request)?;
            let hierarchy = effects.hierarchy_path(&directory)?;
            let readback = effects.apply(&directory, request.plan)?;
            require_caps(&readback.matches(request.plan)).map_err(anyhow::Error::new)?;
            let launch_usage = effects.usage(&directory, request.host)?;
            effects.gate_holder(request)?;
            effects.attach_holder(&directory)?;
            Ok((directory, hierarchy, launch_usage))
        })();
        let (_directory, hierarchy, launch_usage) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let refusal = error.downcast_ref::<LaunchRefusal>().copied()
                    .unwrap_or(LaunchRefusal::Preparation);
                // Failure to prove cleanup leaves an uncertain intent, never a retry.
                let _exit_usage = effects.stop_and_sample(request.host)?;
                journal.refusal(request.intent, refusal)?;
                return Err(error);
            }
        };
        // Release failure may follow a successful gate write. Retain the uncertain
        // intent and no-retry rule, but stop/remove J; never record a refusal.
        let holder_pid = match effects.release() {
            Ok(pid) => pid,
            Err(error) => {
                let exit_usage = effects.stop_and_sample(request.host)?;
                return Err(UncertainRelease { exit_usage, release_error: error }.into());
            }
        };
        effects.wait_for_exit()?;
        let receipt = std::fs::read(&request.intent.outcome_file)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice::<Receipt>(&bytes)?));
        // Success and abort use the same kill/empty/sample/remove path.
        let exit_usage = effects.stop_and_sample(request.host)?;
        let receipt = receipt?;
        ensure!(receipt.matches(request.job_id, holder_pid,
            hierarchy.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 cgroup identity"))?),
            "B6 receipt identity/outcome mismatch");
        Ok(JobLaunchResult {
            holder_pid, outcome: receipt.outcome.into(), launch_usage, exit_usage,
        })
    }
}
