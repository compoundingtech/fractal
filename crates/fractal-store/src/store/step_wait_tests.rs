//! A step that waits for a build slot or the merge queue is not executing: a declared wait stops
//! its execution clock, and the clock starts again at the resume or at the wait's own deadline.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::step_wait_tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::step_wait_tests_shared_missions::{HOUR, MINUTE, Waiting, request};
use super::*;

const T: u128 = 1_000_000_000_000;
const LEASE: u128 = 10 * MINUTE;

fn event(kind: &str, at: u128, lease: Option<u128>, clock: Option<(&str, Option<u128>)>) -> TimingEvent {
    let mut fields = json!({"attempt": 1, "status": "working", "claimant": "agent/alder.worker"});
    if let Some(lease) = lease {
        fields["claim_expires_at_unix_ms"] = json!(lease as u64);
    }
    if let Some((clock, until)) = clock {
        fields["clock"] = json!(clock);
        if let Some(until) = until {
            fields["wait_until_unix_ms"] = json!(until as u64);
        }
    }
    (kind.to_owned(), json!({"fields": fields}), T + at)
}

type TimingEvent = (String, Value, u128);

fn claimed(at: u128) -> TimingEvent {
    event("work.claimed", at, Some(T + at + LEASE), None)
}

fn wait(at: u128, wait: u128) -> TimingEvent {
    event(
        "work.progress",
        at,
        Some(T + at + LEASE),
        Some(("paused", Some(T + at + wait))),
    )
}

fn resume(at: u128) -> TimingEvent {
    event("work.progress", at, Some(T + at + LEASE), Some(("running", None)))
}

/// The renewals a seat's driver publishes while it waits: one every five minutes.
fn renewals(from: u128, to: u128) -> Vec<TimingEvent> {
    (from..to)
        .step_by((5 * MINUTE) as usize)
        .map(|at| event("work.renewed", at, Some(T + at + LEASE), None))
        .collect()
}

fn timing(events: &[TimingEvent], at: u128) -> StepTiming {
    fold_step_clock(events, 1, T + at, true)
}

#[test]
fn two_hours_of_waiting_then_ten_minutes_of_review_use_twelve_minutes() {
    // Claimed, two minutes of preparation, a two hour wait, ten minutes of review.
    let mut events = vec![claimed(0), wait(2 * MINUTE, 4 * 60 * MINUTE)];
    events.extend(renewals(7 * MINUTE, 122 * MINUTE));
    events.push(resume(122 * MINUTE));
    events.extend(renewals(127 * MINUTE, 132 * MINUTE));
    events.push(event("work.submitted", 132 * MINUTE, None, None));

    let after = timing(&events, 133 * MINUTE);
    assert_eq!(after.started, None);
    assert_eq!(after.paused_until, None);
    assert_eq!(after.elapsed, 12 * MINUTE);
}

#[test]
fn the_clock_is_stopped_for_the_whole_wait_and_the_view_names_when_it_ends() {
    let mut events = vec![claimed(0), wait(2 * MINUTE, 60 * MINUTE)];
    events.extend(renewals(7 * MINUTE, 90 * MINUTE));

    for minutes in [3, 40, 61] {
        assert_eq!(
            timing(&events, minutes * MINUTE),
            StepTiming {
                started: None,
                elapsed: 2 * MINUTE,
                paused_until: Some(T + 62 * MINUTE),
                waited: (minutes - 2) * MINUTE,
            },
            "{minutes} minutes in"
        );
    }
}

#[test]
fn a_wait_nobody_resumes_ends_at_its_deadline() {
    // The seat stopped answering: the wait cannot hide a hung step.
    let mut events = vec![claimed(0), wait(2 * MINUTE, 60 * MINUTE)];
    events.extend(renewals(7 * MINUTE, 120 * MINUTE));

    let at = timing(&events, 90 * MINUTE);
    assert_eq!(at.started, Some(T + 62 * MINUTE));
    assert_eq!(at.paused_until, None);
    assert_eq!(at.elapsed, 2 * MINUTE + 28 * MINUTE);
}

#[test]
fn progress_and_renewals_during_a_wait_leave_the_clock_stopped() {
    let mut events = vec![claimed(0), wait(MINUTE, 60 * MINUTE)];
    events.extend(renewals(6 * MINUTE, 30 * MINUTE));
    events.push(event("work.progress", 30 * MINUTE, Some(T + 40 * MINUTE), None));
    events.push(event("work.extended", 31 * MINUTE, Some(T + 41 * MINUTE), None));

    let at = timing(&events, 35 * MINUTE);
    assert_eq!(at.started, None);
    assert_eq!(at.elapsed, MINUTE);
    assert_eq!(at.paused_until, Some(T + 61 * MINUTE));
}

