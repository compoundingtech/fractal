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
        // Mission evaluation materializes real run-owned desired execs without delivering
        // them to the worker or depending on process scheduling.
        for _ in 0..3 {
            let current = store.mission_run(&run.id).unwrap().unwrap();
            reconciler.evaluate_mission_run(&current, &mission).unwrap();
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
    fixture.store.set_step_state(&fixture.view().subject, "working", None).unwrap();
    let next = fixture.view().attempt;
    assert_eq!(next, first + 1);
    fixture.at(START + 5_000);
    fixture.phase(first, "execution-timeout", START + 1_001);
    assert_eq!(fixture.reconciler.native_step_timed_out(&fixture.view(), TIMEOUT).unwrap(), Some(false),
        "a previous attempt's start must not time out an undispatched retry");
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
