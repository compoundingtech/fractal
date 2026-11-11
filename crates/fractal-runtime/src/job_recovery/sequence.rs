use super::{LocatedJob, RecoveryEffects};
use crate::{ExecRuntime, job_launch::ExitUsage};
use anyhow::{Result, ensure};
use fractal_jobs::{State, host_authority::JobHostBinding, launch::{CountersUnavailable, LaunchIntent},
    recovery::{Receipt, RecoveryEvidence, RecoveryPin, Released, Resolution, check_authority}};
use std::{fs, path::Path};

pub struct RecoveryRequest<'a> {
    pub binding: &'a JobHostBinding,
    pub state: State,
    pub intent: Option<&'a LaunchIntent>,
    pub released: Option<&'a Released>,
    pub pin: &'a RecoveryPin,
    pub heavy_slice: &'a str,
}
/// Not deserializable. A future post-sealing adapter obtains these facts from one
/// consistent membership/admitted-binding snapshot and verifies the full recovery
/// record's signature, signer/key, nonce and own open authenticated writer window.
/// A historical projection, request flag or daemon boot is not that proof.
pub struct RecoveryAuthentication {
    pub current: RecoveryPin,
    pub local_host: String,
    pub window_open: bool,
    pub authenticated_record: bool,
}
pub trait RecoveryJournal {
    fn authenticate(&mut self, request: &RecoveryRequest<'_>) -> Result<RecoveryAuthentication>;
    /// Append an authenticated recovery record through the existing public store
    /// append API. Recheck the pin/window and expected generation/state atomically.
    /// Persist usage (including typed unavailable) in that same recovery record.
    fn record(&mut self, request: &RecoveryRequest<'_>, resolution: &Resolution, usage: Option<&ExitUsage>) -> Result<()>;
}
#[derive(Debug)]
pub struct RecoveryResult {
    /// None means already terminal/unknown; no state was reopened.
    pub resolution: Option<Resolution>,
    pub usage: Option<ExitUsage>,
}
impl ExecRuntime {
    /// INACTIVE: no boot/runtime.rs hook or production caller. Activation must
    /// enumerate pinned nonterminal jobs, authenticate each recovery, and poll
    /// KeepRunning jobs until a receipt/stop yields a terminal state. Run this
    /// before considering new work; never re-pin the host or call launch_job.
    pub fn recover_job(
        &self, request: &RecoveryRequest<'_>, journal: &mut dyn RecoveryJournal,
        effects: &mut dyn RecoveryEffects,
    ) -> Result<RecoveryResult> {
        let authenticated = journal.authenticate(request)?;
        check_authority(request.binding, request.pin, &RecoveryEvidence {
            local_host: &authenticated.local_host, current: &authenticated.current,
            window_open: authenticated.window_open,
            authenticated_record: authenticated.authenticated_record,
        }).map_err(|error| anyhow::anyhow!("recovery authority refused: {error:?}"))?;
        ensure!(request.released.is_none_or(|released| released.pin == *request.pin), "recovery release pin mismatch");
        if !matches!(request.state, State::Queued | State::Preparing | State::Prepared
            | State::Launching | State::Running | State::Recovering
            | State::CancelRequested | State::RecoveringCancelRequested)
        {
            return Ok(RecoveryResult { resolution: None, usage: None });
        }
        let Some(intent) = request.intent else {
            let resolution = Resolution::UnknownAfterHostLoss;
            journal.record(request, &resolution, None)?;
            return Ok(RecoveryResult { resolution: Some(resolution), usage: None });
        };
        ensure!(Path::new(&intent.outcome_file).is_absolute(), "recovery requires recorded absolute outcome path");
        let Some(job) = effects.locate(request.binding, intent, request.heavy_slice)? else {
            let resolution = Resolution::UnknownAfterHostLoss;
            let usage = ExitUsage::CountersUnavailable(CountersUnavailable::CgroupVanished);
            journal.record(request, &resolution, Some(&usage))?;
            return Ok(RecoveryResult { resolution: Some(resolution), usage: Some(usage) });
        };
        if let Some(released) = request.released {
            if effects.holder_matches(&job, released)? && effects.populated(&job)? {
                let resolution = Resolution::KeepRunning;
                journal.record(request, &resolution, None)?;
                return Ok(RecoveryResult { resolution: Some(resolution), usage: None });
            }
        }
        // An uncertain/unreleased J is stopped, never released or relaunched.
        // Even normal receipt recovery uses kill->empty->sample->remove. Preserve
        // the sample in the authenticated record before reporting resolution.
        let usage = effects.finish(&job, &request.binding.execution_host)?;
        let resolution = receipt_resolution(request, &job);
        journal.record(request, &resolution, Some(&usage))?;
        Ok(RecoveryResult { resolution: Some(resolution), usage: Some(usage) })
    }
}
fn receipt_resolution(request: &RecoveryRequest<'_>, job: &LocatedJob) -> Resolution {
    let Some(released) = request.released else { return Resolution::UnknownAfterHostLoss; };
    let intent = request.intent.expect("located J requires intent");
    // Missing, unreadable, malformed and mismatched receipts are all unprovable;
    // none becomes a synthetic child failure or a permission to rerun.
    match fs::read(&intent.outcome_file).ok().and_then(|bytes| serde_json::from_slice::<Receipt>(&bytes).ok()) {
        Some(receipt) if receipt.matches(&request.binding.job_id, released.holder_pid, &job.hierarchy) => {
            Resolution::Outcome { outcome: receipt.outcome }
        }
        _ => Resolution::UnknownAfterHostLoss,
    }
}
