//! Fold native execution intervals without charging dispatch, admission or result delivery.
use super::StepTiming;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn fold(events: &[(String, Value, u128)], attempt: u32, snapshot: u128,
    active: bool) -> Option<StepTiming> {
    let mut execs = BTreeMap::<&str, (Option<u128>, Option<u128>)>::new();
    for (kind, body, accepted) in events {
        if kind != "step.execution-phase" || *accepted > snapshot { continue; }
        let fields = body.get("fields").unwrap_or(body);
        if fields["attempt"].as_u64() != Some(u64::from(attempt)) { continue; }
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
    let started = if active { execs.values().filter_map(|(start, end)|
        end.is_none().then_some(*start).flatten()).min() } else { None };
    let mut intervals = execs.values().filter_map(|(start, end)| {
        let start = (*start)?;
        let end = end.or_else(|| active.then_some(snapshot))?;
        Some((start, end.max(start)))
    }).collect::<Vec<_>>();
    intervals.sort_unstable();
    let mut elapsed: u128 = 0;
    let mut through = 0;
    for (start, end) in intervals {
        elapsed = elapsed.saturating_add(end.saturating_sub(start.max(through)));
        through = through.max(end);
    }
    Some(StepTiming { started, elapsed, paused_until: None, waited: 0 })
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
}
