use super::*;
use serde_json::json;

const START: u128 = 1_800_000_000_000;
const TIMEOUT: u64 = 1_000;
const PHASE_KIND: &str = "step.execution-phase";

struct Clock;

impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

struct Fixture {
    _clock: Clock,
    store: Arc<Store>,
    reconciler: Reconciler<FakeRuntime>,
    run: MissionRunView,
    mission: MissionSpec,
    exec: String,
}

impl Fixture {
    fn new() -> Self {
        Self::with_legacy_state(None)
    }

    fn with_legacy_state(legacy_state: Option<&str>) -> Self {
        smallclaims::store::set_thread_clock(Some(START));
        let clock = Clock;
        let store = Arc::new(Store::open_memory("coordinator").unwrap());
        store.set_write_clock_at(START).unwrap();
        apply_source(&store, r#"version 2
mission "execution-phases" state="ready" {
  goal "Keep dispatch and execution budgets separate."
  step "work" timeout="1s" {
    agentless
    exec "payload" { host "worker"; workspace "/tmp"; command "true"; restart "never"; }
  }
}"#, "execution-phases");
        let run = store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "execution-phases".into(), revision: None, workspace: "/tmp".into(),
            requester: None, mode: None, inputs: BTreeMap::new(),
            idempotency_key: "execution-phases-run".into(),
        }).unwrap();
        let mission = store.mission_spec("execution-phases", None).unwrap().unwrap();
        let reconciler = Reconciler::new(
            store.clone(), Arc::new(FakeRuntime::default()), "coordinator".into(),
            Arc::new(Notify::new()),
        );
        if let Some(status) = legacy_state {
            // Publish exactly the old run-owned declaration shape, without calling the
            // phase-aware materializer. Upgrade must not enroll this in-flight attempt.
            let steps = flatten_mission_steps(&mission);
            let step = steps.iter().find(|step| step.spec.path == "work").unwrap();
            let view = store.step_run(&run.steps[0].subject).unwrap().unwrap();
            let variables = run_variables(&run, step, &view);
            let source = crate::mission::interpolate_kdl(
                step.spec.declarations_kdl.as_deref().unwrap(),
                &variables,
            ).unwrap();
            let source = crate::mission::with_mission_capacity(&source, step.mission_capacity).unwrap();
            let mut intent = crate::graph::parse_execution_intent(&source, "coordinator", &run.id).unwrap();
            for desired in intent.subjects.values_mut() {
                desired.owner_run = Some(run.subject.clone());
                desired.owner_generation = Some(run.generation.clone());
                desired.owner_step = Some(view.subject.clone());
                if let Some(member) = desired.member.as_mut() {
                    reconciler.bind_resource_lane(&run, step, &view, member, &desired.subject).unwrap();
                    let workspace = PathBuf::from(&member.workspace);
                    if workspace.is_relative() {
                        member.workspace = Path::new(&run.workspace).join(&workspace)
                            .to_string_lossy().into_owned();
                    }
                    let cwd = PathBuf::from(&member.cwd);
                    if cwd.is_relative() {
                        member.cwd = Path::new(&run.workspace).join(&cwd)
                            .to_string_lossy().into_owned();
                    }
                    member.environment.extend(variables.clone());
                    member.environment.insert("ST3_RUN_DIR".into(), run.workspace.clone());
                    member.environment.insert("ST3_ENDPOINT".into(), reconciler.endpoint.clone());
                }
            }
            store.apply_internal(&intent, &format!("materialize:{}:{}", view.subject, view.attempt)).unwrap();
            store.set_step_state(&view.subject, status, None).unwrap();
        } else {
            // Mission evaluation materializes real run-owned desired execs without
            // delivering them to the worker or depending on process scheduling.
            for _ in 0..3 {
                let current = store.mission_run(&run.id).unwrap().unwrap();
                reconciler.evaluate_mission_run(&current, &mission).unwrap();
            }
        }
        let exec = store.desired_subjects_for_owner_step(&run.steps[0].subject).unwrap()
            .into_iter().find(|desired| desired.kind == "exec")
            .expect("the step must publish its native exec").subject;
        Self { _clock: clock, store, reconciler, run, mission, exec }
    }

    fn at(&self, now: u128) {
        smallclaims::store::set_thread_clock(Some(now));
        self.store.set_write_clock_at(now).unwrap();
    }

    fn view(&self) -> crate::model::StepRunView {
        self.store.step_run(&self.run.steps[0].subject).unwrap().unwrap()
    }

    fn phase(&self, attempt: u32, phase: &str, at: u128) {
        self.store.append_claim(&ClaimInput {
            subject: self.run.steps[0].subject.clone(), kind: PHASE_KIND.into(),
            actor: None,
            fields: BTreeMap::from([
                ("attempt".into(), json!(attempt)), ("phase".into(), json!(phase)),
                ("exec".into(), json!(self.exec)), ("at_unix_ms".into(), json!(at)),
                ("host".into(), json!("worker")),
                ("incarnation_id".into(), json!("physical-exec")),
                ("start_operation".into(), json!(format!("operation-{attempt}"))),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }

    fn timed_out(&self) -> bool {
        let steps = flatten_mission_steps(&self.mission);
        let step = steps.iter().find(|step| step.spec.path == "work").unwrap();
        self.reconciler.step_timed_out(&self.view(), step).unwrap()
    }

    fn evaluate(&self) {
        let current = self.store.mission_run(&self.run.id).unwrap().unwrap();
        self.reconciler.evaluate_mission_run(&current, &self.mission).unwrap();
    }
}

#[test]
fn cross_host_dispatch_delay_does_not_consume_native_execution_timeout() {
    let fixture = Fixture::new();
    fixture.at(START + 5_000);
    assert_eq!(fixture.view().execution_elapsed_ms, 0, "dispatch does not open the execution clock");
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false));
    assert!(!fixture.timed_out(), "undelivered desired state is not running execution");

    assert!(fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    assert_eq!(fixture.view().status, "failed");
    let claims = fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap();
    assert!(claims.iter().any(|claim| {
        let fields = &claim.body["fields"];
        fields["phase"] == "dispatch-stalled"
            && fields["attempt"] == json!(fixture.view().attempt)
            && fields["exec"] == fixture.exec
    }), "dispatch expiry must retain its attributed internal phase");
}

#[test]
fn delayed_exec_gets_its_full_execution_budget_from_replicated_start() {
    let fixture = Fixture::new();
    let attempt = fixture.view().attempt;
    fixture.at(START + 1_500);
    fixture.phase(attempt, "received", START + 1_400);
    fixture.phase(attempt, "admitted", START + 1_450);
    fixture.phase(attempt, "exec-started", START + 1_500);
    fixture.at(START + 2_499);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false));
    assert!(!fixture.timed_out());
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
}

