//! What a mission run waits on, and the fleet's blocked runs grouped by cause: the answer to "why
//! is this stuck" read from state st already keeps current, never from claim history
//! (docs/st3/waiting-on.md).

use serde::{Deserialize, Serialize};

/// Why an open step is not moving, or that it is. The first five are the causes a blocked run is
/// grouped by, in the order a run's cause is chosen; the rest are not blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaitKind {
    /// An open ask or human gate waits for a person.
    Ask,
    /// The seat that holds or would take the step is stopped, failed, suspended, down or retired.
    SeatDown,
    /// A seat holds the step and has recorded no progress for longer than the quiet limit.
    NoProgress,
    /// Ready for longer than the quiet limit, and no seat has claimed it.
    Unclaimed,
    /// Held outside the fleet: blocked with a reason, a declared `work wait`, a retry backoff, or
    /// st's wait for another run.
    Hold,
    /// A seat holds it and has recorded progress within the quiet limit.
    Working,
    /// Submitted, and st is checking its gates.
    Verifying,
    /// Ready within the quiet limit; nobody has claimed it yet.
    Ready,
    /// Ready, and waiting its turn behind other work on a busy seat.
    Queued,
    /// st's own keep-open step, which holds the run open while its observers watch.
    Watching,
}

impl WaitKind {
    /// The causes a run can be blocked by, in the order a run's cause is chosen.
    pub const CAUSES: [WaitKind; 5] = [
        WaitKind::Ask,
        WaitKind::SeatDown,
        WaitKind::NoProgress,
        WaitKind::Unclaimed,
        WaitKind::Hold,
    ];

    pub fn is_cause(self) -> bool {
        Self::CAUSES.contains(&self)
    }

    /// Whether the step is stuck in a way st cannot resolve by waiting: nobody is moving it.
    pub fn stalls(self) -> bool {
        matches!(self, WaitKind::SeatDown | WaitKind::NoProgress | WaitKind::Unclaimed)
    }

    pub fn name(self) -> &'static str {
        match self {
            WaitKind::Ask => "ask",
            WaitKind::SeatDown => "seat-down",
            WaitKind::NoProgress => "no-progress",
            WaitKind::Unclaimed => "unclaimed",
            WaitKind::Hold => "hold",
            WaitKind::Working => "working",
            WaitKind::Verifying => "verifying",
            WaitKind::Ready => "ready",
            WaitKind::Queued => "queued",
            WaitKind::Watching => "watching",
        }
    }

    /// The heading a fleet list groups runs of this cause under.
    pub fn heading(self) -> &'static str {
        match self {
            WaitKind::Ask => "waiting on a person",
            WaitKind::SeatDown => "agent stopped or down",
            WaitKind::NoProgress => "claimed, no progress",
            WaitKind::Unclaimed => "ready, nobody claims it",
            WaitKind::Hold => "held",
            WaitKind::Working => "working",
            WaitKind::Verifying => "verifying",
            WaitKind::Ready => "ready",
            WaitKind::Queued => "queued",
            WaitKind::Watching => "watching",
        }
    }
}

/// One open step and what it waits on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepWait {
    /// The step-run subject.
    pub step: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The step's effective state.
    pub state: String,
    pub kind: WaitKind,
    /// Whom it waits on: the person an ask waits for, the seat that holds the step, or the seat
    /// that would take it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    /// That seat's state, as the agents roster says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat_state: Option<String>,
    /// Since when it has been this way: the last progress (or claim) of a held step, when a
    /// ready step became ready, when an ask was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_unix_ms: Option<u128>,
    /// The holder's lease, for a held step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at_unix_ms: Option<u128>,
    /// When a declared wait or retry backoff ends on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_unix_ms: Option<u128>,
    /// What st knows of it in words: the last progress, the reason it is blocked, the ask's
    /// question, what a queued step waits behind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// How many times st has woken the seat for this ready step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_attempts: Option<u32>,
}

/// A step that is not open: finished, or pending on its dependencies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepSettled {
    pub step: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub state: String,
    pub at_unix_ms: u128,
}

