#![allow(unused_imports)]
use super::*;
use st3_delivery::store::step_wait_tests_shared_missions::*;
use st3_delivery::store::step_wait_tests_shared_missions::{HOUR, MINUTE, Waiting, request};


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

#[test]
fn chained_waits_stop_being_accepted_at_the_budget() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            smallclaims::store::set_thread_clock(None);
        }
    }
    let _reset = Reset;
    let Waiting { store, subject, .. } = claimed_step();
    let start = now_ms();
    let at = |moment: u128| {
        smallclaims::store::set_thread_clock(Some(moment));
        store.set_write_clock_at(moment).unwrap();
    };
    let live_until = |until: u128| {
        // The seat's driver renews while it waits.
        let mut moment = now_ms();
        while moment < until {
            moment = (moment + 9 * MINUTE).min(until);
            at(moment);
            store
                .work_action(&subject, "renew", &request(&format!("renew-{moment}"), None, None))
                .unwrap();
        }
    };

    at(start + MINUTE);
    store
        .work_action(&subject, "wait", &request("wait-a", Some("build slot"), Some(MAX_STEP_WAIT_MS)))
        .unwrap();
    live_until(start + 8 * HOUR);

    // Eight hours are spent. A second declaration for the full cap is refused, and says what is left.
    let refused = store
        .work_action(&subject, "wait", &request("wait-b", Some("build slot"), Some(MAX_STEP_WAIT_MS)))
        .unwrap_err();
    assert_eq!(refused.code, "wait-budget-exhausted");
    assert!(
        refused.message.contains("7h59m") || refused.message.contains("8h"),
        "{}",
        refused.message
    );
    assert!(refused.message.contains("12h"), "{}", refused.message);

    // What is left can still be declared.
    let left = MAX_STEP_WAIT_MS - u64::try_from(8 * HOUR - MINUTE).unwrap();
    store
        .work_action(&subject, "wait", &request("wait-c", Some("build slot"), Some(left)))
        .unwrap();

    // Once it is all spent, a further declaration is refused with nothing left.
    live_until(start + 13 * HOUR);
    let spent = store
        .work_action(&subject, "wait", &request("wait-d", Some("build slot"), Some(1_000)))
        .unwrap_err();
    assert_eq!(spent.code, "wait-budget-exhausted");
    assert!(spent.message.contains("12h of its 12h"), "{}", spent.message);
    assert!(spent.message.contains("0m is left"), "{}", spent.message);
}

#[test]
fn a_resumed_wait_gives_back_what_it_did_not_use() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            smallclaims::store::set_thread_clock(None);
        }
    }
    let _reset = Reset;
    let Waiting { store, subject, .. } = claimed_step();
    let start = now_ms();
    let at = |moment: u128| {
        smallclaims::store::set_thread_clock(Some(moment));
        store.set_write_clock_at(moment).unwrap();
    };
    at(start + MINUTE);
    store
        .work_action(&subject, "wait", &request("wait-a", Some("build slot"), Some(MAX_STEP_WAIT_MS)))
        .unwrap();
    at(start + 6 * MINUTE);
    store
        .work_action(&subject, "resume", &request("resume-a", None, None))
        .unwrap();
    // Five minutes were spent, so nearly the whole cap is still available.
    at(start + 7 * MINUTE);
    store
        .work_action(
            &subject,
            "wait",
            &request("wait-b", Some("merge queue"), Some(MAX_STEP_WAIT_MS - 6 * 60_000)),
        )
        .unwrap();
}
