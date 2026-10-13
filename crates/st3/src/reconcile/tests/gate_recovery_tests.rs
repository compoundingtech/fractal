//! Missing runtime recovery uses real round ownership and the durable launch path.
use super::*;

fn fixture(restart: &str) -> (Arc<Store>, Arc<FakeRuntime>, DesiredSubject) {
    fixture_with_launch(restart, true)
}

fn fixture_with_launch(restart: &str, launch: bool) -> (Arc<Store>, Arc<FakeRuntime>, DesiredSubject) {
    let preparation = if launch { "" } else {
        "step \"prepare\" { assigned-to \"agent/unavailable\"; goal \"Wait for preparation.\" }"
    };
    let dependencies = if launch { "" } else {
        "depends-on { step \"prepare\" completed };"
    };
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(
        &store,
        &format!(r#"version 2
mission "dotfiles/ui-design/recovery" state="ready" {{
  goal "Recover the selected refine gate."
  loop "refine" {{
    max-rounds 1
    round {{
      agent "gate" {{ harness "codex" {{}}; restart "{restart}" }}
      {preparation}
      step "judge" {{ {dependencies} assigned-to "gate"; goal "Judge this round." }}
    }}
  }}
}}
"#),
        "gate-recovery-source",
    );
    store.create_mission_run(&MissionRunRequest {
        mission: "dotfiles/ui-design/recovery".into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: None,
        inputs: BTreeMap::new(),
        idempotency_key: "gate-recovery-run".into(),
    }).unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    for _ in 0..12 {
        reconciler.reconcile_once().unwrap();
        if let Some(gate) = store.desired_subjects().unwrap().into_iter()
            .find(|subject| store.is_loop_gate(subject).unwrap())
        {
            if !launch {
                assert!(runtime.starts.lock().unwrap().is_empty());
                return (store, runtime, gate);
            }
            let owner = store.mission_run(gate.owner_run.as_deref().unwrap()).unwrap().unwrap();
            let judge = owner.steps.iter().find(|step| step.step == "judge").unwrap();
            store.set_step_state(&judge.subject, "ready", None).unwrap();
            if runtime.starts.lock().unwrap().is_empty() {
                reconciler.perform_start(&gate, gate.member.as_ref().unwrap(), "regression fixture").unwrap();
            }
            assert!(store.runtime_start_for(&gate.subject).unwrap().is_none());
            assert_eq!(status(&store, &gate), "starting");
            return (store, runtime, gate);
        }
    }
    panic!("the real refine round did not materialize its gate");
}

fn status(store: &Store, gate: &DesiredSubject) -> String {
    let actual = store.latest_actual_value(&gate.subject).unwrap().unwrap();
    actual_field(&actual, "status").unwrap().as_str().unwrap().to_owned()
}

fn diagnostic(store: &Store, gate: &DesiredSubject) -> bool {
    store.claims_for(&gate.subject, Some("harness.diagnostic")).unwrap().iter()
        .any(|claim| claim.body["fields"]["code"] == "gate-runtime-missing")
}

#[test]
fn a_missing_starting_round_gate_is_reported_after_daemon_restart_without_relaunch() {
    let (store, _, gate) = fixture("never");
    let runtime = Arc::new(FakeRuntime::default());
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    restarted.reconcile_once().unwrap();
    assert_eq!(status(&store, &gate), "vanished");
    assert!(diagnostic(&store, &gate));
    assert!(runtime.starts.lock().unwrap().is_empty());
    let fault = store.member_reconcile_fault(&gate.subject, None).unwrap().unwrap();
    assert!(fault.contains("restart policy does not permit recovery"));
    restarted.reconcile_once().unwrap();
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert_eq!(store.member_reconcile_fault(&gate.subject, None).unwrap().as_deref(), Some(fault.as_str()));
}

#[test]
fn a_live_starting_round_gate_is_adopted_after_daemon_restart() {
    let (store, _, gate) = fixture("never");
    let runtime = Arc::new(FakeRuntime::default());
    runtime.ptys.lock().unwrap().push(RuntimeObservation {
        runtime_id: gate.member.as_ref().unwrap().runtime_id.clone(),
        terminal: true,
        status: "running".into(),
        exit_code: None,
        incarnation_id: Some("surviving-gate".into()),
    });
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    restarted.reconcile_once().unwrap();
    assert_eq!(status(&store, &gate), "running");
    assert!(!diagnostic(&store, &gate));
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert!(runtime.stops.lock().unwrap().is_empty());
    assert!(runtime.kills.lock().unwrap().is_empty());
}

#[test]
fn a_missing_starting_round_gate_uses_its_declared_restart_policy() {
    let (store, _, gate) = fixture("always");
    let runtime = Arc::new(FakeRuntime::default());
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    restarted.reconcile_once().unwrap();
    assert!(diagnostic(&store, &gate));
    assert_eq!(*runtime.starts.lock().unwrap(), [gate.member.as_ref().unwrap().runtime_id.clone()]);
    assert_eq!(status(&store, &gate), "starting");
}

#[test]
fn a_manual_stop_of_a_missing_starting_round_gate_survives_daemon_restart() {
    let (store, _, gate) = fixture("always");
    let stop = crate::graph::parse_internal_intent(
        &format!("version 2\nstop {:?}\n", gate.subject), "node",
    ).unwrap();
    store.apply(&stop, &BTreeMap::new(), "gate-recovery-stop").unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    for _ in 0..3 { restarted.reconcile_once().unwrap(); }
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert!(matches!(status(&store, &gate).as_str(), "stopped" | "absent"));
    assert!(!diagnostic(&store, &gate));
}

struct UnverifiedRuntime(FakeRuntime, Option<Arc<std::sync::Barrier>>);
impl RuntimeControl for UnverifiedRuntime {
    fn snapshot_ptys(&self) -> Result<Vec<RuntimeObservation>> { self.0.snapshot_ptys() }
    fn observe_exec(&self, id: &str) -> Result<Option<RuntimeObservation>> { self.0.observe_exec(id) }
    fn observe_start(&self, _: &MemberSpec) -> Result<Option<RuntimeObservation>> { Ok(None) }
    fn start(&self, member: &MemberSpec) -> Result<()> {
        if let Some(barrier) = &self.1 {
            barrier.wait(); // The test observes the actual worker entering the backend.
            barrier.wait(); // The test releases it after the reconciliation pass.
        }
        self.0.start(member)
    }
    fn start_operation_closed(&self, _: &MemberSpec) -> Result<bool> { Ok(false) }
    fn stop(&self, id: &str, terminal: bool, incarnation: Option<&str>) -> Result<()> {
        self.0.stop(id, terminal, incarnation)
    }
    fn kill(&self, id: &str, terminal: bool, incarnation: Option<&str>) -> Result<()> {
        self.0.kill(id, terminal, incarnation)
    }
    fn remove(&self, id: &str, terminal: bool) -> Result<()> { self.0.remove(id, terminal) }
    fn screen(&self, id: &str) -> Result<String> { self.0.screen(id) }
    fn send_key(&self, id: &str, key: &str) -> Result<()> { self.0.send_key(id, key) }
    fn read_exec_log(&self, id: &str) -> Result<Option<String>> { self.0.read_exec_log(id) }
}

#[test]
fn a_finished_unverified_round_gate_start_keeps_admission_and_reports_a_durable_fault() {
    let (store, _, gate) = fixture("never");
    let runtime = Arc::new(UnverifiedRuntime(FakeRuntime::default(), None));
    let reconciler = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    reconciler.perform_start(&gate, gate.member.as_ref().unwrap(), "unverified fixture start").unwrap();
    let operation = store.runtime_start_for(&gate.subject).unwrap().unwrap();
    assert!(operation.finished);
    assert!(!operation.applied);
    reconciler.reconcile_once().unwrap();
    let fault = store.member_reconcile_fault(&gate.subject, None).unwrap().unwrap();
    assert!(fault.contains(&operation.id));
    assert!(fault.contains("producer closure"));
    assert_eq!(status(&store, &gate), "starting");
    // Restart reconciliation preserves the unresolved admission rather than duplicating it.
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    restarted.reconcile_once().unwrap();
    assert_eq!(store.runtime_start_for(&gate.subject).unwrap().unwrap().id, operation.id);
    assert_eq!(runtime.0.starts.lock().unwrap().len(), 1);
    assert!(store.member_reconcile_fault(&gate.subject, None).unwrap().is_some());
}

#[test]
fn a_live_in_flight_round_gate_start_is_not_retired_or_duplicated() {
    let (store, _, gate) = fixture("never");
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let runtime = Arc::new(UnverifiedRuntime(FakeRuntime::default(), Some(barrier.clone())));
    let reconciler = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    reconciler.wait_for_test_starts.store(false, Ordering::Release);
    reconciler.perform_start(&gate, gate.member.as_ref().unwrap(), "held fixture start").unwrap();
    barrier.wait();
    let operation = store.runtime_start_for(&gate.subject).unwrap().unwrap();
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    let result = restarted.reconcile_once();
    let during_start = status(&store, &gate);
    let reported_missing = diagnostic(&store, &gate);
    let fault = store.member_reconcile_fault(&gate.subject, None).unwrap();
    barrier.wait();
    // Await the same worker completion path, never a sleep or a second launcher.
    let (live, changed) = &*reconciler.start_jobs.live;
    let mut live = live.lock().unwrap_or_else(PoisonError::into_inner);
    while live.contains(&operation.id) {
        live = changed.wait(live).unwrap_or_else(PoisonError::into_inner);
    }
    drop(live);
    result.unwrap();
    assert_eq!(during_start, "starting");
    assert!(!reported_missing);
    assert!(fault.is_none());
    assert_eq!(runtime.0.starts.lock().unwrap().len(), 1);
    assert!(runtime.0.stops.lock().unwrap().is_empty());
    assert!(runtime.0.kills.lock().unwrap().is_empty());
}

#[test]
fn an_explicit_restart_recovers_a_missing_gate_once_and_clears_its_fault() {
    let (store, _, gate) = fixture("never");
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    reconciler.reconcile_once().unwrap();
    assert!(store.member_reconcile_fault(&gate.subject, None).unwrap().is_some());
    let actual = store.latest_actual_value(&gate.subject).unwrap().unwrap();
    let incarnation = actual_field(&actual, "incarnation_id").cloned().unwrap();
    store.append_claim(&ClaimInput {
        subject: gate.subject.clone(),
        kind: "runtime.action.requested".into(),
        actor: Some("person/test".into()),
        fields: BTreeMap::from([
            ("action".into(), Value::from("restart")),
            ("incarnation_id".into(), incarnation),
            ("reason".into(), Value::from("recover the missing gate")),
        ]),
        evidence: vec![store.selected_desired_token(&gate.subject).unwrap().unwrap()],
        expected_subject: None,
        idempotency_key: Some("gate-recovery-explicit-restart".into()),
    }).unwrap();
    reconciler.reconcile_once().unwrap();
    assert_eq!(runtime.starts.lock().unwrap().len(), 1);
    let launched = runtime.started_members.lock().unwrap()[0].clone();
    let incarnation = format!("fake-start:{}", launched.tags[start_jobs::OPERATION_TAG]);
    runtime.ptys.lock().unwrap().push(RuntimeObservation {
        runtime_id: launched.runtime_id,
        terminal: true,
        status: "running".into(),
        exit_code: None,
        incarnation_id: Some(incarnation.clone()),
    });
    store.append_claim(&harness_claim(&gate.subject, "ready", &incarnation)).unwrap();
    // A restarted reconciler consumes the original request, not a new retry grant.
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    for _ in 0..3 { restarted.reconcile_once().unwrap(); }
    assert_eq!(runtime.starts.lock().unwrap().len(), 1);
    assert!(store.member_reconcile_fault(&gate.subject, None).unwrap().is_none());
}

#[test]
fn a_starting_round_gate_without_launch_lineage_or_ready_work_reports_instead_of_waiting_forever() {
    let (store, _, gate) = fixture_with_launch("never", false);
    assert!(!Reconciler::new(
        store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()),
    ).member_was_launched_for_selected_desired(&gate.subject).unwrap());
    assert!(store.loop_gate_ready_at(&gate).unwrap().is_none());
    store.append_claim(&ClaimInput {
        subject: gate.subject.clone(),
        kind: "runtime.observed".into(),
        actor: None,
        fields: member_fields(gate.member.as_ref().unwrap(), "starting", Some("crashed-without-receipt"), false),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some("gate-recovery-no-lineage".into()),
    }).unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    for _ in 0..3 { restarted.reconcile_once().unwrap(); }
    assert_eq!(status(&store, &gate), "vanished");
    assert!(diagnostic(&store, &gate));
    let fault = store.member_reconcile_fault(&gate.subject, None).unwrap().unwrap();
    assert!(fault.contains("no ready work authorizes a replacement"));
    assert!(runtime.starts.lock().unwrap().is_empty());
    // Ready work for the selected revision now permits the ordinary revision-change launch.
    let owner = store.mission_run(gate.owner_run.as_deref().unwrap()).unwrap().unwrap();
    let prepare = owner.steps.iter().find(|step| step.step == "prepare").unwrap();
    let judge = owner.steps.iter().find(|step| step.step == "judge").unwrap();
    store.set_step_state(&prepare.subject, "completed", None).unwrap();
    store.set_step_state(&judge.subject, "ready", None).unwrap();
    restarted.reconcile_once().unwrap();
    assert_eq!(runtime.starts.lock().unwrap().len(), 1);
    assert_eq!(status(&store, &gate), "starting");
    assert!(store.member_reconcile_fault(&gate.subject, None).unwrap().is_none());
}
