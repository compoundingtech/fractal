use super::*;

const AGENT: &str = "agent/node.worker";
const NOW: u128 = 1_800_000_000_000;

struct Clock;
impl Clock {
    fn new() -> Self {
        smallclaims::store::set_thread_clock(Some(NOW));
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

struct Fixture {
    root: tempfile::TempDir,
    repository: PathBuf,
    store: Arc<Store>,
    runtime: Arc<FakeRuntime>,
    reconciler: Reconciler<FakeRuntime>,
    subject: DesiredSubject,
}

fn join_worker(reconciler: &Reconciler<FakeRuntime>) {
    let (live, changed) = &*reconciler.start_jobs.live;
    let mut live = live.lock().unwrap();
    while !live.is_empty() {
        live = changed.wait(live).unwrap();
    }
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_support::canonical_tempdir();
        let repository = crate::checkout::test_support::repository(root.path());
        let store = Arc::new(Store::open(&root.path().join("claims.db"), "node").unwrap());
        apply_source(&store, &format!(
            "version 2\nagent \"worker\" {{ workspace {:?}; command \"true\"; }}",
            repository.display().to_string(),
        ), "entry-fence-fixture");
        let subject = store.desired_subject_with_writer(AGENT).unwrap().unwrap().0;
        let runtime = Arc::new(FakeRuntime::default());
        // Absence of a native owner/generation must not rescue an unknown launch.
        runtime.start_closure_unavailable.store(true, Ordering::Release);
        let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
        reconciler.wait_for_test_starts.store(false, Ordering::Release);
        Self { root, repository, store, runtime, reconciler, subject }
    }

    fn start(&self) {
        self.reconciler.perform_start(&self.subject, self.subject.member.as_ref().unwrap(), "entry fence fixture").unwrap();
        join_worker(&self.reconciler);
    }

    fn interrupted_capture(&self) -> crate::store::RuntimeStartOperation {
        self.store.set_launch_freeze("person/operator", "entry-fence", "queue fixture", 60_000, 0).unwrap();
        self.start();
        let queued = self.store.runtime_start_for(AGENT).unwrap().unwrap();
        assert!(!queued.dispatched);
        self.store.clear_launch_freeze("person/operator", "entry-fence").unwrap();
        // Reproduce the durable state at process loss after dispatch and before capture
        // completes. No worker survives, and no backend has been invoked.
        assert!(self.store.dispatch_runtime_start(&queued.id, start_jobs::MAX_START_JOBS).unwrap());
        let dispatched = self.store.runtime_start_for(AGENT).unwrap().unwrap();
        assert!(dispatched.before_backend());
        let _: start_jobs::PreparedStart = serde_json::from_value(dispatched.body.clone()).unwrap();
        dispatched
    }

    fn restarted(self) -> Self {
        let Self { root, repository, store, runtime, reconciler, subject } = self;
        drop(reconciler);
        drop(store);
        let store = Arc::new(Store::open(&root.path().join("claims.db"), "node").unwrap());
        let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
        reconciler.wait_for_test_starts.store(false, Ordering::Release);
        Self { root, repository, store, runtime, reconciler, subject }
    }

    fn recover_and_replace(&self) {
        self.reconciler.reconcile_start_jobs().unwrap();
        let closed = self.store.runtime_start_for(AGENT).unwrap().unwrap();
        assert!(closed.finished && closed.cleanup_verified);
        assert_eq!(closed.result.as_ref().unwrap()["closure_proof"]["witness"]["kind"], "before-backend");
        self.reconciler.reconcile_start_jobs().unwrap();
        assert!(self.store.runtime_start_for(AGENT).unwrap().is_none());
        assert!(crate::checkout::activation_lease(&self.repository, &self.repository).is_ok());
        assert_eq!(self.store.unresolved_runtime_start_counts().unwrap(), (0, 0));
        assert!(self.reconciler.perform_start_for_request(
            &self.subject, self.subject.member.as_ref().unwrap(), "replacement after restart", None,
        ).unwrap(), "replacement must not remain SeatBusy");
        join_worker(&self.reconciler);
        self.reconciler.reconcile_start_jobs().unwrap();
        assert_eq!(self.runtime.starts.lock().unwrap().len(), 1);
        assert!(self.store.latest_observation(AGENT, "runtime.action.succeeded").unwrap().is_some());
    }
}

#[test]
fn restart_during_capture_closes_before_backend_and_admits_replacement() {
    let _clock = Clock::new();
    let fixture = Fixture::new();
    let operation = fixture.interrupted_capture();
    assert!(fixture.runtime.starts.lock().unwrap().is_empty());
    // The proof must bind both identities and recognize only this protocol.
    for field in ["owner", "operation", "version"] {
        let mut mismatched = operation.clone();
        mismatched.body["backend_entry"][field] = Value::from("other");
        assert!(!mismatched.before_backend());
    }
    fixture.restarted().recover_and_replace();
}

#[test]
fn restart_during_capture_backoff_closes_and_admits_replacement() {
    let _clock = Clock::new();
    let fixture = Fixture::new();
    fixture.runtime.capture_timeouts.lock().unwrap().0 = 1;
    fixture.start();
    fixture.reconciler.reconcile_start_jobs().unwrap();
    let operation = fixture.store.runtime_start_for(AGENT).unwrap().unwrap();
    assert!(!operation.finished && operation.before_backend());
    smallclaims::store::set_thread_clock(Some(NOW + 1_000));
    fixture.restarted().recover_and_replace();
}

#[test]
fn entered_and_legacy_operations_keep_conservative_restart_recovery() {
    let _clock = Clock::new();
    for legacy in [false, true] {
        let fixture = Fixture::new();
        let operation = fixture.interrupted_capture();
        if legacy {
            fixture.store.connection.lock().unwrap().execute(
                /* sqlite-large-value: allow-for-now lv-h5121fed39a10 owner=fractal_reconcile::reconcile::tests::capture_restart TODO=bound-or-reference-values-and-reclamation */
                "UPDATE local_runtime_starts SET body=json_remove(body,'$.backend_entry') WHERE id=?1",
                [&operation.id],
            ).unwrap();
        } else {
            fixture.store.enter_runtime_start_backend(&operation.id, &operation.owner).unwrap();
        }
        let fixture = fixture.restarted();
        fixture.reconciler.reconcile_start_jobs().unwrap();
        fixture.reconciler.reconcile_start_jobs().unwrap();
        let pending = fixture.store.runtime_start_for(AGENT).unwrap().unwrap();
        assert!(pending.finished && !pending.cleanup_verified);
        assert_eq!(pending.result.as_ref().unwrap()["error"], "launch owner ended before its result");
        assert_ne!(pending.result.as_ref().unwrap()["producer_closed"], true);
        assert!(fixture.runtime.starts.lock().unwrap().is_empty());
        assert!(crate::checkout::activation_lease(&fixture.repository, &fixture.repository).is_err());
    }
}

#[test]
fn backend_entry_write_failure_aborts_before_launch() {
    let _clock = Clock::new();
    let fixture = Fixture::new();
    fixture.store.connection.lock().unwrap().execute_batch(
        /* sqlite-large-value: allow-for-now lv-hc4e2bcbba425 owner=fractal_reconcile::reconcile::tests::capture_restart TODO=bound-or-reference-values-and-reclamation */
        "CREATE TEMP TRIGGER fail_backend_entry BEFORE UPDATE OF body ON local_runtime_starts
         WHEN json_extract(NEW.body,'$.backend_entry.phase')='backend-entering'
         BEGIN SELECT RAISE(FAIL,'injected backend entry write failure'); END;",
    ).unwrap();
    fixture.start();
    fixture.reconciler.reconcile_start_jobs().unwrap();
    assert!(fixture.runtime.starts.lock().unwrap().is_empty());
    let failure = fixture.store.runtime_start_failure(AGENT).unwrap().unwrap();
    assert!(failure.contains("injected backend entry write failure"), "{failure}");
    assert!(fixture.reconciler.step_deadlines.lock().unwrap().is_empty());
    let closed: bool = fixture.store.readers.get().query_row(
        "SELECT cleanup_verified=1 AND json_extract(result,'$.closure_proof.witness.kind')='before-backend' FROM local_runtime_starts WHERE subject=?1",
        [AGENT], |row| row.get(0),
    ).unwrap();
    assert!(closed);
}

#[test]
fn non_timeout_capture_error_is_terminal_without_a_backoff() {
    let _clock = Clock::new();
    let fixture = Fixture::new();
    *fixture.runtime.capture_error.lock().unwrap() = Some("capture shell exited unsuccessfully".into());
    fixture.start();
    fixture.reconciler.reconcile_start_jobs().unwrap();
    assert!(fixture.store.runtime_start_failure(AGENT).unwrap().unwrap().contains("capture shell exited unsuccessfully"));
    assert_eq!(fixture.runtime.capture_timeouts.lock().unwrap().1, 1);
    assert!(fixture.runtime.starts.lock().unwrap().is_empty());
    assert!(fixture.reconciler.step_deadlines.lock().unwrap().is_empty());
    assert!(fixture.store.observations_for(AGENT, "harness.diagnostic").unwrap().iter()
        .all(|claim| claim.body["fields"]["code"] != "environment-capture-timeout"));
    smallclaims::store::set_thread_clock(Some(NOW + 60_000));
    fixture.reconciler.reconcile_start_jobs().unwrap();
    assert_eq!(fixture.runtime.capture_timeouts.lock().unwrap().1, 1);
}
