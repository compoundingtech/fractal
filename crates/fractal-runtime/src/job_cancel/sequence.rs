use super::StopEffects;
use crate::{ExecRuntime, job_recovery::RecoveryAuthentication};
use anyhow::Result;
use fractal_jobs::{State, cancel::{StopAudit, StopRefusal, StopResolution, StopTiming},
    host_authority::JobHostBinding, launch::LaunchIntent,
    recovery::{RecoveryEvidence, RecoveryPin, check_authority}};
use std::time::Duration;

pub struct StopRequest<'a> {
    pub binding: &'a JobHostBinding,
    pub intent: &'a LaunchIntent,
    pub pin: &'a RecoveryPin,
    pub audit: &'a StopAudit,
    pub grace: Option<Duration>,
    pub escalation: Option<Duration>,
}
/// Trusted adapter output from one current, authenticated writer snapshot. None
/// of these facts can come from a request body, label or replicated projection.
pub struct StopAuthentication {
    pub authority: RecoveryAuthentication,
    pub state: State,
    pub intent: LaunchIntent,
    pub requester: String,
    pub authenticated_origin: bool,
    pub person_scoped: bool,
    /// Requires the exact reserved PERSON from deployment-owned policy, not an
    /// arbitrary person principal or agent-granted operator role. No default.
    pub reserved_operator_authenticated: bool,
}
pub trait StopJournal {
    fn authenticate(&mut self, request: &StopRequest<'_>) -> Result<StopAuthentication>;
    /// Append StopStarted through public append APIs before ANY stop effect.
    /// Recheck binding, generation, pending cancellation, origin, pin and own
    /// open writer window atomically. The real activation adapter is not wired.
    fn begin(&mut self, request: &StopRequest<'_>) -> Result<()>;
    /// Append StopResolved including its sample. Recheck the same facts. A
    /// concurrent natural ChildExit cannot outrank an already pending cancel.
    fn record(&mut self, request: &StopRequest<'_>, resolution: &StopResolution) -> Result<()>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum StopResult {
    AlreadyCancelled,
    Refused(StopRefusal),
    Resolved(StopResolution),
}
impl ExecRuntime {
    /// INACTIVE, no endpoint or scheduler caller. Unit 6 records the cancel
    /// request; this path only acknowledges it AFTER whole-tree retirement.
    /// Caller retains the actual shared per-job generation mutation guard,
    /// including the launch gate if unreleased, through this function's return.
    /// Daemon-limit and reserved person-scoped operator sources must also record
    /// cancel-requested before entry; no direct agent/process kill capability.
    pub fn cancel_job(
        &self, request: &StopRequest<'_>, journal: &mut dyn StopJournal,
        effects: &mut dyn StopEffects,
    ) -> Result<StopResult> {
        let authenticated = journal.authenticate(request)?;
        let authority = &authenticated.authority;
        if check_authority(request.binding, request.pin, &RecoveryEvidence {
            local_host: &authority.local_host, current: &authority.current,
            window_open: authority.window_open, authenticated_record: authority.authenticated_record,
        }).is_err() {
            return Ok(StopResult::Refused(StopRefusal::AuthorityUnavailable));
        }
        if !authenticated.authenticated_origin
            || request.audit.validate(&authenticated.requester).is_err()
            || matches!(request.audit, StopAudit::Operator { .. })
                && (!authenticated.person_scoped || !authenticated.reserved_operator_authenticated)
        {
            return Ok(StopResult::Refused(StopRefusal::Unauthorized));
        }
        if authenticated.intent != *request.intent {
            return Ok(StopResult::Refused(StopRefusal::GenerationMismatch));
        }
        if authenticated.state == State::Cancelled { return Ok(StopResult::AlreadyCancelled); }
        if !matches!(authenticated.state, State::CancelRequested | State::RecoveringCancelRequested) {
            return Ok(StopResult::Refused(StopRefusal::InvalidState));
        }
        let timing = match StopTiming::configured(request.grace, request.escalation) {
            Ok(timing) => timing,
            Err(refusal) => return Ok(StopResult::Refused(refusal)),
        };
        journal.begin(request)?;
        let resolution = match effects.stop(timing.grace, timing.escalation) {
            Ok(usage) => StopResolution::WholeTreeStopped { usage },
            Err(reason) => StopResolution::StopUncertain { reason },
        };
        // Journal failure is NOT an acknowledgement, even after kernel stop.
        journal.record(request, &resolution)?;
        Ok(StopResult::Resolved(resolution))
    }
}