#[test]
fn native_exec_running_past_its_budget_times_out() {
    let fixture = Fixture::new();
    fixture.phase(fixture.view().attempt, "exec-started", START);
    fixture.at(START + 1_001);
    fixture.phase(fixture.view().attempt, "execution-timeout", START + 1_001);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(true));
    assert!(fixture.timed_out());
    fixture.evaluate();
    assert_eq!(fixture.view().status, "failed");
}

#[test]
fn replicated_exit_stops_execution_clock_before_result_arrives() {
    let fixture = Fixture::new();
    let attempt = fixture.view().attempt;
    fixture.phase(attempt, "exec-started", START);
    fixture.at(START + 900);
    fixture.phase(attempt, "exec-exited", START + 900);
    fixture.at(START + 1_500);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false));
    assert!(!fixture.timed_out());
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
}

#[test]
fn late_exit_does_not_erase_execution_timeout() {
    let fixture = Fixture::new();
    let attempt = fixture.view().attempt;
    fixture.phase(attempt, "exec-started", START);
    fixture.at(START + 1_001);
    fixture.phase(attempt, "exec-exited", START + 1_001);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(true));
}

#[test]
fn stale_attempt_start_and_exit_cannot_change_current_execution_budget() {
    let fixture = Fixture::new();
    let first = fixture.view().attempt;
    fixture.phase(first, "exec-started", START);
    fixture.store.set_step_state(&fixture.view().subject, "failed", Some("retry fixture")).unwrap();
    fixture.store.retry_step(&fixture.view().subject, "next attempt", 0).unwrap();
    let steps = flatten_mission_steps(&fixture.mission);
    let step = steps.iter().find(|step| step.spec.path == "work").unwrap();
    fixture.reconciler.materialize_step_declarations(
        &fixture.run, &fixture.mission, step, &fixture.view(), false,
    ).unwrap();
    fixture.store.set_step_state(&fixture.view().subject, "working", None).unwrap();
    let next = fixture.view().attempt;
    assert_eq!(next, first + 1);
    fixture.at(START + 5_000);
    fixture.phase(first, "execution-timeout", START + 1_001);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false),
        "a previous attempt's start must not time out the newly enrolled retry");
    fixture.phase(next, "exec-started", START + 5_000);
    // A later-replicating old exit must not stop this attempt's clock.
    fixture.phase(first, "exec-exited", START + 100);
    fixture.at(START + 6_001);
    fixture.phase(next, "execution-timeout", START + 6_001);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(true));
}

