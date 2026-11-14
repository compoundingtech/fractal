//! Synchronous execution admission, not an activity snapshot or permission to stop.
//!
//! Adapters default to `Unsupported`. Opting in requires routing EVERY tool, turn,
//! mailbox turn and background-job start through one gate before any effects, and
//! retaining its `Work` until effects finish (including cancellation). Only durably
//! captured person waits may be excluded; permission-blocked effects remain work.
//! A gate belongs to one incarnation/owner sequence and must not be recreated open
//! after an uncertain crash. Recover the owned seal or remain unsupported/closed.

use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateState {
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityReceipt {
    pub incarnation: Arc<str>,
    pub owner_sequence: u64,
    pub epoch: u64,
    /// Includes turns and jobs, not just individual tool invocations.
    pub open_non_person_tools: u64,
    pub gate: GateState,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CloseResult {
    Unsupported,
    Refused,
    Closed(ActivityReceipt),
}

/// No default implementation may infer support from idle observations or signals.
/// Implementations must serialize close, reopen, starts and completions together.
pub trait RefreshAdmission: Send + Sync {
    fn activity(&self) -> Option<ActivityReceipt> {
        None
    }
    fn close_for_refresh(&self, _expected_incarnation: &str, _expected_epoch: u64) -> CloseResult {
        CloseResult::Unsupported
    }
    /// Only the exact returned closed receipt can release its seal, never a timer.
    fn reopen(&self, _seal: &ActivityReceipt) -> bool {
        false
    }
}

/// The capability for every harness until its real execution boundary opts in.
#[derive(Default)]
pub struct Unsupported;
impl RefreshAdmission for Unsupported {}

/// Reusable gate for a positively integrated adapter; constructing it is not support.
/// No lock is held while executing work. A poisoned lock fails closed by panicking.
pub struct AdmissionGate(Mutex<ActivityReceipt>);

impl AdmissionGate {
    pub fn new(incarnation: Arc<str>, owner_sequence: u64) -> Self {
        Self(Mutex::new(ActivityReceipt {
            incarnation,
            owner_sequence,
            epoch: 0,
            open_non_person_tools: 0,
            gate: GateState::Open,
        }))
    }

    /// Acquire BEFORE starting effects; a closed gate refuses without executing work.
    pub fn start(&self) -> Option<Work<'_>> {
        let mut state = self.0.lock().expect("admission gate poisoned");
        if state.gate != GateState::Open {
            return None;
        }
        let epoch = state
            .epoch
            .checked_add(1)
            .expect("admission epoch exhausted");
        let count = state
            .open_non_person_tools
            .checked_add(1)
            .expect("admission count exhausted");
        state.epoch = epoch;
        state.open_non_person_tools = count;
        Some(Work(self))
    }
}

/// Dropping a controller receipt never opens admission. Only work completion drops
/// this guard; detached jobs must retain it rather than ending it with their turn.
#[must_use]
pub struct Work<'a>(&'a AdmissionGate);

impl Drop for Work<'_> {
    fn drop(&mut self) {
        let mut state = self.0.0.lock().expect("admission gate poisoned");
        state.open_non_person_tools -= 1;
    }
}

impl RefreshAdmission for AdmissionGate {
    fn activity(&self) -> Option<ActivityReceipt> {
        Some(self.0.lock().expect("admission gate poisoned").clone())
    }

    fn close_for_refresh(&self, expected_incarnation: &str, expected_epoch: u64) -> CloseResult {
        let mut state = self.0.lock().expect("admission gate poisoned");
        if state.incarnation.as_ref() != expected_incarnation
            || state.epoch != expected_epoch
            || state.open_non_person_tools != 0
            || state.gate != GateState::Open
        {
            return CloseResult::Refused;
        }
        state.epoch = state
            .epoch
            .checked_add(1)
            .expect("admission epoch exhausted");
        state.gate = GateState::Closed;
        CloseResult::Closed(state.clone())
    }

    fn reopen(&self, seal: &ActivityReceipt) -> bool {
        let mut state = self.0.lock().expect("admission gate poisoned");
        if state.gate != GateState::Closed || *state != *seal {
            return false;
        }
        state.epoch = state
            .epoch
            .checked_add(1)
            .expect("admission epoch exhausted");
        state.gate = GateState::Open;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeDriver(AdmissionGate);
    impl FakeDriver {
        fn new() -> Self {
            Self(AdmissionGate::new(Arc::from("incumbent"), 7))
        }
        fn close(&self) -> CloseResult {
            let receipt = self.0.activity().unwrap();
            self.0
                .close_for_refresh(&receipt.incarnation, receipt.epoch)
        }
    }

    #[test]
    fn idle_close_blocks_starts_until_owned_reopen() {
        let driver = FakeDriver::new();
        let CloseResult::Closed(seal) = driver.close() else {
            panic!("idle close refused")
        };
        assert_eq!(seal.owner_sequence, 7);
        assert_eq!(seal.gate, GateState::Closed);
        assert!(driver.0.start().is_none());
        assert_eq!(driver.close(), CloseResult::Refused);
        let mut foreign = seal.clone();
        foreign.owner_sequence += 1;
        assert!(!driver.0.reopen(&foreign));
        assert!(driver.0.reopen(&seal));
        assert!(driver.0.start().is_some());
        assert!(!driver.0.reopen(&seal));
    }

    #[test]
    fn start_wins_close_refuses_open_tools_turns_and_jobs() {
        let driver = FakeDriver::new();
        let tool = driver.0.start().unwrap();
        let job = driver.0.start().unwrap();
        assert_eq!(driver.0.activity().unwrap().open_non_person_tools, 2);
        assert_eq!(driver.close(), CloseResult::Refused);
        drop(tool);
        assert_eq!(driver.close(), CloseResult::Refused);
        drop(job);
        assert!(matches!(driver.close(), CloseResult::Closed(_)));
    }

    #[test]
    fn completed_work_stales_epoch_and_predecessor_cannot_close() {
        let driver = FakeDriver::new();
        let before = driver.0.activity().unwrap();
        drop(driver.0.start().unwrap());
        assert_eq!(
            driver.0.close_for_refresh("incumbent", before.epoch),
            CloseResult::Refused
        );
        let now = driver.0.activity().unwrap();
        assert_eq!(
            driver.0.close_for_refresh("predecessor", now.epoch),
            CloseResult::Refused
        );
        assert!(matches!(driver.close(), CloseResult::Closed(_)));
    }

    #[test]
    fn abandoned_seal_stays_closed_and_old_receipt_cannot_reopen_new_seal() {
        let driver = FakeDriver::new();
        let CloseResult::Closed(old) = driver.close() else {
            panic!("idle close refused")
        };
        assert!(driver.0.reopen(&old));
        assert!(matches!(driver.close(), CloseResult::Closed(_)));
        assert!(!driver.0.reopen(&old));
        assert!(driver.0.start().is_none());
        let successor = AdmissionGate::new(Arc::from("successor"), 8);
        assert!(!successor.reopen(&old));
    }

    #[test]
    fn unsupported_default_never_closes() {
        let driver = Unsupported;
        assert!(driver.activity().is_none());
        assert_eq!(
            driver.close_for_refresh("incumbent", 0),
            CloseResult::Unsupported
        );
        assert!(!driver.reopen(&FakeDriver::new().0.activity().unwrap()));
    }
}
