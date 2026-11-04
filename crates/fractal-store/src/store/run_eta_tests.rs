use super::run_eta::{person_adjusted_duration, step_timestamps};
use serde_json::json;

#[test]
fn eta_person_holds_are_clipped_and_union_counted_without_removing_other_waits() {
    let mut holds = [(12, 18), (15, 22), (0, 11), (29, 40), (50, 60)];
    assert_eq!(person_adjusted_duration(10, 30, &mut holds), 8);
    assert_eq!(person_adjusted_duration(10, 30, &mut []), 20);
}

#[test]
fn eta_duration_boundaries_use_selected_attempt_for_workers_and_agentless_steps() {
    let event = |kind: &str, fields, at| (kind.to_owned(), json!({"fields": fields}), at);
    let events = [
        event("work.claimed", json!({"attempt":1}), 10),
        event("work.failed", json!({"attempt":1,"status":"failed"}), 20),
        event("step-run.retried", json!({"attempt":2}), 30),
        event("work.claimed", json!({"attempt":2}), 40),
        event(
            "work.progress",
            json!({"attempt":2,"clock":"paused","wait_until_unix_ms":80}),
            50,
        ),
        event("work.released", json!({"attempt":2}), 70),
        event("work.claimed", json!({"attempt":2}), 90),
        event("step-run.state", json!({"status":"completed"}), 100),
    ];
    let (start, end) = step_timestamps(&events, 2);
    assert_eq!((start, end), (Some(40), Some(100)));
    assert_eq!(
        person_adjusted_duration(start.unwrap(), end.unwrap(), &mut []),
        60
    );
    let automatic = [
        event(
            "step-run.state",
            json!({"attempt":1,"status":"working"}),
            200,
        ),
        event("step-run.state", json!({"status":"completed"}), 275),
    ];
    assert_eq!(step_timestamps(&automatic, 1), (Some(200), Some(275)));
}
