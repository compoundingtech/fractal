use super::*;

struct Clock;
impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

fn join_capture(reconciler: &Reconciler<FakeRuntime>) {
    let (live, changed) = &*reconciler.start_jobs.live;
    let mut live = live.lock().unwrap();
    while !live.is_empty() {
        live = changed.wait(live).unwrap();
    }
}

fn warnings(store: &Store, subject: &str) -> Vec<crate::model::ClaimRecord> {
    store.observations_for(subject, "harness.diagnostic").unwrap().into_iter()
        .filter(|claim| claim.body["fields"]["code"] == "environment-capture-timeout"
            && claim.body["fields"]["status"] == "warning")
        .collect()
}

#[test]
fn environment_capture_timeout_retries_then_starts_without_a_terminal_failure() {
    capture_attempts(1);
}

#[test]
fn environment_capture_repeated_timeouts_fail_only_after_three_attempts() {
    capture_attempts(3);
}

fn capture_attempts(timeouts: u32) {
    let _clock = Clock;
    let mut now = 1_800_000_000_000;
    smallclaims::store::set_thread_clock(Some(now));
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, "version 2\nagent \"worker\" { workspace \"/tmp\"; command \"true\"; }", "capture-retry");
    let subject = store.desired_subject_with_writer("agent/node.worker").unwrap().unwrap().0;
    let member = subject.member.as_ref().unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    runtime.capture_timeouts.lock().unwrap().0 = timeouts;
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
    reconciler.wait_for_test_starts.store(false, Ordering::Release);
    reconciler.perform_start(&subject, member, "capture retry fixture").unwrap();
    join_capture(&reconciler);
    let operation_id = store.runtime_start_for(&subject.subject).unwrap().unwrap().id;
    for attempt in 1..=timeouts.min(2) {
        reconciler.reconcile_start_jobs().unwrap();
        let pending = store.runtime_start_for(&subject.subject).unwrap().unwrap();
        assert_eq!(pending.id, operation_id, "retry retains the original guarded operation");
        assert!(!pending.finished);
        assert!(runtime.starts.lock().unwrap().is_empty());
        assert!(store.observations_for(&subject.subject, "runtime.action.failed").unwrap().is_empty());
        assert_eq!(warnings(&store, &subject.subject).len(), 1);
        let due = now + u128::from(attempt) * 2_000;
        smallclaims::store::set_thread_clock(Some(due - 1));
        reconciler.reconcile_start_jobs().unwrap();
        assert_eq!(runtime.capture_timeouts.lock().unwrap().1, attempt, "no capture before its deadline");
        smallclaims::store::set_thread_clock(Some(due));
        reconciler.reconcile_start_jobs().unwrap();
        join_capture(&reconciler);
        now = due;
    }
    reconciler.reconcile_start_jobs().unwrap();
    let operation = store.runtime_start_for(&subject.subject).unwrap().unwrap();
    assert!(operation.finished && operation.applied);
    let warnings = warnings(&store, &subject.subject);
    assert_eq!(warnings.len(), 1, "unchanged warnings are deduplicated across attempts and passes");
    assert_eq!(warnings[0].body["fields"]["severity"], "warning");
    assert_eq!(runtime.capture_timeouts.lock().unwrap().1, timeouts.min(2) + 1);
    if timeouts == 1 {
        assert_eq!(runtime.starts.lock().unwrap().len(), 1);
        assert!(store.observations_for(&subject.subject, "runtime.action.failed").unwrap().is_empty());
        assert!(store.latest_observation(&subject.subject, "runtime.action.succeeded").unwrap().is_some());
        assert!(store.runtime_start_failure(&subject.subject).unwrap().is_none());
        let diagnostic = store.observations_for(&subject.subject, "harness.diagnostic").unwrap()
            .into_iter().rev().find(|claim| claim.body["fields"]["code"] == "environment-capture-timeout").unwrap();
        assert_eq!(diagnostic.body["fields"]["status"], "resolved");
    } else {
        assert!(runtime.starts.lock().unwrap().is_empty());
        let reason = store.runtime_start_failure(&subject.subject).unwrap().unwrap();
        assert!(reason.contains("environment capture timed out after 3 attempts"), "{reason}");
        let failed = store.latest_observation(&subject.subject, "runtime.action.failed").unwrap().unwrap();
        assert!(failed.body["fields"]["reason"].as_str().unwrap().contains("timed out after 3 attempts"));
    }
}
