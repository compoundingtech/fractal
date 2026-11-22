//! Fold native execution intervals without charging dispatch, admission or result delivery.
//! Attempt-scoped terminal events settle missing exits without charging later terminal idle time.
use super::StepTiming;
use fractal_kernel::step_execution::execution_wait;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn fold(events: &[(String, Value, u128)], attempt: u32, snapshot: u128,
    active: bool) -> Option<StepTiming> {
    let mut execs = BTreeMap::<&str, (Option<u128>, Option<u128>)>::new();
    let mut closures = Vec::new();
    let mut observed_attempt = None;
    for (kind, body, accepted) in events {
        if *accepted > snapshot { continue; }
        let fields = body.get("fields").unwrap_or(body);
        let event_attempt = fields["attempt"].as_u64();
        if let Some(event_attempt) = event_attempt {
            observed_attempt = Some(observed_attempt.map_or(event_attempt,
                |previous: u64| previous.max(event_attempt)));
        }
        let closes = match kind.as_str() {
            "work.failed" | "work.submitted" | "work.released" => true,
            "step-run.state" => matches!(fields["status"].as_str(),
                Some("completed" | "failed" | "cancelled")),
            _ => false,
        };
        // State projections may omit the attempt. Scope those to the latest observed
        // attempt so a previous attempt's terminal state cannot close a later execution.
        let closing_attempt = if kind == "step-run.state" {
            event_attempt.or(observed_attempt)
        } else {
            event_attempt
        };
        if closes && closing_attempt == Some(u64::from(attempt)) {
            closures.push(*accepted);
        }
        if kind != "step.execution-phase"
            || event_attempt != Some(u64::from(attempt)) { continue; }
        let (Some(exec), Some(at)) = (fields["exec"].as_str(), fields["at_unix_ms"].as_u64()) else { continue; };
        let at = u128::from(at);
        if at > snapshot { continue; }
        let interval = execs.entry(exec).or_default();
        let target = match fields["phase"].as_str() {
            Some("exec-started") => &mut interval.0,
            Some("exec-exited") => &mut interval.1,
            _ => continue,
        };
        *target = Some(target.map_or(at, |previous| previous.min(at)));
    }
    if execs.is_empty() { return None; }
    closures.sort_unstable();
    let closing_boundary = |start: u128| {
        closures.get(closures.partition_point(|at| *at < start)).copied()
    };
    let (_, paused_until) = execution_wait(events.iter().map(|(kind, body, at)|
        (kind.as_str(), body, *at)), attempt, 0, snapshot);
    let started = if active && paused_until.is_none() { execs.values().filter_map(|(start, end)|
        start.filter(|start| end.is_none() && closing_boundary(*start).is_none())).min() } else { None };
    let mut intervals = execs.values().filter_map(|(start, end)| {
        let start = (*start)?;
        let terminal = closing_boundary(start);
        let end = match (*end, terminal) {
            (Some(end), Some(terminal)) => Some(end.min(terminal)),
            (end, terminal) => end.or(terminal),
        }.or_else(|| active.then_some(snapshot))?;
        Some((start, end.max(start)))
    }).collect::<Vec<_>>();
    intervals.sort_unstable();
    let mut elapsed: u128 = 0;
    let mut waited = 0_u128;
    let mut through = 0;
    for (start, end) in intervals {
        let start = start.max(through);
        let (interval_waited, _) = execution_wait(events.iter().map(|(kind, body, at)|
            (kind.as_str(), body, *at)), attempt, start, end);
        waited = waited.saturating_add(interval_waited);
        elapsed = elapsed.saturating_add(end.saturating_sub(start).saturating_sub(interval_waited));
        through = through.max(end);
    }
    Some(StepTiming { started, elapsed, paused_until: paused_until.filter(|_| active), waited })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(exec: &str, phase: &str, at: u64, accepted: u128) -> (String, Value, u128) {
        ("step.execution-phase".into(), json!({"fields": {
            "attempt": 1, "exec": exec, "phase": phase, "at_unix_ms": at,
        }}), accepted)
    }

    #[test]
    fn native_clock_excludes_dispatch_and_result_replication() {
        let mut events = vec![event("exec/a", "dispatched", 100, 100)];
        assert_eq!(fold(&events, 1, 10_000, true).unwrap().elapsed, 0);
        events.push(event("exec/a", "exec-started", 10_000, 11_000));
        events.push(event("exec/a", "exec-exited", 10_015, 12_000));
        let timing = fold(&events, 1, 20_000, true).unwrap();
        assert_eq!(timing.elapsed, 15);
        assert_eq!(timing.started, None);
        assert!(fold(&events, 2, 20_000, true).is_none());
    }

    #[test]
    fn parallel_execs_count_overlapping_execution_once_but_not_admission_gaps() {
        let events = vec![event("exec/a", "exec-started", 100, 100),
            event("exec/a", "exec-exited", 200, 200),
            event("exec/b", "exec-started", 150, 150),
            event("exec/b", "exec-exited", 250, 250),
            event("exec/c", "exec-started", 400, 400),
            event("exec/c", "exec-exited", 410, 410)];
        assert_eq!(fold(&events, 1, 1_000, true).unwrap().elapsed, 160);
    }

    fn phase(attempt: u32, at: u64, accepted: u128) -> (String, Value, u128) {
        ("step.execution-phase".into(), json!({"fields": {
            "attempt": attempt, "exec": format!("exec/{attempt}"),
            "phase": "exec-started", "at_unix_ms": at,
        }}), accepted)
    }

    #[test]
    fn failed_open_execution_retains_elapsed_without_terminal_idle() {
        for kind in ["work.failed", "work.submitted", "work.released"] {
            let events = vec![
                phase(1, 100, 110),
                (kind.into(), json!({"fields": {"attempt": 1}}), 150),
            ];
            for snapshot in [150, 10_000] {
                for active in [false, true] {
                    let timing = fold(&events, 1, snapshot, active).unwrap();
                    assert_eq!(timing.elapsed, 50, "{kind}");
                    assert_eq!(timing.started, None, "{kind}");
                }
            }
        }
    }

    #[test]
    fn cancelled_open_execution_closes_at_attemptless_state_boundary() {
        for status in ["cancelled", "failed", "completed"] {
            let events = vec![
                phase(1, 100, 110),
                ("step-run.state".into(), json!({"fields": {"status": status}}), 150),
            ];
            let before = fold(&events, 1, 149, true).unwrap();
            assert_eq!(before.elapsed, 49);
            assert_eq!(before.started, Some(100));
            for snapshot in [150, 10_000] {
                let timing = fold(&events, 1, snapshot, false).unwrap();
                assert_eq!(timing.elapsed, 50, "{status}");
                assert_eq!(timing.started, None, "{status}");
            }
        }
    }

    #[test]
    fn stale_attempt_boundaries_do_not_close_current_execution() {
        let mut events = vec![
            phase(1, 100, 110),
            ("step-run.state".into(), json!({"fields": {"status": "failed"}}), 150),
            phase(2, 200, 210),
            ("work.failed".into(), json!({"fields": {"attempt": 1}}), 250),
            ("step-run.state".into(),
                json!({"fields": {"attempt": 1, "status": "cancelled"}}), 260),
        ];
        let timing = fold(&events, 2, 300, true).unwrap();
        assert_eq!(timing.elapsed, 100);
        assert_eq!(timing.started, Some(200));
        events.push(("step-run.state".into(),
            json!({"fields": {"status": "cancelled"}}), 350));
        let timing = fold(&events, 2, 10_000, false).unwrap();
        assert_eq!(timing.elapsed, 150);
        assert_eq!(timing.started, None);
        assert_eq!(fold(&events, 1, 10_000, false).unwrap().elapsed, 50);
    }

    #[test]
    fn delayed_start_delivery_uses_earlier_attempt_scoped_terminal_boundary() {
        let events = vec![
            ("work.failed".into(), json!({"fields": {"attempt": 1}}), 150),
            phase(1, 100, 200),
        ];
        let timing = fold(&events, 1, 10_000, false).unwrap();
        assert_eq!(timing.elapsed, 50);
        assert_eq!(timing.started, None);
    }
}

