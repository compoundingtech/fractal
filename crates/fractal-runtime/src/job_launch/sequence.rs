use super::{JobLaunch, JobLaunchEffects, JobLaunchResult, LaunchJournal};
use crate::ExecRuntime;
use anyhow::{Result, ensure};
use fractal_jobs::launch::{HolderOutcome, LaunchRefusal, require_caps};
use serde::Deserialize;
use std::path::Path;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    mode: String,
    job_label: String,
    holder_pid: u32,
    job_cgroup: String,
    launched: bool,
    #[serde(flatten)]
    outcome: HolderOutcome,
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
        let (directory, hierarchy, launch_usage) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let refusal = error.downcast_ref::<LaunchRefusal>().copied()
                    .unwrap_or(LaunchRefusal::Preparation);
                // Failure to prove cleanup leaves an uncertain intent, never a retry.
                effects.abort_and_confirm_empty()?;
                journal.refusal(request.intent, refusal)?;
                return Err(error);
            }
        };
        // Release failure may follow a successful gate write. Retain the uncertain
        // intent and no-retry rule, but stop/remove J; never record a refusal.
        let holder_pid = match effects.release() {
            Ok(pid) => pid,
            Err(error) => {
                effects.abort_and_confirm_empty()?;
                return Err(error);
            }
        };
        effects.wait_for_exit()?;
        let exit_usage = effects.exit_usage(&directory, request.host);
        let receipt = std::fs::read(&request.intent.outcome_file)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice::<Receipt>(&bytes)?));
        // J persists independently of the holder; sample promptly, then remove it.
        effects.remove_group()?;
        let receipt = receipt?;
        ensure!(receipt.mode == "scope-none" && receipt.job_label == request.job_id
            && receipt.holder_pid == holder_pid
            && Path::new(&receipt.job_cgroup) == hierarchy, "B6 receipt identity mismatch");
        let consistent = match &receipt.outcome {
            HolderOutcome::Refused { .. } | HolderOutcome::Withdrawn => !receipt.launched,
            HolderOutcome::MaxHold | HolderOutcome::ChildExit { .. } => receipt.launched,
            HolderOutcome::HolderLost => true,
        };
        ensure!(consistent, "B6 receipt launched/outcome mismatch");
        Ok(JobLaunchResult {
            holder_pid, outcome: receipt.outcome.into(), launch_usage, exit_usage,
        })
    }
}