#[test]
fn materialization_records_dispatched_phase_once_per_attempt() {
    let fixture = Fixture::new();
    fixture.evaluate();
    fixture.evaluate();
    let view = fixture.view();
    let claims = fixture.store.claims_for(&view.subject, Some(PHASE_KIND)).unwrap();
    let dispatched = claims.iter().filter(|claim| claim.body["fields"]["phase"] == "dispatched")
        .collect::<Vec<_>>();
    assert_eq!(dispatched.len(), 1);
    let fields = &dispatched[0].body["fields"];
    assert_eq!(fields["attempt"], json!(view.attempt));
    assert_eq!(fields["exec"], fixture.exec);
    assert_eq!(fields["at_unix_ms"], json!(START));
    assert!(fields["host"].as_str().is_some_and(|host| !host.is_empty()));
}

#[test]
fn missing_result_expires_replication_budget_without_execution_timeout() {
    let fixture = Fixture::new();
    let attempt = fixture.view().attempt;
    fixture.phase(attempt, "exec-started", START);
    fixture.at(START + 100);
    fixture.phase(attempt, "exec-exited", START + 100);
    fixture.at(START + 1_101);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false));
    assert!(fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    assert_eq!(fixture.view().status, "failed");
    let stalled = fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .into_iter().find(|claim| claim.body["fields"]["phase"] == "dispatch-stalled")
        .expect("missing result must retain a dispatch-stalled phase");
    assert!(stalled.body["fields"]["reason"].as_str().unwrap().contains("result replication"));
}

#[test]
fn replicated_result_records_completion_subphase_without_restarting_execution_clock() {
    let fixture = Fixture::new();
    let attempt = fixture.view().attempt;
    fixture.phase(attempt, "exec-started", START);
    fixture.at(START + 100);
    fixture.phase(attempt, "exec-exited", START + 100);
    fixture.at(START + 500);
    fixture.store.append_claim(&ClaimInput {
        subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
        fields: BTreeMap::from([
            ("status".into(), json!("exited")), ("exit_code".into(), json!(0)),
            ("incarnation_id".into(), json!("physical-exec")),
            ("start_operation".into(), json!(format!("operation-{attempt}"))),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    let claims = fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap();
    let results = claims.iter().filter(|claim| claim.body["fields"]["phase"] == "result-replicated")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].body["fields"]["attempt"], json!(attempt));
    assert_eq!(results[0].body["fields"]["exec"], fixture.exec);
    assert_eq!(results[0].body["fields"]["at_unix_ms"], json!(START + 500));
    fixture.at(START + 5_000);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false));
    assert!(!fixture.timed_out());
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap(),
        "a replicated result must close delivery's deadline, including the overall guard");
    assert!(!fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .iter().any(|claim| claim.body["fields"]["phase"] == "dispatch-stalled"));
}