#[cfg(test)]
mod wait_tests {
    use super::*;
    use serde_json::json;

    fn progress(clock: &str, at: u128, deadline: u64) -> (String, Value, u128) {
        ("work.progress".into(), json!({"fields": {
            "attempt": 1, "clock": clock, "wait_until_unix_ms": deadline
        }}), at)
    }

    #[test]
    fn native_waits_pause_live_and_completed_execution_without_charging_dispatch() {
        let mut events = vec![
            ("step.execution-phase".into(), json!({"fields": {
                "attempt": 1, "exec": "exec/test", "phase": "exec-started", "at_unix_ms": 100
            }}), 100),
            progress("paused", 200, 2_000),
        ];
        let live = fold(&events, 1, 1_500, true).unwrap();
        assert_eq!((live.elapsed, live.waited, live.started, live.paused_until),
            (100, 1_300, None, Some(2_000)));
        events.push(progress("running", 1_700, 0));
        events.push(("step.execution-phase".into(), json!({"fields": {
            "attempt": 1, "exec": "exec/test", "phase": "exec-exited", "at_unix_ms": 1_900
        }}), 1_950));
        let completed = fold(&events, 1, 5_000, false).unwrap();
        assert_eq!((completed.elapsed, completed.waited), (300, 1_500));
    }

    #[test]
    fn missing_resume_restarts_clock_at_bounded_wait_deadline() {
        let events = vec![progress("paused", 50, 500)];
        let fold_wait = |start, end| execution_wait(events.iter().map(|(kind, body, at)|
            (kind.as_str(), body, *at)), 1, start, end);
        assert_eq!(fold_wait(100, 400), (300, Some(500)));
        assert_eq!(fold_wait(100, 1_000), (400, None));
    }
}
