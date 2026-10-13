//! A blocked backend start must leave the real reconcile loop free to stop it.
use super::*;
use tokio::sync::mpsc;

struct BarrierRuntime {
    inner: FakeRuntime,
    store: Arc<Store>,
    entered: mpsc::UnboundedSender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    stopped: mpsc::UnboundedSender<Option<String>>,
    publish_early: bool,
    honor_guard: bool,
    operations: Mutex<HashMap<String, String>>,
    started: mpsc::UnboundedSender<String>,
    reject_gates: AtomicBool,
    verification_hidden: AtomicBool,
    provenance_error: AtomicBool,
}

impl RuntimeControl for BarrierRuntime {
    fn snapshot_ptys(&self) -> Result<Vec<RuntimeObservation>> {
        self.inner.snapshot_ptys()
    }
    fn observe_exec(&self, id: &str) -> Result<Option<RuntimeObservation>> {
        self.inner.observe_exec(id)
    }
    fn observe_start(&self, member: &MemberSpec) -> Result<Option<RuntimeObservation>> {
        if self.verification_hidden.load(Ordering::Acquire) {
            return Ok(None);
        }
        if self
            .operations
            .lock()
            .unwrap()
            .contains_key(&member.runtime_id)
        {
            Ok(self
                .inner
                .ptys
                .lock()
                .unwrap()
                .iter()
                .find(|item| item.runtime_id == member.runtime_id)
                .cloned())
        } else {
            self.inner.observe_start(member)
        }
    }
    fn start(&self, member: &MemberSpec) -> Result<()> {
        self.start_guarded(member, &|| Ok(()))
    }
    fn start_guarded(&self, member: &MemberSpec, guard: &dyn Fn() -> Result<()>) -> Result<()> {
        if member.runtime_id == "example.worker" || member.runtime_id.starts_with("example.held") {
            self.operations.lock().unwrap().insert(
                member.runtime_id.clone(),
                member.tags[start_jobs::OPERATION_TAG].clone(),
            );
            if self.publish_early {
                self.publish(member);
            }
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv()?;
            if self.honor_guard {
                guard()?;
            }
            if !self.publish_early {
                self.publish(member);
            }
        } else {
            guard()?;
        }
        if self.reject_gates.load(Ordering::Acquire)
            && member
                .driver
                .as_deref()
                .is_some_and(|driver| driver.ends_with("gate"))
        {
            self.inner
                .failed_starts
                .lock()
                .unwrap()
                .insert(member.runtime_id.clone());
        }
        self.inner.start(member)?;
        let _ = self.started.send(member.runtime_id.clone());
        Ok(())
    }
    fn start_operation(
        &self,
        id: &str,
        _terminal: bool,
        incarnation: &str,
    ) -> Result<Option<String>> {
        anyhow::ensure!(
            !self.provenance_error.load(Ordering::Acquire),
            "fixture provenance unavailable"
        );
        if incarnation != "blocked-start-incarnation" {
            return self.inner.start_operation(id, _terminal, incarnation);
        }
        Ok(self.operations.lock().unwrap().get(id).cloned())
    }
    fn start_operation_closed(&self, member: &MemberSpec) -> Result<bool> {
        // Held backends may have published early or may still have a detached producer.
        Ok(self.reject_gates.load(Ordering::Acquire)
            && member
                .driver
                .as_deref()
                .is_some_and(|driver| driver.ends_with("gate")))
    }
    fn stop(&self, id: &str, terminal: bool, incarnation: Option<&str>) -> Result<()> {
        let subject = format!("agent/{}", id.replace('.', "/"));
        let operation = self.store.runtime_start_for(&subject)?.unwrap();
        assert!(
            operation.revoked_by.is_some(),
            "physical control preceded durable revocation"
        );
        assert!(
            self.store
                .observations_for(&subject, "runtime.action.requested")?
                .iter()
                .any(|claim| claim.body["fields"]["action"] == "terminate")
        );
        self.inner.stop(id, terminal, incarnation)?;
        let _ = self.stopped.send(incarnation.map(str::to_owned));
        Ok(())
    }
    fn kill(&self, id: &str, terminal: bool, incarnation: Option<&str>) -> Result<()> {
        let subject = format!("agent/{}", id.replace('.', "/"));
        assert!(
            self.store
                .runtime_start_for(&subject)?
                .unwrap()
                .revoked_by
                .is_some()
        );
        let deadline = self
            .store
            .latest_observation(&subject, "runtime.action.deadline-reached")?
            .unwrap();
        assert_eq!(
            deadline.body["fields"]["incarnation_id"].as_str(),
            incarnation
        );
        assert!(
            self.store
                .observations_for(&subject, "runtime.action.requested")?
                .iter()
                .any(|request| deadline.body["evidence"][0] == request.id)
        );
        self.inner.kill(id, terminal, incarnation)
    }
    fn remove(&self, id: &str, terminal: bool) -> Result<()> {
        self.inner.remove(id, terminal)
    }
    fn screen(&self, id: &str) -> Result<String> {
        self.inner.screen(id)
    }
    fn send_key(&self, id: &str, key: &str) -> Result<()> {
        self.inner.send_key(id, key)
    }
    fn read_exec_log(&self, id: &str) -> Result<Option<String>> {
        self.inner.read_exec_log(id)
    }
}

