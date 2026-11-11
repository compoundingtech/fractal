//! INACTIVE restart fencing. No decision here permits launch, gate release or re-pinning.
use crate::{Event, RefusalReason, State, TransitionError, host_authority::JobHostBinding,
    launch::{HolderOutcome, LaunchOutcome}, transition};
use serde::{Deserialize, Serialize};

/// Persist with the admitted launch. A daemon restart never rotates this pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPin {
    pub host: String,
    pub member_key: String,
    /// Unit 3 authority_declaration is Incarnation.admitted_claim, not a boot ID.
    pub admitted_claim: String,
}

/// Trusted adapter inputs, not request JSON or execution capabilities. The adapter
/// must authenticate the complete recovery record in its own open writer window,
/// and resolve membership and the persisted pin from one consistent snapshot.
/// 4c-2 supplies that adapter; this module verifies no signatures itself.
pub struct RecoveryEvidence<'a> {
    pub local_host: &'a str,
    pub current: &'a RecoveryPin,
    pub window_open: bool,
    pub authenticated_record: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityError { HostMismatch, IncarnationMismatch, ClosedWindow, Unauthenticated }

pub fn check_authority(
    binding: &JobHostBinding, pin: &RecoveryPin, evidence: &RecoveryEvidence<'_>,
) -> Result<(), AuthorityError> {
    if binding.execution_host != pin.host || evidence.local_host != pin.host {
        return Err(AuthorityError::HostMismatch);
    }
    if pin != evidence.current || pin.member_key.is_empty() || pin.admitted_claim.is_empty()
        || binding.authority_declaration != pin.admitted_claim
    {
        return Err(AuthorityError::IncarnationMismatch);
    }
    if !evidence.window_open { return Err(AuthorityError::ClosedWindow); }
    if !evidence.authenticated_record { return Err(AuthorityError::Unauthenticated); }
    Ok(())
}

/// Kernel incarnation of a process, not just a reusable numeric PID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub boot_id: String,
    pub start_ticks: u64,
}

/// Authenticated durable release fact. Absence means uncertainty even if a PID
/// exists. PID identity is checked against J, never via kill(pid, 0) alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Released {
    pub holder_pid: u32,
    pub pin: RecoveryPin,
    pub identity: ProcessIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "resolution", rename_all = "kebab-case")]
pub enum Resolution {
    KeepRunning,
    UnknownAfterHostLoss,
    Outcome { outcome: HolderOutcome },
}

/// Retained in the authenticated recovery record, including after an uncertain
/// release. Missing counters are explicit evidence, never a synthetic zero.
/// Samples are boxed in memory; serde retains the same tagged sample payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "usage", content = "value", rename_all = "kebab-case")]
pub enum RecoveryUsage {
    Sample(Box<crate::scope::observation::RawScopeUsage>),
    CountersUnavailable(crate::launch::CountersUnavailable),
}

/// Applies only after runtime fencing/observation and authenticated recording.
/// Terminal/unknown states are not reopened. Pending jobs become unknown rather
/// than re-entering the queue; cancellation remains attached to unknown state.
pub fn resolve(state: State, resolution: &Resolution) -> Result<State, TransitionError> {
    let recovering = match state {
        State::Queued | State::Preparing | State::Prepared => {
            return transition(state, Event::HostLost);
        }
        State::Launching | State::Running | State::CancelRequested => transition(state, Event::Recover)?,
        State::Recovering | State::RecoveringCancelRequested => state,
        _ => return Err(TransitionError { state, event: Event::Recover }),
    };
    match resolution {
        Resolution::KeepRunning => {
            if recovering == State::RecoveringCancelRequested { Ok(recovering) }
            else { transition(recovering, Event::Started) }
        }
        Resolution::UnknownAfterHostLoss => transition(recovering, Event::HostLost),
        Resolution::Outcome { outcome } => match LaunchOutcome::from(outcome.clone()) {
            // The runtime has fenced and confirmed the entire tree stopped.
            // Recovering an already-stopped cancellation is not an ack race.
            LaunchOutcome::Exited(_) if recovering == State::RecoveringCancelRequested => {
                transition(recovering, Event::WholeTreeStopped)
            }
            LaunchOutcome::Exited(outcome) => transition(recovering, Event::Exited { outcome }),
            LaunchOutcome::Failed(failure) => transition(recovering, Event::FailedTreeStopped { failure }),
            LaunchOutcome::Withdrawn => Ok(State::Withdrawn),
            LaunchOutcome::Refused { .. } => Ok(State::Refused { refusal: RefusalReason::HolderPrecondition }),
        },
    }
}

