//! Recovery uses upstream round admission and restores retained crash state when work is not ready.
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
      completion {{ when "all-steps-exhausted" }}
      agent "gate" {{ workspace "/tmp"; harness "codex" {{}}; restart "{restart}" }}
      {preparation}
      step "gate" {{ {dependencies} assigned-to "agent/${{ST_MISSION_RUN}}/gate"; goal "Judge this round." }}
    }}
  }}
}}
"#),
        "gate-recovery-source",
    );
    let parent = store.create_mission_run(&MissionRunRequest {
        mission: "dotfiles/ui-design/recovery".into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: None,
        inputs: BTreeMap::new(),
        idempotency_key: "gate-recovery-run".into(),
    }).unwrap();
    let round = store.mission_run_subject_for_idempotency_key(
        &format!("loop-round:{}:1", parent.steps[0].subject),
    );
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    for _ in 0..12 {
        reconciler.evaluate_mission_runs().unwrap();
        let Some(owner) = store.mission_run(&round).unwrap() else { continue };
        assert!(owner.mission.starts_with("mission/__st3/"));
        let expected_gate = Reconciler::<FakeRuntime>::round_gate_agent(&owner)
            .expect("the upstream gate classifier must recognize this real round").to_owned();
        if !launch && store.desired_subjects_named(std::slice::from_ref(&expected_gate)).unwrap().is_empty() {
            // Restore an already-declared, pre-upgrade gate. Do not admit new unready work.
            let source = format!("version 2\nagent \"gate\" {{ workspace \"/tmp\"; harness \"codex\" {{}}; restart {restart:?} }}\n");
            let mut intent = crate::graph::parse_execution_intent(&source, "node", &owner.id).unwrap();
            let variables = crate::store::mission_run_variables(&owner, &owner.revision);
            for desired in intent.subjects.values_mut() {
                desired.owner_run = Some(owner.subject.clone());
                desired.owner_generation = Some(owner.generation.clone());
                desired.owner_step = None;
                if let Some(member) = desired.member.as_mut() {
                    member.workspace = owner.workspace.clone();
                    member.cwd = owner.workspace.clone();
                    member.environment.extend(variables.clone());
                    member.environment.insert("ST3_RUN_DIR".into(), owner.workspace.clone());
                    member.environment.insert("ST3_ENDPOINT".into(), reconciler.endpoint.clone());
                }
            }
            store.apply_internal(&intent, "restore-pre-upgrade-gate").unwrap();
        }
        if let Some(gate) = store.desired_subjects_for_owner_run(&round).unwrap().into_iter()
            .find(|subject| subject.kind == "agent" && subject.subject == expected_gate)
        {
            if !launch {
                assert!(runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
                return (store, runtime, gate);
            }
            let gate_work = owner.steps.iter().find(|step| step.step == "gate").unwrap();
            assert_eq!(gate_work.status, "ready", "upstream admission must persist ready work");
            assert!(store.work_for_reconcile(&gate.subject).unwrap().iter().any(|work| work.status == "ready"));
            reconciler.perform_start(&gate, gate.member.as_ref().unwrap(), "regression fixture").unwrap();
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
    store.member_reconcile_fault(&gate.subject, None).unwrap().is_some()
}

#[test]
fn a_missing_starting_round_gate_is_reported_after_daemon_restart_without_relaunch() {
    let (store, _, gate) = fixture("never");
    let runtime = Arc::new(FakeRuntime::default());
[…73ln elided…]
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
    assert!(!fault.is_empty());
    assert_eq!(status(&store, &gate), "starting");
    // Restart reconciliation preserves the unresolved admission rather than duplicating it.
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    restarted.reconcile_once().unwrap();
    assert_eq!(store.runtime_start_for(&gate.subject).unwrap().unwrap().id, operation.id);
    assert_eq!(runtime.0.starts.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
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
    assert_eq!(runtime.0.starts.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
    assert!(runtime.0.stops.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
    assert!(runtime.0.kills.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
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
    assert_eq!(runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
    let launched = runtime.started_members.lock().unwrap_or_else(PoisonError::into_inner)[0].clone();
    let incarnation = format!("fake-start:{}", launched.tags[start_jobs::OPERATION_TAG]);
    runtime.ptys.lock().unwrap_or_else(PoisonError::into_inner).push(RuntimeObservation {
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
    assert_eq!(runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
    assert!(store.member_reconcile_fault(&gate.subject, None).unwrap().is_none());
}

#[test]
fn a_starting_round_gate_without_launch_lineage_or_ready_work_reports_instead_of_waiting_forever() {
    let (store, _, gate) = fixture_with_launch("never", false);
    assert!(!Reconciler::new(
        store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()),
    ).member_was_launched_for_selected_desired(&gate.subject).unwrap());
    assert!(store.work_for_reconcile(&gate.subject).unwrap().iter().all(|work| work.status != "ready"));
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
    assert_ne!(status(&store, &gate), "starting", "a missing runtime must not remain silently starting");
    assert!(diagnostic(&store, &gate));
    let fault = store.member_reconcile_fault(&gate.subject, None).unwrap().unwrap();
    assert!(!fault.is_empty());
    assert!(runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
    // Ready work for the selected revision now permits the ordinary revision-change launch.
    let owner = store.mission_run(gate.owner_run.as_deref().unwrap()).unwrap().unwrap();
    let prepare = owner.steps.iter().find(|step| step.step == "prepare").unwrap();
    let judge = owner.steps.iter().find(|step| step.step == "gate").unwrap();
    store.set_step_state(&prepare.subject, "completed", None).unwrap();
    store.set_step_state(&judge.subject, "ready", None).unwrap();
    restarted.reconcile_once().unwrap();
    assert_eq!(runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
    assert_eq!(status(&store, &gate), "starting");
    assert!(store.member_reconcile_fault(&gate.subject, None).unwrap().is_none());
}

#[test]
fn a_completed_round_terminalizes_its_missing_starting_gate_without_restarting_it() {
    // Restore the retained crash state reported by the Manager: an ended owner,
    // a starting gate observation, and no runtime or launch producer to retire.
    let (store, _, gate) = fixture_with_launch("never", false);
    store.append_claim(&ClaimInput {
        subject: gate.subject.clone(),
        kind: "runtime.observed".into(),
        actor: None,
        fields: member_fields(
            gate.member.as_ref().unwrap(), "starting", Some("ended-round-gate"), false,
        ),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some("ended-round-starting-gate".into()),
    }).unwrap();
    let owner = store.mission_run(gate.owner_run.as_deref().unwrap()).unwrap().unwrap();
    store.set_mission_run_state(&owner.id, "completed", "terminal", None).unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    let restarted = Reconciler::new(
        store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()),
    );
    for _ in 0..3 { restarted.reconcile_once().unwrap(); }
    assert!(
        matches!(status(&store, &gate).as_str(), "stopped" | "absent" | "retired"),
        "a completed owner must not leave an unstarted gate in the active roster",
    );
    assert!(
        store.desired_subjects_named(std::slice::from_ref(&gate.subject)).unwrap()
            .iter().all(|desired| desired.kind != "agent"),
        "the completed round cannot retain a live gate declaration",
    );
    assert!(runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
}

[Showing lines 1-97 and 171-367 of 367; 73 middle lines (3.5KB) elided. Use :368 to continue. Read artifact://20314 for full output]