impl BarrierRuntime {
    fn publish(&self, member: &MemberSpec) {
        self.inner.ptys.lock().unwrap().push(RuntimeObservation {
            runtime_id: member.runtime_id.clone(),
            terminal: true,
            status: "running".into(),
            exit_code: None,
            incarnation_id: Some("blocked-start-incarnation".into()),
        });
    }
}

struct ReleaseOnDrop(std::sync::mpsc::Sender<()>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        for _ in 0..8 {
            let _ = self.0.send(());
        }
    }
}

async fn workers_done(reconciler: &Arc<Reconciler<BarrierRuntime>>) {
    let live = reconciler.start_jobs.live.clone();
    tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || {
            let mut jobs = live.0.lock().unwrap();
            while !jobs.is_empty() {
                jobs = live.1.wait(jobs).unwrap();
            }
        }),
    )
    .await
    .unwrap()
    .unwrap();
}

fn fixture(
    publish_early: bool,
    honor_guard: bool,
) -> (
    tempfile::TempDir,
    Arc<Store>,
    Arc<BarrierRuntime>,
    Arc<Reconciler<BarrierRuntime>>,
    ReleaseOnDrop,
    mpsc::UnboundedReceiver<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("claims.db"), "node").unwrap());
    let (entered, entered_rx) = mpsc::unbounded_channel();
    let (release, held) = std::sync::mpsc::channel();
    let (stopped, _) = mpsc::unbounded_channel();
    let (started, _) = mpsc::unbounded_channel();
    let runtime = Arc::new(BarrierRuntime {
        inner: FakeRuntime::default(),
        store: store.clone(),
        entered,
        release: Mutex::new(held),
        stopped,
        publish_early,
        honor_guard,
        operations: Mutex::default(),
        started,
        reject_gates: AtomicBool::new(false),
        verification_hidden: AtomicBool::new(false),
        provenance_error: AtomicBool::new(false),
    });
    let reconciler = Arc::new(Reconciler::new(
        store.clone(),
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    ));
    reconciler
        .wait_for_test_starts
        .store(false, Ordering::Release);
    (
        directory,
        store,
        runtime,
        reconciler,
        ReleaseOnDrop(release),
        entered_rx,
    )
}

fn launch(reconciler: &Reconciler<BarrierRuntime>, subject: &str) -> bool {
    let desired = reconciler
        .store
        .desired_subjects_named(&[subject.to_owned()])
        .unwrap()
        .remove(0);
    reconciler
        .perform_start_for_request(
            &desired,
            desired.member.as_ref().unwrap(),
            "test launch",
            None,
        )
        .unwrap()
}