#[test]
fn resuming_a_clock_that_is_running_changes_nothing() {
    let events = vec![claimed(0), resume(3 * MINUTE)];
    let at = timing(&events, 5 * MINUTE);
    assert_eq!(at.started, Some(T));
    assert_eq!(at.elapsed, 5 * MINUTE);
}

#[test]
fn a_new_claim_ends_the_previous_holders_wait() {
    // The first holder waited and lost its lease; another seat claims the step at 30 minutes.
    let events = vec![
        claimed(0),
        wait(MINUTE, 120 * MINUTE),
        event("work.released", 20 * MINUTE, None, None),
        claimed(30 * MINUTE),
    ];
    let at = timing(&events, 38 * MINUTE);
    assert_eq!(at.started, Some(T + 30 * MINUTE));
    assert_eq!(at.paused_until, None);
    assert_eq!(at.elapsed, MINUTE + 8 * MINUTE);
}

#[test]
fn a_wait_declared_after_the_lease_lapsed_adds_nothing() {
    // The lease ran out four minutes in; only the time the seat held it counts.
    let events = vec![
        claimed(0),
        event("work.progress", 2 * MINUTE, Some(T + 4 * MINUTE), None),
        wait(8 * MINUTE, 30 * MINUTE),
    ];
    let at = timing(&events, 20 * MINUTE);
    assert_eq!(at.elapsed, 4 * MINUTE);
    assert_eq!(at.started, None);
    assert_eq!(at.paused_until, None);
}

#[test]
fn the_budget_is_time_spent_waiting_not_time_declared() {
    // Ten hours declared, resumed after one: one hour is spent.
    let events = vec![claimed(0), wait(MINUTE, 10 * HOUR), resume(MINUTE + HOUR)];
    assert_eq!(timing(&events, 2 * HOUR).waited, HOUR);
    // The budget does not grow after the resume.
    assert_eq!(timing(&events, 20 * HOUR).waited, HOUR);
}

#[test]
fn a_wait_that_ends_at_its_deadline_counts_its_full_length() {
    let mut events = vec![claimed(0), wait(MINUTE, HOUR)];
    events.extend(renewals(6 * MINUTE, 3 * HOUR));
    let timing = timing(&events, 3 * HOUR);
    assert_eq!(timing.waited, HOUR);
    assert_eq!(timing.paused_until, None);
}

#[test]
fn a_wait_still_running_counts_up_to_now() {
    let mut events = vec![claimed(0), wait(MINUTE, 10 * HOUR)];
    events.extend(renewals(6 * MINUTE, 3 * HOUR));
    assert_eq!(timing(&events, 3 * HOUR).waited, 3 * HOUR - MINUTE);
}

#[test]
fn chained_waits_draw_on_one_budget() {
    // Declaring again while a wait runs spends the time so far and starts the next span.
    let mut events = vec![claimed(0), wait(MINUTE, 6 * HOUR)];
    events.extend(renewals(6 * MINUTE, 5 * HOUR));
    events.push(wait(5 * HOUR, 6 * HOUR));
    events.extend(renewals(5 * HOUR + 5 * MINUTE, 9 * HOUR));
    events.push(wait(9 * HOUR, 6 * HOUR));
    // Declared time is 18 hours; the time spent is what the clock has been stopped for.
    assert_eq!(timing(&events, 10 * HOUR).waited, 10 * HOUR - MINUTE);
}

#[test]
fn another_holder_in_the_same_attempt_keeps_the_spent_budget_and_a_new_attempt_starts_fresh() {
    let mut events = vec![claimed(0), wait(MINUTE, 6 * HOUR)];
    events.extend(renewals(6 * MINUTE, 2 * HOUR));
    events.push(event("work.released", 2 * HOUR, None, None));
    events.push(claimed(2 * HOUR + MINUTE));
    // The wait ended at the release; what it spent stays spent for the attempt.
    assert_eq!(timing(&events, 3 * HOUR).waited, 2 * HOUR - MINUTE);
    // A later attempt folds none of the earlier attempt's events.
    assert_eq!(fold_step_clock(&events, 2, T + 3 * HOUR, true).waited, 0);
}