/// Whether a run is moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunVerdict {
    /// Some step is moving, or will without anyone acting.
    Moving,
    /// Nothing is moving, and a step waits on a person or a hold.
    Waiting,
    /// A step is stuck: its seat is down, its holder is quiet, or nobody claims it.
    Stalled,
    /// The run has finished.
    Finished,
}

/// What one mission run waits on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunWaitingOn {
    pub run: String,
    pub mission: String,
    pub status: String,
    pub generation: String,
    pub verdict: RunVerdict,
    /// The cause the run is blocked by, when it is waiting or stalled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<WaitKind>,
    /// The verdict in words, naming the step and whom it waits on.
    pub reason: String,
    /// Each open step, most blocking first.
    pub open: Vec<StepWait>,
    /// Steps not yet ready.
    pub pending: Vec<StepSettled>,
    /// Steps that completed, failed or were cancelled, most recent first.
    pub finished: Vec<StepSettled>,
    /// The quiet limit a held step's progress and a ready step's age are measured against.
    pub quiet_after_ms: u64,
    /// The graph cut of the published work the open steps were read from.
    pub work_cut: u64,
    pub as_of_unix_ms: u128,
}

/// One blocked run in the fleet list: its cause and the step that causes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedRun {
    pub run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<String>,
    pub cause: WaitKind,
    pub verdict: RunVerdict,
    /// The step that causes it, and what that step waits on.
    pub step: StepWait,
    /// How many of the run's open steps are blocked.
    pub blocked_steps: usize,
}

/// The runs of one cause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedGroup {
    pub cause: WaitKind,
    pub runs: Vec<BlockedRun>,
}

/// Every blocked run in the fleet, grouped by cause in the causes' order, longest blocked first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedRuns {
    pub groups: Vec<BlockedGroup>,
    /// How many runs with open work were read.
    pub runs_read: usize,
    pub quiet_after_ms: u64,
    pub work_cut: u64,
    pub as_of_unix_ms: u128,
}

/// A run's header and every step of its current generation as `step_runs` stores them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSteps {
    pub run: String,
    pub mission: String,
    pub status: String,
    pub generation: String,
    pub steps: Vec<RunStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStep {
    pub subject: String,
    pub path: String,
    pub title: Option<String>,
    /// The stored state, before effective state: a step the work list shows reads its state there.
    pub status: String,
    pub updated_at_unix_ms: u128,
}

/// A duration in the largest two units that fit, as `3h 10m`, `45m` or `20s`.
pub fn describe_age(ms: u128) -> String {
    let seconds = ms / 1000;
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{seconds}s"),
        (0, 0, minutes) => format!("{minutes}m"),
        (0, hours, 0) => format!("{hours}h"),
        (0, hours, minutes) => format!("{hours}h {minutes}m"),
        (days, 0, _) => format!("{days}d"),
        (days, hours, _) => format!("{days}d {hours}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_name_their_two_largest_units() {
        assert_eq!(describe_age(20_000), "20s");
        assert_eq!(describe_age(45 * 60_000 + 5_000), "45m");
        assert_eq!(describe_age(3 * 3_600_000), "3h");
        assert_eq!(describe_age(3 * 3_600_000 + 10 * 60_000), "3h 10m");
        assert_eq!(describe_age(2 * 86_400_000 + 5 * 3_600_000 + 1), "2d 5h");
    }

    #[test]
    fn causes_sort_before_the_kinds_that_move_and_in_their_choosing_order() {
        let mut kinds = vec![WaitKind::Working, WaitKind::Hold, WaitKind::Ask, WaitKind::Unclaimed];
        kinds.sort();
        assert_eq!(kinds, [WaitKind::Ask, WaitKind::Unclaimed, WaitKind::Hold, WaitKind::Working]);
        assert!(WaitKind::CAUSES.iter().all(|cause| cause.is_cause()));
        assert!(!WaitKind::Queued.is_cause() && !WaitKind::Queued.stalls());
    }
}
