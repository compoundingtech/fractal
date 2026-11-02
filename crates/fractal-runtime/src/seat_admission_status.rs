//! Ephemeral admission diagnostics shared by the planner and live API reads.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

/// A laned seat whose lane does not fit its host yet (docs/st3/seat-lanes.md, "Admission by fit").
#[derive(Clone)]
pub struct LaneWait {
    pub subject: String,
    pub since: u64,
    pub reason: String,
    /// In line for room. A lane larger than the host can ever give waits out of line, so it holds
    /// no one back.
    pub queued: bool,
    /// When admission last found it waiting. A seat no longer tried, such as one suspended or
    /// crash-looping, stops holding its place once this is older than [`LANE_WAIT_STALE_SECS`].
    pub checked: u64,
}

/// How long a waiting seat keeps its place without being tried again: longer than a full
/// reconcile pass, which tries every seat that should start.
pub const LANE_WAIT_STALE_SECS: u64 = 360;

pub struct State {
    pub decision: Decision,
    pub pending: VecDeque<Pending>,
    pub replacements: BTreeSet<String>,
    pub lane_waits: VecDeque<LaneWait>,
    /// Lanes admitted to start that no observation shows running yet, with when, so two starts in
    /// one pass cannot both take the same room.
    pub lane_starts: BTreeMap<String, (crate::Capacity, u64)>,
    /// The lane each seat holds above its own while it holds a step that raises it.
    pub lane_raised: BTreeMap<String, crate::Capacity>,
    /// The lane this daemon last set on each seat's slice, and the incarnation it set it for.
    pub lane_applied: BTreeMap<String, (String, crate::Capacity)>,
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
            lane_waits: VecDeque::new(),
            lane_starts: BTreeMap::new(),
            lane_raised: BTreeMap::new(),
            lane_applied: BTreeMap::new(),
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
            "laneWaits": state.lane_waits.iter().map(|wait| lane_wait_value(&state, wait, now)).collect::<Vec<_>>(),
        })
    }

    pub fn subject(&self, subject: &str, now: u64) -> Option<Value> {
        let state = self.state();
        if let Some(wait) = state.lane_waits.iter().find(|wait| wait.subject == subject) {
            return Some(lane_wait_value(&state, wait, now));
        }
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

impl Status {
    /// The first seat in line for room, which a later one may not pass. A wait not tried for
    /// [`LANE_WAIT_STALE_SECS`] holds no place.
    pub fn lane_head(&self, now: u64) -> Option<String> {
        self.state().lane_waits.iter()
            .find(|wait| wait.queued && now.saturating_sub(wait.checked) <= LANE_WAIT_STALE_SECS)
            .map(|wait| wait.subject.clone())
    }

    /// Records why `subject` waits for room, keeping its place in line. True when the wait is new
    /// or moved in or out of line; a reason whose figures moved is served as it is, unannounced.
    pub fn lane_wait(&self, subject: &str, reason: String, queued: bool, now: u64) -> bool {
        let mut state = self.state();
        if let Some(wait) = state.lane_waits.iter_mut().find(|wait| wait.subject == subject) {
            let changed = wait.queued != queued;
            wait.reason = reason;
            wait.queued = queued;
            wait.checked = now;
            return changed;
        }
        state.lane_waits.push_back(LaneWait { subject: subject.into(), since: now, reason, queued, checked: now });
        true
    }

    /// `subject` was tried for a start and its lane fit, but something else held it: it keeps its
    /// place in line.
    pub fn lane_tried(&self, subject: &str, now: u64) {
        if let Some(wait) = self.state().lane_waits.iter_mut().find(|wait| wait.subject == subject) {
            wait.checked = now;
        }
    }

    /// `subject`'s lane fit and it starts: it leaves the line and holds its room until it is seen
    /// running.
    pub fn lane_admitted(&self, subject: &str, capacity: crate::Capacity, now: u64) {
        let mut state = self.state();
        state.lane_waits.retain(|wait| wait.subject != subject);
        state.lane_starts.insert(subject.into(), (capacity, now));
    }
}

fn lane_wait_value(state: &State, wait: &LaneWait, now: u64) -> Value {
    let position = state.lane_waits.iter().filter(|other| other.queued)
        .position(|other| other.subject == wait.subject);
    json!({"subject": wait.subject, "pending": true, "queuePosition": position.map(|position| position + 1),
        "queuedSince": wait.since, "pendingAgeSecs": now.saturating_sub(wait.since), "reason": wait.reason})
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
    fn a_seat_waiting_for_room_shows_its_reason_and_place_until_its_lane_is_admitted() {
        let status = Status::default();
        status.lane_wait("big", "lane 8 cores/23G does not fit".into(), true, 10);
        status.lane_wait("huge", "lane 64 cores/1T can never fit".into(), false, 11);
        status.lane_wait("small", "behind big".into(), true, 12);
        status.lane_wait("big", "lane 8 cores/23G still does not fit".into(), true, 15);
        assert_eq!(status.lane_head(20).as_deref(), Some("big"));
        let big = status.subject("big", 20).unwrap();
        assert_eq!((big["queuePosition"].as_u64(), big["pendingAgeSecs"].as_u64()), (Some(1), Some(10)));
        assert_eq!(big["reason"], "lane 8 cores/23G still does not fit");
        assert_eq!(status.subject("small", 20).unwrap()["queuePosition"], 2);
        assert!(status.subject("huge", 20).unwrap()["queuePosition"].is_null());
        assert_eq!(status.snapshot(20)["laneWaits"].as_array().unwrap().len(), 3);
        status.lane_admitted("big", crate::Capacity { cores: 8, memory: 23 << 30 }, 21);
        assert_eq!(status.lane_head(21).as_deref(), Some("small"));
        assert!(status.subject("big", 21).is_none());
        assert!(status.state().lane_starts.contains_key("big"));
        // A seat no longer tried stops holding its place.
        assert_eq!(status.lane_head(12 + LANE_WAIT_STALE_SECS + 1), None);
        assert!(!status.lane_wait("small", "behind big, 3.0G free".into(), true, 400));
        assert!(status.lane_wait("small", "can never fit".into(), false, 401));
        status.lane_tried("small", 900);
        assert_eq!(status.state().lane_waits.iter().find(|wait| wait.subject == "small").unwrap().checked, 900);
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
