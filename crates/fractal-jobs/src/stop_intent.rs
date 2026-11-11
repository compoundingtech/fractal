//! Adopted Agent Policy v3. These decisions record intent, never stop processes.
//! A requester lease is an explicit trusted expiry input, not a step lease or a disconnect.
//! Running orphans retain declared max-hold/deadline enforcement; no default is invented.
//! `operator-stop` is reserved for a future audited person-scoped unit 9 daemon stop.
use crate::{Event, State};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    RequesterCancelled,
    RequesterLeaseExpired,
    MaxHold,
    MissionBudget,
}

/// Trusted in-process provenance, deliberately not deserializable from API input.
#[derive(Clone, Copy, Debug)]
pub enum StopOrigin<'a> {
    Requester { actor: Option<&'a str> },
    DaemonLimit { reason: LimitReason },
}
#[derive(Clone, Copy, Debug)]
pub enum LimitReason {
    /// A declared max-hold or runtime deadline; the durable reason stays `max-hold`.
    MaxHold,
    MissionBudget,
}
#[derive(Clone, Copy, Debug)]
pub enum StopInput {
    Cancel,
    RequesterLeaseExpired,
    WaitDisconnected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopDecision {
    Record { event: Event, reason: StopReason },
    Unchanged,
    /// Policy obligation, not evidence of a wired timer or runtime enforcement.
    KeepRunningWithDeclaredMaxHoldOrDeadline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopRefusal {
    RequesterUnresolved,
    NotRequester,
    JobTerminal,
    JobStateUncertain,
}
impl StopRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::RequesterUnresolved => "requester-unresolved",
            Self::NotRequester => "not-requester",
            Self::JobTerminal => "job-terminal",
            Self::JobStateUncertain => "job-state-uncertain",
        }
    }
}

pub fn decide(
    state: State,
    requester: &str,
    origin: StopOrigin<'_>,
    input: StopInput,
) -> Result<StopDecision, StopRefusal> {
    let reason = match origin {
        StopOrigin::Requester { actor: None | Some("") } => {
            return Err(StopRefusal::RequesterUnresolved);
        }
        StopOrigin::Requester { actor: Some(actor) } if actor != requester => {
            return Err(StopRefusal::NotRequester);
        }
        StopOrigin::Requester { .. } => StopReason::RequesterCancelled,
        StopOrigin::DaemonLimit { reason: LimitReason::MaxHold } => StopReason::MaxHold,
        StopOrigin::DaemonLimit { reason: LimitReason::MissionBudget } => StopReason::MissionBudget,
    };
    if matches!(input, StopInput::WaitDisconnected | StopInput::RequesterLeaseExpired) {
        if matches!(state, State::Running | State::Recovering | State::CancelRequested
            | State::RecoveringCancelRequested) {
            return Ok(StopDecision::KeepRunningWithDeclaredMaxHoldOrDeadline);
        }
        return Ok(if matches!(input, StopInput::RequesterLeaseExpired) && state == State::Queued {
            StopDecision::Record {
                event: Event::Withdraw,
                reason: StopReason::RequesterLeaseExpired,
            }
        } else {
            StopDecision::Unchanged
        });
    }
    let event = match state {
        State::Queued | State::Preparing | State::Prepared => Event::Withdraw,
        State::Launching | State::Running | State::Recovering => Event::CancelRequest,
        State::CancelRequested | State::RecoveringCancelRequested
        | State::Withdrawn | State::Cancelled => return Ok(StopDecision::Unchanged),
        State::UnknownAfterHostLoss { .. } => return Err(StopRefusal::JobStateUncertain),
        State::Completed { .. } | State::Refused { .. } | State::Failed { .. } => {
            return Err(StopRefusal::JobTerminal);
        }
    };
    Ok(StopDecision::Record { event, reason })
}

#[cfg(test)]
mod tests;
