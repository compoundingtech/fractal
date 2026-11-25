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
    // Inspect the worker's returned result before adoption removes settled starts
    // from the pending-only runtime_start_for lookup.
    let operation = store.runtime_start_for(&subject.subject).unwrap().unwrap();
    assert!(operation.finished && !operation.applied);
    if timeouts == 3 {
        assert!(operation.result.as_ref().unwrap()["error"].as_str().unwrap()
            .contains("environment capture timed out after 3 attempts"));
    }
    reconciler.reconcile_start_jobs().unwrap();
    assert!(store.runtime_start_for(&subject.subject).unwrap().is_none(),
        "adoption must retire the completed start from the pending queue");
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

#[test]
fn cancelling_capture_backoff_releases_lease_and_admits_replacement() {
    let _clock = Clock;
    let now = 1_800_000_000_000;
    smallclaims::store::set_thread_clock(Some(now));
    let root = crate::test_support::canonical_tempdir();
    let repository = crate::checkout::test_support::repository(root.path());
    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = |command: &str| format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; command {command:?}; }}",
        repository.display().to_string(),
    );
    apply_source(&store, &source("true"), "cancel-capture");
    let subject = store.desired_subject_with_writer("agent/node.worker").unwrap().unwrap().0;
    let runtime = Arc::new(FakeRuntime::default());
    runtime.capture_timeouts.lock().unwrap().0 = 1;
    // Match native backends: a producer that never started has no owner file or
    // exited generation from which to reconstruct a positive closure proof.
    runtime.start_closure_unavailable.store(true, Ordering::Release);
    let reconciler = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    reconciler.wait_for_test_starts.store(false, Ordering::Release);
    reconciler.perform_start(&subject, subject.member.as_ref().unwrap(), "cancel capture fixture").unwrap();
    join_capture(&reconciler);
    reconciler.reconcile_start_jobs().unwrap();
    let cancelled = store.runtime_start_for(&subject.subject).unwrap().unwrap();
    assert!(!cancelled.finished);
    assert!(crate::checkout::activation_lease(&repository, &repository).is_err());
    smallclaims::store::set_thread_clock(Some(now + 1_000));
    reconciler.revoke_start(&subject.subject).unwrap();
    reconciler.reconcile_start_jobs().unwrap();
    assert!(store.runtime_start_for(&subject.subject).unwrap().is_none());
    assert_eq!(store.unresolved_runtime_start_counts().unwrap(), (0, 0));
    let closed: bool = store.readers.get().query_row(
        "SELECT cleanup_verified=1 AND json_extract(result,'$.closure_proof.witness.kind')='before-backend' FROM local_runtime_starts WHERE id=?1",
        [&cancelled.id], |row| row.get(0),
    ).unwrap();
    assert!(closed, "cancellation must retain the known non-launch witness");
    assert!(crate::checkout::activation_lease(&repository, &repository).is_ok(),
        "cancelled retry retained its workspace lease");
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert_eq!(runtime.capture_timeouts.lock().unwrap().1, 1);
    apply_source(&store, &source("printf replacement"), "replacement-after-capture");
    let replacement = store.desired_subject_with_writer(&subject.subject).unwrap().unwrap().0;
    assert!(reconciler.perform_start_for_request(
        &replacement, replacement.member.as_ref().unwrap(), "replacement capture fixture", None,
    ).unwrap(), "replacement must be admitted, not held SeatBusy");
    join_capture(&reconciler);
    reconciler.reconcile_start_jobs().unwrap();
    assert_eq!(runtime.starts.lock().unwrap().len(), 1);
    assert!(store.latest_observation(&subject.subject, "runtime.action.succeeded").unwrap().is_some());
}
