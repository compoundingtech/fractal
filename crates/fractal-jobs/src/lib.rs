//! Inactive contract: no runtime consumer, I/O, policy defaults or fleet limits.
//! Requests declare resources, not authority. Host capability/grant checks are external.
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, num::NonZeroU32};

pub mod identity;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub cpu_millicores: u64,
    pub memory_bytes: u64,
    /// Advisory relative weight, NOT an IO rate/IOPS limit or containment guarantee.
    pub io_weight: u64,
    pub tasks_max: u64,
    /// Backfill estimate, never a runtime deadline.
    pub expected_duration_ms: u64,
    /// Optional hard execution clock, independent of the estimate.
    pub run_timeout_ms: Option<u64>,
    pub priority: i32,
    pub requested_class: String,
    pub storage_request: Option<StorageRequest>,
    pub delegated_reservations: Option<DelegatedReservations>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageRequest {
    pub bytes: u64,
    pub purpose: StoragePurpose,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoragePurpose { Scratch, BuildOutput }

/// Opaque reserved slot; no delegation semantics or authority until unit 20.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DelegatedReservations(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit { CpuMillicores, MemoryBytes, IoWeight, TasksMax, ExpectedDurationMs, RunTimeoutMs, StorageBytes }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError { NonPositive(Unit) }
impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") }
}
impl Error for RequestError {}
impl JobRequest {
    /// All units must fit u64 on the wire and be positive; no host limits implied.
    pub fn validate(&self) -> Result<(), RequestError> {
        use Unit::*;
        for (value, unit) in [
            (self.cpu_millicores, CpuMillicores), (self.memory_bytes, MemoryBytes),
            (self.io_weight, IoWeight), (self.tasks_max, TasksMax),
            (self.expected_duration_ms, ExpectedDurationMs),
            (self.run_timeout_ms.unwrap_or(1), RunTimeoutMs),
            (self.storage_request.as_ref().map_or(1, |r| r.bytes), StorageBytes),
        ] {
            if value == 0 { return Err(RequestError::NonPositive(unit)); }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "coverage", rename_all = "kebab-case")]
pub enum Coverage { Measured, Partial { reason: String }, Unknown { reason: String } }
/// Coverage is attached to each field, not shared across a whole observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Covered<T> { pub value: Option<T>, pub coverage: Coverage }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Containment {
    pub cpu: Covered<bool>, pub memory: Covered<bool>,
    pub io: Covered<bool>, pub tasks: Covered<bool>,
}
/// CPU/IO are interval deltas; memory is a peak, never an additive total.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub cpu_usec: Covered<u64>, pub memory_peak_bytes: Covered<u64>,
    pub io_read_bytes: Covered<u64>, pub io_write_bytes: Covered<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum RefusalReason { InvalidRequest, Unfulfillable, PolicyDenied, HolderPrecondition, ExceedsClassCap }
/// MaxHold: held tree killed at deadline. HolderLost: holder died, work emptied,
/// then slot reclaimed. OutcomeUnwritten: holder exit 70, final outcome write failed.
/// These are infrastructure failures, not launcher exit codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum FailureReason { MaxHold, HolderLost, OutcomeUnwritten }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum ExitOutcome { Code(i32), Signal(NonZeroU32) }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum State {
    Queued, Preparing, Prepared, Launching, Running, Recovering,
    CancelRequested, RecoveringCancelRequested,
    UnknownAfterHostLoss { cancel_requested: bool },
    Completed { outcome: ExitOutcome }, Refused { refusal: RefusalReason },
    Withdrawn, Cancelled, Failed { failure: FailureReason },
}
/// Holder Refuse/Withdraw confirms nothing launched, including cancellation during slot wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Prepare, Prepared, LaunchIntent, Started, Recover, HostLost,
    Refuse { refusal: RefusalReason }, Withdraw, CancelRequest,
    Exited { outcome: ExitOutcome }, WholeTreeStopped, FailedTreeStopped { failure: FailureReason },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionError { pub state: State, pub event: Event }
impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") }
}
impl Error for TransitionError {}
/// Events are durable facts supplied by the owner, not commands to signal processes.
/// WholeTreeStopped certifies all descendants stopped and bounded cleanup finished.
/// Disconnect is not an event; queued lease expiry is Withdraw, never running exit.
pub fn transition(state: State, event: Event) -> Result<State, TransitionError> {
    use Event as E;
    use State as S;
    Ok(match (state, event) {
        (S::Queued, E::Prepare) => S::Preparing,
        (S::Preparing, E::Prepared) => S::Prepared,
        (S::Queued | S::Prepared, E::LaunchIntent) => S::Launching,
        (S::Queued | S::Preparing | S::Prepared | S::Launching, E::Refuse { refusal }) => S::Refused { refusal },
        (S::Queued | S::Preparing | S::Prepared | S::Launching | S::CancelRequested, E::Withdraw) => S::Withdrawn,
        (S::Launching | S::Recovering, E::Started) => S::Running,
        (S::Launching | S::Running, E::Recover) => S::Recovering,
        (S::CancelRequested, E::Recover) => S::RecoveringCancelRequested,
        (S::Launching | S::Running | S::Recovering, E::CancelRequest) => S::CancelRequested,
        (S::Launching | S::Running | S::Recovering, E::HostLost) => S::UnknownAfterHostLoss { cancel_requested: false },
        (S::CancelRequested | S::RecoveringCancelRequested, E::HostLost) => S::UnknownAfterHostLoss { cancel_requested: true },
        (S::UnknownAfterHostLoss { cancel_requested: false }, E::Recover) => S::Recovering,
        (S::UnknownAfterHostLoss { cancel_requested: true }, E::Recover) => S::RecoveringCancelRequested,
        (S::Running | S::Recovering, E::Exited { outcome }) => S::Completed { outcome },
        (S::CancelRequested | S::RecoveringCancelRequested, E::WholeTreeStopped) => S::Cancelled,
        (S::Running | S::Recovering | S::CancelRequested | S::RecoveringCancelRequested, E::FailedTreeStopped { failure }) => S::Failed { failure },
        _ => return Err(TransitionError { state, event }),
    })
}
