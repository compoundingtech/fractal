//! Ephemeral admission diagnostics shared by the planner and live API reads.
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};

use crate::seat_admission::Decision;

#[derive(Clone)]
pub struct Pending {
    pub subject: String,
    pub token: Option<String>,
    pub request: Option<String>,
    pub since: u64,
}

pub struct State {
    pub decision: Decision,
    pub pending: VecDeque<Pending>,
    pub replacements: BTreeSet<String>,
}

pub struct Status(Mutex<State>);

impl Default for Status {
    fn default() -> Self {
        Self(Mutex::new(State {
            decision: Decision {
                mode: "off".into(), reason: "seat admission disabled".into(),
                queue: false, observed_at: None,
            },
            pending: VecDeque::new(),
            replacements: BTreeSet::new(),
        }))
    }
}

impl Status {
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn snapshot(&self, now: u64) -> Value {
        let state = self.state();
        json!({
            "mode": state.decision.mode,
            "reason": state.decision.reason,
            "pendingCount": state.pending.len(),
            "oldestPendingAgeSecs": state.pending.front().map_or(0, |p| now.saturating_sub(p.since)),
            "pending": state.pending.iter().enumerate().map(|(index, p)| pending_value(p, index, &state.decision.reason, now)).collect::<Vec<_>>(),
            "replacements": state.replacements,
        })
    }

    pub fn subject(&self, subject: &str, now: u64) -> Option<Value> {
        let state = self.state();
        state.pending.iter().enumerate().find(|(_, p)| p.subject == subject)
            .map(|(index, p)| pending_value(p, index, &state.decision.reason, now))
            .or_else(|| state.replacements.contains(subject).then(|| json!({
                "pending": false, "reason": "admitted: replacement",
            })))
    }

    /// FIFO order is local to this daemon lifetime; it is not a persisted work queue.
    pub fn admit(&self, subject: &str, token: Option<String>, request: Option<String>, replacement: bool, now: u64) -> bool {
        let mut state = self.state();
        state.pending.retain(|p| p.subject != subject || (p.token == token && p.request == request));
        if replacement {
            state.pending.retain(|p| p.subject != subject);
            state.replacements.insert(subject.into());
            return true;
        }
        state.replacements.remove(subject);
        if !state.decision.queue && state.pending.front().is_none_or(|p| p.subject == subject) {
            state.pending.pop_front();
            return true;
        }
        if !state.pending.iter().any(|p| p.subject == subject) {
            state.pending.push_back(Pending { subject: subject.into(), token, request, since: now });
        }
        false
    }
}

fn pending_value(p: &Pending, index: usize, reason: &str, now: u64) -> Value {
    json!({"subject": p.subject, "pending": true, "queuePosition": index + 1,
        "queuedSince": p.since, "pendingAgeSecs": now.saturating_sub(p.since), "reason": reason})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_snapshot_is_fifo_and_ages_without_persistence() {
        let status = Status::default();
        status.state().decision.queue = true;
        assert!(!status.admit("first", None, None, false, 10));
        assert!(!status.admit("second", None, None, false, 11));
        assert!(!status.admit("first", None, None, false, 12));
        assert_eq!(status.snapshot(20)["pendingCount"], 2);
        assert_eq!(status.snapshot(20)["oldestPendingAgeSecs"], 10);
        assert_eq!(status.subject("second", 20).unwrap()["queuePosition"], 2);
        status.state().decision.queue = false;
        assert!(!status.admit("second", None, None, false, 21));
        assert!(status.admit("first", None, None, false, 21));
        assert!(status.admit("second", None, None, false, 21));
        assert_eq!(status.snapshot(21)["pendingCount"], 0);
        assert_eq!(Status::default().snapshot(22)["pendingCount"], 0);
    }

    #[test]
    fn replacement_bypasses_pressure_and_removes_pending_intent() {
        let status = Status::default();
        status.state().decision.queue = true;
        assert!(!status.admit("seat", None, None, false, 10));
        assert!(status.admit("seat", None, None, true, 11));
        assert_eq!(status.snapshot(11)["pendingCount"], 0);
        assert_eq!(status.subject("seat", 11).unwrap()["reason"], "admitted: replacement");
    }
}