#[tokio::test]
async fn operator_stop_preempts_a_blocked_start_in_the_real_loop() {
    let directory = tempfile::tempdir().unwrap();
    // Use a file store, like the daemon, so publishing beside a pass needs no retries.
    let store = Arc::new(Store::open(&directory.path().join("claims.db"), "node").unwrap());
    apply_source(
        &store,
        r#"version 2
agent "example/worker" { workspace "/tmp"; command "true" }
"#,
        "barrier-start",
    );
    let (entered, mut entered_rx) = mpsc::unbounded_channel();
    let (release, held) = std::sync::mpsc::channel();
    let release = ReleaseOnDrop(release);
    let (stopped, mut stopped_rx) = mpsc::unbounded_channel();
    let (started, mut started_rx) = mpsc::unbounded_channel();
    let (probed, mut probed_rx) = mpsc::unbounded_channel();
    let runtime = Arc::new(BarrierRuntime {
        inner: FakeRuntime::default(),
        store: store.clone(),
        entered,
        release: Mutex::new(held),
        stopped,
        publish_early: true,
        honor_guard: true,
        operations: Mutex::default(),
        started,
        reject_gates: AtomicBool::new(false),
        verification_hidden: AtomicBool::new(false),
        provenance_error: AtomicBool::new(false),
    });
    let notify = Arc::new(Notify::new());
    let reconciler = Arc::new(
        Reconciler::new(
            store.clone(),
            runtime.clone(),
            "node".into(),
            notify.clone(),
        )
        .with_disk_probe(
            vec![PathBuf::from("/state")],
            Arc::new(move |_| {
                let _ = probed.send(());
                Ok(crate::disk::DiskSpace {
                    filesystem: 7,
                    available: 100 << 30,
                    total: 200 << 30,
                })
            }),
        ),
    );
    let task = tokio::spawn(reconciler.clone().run());
    tokio::time::timeout(Duration::from_secs(30), entered_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(runtime.inner.stops.lock().unwrap().is_empty());
    while probed_rx.try_recv().is_ok() {}
    apply_source(
        &store,
        "version 2\nstop \"agent/example/worker\"\nagent \"example/unrelated\" { workspace \"/tmp\"; command \"true\" }",
        "barrier-stop",
    );
    let selected = store
        .selected_desired_token("agent/example/worker")
        .unwrap()
        .unwrap();
    assert_eq!(
        store.claim_by_id(&selected).unwrap().unwrap().body["kind"],
        "stop"
    );
    notify.notify_one();
    // A watchdog bounds a failing test; it neither releases nor cancels the start.
    let before_release = tokio::time::timeout(Duration::from_secs(10), stopped_rx.recv()).await;
    let unrelated_before_release =
        tokio::time::timeout(Duration::from_secs(10), started_rx.recv()).await;
    let safety_before_release =
        tokio::time::timeout(Duration::from_secs(10), probed_rx.recv()).await;
    let physical_stops_before_release = runtime.inner.stops.lock().unwrap().len();
    // Always release before asserting, including on the old broken path, so the test
    // cannot strand a blocking worker during Tokio shutdown.
    release.0.send(()).unwrap();
    let incarnation = match before_release {
        Ok(Some(incarnation)) => incarnation,
        _ => tokio::time::timeout(Duration::from_secs(30), stopped_rx.recv())
            .await
            .unwrap()
            .unwrap(),
    };
    workers_done(&reconciler).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(incarnation.as_deref(), Some("blocked-start-incarnation"));
    assert_eq!(
        unrelated_before_release.unwrap().unwrap(),
        "example.unrelated"
    );
    safety_before_release.unwrap().unwrap();
    assert_eq!(
        physical_stops_before_release, 1,
        "the Store committed a stop, but the physical stop waited for the start barrier"
    );
}

#[tokio::test]
async fn unpublished_start_is_not_a_settled_stop_and_revoked_completion_is_suppressed() {
    let (_directory, store, runtime, reconciler, release, mut entered) = fixture(false, true);
    apply_source(
        &store,
        "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
        "unpublished",
    );
    assert!(launch(&reconciler, "agent/example/worker"));
    tokio::time::timeout(Duration::from_secs(30), entered.recv())
        .await
        .unwrap()
        .unwrap();
    apply_source(
        &store,
        "version 2\nstop \"agent/example/worker\"",
        "unpublished-stop",
    );
    reconciler.reconcile_once().unwrap();
    let operation = store
        .runtime_start_for("agent/example/worker")
        .unwrap()
        .unwrap();
    assert!(operation.revoked_by.is_some() && !operation.finished);
    assert!(
        !reconciler
            .stop_is_settled(&store.desired_subjects().unwrap().remove(0))
            .unwrap()
    );
    assert_eq!(store.runtime_start_counts().unwrap(), (1, 1));
    release.0.send(()).unwrap();
    workers_done(&reconciler).await;
    reconciler.reconcile_once().unwrap();
    assert!(runtime.inner.starts.lock().unwrap().is_empty());
    assert!(
        store
            .observations_for("agent/example/worker", "runtime.action.succeeded")
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.runtime_start_counts().unwrap(), (0, 0));
    assert!(
        store
            .runtime_start_for("agent/example/worker")
            .unwrap()
            .is_some(),
        "backend rejection alone cannot prove detached producer closure"
    );
}

#[tokio::test]
async fn revoked_jobs_hold_capacity_and_completion_admits_a_queued_seat() {
    let (_directory, store, _runtime, reconciler, release, mut entered) = fixture(false, true);
    apply_source(
        &store,
        "version 2\nagent \"example/held1\" { workspace \"/tmp\"; command \"true\" }\nagent \"example/held2\" { workspace \"/tmp\"; command \"true\" }\nagent \"example/queued\" { workspace \"/tmp\"; command \"true\" }",
        "cap",
    );
    let started_at = std::time::Instant::now();
    let threads_before = std::fs::read_dir("/proc/self/task")
        .ok()
        .map(|entries| entries.count());
    assert!(launch(&reconciler, "agent/example/held1"));
    assert!(launch(&reconciler, "agent/example/held2"));
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(30), entered.recv())
            .await
            .unwrap()
            .unwrap();
    }
    let threads_held = std::fs::read_dir("/proc/self/task")
        .ok()
        .map(|entries| entries.count());
    eprintln!(
        "cap=2: two launch admissions reached held backends in {:?}; process threads before={threads_before:?}, held={threads_held:?}",
        started_at.elapsed()
    );
    assert!(!launch(&reconciler, "agent/example/held1"));
    assert!(launch(&reconciler, "agent/example/queued"));
    assert!(
        !store
            .runtime_start_for("agent/example/queued")
            .unwrap()
            .unwrap()
            .dispatched
    );
    reconciler.revoke_start("agent/example/held1").unwrap();
    reconciler.revoke_start("agent/example/held2").unwrap();
    assert_eq!(store.runtime_start_counts().unwrap(), (2, 2));
    reconciler.reconcile_start_jobs().unwrap();
    assert!(
        !store
            .runtime_start_for("agent/example/queued")
            .unwrap()
            .unwrap()
            .dispatched
    );
    release.0.send(()).unwrap();
    release.0.send(()).unwrap();
    workers_done(&reconciler).await;
    reconciler.reconcile_start_jobs().unwrap();
    workers_done(&reconciler).await;
    reconciler.reconcile_start_jobs().unwrap();
    assert!(
        store
            .runtime_start_for("agent/example/queued")
            .unwrap()
            .is_none(),
        "queued completion faults: {:?}",
        reconciler.raised_faults.lock().unwrap()
    );
    assert_eq!(store.runtime_start_counts().unwrap(), (0, 0));
}

