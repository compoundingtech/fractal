//! INACTIVE whole-tree stop acknowledgements. Audit labels are not capabilities;
//! only a trusted, host-authorized daemon adapter may attest their authenticity.
use crate::{Event, State, TransitionError, recovery::RecoveryUsage, transition};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnforcementReason { MissionBudget, MaxHold }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StopAudit {
    Requester { principal: String },
    DaemonLimit { reason: EnforcementReason },
    /// Reserved: no endpoint. Activation must pin the product-approved PERSON
    /// identity outside public source and authenticate the reserved operator grant.
    Operator { principal: String, audit_id: String },
}
impl StopAudit {
    /// Shape/ownership validation only; does NOT authenticate the supplied label.
    pub fn validate(&self, requester: &str) -> Result<(), StopRefusal> {
        match self {
            Self::Requester { principal } if principal == requester => Ok(()),
            Self::DaemonLimit { .. } => Ok(()),
            Self::Operator { principal, audit_id }
                if principal.strip_prefix("person/").is_some_and(|id| !id.is_empty())
                    && !audit_id.trim().is_empty() => Ok(()),
            _ => Err(StopRefusal::Unauthorized),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopRefusal {
    Unauthorized, MissingGrace, MissingEscalationTimeout, InvalidState,
    GenerationMismatch, AuthorityUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopUncertain {
    HolderIdentity, SignalFailed, GraceFailed, KillFailed, EmptyTimeout,
    PopulationReadFailed, RemovalFailed, LaunchFenceUnavailable,
}

/// Retained in the authenticated job history. A natural ChildExit racing the
/// request does not choose Completed: confirmed whole-tree stop chooses Cancelled,
/// consistently with unit 8's RecoveringCancelRequested resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stop", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StopResolution {
    WholeTreeStopped { usage: RecoveryUsage },
    StopUncertain { reason: StopUncertain },
}

pub struct StopTiming {
    pub grace: Duration,
    pub escalation: Duration,
}
impl StopTiming {
    pub fn configured(grace: Option<Duration>, escalation: Option<Duration>) -> Result<Self, StopRefusal> {
        Ok(Self {
            grace: grace.ok_or(StopRefusal::MissingGrace)?,
            escalation: escalation.ok_or(StopRefusal::MissingEscalationTimeout)?,
        })
    }
}

pub fn resolve(state: State, resolution: &StopResolution) -> Result<State, TransitionError> {
    match state {
        State::Cancelled => Ok(state), // Repeated acknowledgement is idempotent.
        State::CancelRequested | State::RecoveringCancelRequested => match resolution {
            StopResolution::WholeTreeStopped { .. } => transition(state, Event::WholeTreeStopped),
            StopResolution::StopUncertain { .. } => Ok(state),
        },
        _ => Err(TransitionError { state, event: Event::WholeTreeStopped }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_requires_confirmation_not_a_holder_exit_or_timeout() {
        let stopped = StopResolution::WholeTreeStopped {
            usage: RecoveryUsage::CountersUnavailable(crate::launch::CountersUnavailable::CgroupVanished),
        };
        for state in [State::CancelRequested, State::RecoveringCancelRequested] {
            assert!(transition(state, Event::Exited { outcome: crate::ExitOutcome::Code(0) }).is_err());
            assert_eq!(resolve(state, &stopped).unwrap(), State::Cancelled);
            for reason in [StopUncertain::HolderIdentity, StopUncertain::SignalFailed,
                StopUncertain::GraceFailed, StopUncertain::KillFailed, StopUncertain::EmptyTimeout,
                StopUncertain::PopulationReadFailed, StopUncertain::RemovalFailed,
                StopUncertain::LaunchFenceUnavailable] {
                assert_eq!(resolve(state, &StopResolution::StopUncertain { reason }).unwrap(), state);
            }
        }
        assert_eq!(resolve(State::Cancelled, &stopped).unwrap(), State::Cancelled);
        assert!(resolve(State::Launching, &stopped).is_err());
        assert!(resolve(State::Running, &stopped).is_err());
    }
    #[test]
    fn only_configured_timings_and_permitted_audits_are_accepted() {
        assert!(matches!(StopTiming::configured(None, None), Err(StopRefusal::MissingGrace)));
        assert!(matches!(StopTiming::configured(Some(Duration::ZERO), None),
            Err(StopRefusal::MissingEscalationTimeout)));
        assert!(StopTiming::configured(Some(Duration::ZERO), Some(Duration::ZERO)).is_ok());
        assert!(StopAudit::Requester { principal: "person/other".into() }.validate("person/owner").is_err());
        assert!(StopAudit::Requester { principal: "person/owner".into() }.validate("person/owner").is_ok());
        for principal in ["agent/fixture", "person/"] {
            assert!(StopAudit::Operator { principal: principal.into(), audit_id: "audit".into() }
                .validate("person/owner").is_err());
        }
        assert!(StopAudit::Operator { principal: "person/fixture".into(), audit_id: " ".into() }
            .validate("person/owner").is_err());
        // Shape only: the runtime also requires the reserved authenticated grant.
        assert!(StopAudit::Operator { principal: "person/fixture".into(), audit_id: "audit".into() }
            .validate("person/owner").is_ok());
        for reason in [EnforcementReason::MissionBudget, EnforcementReason::MaxHold] {
            assert!(StopAudit::DaemonLimit { reason }.validate("person/owner").is_ok());
        }
    }
}
