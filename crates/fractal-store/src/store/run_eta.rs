//! Selected-attempt wall-time boundaries and person-held duration accounting.
use super::*;

/// Overlapping person holds count once; queueing, released intervals and agent waits remain.
pub fn person_adjusted_duration(start: u128, end: u128, holds: &mut [(u128, u128)]) -> u128 {
    holds.sort_unstable();
    let mut through = start;
    let mut held = 0;
    for &(low, high) in holds.iter() {
        let low = low.max(start).max(through);
        let high = high.min(end);
        held += high.saturating_sub(low);
        through = through.max(high);
    }
    end.saturating_sub(start).saturating_sub(held)
}

/// First execution opening and authoritative terminal edge, independent of lease intervals.
pub fn step_timestamps(
    events: &[(String, Value, u128)],
    attempt: u32,
) -> (Option<u128>, Option<u128>) {
    let (mut started, mut completed) = (None, None);
    let mut current_attempt = 1;
    for (kind, body, at) in events {
        let fields = body.get("fields").unwrap_or(body);
        if matches!(
            kind.as_str(),
            "step-run.state" | "step-run.carried" | "step-run.retried"
        ) && let Some(value) = fields["attempt"].as_u64()
        {
            current_attempt = value;
        }
        if fields["attempt"].as_u64().unwrap_or(current_attempt) != u64::from(attempt) {
            continue;
        }
        let status = fields["status"].as_str().unwrap_or_default();
        if kind == "work.claimed"
            || (matches!(kind.as_str(), "step-run.state" | "step-run.carried")
                && matches!(status, "claimed" | "working"))
        {
            started.get_or_insert(*at);
        }
        if matches!(
            kind.as_str(),
            "step-run.state" | "work.failed" | "work.person-done" | "work.person-cancelled"
        ) && matches!(status, "completed" | "failed" | "cancelled" | "skipped")
        {
            completed.get_or_insert(*at);
        }
    }
    (started, completed)
}