#[tokio::test]
async fn a_late_revoked_runtime_is_stopped_by_provenance_and_a_replacement_survives() {
    let (_directory, store, runtime, reconciler, release, mut entered) = fixture(false, false);
    apply_source(
        &store,
        "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
        "late",
    );
    assert!(launch(&reconciler, "agent/example/worker"));
    tokio::time::timeout(Duration::from_secs(30), entered.recv())
        .await
        .unwrap()
        .unwrap();
    reconciler.revoke_start("agent/example/worker").unwrap();
    release.0.send(()).unwrap();
    workers_done(&reconciler).await;
    reconciler.reconcile_start_jobs().unwrap();
    assert_eq!(*runtime.inner.stops.lock().unwrap(), ["example.worker"]);
    assert!(
        store
            .observations_for("agent/example/worker", "runtime.action.succeeded")
            .unwrap()
            .is_empty()
    );
    runtime.inner.ptys.lock().unwrap()[0].incarnation_id = Some("replacement".into());
    reconciler.reconcile_start_jobs().unwrap();
    assert_eq!(runtime.inner.stops.lock().unwrap().len(), 1);
    assert!(
        store
            .runtime_start_for("agent/example/worker")
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn an_owner_restart_never_replays_a_pending_launch_and_controls_bypass_its_lock() {
    let (_directory, store, runtime, reconciler, release, mut entered) = fixture(true, true);
    apply_source(
        &store,
        "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
        "owner",
    );
    assert!(launch(&reconciler, "agent/example/worker"));
    tokio::time::timeout(Duration::from_secs(30), entered.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .try_runtime_start_lock("agent/example/worker")
            .unwrap()
            .is_none()
    );
    let next = Arc::new(Reconciler::new(
        store.clone(),
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    ));
    next.wait_for_test_starts.store(false, Ordering::Release);
    next.reconcile_start_jobs().unwrap();
    assert!(!launch(&next, "agent/example/worker"));
    apply_source(
        &store,
        "version 2\nstop \"agent/example/worker\"",
        "owner-stop",
    );
    next.reconcile_once().unwrap();
    assert!(!runtime.inner.stops.lock().unwrap().is_empty());
    release.0.send(()).unwrap();
    workers_done(&reconciler).await;
    next.reconcile_start_jobs().unwrap();
    assert!(
        store
            .try_runtime_start_lock("agent/example/worker")
            .unwrap()
            .is_some()
    );
    assert!(runtime.inner.starts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn account_limit_enforcement_physically_stops_a_blocked_start() {
    let (_directory, store, runtime, reconciler, release, mut entered) = fixture(true, true);
    apply_source(
        &store,
        "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
        "limit",
    );
    assert!(launch(&reconciler, "agent/example/worker"));
    tokio::time::timeout(Duration::from_secs(30), entered.recv())
        .await
        .unwrap()
        .unwrap();
    let now = now_ms();
    store
        .append_claim(&ClaimInput {
            subject: "agent/example/worker".into(),
            kind: "harness.limits".into(),
            actor: Some("agent/example/worker".into()),
            fields: BTreeMap::from([
                ("driver".into(), Value::from("claude")),
                ("account".into(), Value::from("claude/example")),
                (
                    "incarnation_id".into(),
                    Value::from("blocked-start-incarnation"),
                ),
                ("weekly_percent".into(), Value::from(96)),
                ("five_hour_percent".into(), Value::from(40)),
                ("measured_at_unix_ms".into(), Value::from(now as u64)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let policy = crate::store::LimitsPolicy {
        stop_at_weekly_percent: 95,
        keep: BTreeSet::new(),
        exempt_accounts: BTreeSet::new(),
        exempt_harnesses: BTreeSet::new(),
        notify: "agent/example/operations".into(),
        fresh_ms: 3_600_000,
    };
    let outcome = store.enforce_account_limits(&policy, now).unwrap();
    assert_eq!(outcome.stopped, ["agent/example/worker"]);
    reconciler.reconcile_once().unwrap();
    assert_eq!(*runtime.inner.stops.lock().unwrap(), ["example.worker"]);
    assert_eq!(store.runtime_start_counts().unwrap(), (1, 1));
    release.0.send(()).unwrap();
    workers_done(&reconciler).await;
    assert!(runtime.inner.starts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn operator_stop_reaches_its_incarnation_fenced_kill_while_start_is_blocked() {
    let (_directory, store, runtime, reconciler, release, mut entered) = fixture(true, true);
    apply_source(
        &store,
        "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\"; shutdown-timeout \"1ms\" }",
        "kill-start",
    );
    assert!(launch(&reconciler, "agent/example/worker"));
    tokio::time::timeout(Duration::from_secs(30), entered.recv())
        .await
        .unwrap()
        .unwrap();
    apply_source(
        &store,
        "version 2\nstop \"agent/example/worker\"",
        "kill-stop",
    );
    reconciler.reconcile_once().unwrap();
    // Let the authored shutdown grace elapse without releasing the launch barrier.
    tokio::time::sleep(Duration::from_millis(2)).await;
    reconciler.reconcile_once().unwrap();
    assert_eq!(*runtime.inner.kills.lock().unwrap(), ["example.worker"]);
    assert_eq!(store.runtime_start_counts().unwrap(), (1, 1));
    release.0.send(()).unwrap();
    workers_done(&reconciler).await;
    assert!(runtime.inner.starts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn asynchronous_gate_start_failures_are_broken_checks_without_relaunch() {
    let (_directory, _store, runtime, reconciler, _release, _entered) = fixture(false, true);
    runtime.reject_gates.store(true, Ordering::Release);
    let stage = GateContext {
        subject: "step-run/example/verify".into(),
        name: "verify".into(),
        started_at_unix_ms: now_ms(),
        attempt: 1,
        run: String::new(),
        generation: String::new(),
        eval: false,
    };
    let environment = BTreeMap::new();
    let mechanical = || {
        reconciler
            .run_mechanical(
                &stage,
                "mechanical",
                "true",
                "node",
                "/tmp",
                &environment,
                60_000,
            )
            .unwrap()
    };
    assert!(matches!(mechanical(), GateOutcome::Pending));
    workers_done(&reconciler).await;
    reconciler.reconcile_start_jobs().unwrap();
    assert!(matches!(mechanical(), GateOutcome::Broken(_)));
    assert!(matches!(mechanical(), GateOutcome::Broken(_)));
    assert_eq!(runtime.inner.starts.lock().unwrap().len(), 1);
    let llm = || {
        reconciler
            .run_llm_gate(
                &stage,
                "llm",
                "example-model",
                "node",
                "/tmp",
                &[],
                &environment,
                1_000,
                60_000,
                "Check the result.",
            )
            .unwrap()
    };
    assert!(matches!(llm(), GateOutcome::Pending));
    workers_done(&reconciler).await;
    reconciler.reconcile_start_jobs().unwrap();
    assert!(matches!(llm(), GateOutcome::Broken(_)));
    assert!(matches!(llm(), GateOutcome::Broken(_)));
    assert_eq!(runtime.inner.starts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn accepted_but_unverified_starts_retain_admission_until_exact_verification_or_retirement() {
    for revoke in [false, true] {
        let (_directory, store, runtime, reconciler, release, mut entered) = fixture(true, false);
        apply_source(
            &store,
            "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
            "verification-start",
        );
        runtime.verification_hidden.store(true, Ordering::Release);
        assert!(launch(&reconciler, "agent/example/worker"));
        tokio::time::timeout(Duration::from_secs(30), entered.recv())
            .await
            .unwrap()
            .unwrap();
        release.0.send(()).unwrap();
        workers_done(&reconciler).await;
        reconciler.reconcile_start_jobs().unwrap();
        let operation = store
            .runtime_start_for("agent/example/worker")
            .unwrap()
            .unwrap();
        assert!(operation.finished && !operation.applied && !operation.receipted);
        assert_eq!(store.unresolved_runtime_start_counts().unwrap(), (1, 0));
        assert!(!launch(&reconciler, "agent/example/worker"));
        assert!(
            store
                .observations_for("agent/example/worker", "runtime.action.succeeded")
                .unwrap()
                .is_empty()
        );
        if revoke {
            reconciler.revoke_start("agent/example/worker").unwrap();
        }
        runtime.verification_hidden.store(false, Ordering::Release);
        runtime
            .operations
            .lock()
            .unwrap()
            .insert("example.worker".into(), "foreign-operation".into());
        reconciler.reconcile_start_jobs().unwrap();
        assert!(
            store
                .runtime_start_for("agent/example/worker")
                .unwrap()
                .is_some()
        );
        assert!(runtime.inner.stops.lock().unwrap().is_empty());
        runtime
            .operations
            .lock()
            .unwrap()
            .insert("example.worker".into(), operation.id.clone());
        runtime.provenance_error.store(true, Ordering::Release);
        reconciler.reconcile_start_jobs().unwrap();
        assert!(
            store
                .runtime_start_for("agent/example/worker")
                .unwrap()
                .is_some()
        );
        runtime.provenance_error.store(false, Ordering::Release);
        reconciler.reconcile_start_jobs().unwrap();
        if revoke {
            assert_eq!(*runtime.inner.stops.lock().unwrap(), ["example.worker"]);
            runtime.inner.ptys.lock().unwrap()[0].status = "exited".into();
            reconciler.reconcile_start_jobs().unwrap();
            assert!(
                store
                    .observations_for("agent/example/worker", "runtime.action.succeeded")
                    .unwrap()
                    .is_empty()
            );
        } else {
            let succeeded = store
                .observations_for("agent/example/worker", "runtime.action.succeeded")
                .unwrap();
            assert_eq!(succeeded.len(), 1);
            assert_eq!(
                succeeded[0].body["fields"]["incarnation_id"],
                "blocked-start-incarnation"
            );
            assert!(
                succeeded[0].body["evidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|evidence| evidence == &operation.body["start_request"])
            );
        }
        assert!(
            store
                .runtime_start_for("agent/example/worker")
                .unwrap()
                .is_none()
        );
        assert_eq!(store.unresolved_runtime_start_counts().unwrap(), (0, 0));
    }
}

#[test]
fn native_rejected_starts_prove_closure_and_allow_a_later_start_for_the_same_seat() {
    if crate::test_support::supervise_test() {
        return;
    }
    for failure in ["missing-program", "missing-cwd"] {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&root.path().join("graph.db"), "node").unwrap());
        let runtime = Arc::new(NativeRuntime::new(
            root.path(),
            None,
            Path::new("unused-pty"),
        ));
        let reconciler = Reconciler::new(
            store.clone(),
            runtime.clone(),
            "node".into(),
            Arc::new(Notify::new()),
        );
        // Exercise NativeRuntime's producer boundary directly through the durable jobs.
        // The ordinary exec member preparation wraps its command in the st3 driver;
        // a missing command inside that wrapper is a later readiness failure.
        let dispatch = |desired: &DesiredSubject| {
            let member = desired.member.as_ref().unwrap().clone();
            reconciler.dispatch_start(start_jobs::PreparedStart {
                subject: desired.clone(),
                launch_member: member.clone(),
                member,
                reason: "native producer closure regression".into(),
                request: None,
                desired_token: reconciler.launch_token(&desired.subject).unwrap(),
                selected_token: store.selected_desired_token(&desired.subject).unwrap(),
                placement_evidence: Vec::new(),
                start_request: String::new(),
                expected_incarnation: None,
                resume_operation: None,
            })
        };
        let cwd = if failure == "missing-cwd" {
            root.path().join("missing-cwd")
        } else {
            root.path().to_owned()
        };
        let program = if failure == "missing-program" {
            root.path().join("missing-program").display().to_string()
        } else {
            "/bin/sleep".into()
        };
        apply_source(
            &store,
            &format!(
                "version 2\nexec \"example/rejected\" {{ workspace {:?}; argv {:?} \"1000\"; }}",
                cwd.display().to_string(),
                program
            ),
            "rejected-native",
        );
        let desired = store
            .desired_subjects_named(&["exec/example/rejected".into()])
            .unwrap()
            .remove(0);
        assert!(dispatch(&desired).is_err());
        assert!(store.runtime_start_for(&desired.subject).unwrap().is_none());
        let result: String = store
            .readers
            .get()
            .query_row(
                "SELECT result FROM local_runtime_starts WHERE subject=?1 ORDER BY id DESC LIMIT 1",
                [&desired.subject],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&result).unwrap()["producer_closed"],
            true
        );
        apply_source(
            &store,
            &format!(
                "version 2\nexec \"example/rejected\" {{ workspace {:?}; argv \"/bin/sleep\" \"1000\"; }}",
                root.path().display().to_string()
            ),
            "valid-native",
        );
        let desired = store
            .desired_subjects_named(&["exec/example/rejected".into()])
            .unwrap()
            .remove(0);
        assert!(dispatch(&desired).unwrap());
        assert!(store.runtime_start_for(&desired.subject).unwrap().is_none());
        let observation = runtime
            .observe_exec(&desired.member.as_ref().unwrap().runtime_id)
            .unwrap()
            .unwrap();
        runtime
            .kill(
                &observation.runtime_id,
                false,
                observation.incarnation_id.as_deref(),
            )
            .unwrap();
    }
}