/// B6 receipt identity is shared by initial launch and restart recovery.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub mode: String,
    pub job_label: String,
    pub holder_pid: u32,
    pub job_cgroup: String,
    pub launched: bool,
    #[serde(flatten)]
    pub outcome: HolderOutcome,
}
impl Receipt {
    pub fn matches(&self, job: &str, holder: u32, hierarchy: &str) -> bool {
        self.mode == "scope-none" && self.job_label == job && self.holder_pid == holder
            && self.job_cgroup == hierarchy && match self.outcome {
                HolderOutcome::Refused { .. } | HolderOutcome::Withdrawn => !self.launched,
                HolderOutcome::MaxHold | HolderOutcome::ChildExit { .. } => self.launched,
                HolderOutcome::HolderLost => true,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_exit_after_confirmed_stop_resolves_cancellation() {
        let resolution = Resolution::Outcome {
            outcome: HolderOutcome::ChildExit { child_exit_code: 0 },
        };
        for state in [State::CancelRequested, State::RecoveringCancelRequested] {
            assert_eq!(resolve(state, &resolution).unwrap(), State::Cancelled);
        }
    }
    #[test]
    fn restart_keeps_pin_and_requires_every_authority_fact() {
        let pin = RecoveryPin { host: "node".into(), member_key: "key".into(), admitted_claim: "admission".into() };
        let binding = JobHostBinding { job_id: "job".into(), execution_host: pin.host.clone(), authority_declaration: pin.admitted_claim.clone() };
        let mut evidence = RecoveryEvidence { local_host: "node", current: &pin, window_open: true, authenticated_record: true };
        assert_eq!(check_authority(&binding, &pin, &evidence), Ok(()));
        evidence.local_host = "other";
        assert_eq!(check_authority(&binding, &pin, &evidence), Err(AuthorityError::HostMismatch));
        evidence.local_host = "node";
        evidence.window_open = false;
        assert_eq!(check_authority(&binding, &pin, &evidence), Err(AuthorityError::ClosedWindow));
        evidence.window_open = true;
        evidence.authenticated_record = false;
        assert_eq!(check_authority(&binding, &pin, &evidence), Err(AuthorityError::Unauthenticated));
        let rotated = RecoveryPin { member_key: "new-key".into(), ..pin.clone() };
        evidence.current = &rotated;
        assert_eq!(check_authority(&binding, &pin, &evidence), Err(AuthorityError::IncarnationMismatch));
    }
    #[test]
    fn recovery_never_requeues_pending_or_reopens_unknown() {
        for state in [State::Queued, State::Preparing, State::Prepared, State::Launching, State::Running] {
            assert_eq!(resolve(state, &Resolution::UnknownAfterHostLoss).unwrap(), State::UnknownAfterHostLoss { cancel_requested: false });
        }
        let unknown = State::UnknownAfterHostLoss { cancel_requested: false };
        assert!(resolve(unknown, &Resolution::KeepRunning).is_err());
        assert_eq!(resolve(State::Running, &Resolution::KeepRunning).unwrap(), State::Running);
        assert_eq!(resolve(State::CancelRequested, &Resolution::UnknownAfterHostLoss).unwrap(), State::UnknownAfterHostLoss { cancel_requested: true });
    }
    #[test]
    fn receipt_identity_and_outcome_must_agree() {
        let mut receipt: Receipt = serde_json::from_str(r#"{"mode":"scope-none","jobLabel":"job","holderPid":7,"jobCgroup":"/keeper/J","launched":true,"holderOutcome":"child-exit","childExitCode":0}"#).unwrap();
        assert!(receipt.matches("job", 7, "/keeper/J"));
        assert!(!receipt.matches("job", 8, "/keeper/J"));
        assert!(!receipt.matches("job", 7, "/different-generation"));
        receipt.launched = false;
        assert!(!receipt.matches("job", 7, "/keeper/J"));
    }
}
