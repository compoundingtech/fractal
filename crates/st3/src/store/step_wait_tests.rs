//! A step that waits for a build slot or the merge queue is not executing: a declared wait stops
//! its execution clock, and the clock starts again at the resume or at the wait's own deadline.
use super::*;

const T: u128 = 1_000_000_000_000;
const MINUTE: u128 = 60_000;
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

struct Waiting {
    _directory: tempfile::TempDir,
    store: Store,
    subject: String,
}

fn request(key: &str, reason: Option<&str>, wait_ms: Option<u64>) -> WorkRequest {
    WorkRequest {
        wait_ms,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: reason.map(Into::into),
        evidence: Vec::new(),
        idempotency_key: key.into(),
    }
}

fn claimed_step() -> Waiting {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("wait.sqlite3"), "node").unwrap();
    let source = "version 2\nmission \"waiting\" state=\"ready\" { goal \"Review.\"; step \"review\" timeout=\"30m\" { assigned-to \"agent/node.worker\" } }";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store.apply(&intent, &plan.subject_tokens, "waiting-mission").unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "waiting".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "waiting-run".into(),
        })
        .unwrap();
    let subject = run.steps[0].subject.clone();
    store.set_step_state(&subject, "ready", None).unwrap();
    store
        .work_action(&subject, "claim", &request("waiting-claim", None, None))
        .unwrap();
    Waiting {
        _directory: directory,
        store,
        subject,
    }
}

#[test]
fn a_wait_stops_the_clock_a_resume_starts_it_and_the_deadline_ends_a_forgotten_wait() {
    let Waiting { store, subject, .. } = claimed_step();
    std::thread::sleep(std::time::Duration::from_millis(5));

    let waiting = store
        .work_action(&subject, "wait", &request("wait-one", Some("build slot"), Some(60_000)))
        .unwrap();
    assert_eq!(waiting.status, "working");
    assert_eq!(waiting.blocked_reason.as_deref(), Some("build slot"));
    assert_eq!(waiting.execution_started_at_unix_ms, None);
    assert!(waiting.execution_paused_until_unix_ms.is_some());
    let used = waiting.execution_elapsed_ms;
    assert!(used > 0);

    std::thread::sleep(std::time::Duration::from_millis(20));
    let still = store.step_run(&subject).unwrap().unwrap();
    assert_eq!(still.execution_elapsed_ms, used);
    assert_eq!(still.execution_started_at_unix_ms, None);

    let resumed = store
        .work_action(&subject, "resume", &request("resume-one", None, None))
        .unwrap();
    assert_eq!(resumed.blocked_reason, None);
    assert!(resumed.execution_started_at_unix_ms.is_some());
    assert_eq!(resumed.execution_paused_until_unix_ms, None);
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(store.step_run(&subject).unwrap().unwrap().execution_elapsed_ms > used);

    // A wait that is never resumed ends by itself.
    store
        .work_action(&subject, "wait", &request("wait-two", Some("merge queue"), Some(30)))
        .unwrap();
    let paused = store.step_run(&subject).unwrap().unwrap();
    let before = paused.execution_elapsed_ms;
    std::thread::sleep(std::time::Duration::from_millis(80));
    let ended = store.step_run(&subject).unwrap().unwrap();
    assert!(ended.execution_started_at_unix_ms.is_some());
    assert_eq!(ended.execution_paused_until_unix_ms, None);
    assert!(ended.execution_elapsed_ms > before + 20);
}

#[test]
fn a_wait_needs_a_bounded_duration_and_a_reason_and_only_a_wait_takes_one() {
    let Waiting { store, subject, .. } = claimed_step();
    let code = |result: Result<StepRunView, St3Error>| result.unwrap_err().code;
    assert_eq!(
        code(store.work_action(&subject, "wait", &request("no-duration", Some("build slot"), None))),
        "invalid-wait"
    );
    assert_eq!(
        code(store.work_action(&subject, "wait", &request("zero", Some("build slot"), Some(0)))),
        "invalid-wait"
    );
    assert_eq!(
        code(store.work_action(
            &subject,
            "wait",
            &request("too-long", Some("build slot"), Some(MAX_STEP_WAIT_MS + 1)),
        )),
        "invalid-wait"
    );
    assert_eq!(
        code(store.work_action(&subject, "wait", &request("no-reason", Some("  "), Some(1_000)))),
        "wait-needs-reason"
    );
    assert_eq!(
        code(store.work_action(&subject, "progress", &request("progress-wait", None, Some(1_000)))),
        "invalid-work-action"
    );
    let untouched = store.step_run(&subject).unwrap().unwrap();
    assert!(untouched.execution_started_at_unix_ms.is_some());
}

#[test]
fn only_the_holder_of_a_live_lease_can_wait() {
    let Waiting { store, subject, .. } = claimed_step();
    store
        .work_action(&subject, "release", &request("release", None, None))
        .unwrap();
    let error = store
        .work_action(&subject, "wait", &request("late-wait", Some("build slot"), Some(1_000)))
        .unwrap_err();
    assert_eq!(error.code, "work-not-claimed");
}