#[test]
fn missing_remote_exit_is_replication_uncertainty_not_execution_timeout() {
    let fixture = Fixture::new();
    fixture.phase(fixture.view().attempt, "exec-started", START);
    fixture.at(START + 1_500);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false));
    assert!(!fixture.timed_out(), "a delayed exit claim cannot prove the remote process is running");
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    fixture.at(START + 3_001);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false));
    assert!(fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    assert_eq!(fixture.view().status, "failed");
    let stalled = fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .into_iter().find(|claim| claim.body["fields"]["phase"] == "dispatch-stalled")
        .expect("uncertain remote completion must eventually exhaust the overall guard");
    assert!(stalled.body["fields"]["reason"].as_str().unwrap().contains("result replication"));
}

fn executor(fixture: &Fixture, started: u128) -> (Reconciler<FakeRuntime>, DesiredSubject, RuntimeObservation) {
    let desired = fixture.store.desired_subjects_for_owner_step(&fixture.view().subject).unwrap()
        .into_iter().find(|desired| desired.subject == fixture.exec).unwrap();
    let member = desired.member.as_ref().unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    runtime.exec_started.lock().unwrap().insert((member.runtime_id.clone(), "physical-exec".into()), started);
    runtime.start_operations.lock().unwrap().insert(
        (member.runtime_id.clone(), "physical-exec".into()), format!("operation-{}", fixture.view().attempt));
    fixture.store.append_claim(&ClaimInput {
        subject: desired.subject.clone(), kind: "runtime.action.succeeded".into(), actor: None,
        fields: BTreeMap::from([
            ("action".into(), json!("start")), ("operation".into(), json!(format!("operation-{}", fixture.view().attempt))),
            ("incarnation_id".into(), json!("physical-exec")),
            ("desired_token".into(), json!(fixture.store.selected_desired_token(&desired.subject).unwrap().unwrap())),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    runtime.exec_payloads.lock().unwrap().insert(
        (member.runtime_id.clone(), "physical-exec".into()),
        fractal_runtime::payload_receipt::PayloadState::Running,
    );
    let observation = RuntimeObservation { runtime_id: member.runtime_id.clone(), terminal: false,
        status: "running".into(), exit_code: None, incarnation_id: Some("physical-exec".into()) };
    let reconciler = Reconciler::new(fixture.store.clone(), runtime, "worker".into(), Arc::new(Notify::new()));
    (reconciler, desired, observation)
}

#[test]
fn executor_records_physical_start_and_payload_exit_before_result_delivery() {
    let fixture = Fixture::new();
    fixture.at(START + 1_500);
    let (executor, desired, observation) = executor(&fixture, START + 1_500);
    executor.record_native_exec_phase(&desired, "received", START + 1_400, None).unwrap();
    executor.record_native_exec_phase(&desired, "admitted", START + 1_450, None).unwrap();
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    fixture.at(START + 2_500);
    fixture.store.append_claim(&ClaimInput {
        subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
        fields: BTreeMap::from([("status".into(), json!("exited")), ("exit_code".into(), json!(0)),
            ("incarnation_id".into(), json!("physical-exec")),
            ("start_operation".into(), json!(format!("operation-{}", fixture.view().attempt))),
            ("exec_exited_at_unix_ms".into(), json!(START + 1_515))]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    assert!(!fixture.timed_out());
    fixture.evaluate();
    let claims = fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap();
    for phase in ["dispatched", "received", "admitted", "exec-started", "exec-exited", "result-replicated"] {
        assert!(claims.iter().any(|claim| claim.body["fields"]["phase"] == phase), "{phase}");
    }
    let exited = claims.iter().find(|claim| claim.body["fields"]["phase"] == "exec-exited").unwrap();
    assert_eq!(exited.body["fields"]["at_unix_ms"], json!(START + 1_515));
    assert_eq!(fixture.view().execution_elapsed_ms, 15);
    assert_eq!(fixture.view().status, "completed");
}

#[test]
fn only_executor_live_registry_can_attest_a_running_execution_timeout() {
    let fixture = Fixture::new();
    let (executor, desired, observation) = executor(&fixture, START);
    fixture.at(START + 1_001);
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    assert!(fixture.timed_out());
    fixture.evaluate();
    assert_eq!(fixture.view().status, "failed");
    assert!(fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .iter().any(|claim| claim.body["fields"]["phase"] == "execution-timeout"));
}

#[test]
fn completed_delivery_does_not_expire_during_review_or_product_wait() {
    for status in ["verifying", "blocked"] {
        let fixture = Fixture::new();
        let attempt = fixture.view().attempt;
        fixture.phase(attempt, "exec-started", START);
        fixture.at(START + 100);
        fixture.phase(attempt, "exec-exited", START + 100);
        fixture.store.append_claim(&ClaimInput {
            subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
            fields: BTreeMap::from([
                ("status".into(), json!("exited")), ("exit_code".into(), json!(0)),
                ("incarnation_id".into(), json!("physical-exec")),
                ("start_operation".into(), json!(format!("operation-{attempt}"))),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
        fixture.store.set_step_state(&fixture.view().subject, status, None).unwrap();
        fixture.at(START + 10_000);
        assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap(),
            "{status} is not pending native delivery");
        assert!(!fixture.timed_out());
        assert_eq!(fixture.view().status, status);
        assert!(!fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
            .iter().any(|claim| claim.body["fields"]["phase"] == "dispatch-stalled"));
    }
}

#[test]
fn completed_delivery_does_not_hide_overbudget_execution_during_review() {
    let fixture = Fixture::new();
    let attempt = fixture.view().attempt;
    fixture.phase(attempt, "exec-started", START);
    fixture.at(START + 1_001);
    fixture.phase(attempt, "exec-exited", START + 1_001);
    fixture.store.append_claim(&ClaimInput {
        subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
        fields: BTreeMap::from([
            ("status".into(), json!("exited")), ("exit_code".into(), json!(0)),
            ("incarnation_id".into(), json!("physical-exec")),
            ("start_operation".into(), json!(format!("operation-{attempt}"))),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    fixture.store.set_step_state(&fixture.view().subject, "verifying", None).unwrap();
    fixture.at(START + 10_000);
    assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
    assert!(fixture.timed_out(), "delivery completion cannot forgive physical execution overrun");
    assert!(!fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .iter().any(|claim| claim.body["fields"]["phase"] == "dispatch-stalled"));
}

#[test]
fn upgrade_retains_legacy_deadline_without_enrolling_new_delivery_guards() {
    for status in ["ready", "working", "claimed", "blocked"] {
        for wrapper_status in ["running", "exited"] {
            let fixture = Fixture::with_legacy_state(Some(status));
            assert!(fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap().is_empty());
            let (executor, desired, mut observation) = executor(&fixture, START);
            observation.status = wrapper_status.into();
            observation.exit_code = (wrapper_status == "exited").then_some(0);
            fixture.store.append_claim(&ClaimInput {
                subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!(wrapper_status)),
                    ("exit_code".into(), json!(observation.exit_code)),
                ]),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
            fixture.at(START + 500);
            executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
            assert!(!fixture.timed_out(), "{status}/{wrapper_status} must retain its pre-upgrade timeout semantics");
            assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());

            // Explicitly exercise rematerialization: it must not retroactively dispatch
            // an attempt whose declarations were already present before the upgrade.
            let steps = flatten_mission_steps(&fixture.mission);
            let step = steps.iter().find(|step| step.spec.path == "work").unwrap();
            fixture.reconciler.materialize_step_declarations(
                &fixture.run, &fixture.mission, step, &fixture.view(), false,
            ).unwrap();
            executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
            fixture.at(START + 999);
            assert!(!fixture.timed_out(), "upgrade must not shorten the original execution budget");
            assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
            assert!(fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap().is_empty(),
                "{status}/{wrapper_status}: upgrade must not invent enrollment, execution, or stall phases");
            assert_eq!(fixture.view().status, status);
            fixture.at(START + 1_001);
            if matches!(status, "working" | "claimed") {
                assert!(fixture.timed_out(), "active legacy work must expire at its original bound");
            } else {
                // The legacy clock is paused while ready/blocked, not disabled by upgrade.
                assert!(!fixture.timed_out());
                fixture.store.set_step_state(&fixture.view().subject, "working", None).unwrap();
                fixture.at(START + 2_002);
                assert!(fixture.timed_out(), "resumed legacy work must remain bounded");
            }
        }
    }
}

#[test]
fn unknown_legacy_exit_timestamp_is_not_replaced_by_observation_time() {
    let fixture = Fixture::new();
    let (executor, desired, mut observation) = executor(&fixture, START);
    observation.status = "exited".into();
    observation.exit_code = Some(0);
    executor.runtime.exec_payloads.lock().unwrap().remove(&(
        desired.member.as_ref().unwrap().runtime_id.clone(), "physical-exec".into(),
    ));
    fixture.store.append_claim(&ClaimInput {
        subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
        fields: BTreeMap::from([
            ("status".into(), json!("exited")), ("exit_code".into(), json!(0)),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    fixture.at(START + 10_000);
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    assert!(!fixture.timed_out(), "observing an old exited wrapper cannot prove an execution overrun");
    assert!(!fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .iter().any(|claim| matches!(claim.body["fields"]["phase"].as_str(),
            Some("exec-exited" | "execution-timeout"))),
        "an unknown physical exit timestamp must stay unknown");
}

#[test]
fn persisted_payload_exit_survives_outage_with_stale_running_observation() {
    let fixture = Fixture::new();
    let (executor, desired, observation) = executor(&fixture, START);
    executor.runtime.exec_payloads.lock().unwrap().insert(
        (desired.member.as_ref().unwrap().runtime_id.clone(), "physical-exec".into()),
        fractal_runtime::payload_receipt::PayloadState::Exited {
            at_unix_ms: (START + 100) as u64,
        },
    );
    fixture.at(START + 5_000);
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    assert!(!fixture.timed_out(), "daemon outage must not count as payload execution");
    let claims = fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap();
    let exited = claims.iter().find(|claim| claim.body["fields"]["phase"] == "exec-exited")
        .expect("the persisted payload receipt must close execution");
    assert_eq!(exited.body["fields"]["at_unix_ms"], json!(START + 100));
    assert!(!claims.iter().any(|claim| claim.body["fields"]["phase"] == "execution-timeout"));
}

#[test]
fn legacy_running_wrapper_without_payload_receipt_cannot_attest_execution_timeout() {
    let fixture = Fixture::new();
    let (executor, desired, observation) = executor(&fixture, START);
    executor.runtime.exec_payloads.lock().unwrap().remove(&(
        desired.member.as_ref().unwrap().runtime_id.clone(), "physical-exec".into(),
    ));
    fixture.at(START + 5_000);
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    assert!(!fixture.timed_out(), "wrapper liveness alone cannot attest payload liveness");
    assert!(!fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .iter().any(|claim| matches!(claim.body["fields"]["phase"].as_str(),
            Some("exec-exited" | "execution-timeout"))));
}

#[test]
fn retry_cannot_use_a_previous_exited_actual_to_close_current_delivery() {
    for mismatch in ["unstarted", "operation", "incarnation"] {
        let fixture = Fixture::new();
        let first = fixture.view().attempt;
        fixture.phase(first, "exec-started", START);
        fixture.at(START + 100);
        fixture.store.append_claim(&ClaimInput {
            subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
            fields: BTreeMap::from([
                ("status".into(), json!("exited")), ("exit_code".into(), json!(0)),
                ("incarnation_id".into(), json!("physical-exec")),
                ("start_operation".into(), json!(format!("operation-{first}"))),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        fixture.store.set_step_state(&fixture.view().subject, "failed", Some("retry fixture")).unwrap();
        fixture.store.retry_step(&fixture.view().subject, "next attempt", 0).unwrap();
        let steps = flatten_mission_steps(&fixture.mission);
        let step = steps.iter().find(|step| step.spec.path == "work").unwrap();
        fixture.reconciler.materialize_step_declarations(
            &fixture.run, &fixture.mission, step, &fixture.view(), false,
        ).unwrap();
        fixture.store.set_step_state(&fixture.view().subject, "working", None).unwrap();
        let next = fixture.view().attempt;
        if mismatch != "unstarted" {
            fixture.phase(next, "exec-started", START + 100);
        }
        if mismatch == "incarnation" {
            fixture.store.append_claim(&ClaimInput {
                subject: fixture.exec.clone(), kind: "runtime.observed".into(), actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!("exited")), ("exit_code".into(), json!(0)),
                    ("incarnation_id".into(), json!("previous-physical-exec")),
                    ("start_operation".into(), json!(format!("operation-{next}"))),
                ]),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        assert!(!fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
        assert!(!fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
            .iter().any(|claim| claim.body["fields"]["attempt"] == json!(next)
                && claim.body["fields"]["phase"] == "result-replicated"), "{mismatch}");
        fixture.at(START + if mismatch == "unstarted" { 1_101 } else { 3_101 });
        assert!(fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap(),
            "{mismatch}: stale actual must not disable the current attempt's guard");
        assert_eq!(fixture.view().status, "failed");
    }
}

#[test]
fn retry_does_not_relabel_a_previous_physical_start_as_current_execution() {
    let fixture = Fixture::new();
    let (previous_executor, desired, mut observation) = executor(&fixture, START);
    previous_executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    fixture.at(START + 100);
    observation.status = "exited".into();
    fixture.store.set_step_state(&fixture.view().subject, "failed", Some("retry fixture")).unwrap();
    fixture.store.retry_step(&fixture.view().subject, "next attempt", 0).unwrap();
    let steps = flatten_mission_steps(&fixture.mission);
    let step = steps.iter().find(|step| step.spec.path == "work").unwrap();
    fixture.reconciler.materialize_step_declarations(
        &fixture.run, &fixture.mission, step, &fixture.view(), false,
    ).unwrap();
    previous_executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    let next = fixture.view().attempt;
    assert!(!fixture.store.claims_for(&fixture.view().subject, Some(PHASE_KIND)).unwrap()
        .iter().any(|claim| claim.body["fields"]["attempt"] == json!(next)
            && claim.body["fields"]["phase"] == "exec-started"),
        "the old operation's launch receipt does not authorize execution in the retry");
    fixture.at(START + 1_101);
    assert!(fixture.reconciler.expire_native_dispatch_budget(&fixture.view(), TIMEOUT).unwrap());
}

#[test]
fn explicit_wait_is_excluded_from_executor_attestation_and_completed_duration() {
    let fixture = Fixture::new();
    let (executor, desired, observation) = executor(&fixture, START);
    let member = desired.member.as_ref().unwrap();
    executor.record_native_exec_observation(&desired, member, &observation).unwrap();
    let progress = |clock: &str, at: u128| {
        fixture.at(at);
        fixture.store.append_claim(&ClaimInput {
            subject: fixture.view().subject, kind: "work.progress".into(), actor: None,
            fields: BTreeMap::from([
                ("attempt".into(), json!(fixture.view().attempt)),
                ("clock".into(), json!(clock)),
                ("wait_until_unix_ms".into(), json!(START + 2_000)),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    };
    progress("paused", START + 100);
    fixture.at(START + 1_500);
    executor.record_native_exec_observation(&desired, member, &observation).unwrap();
    assert!(!fixture.timed_out(), "a live declared wait must not attest overrun");
    assert_eq!(fixture.view().execution_elapsed_ms, 100);
    assert_eq!(fixture.view().execution_waited_ms, 1_400);
    progress("running", START + 1_600);
    fixture.at(START + 1_800);
    fixture.phase(fixture.view().attempt, "exec-exited", START + 1_800);
    assert!(!fixture.timed_out(), "completed duration must also exclude declared waits");
    assert_eq!(fixture.view().execution_elapsed_ms, 300);
    assert_eq!(fixture.view().execution_waited_ms, 1_500);
}

#[test]
fn unresumed_native_wait_expires_and_execution_can_still_time_out() {
    let fixture = Fixture::new();
    let (executor, desired, observation) = executor(&fixture, START);
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    fixture.at(START + 100);
    fixture.store.append_claim(&ClaimInput {
        subject: fixture.view().subject, kind: "work.progress".into(), actor: None,
        fields: BTreeMap::from([
            ("attempt".into(), json!(fixture.view().attempt)),
            ("clock".into(), json!("paused")),
            ("wait_until_unix_ms".into(), json!(START + 1_500)),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    fixture.at(START + 2_401);
    executor.record_native_exec_observation(&desired, desired.member.as_ref().unwrap(), &observation).unwrap();
    assert!(fixture.timed_out(), "only the bounded wait is free, not subsequent execution");
}
