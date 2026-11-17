//! A driverless one-shot can claim its step before ordinary observation catches up.
use super::*;

#[test]
fn member_environment_discards_stale_incarnation_aliases() {
    let inherited = BTreeMap::from([
        ("PATH".into(), "/bin".into()),
        ("ST3_INCARNATION".into(), "inherited-old".into()),
        ("FRACTAL_INCARNATION".into(), "inherited-new".into()),
    ]);
    let declared = BTreeMap::from([("FRACTAL_INCARNATION".into(), "declared-stale".into())]);
    let environment = member_environment_from(inherited, &declared, Path::new("/bin/fractal"), None).unwrap();
    assert!(!environment.contains_key("ST3_INCARNATION"));
    assert!(!environment.contains_key("FRACTAL_INCARNATION"));
}

#[test]
fn native_ordinary_observation_cannot_admit_an_unreleased_launch() {
    let root = tempfile::tempdir().unwrap();
    let runtime = NativeRuntime::new(root.path(), None, Path::new("unused-pty"));
    let operation = "held-native-observation";
    let gate = fractal_runtime::NativeLaunchGate::prepare(&runtime.launch_gate_root, operation).unwrap();
    // A second gate holds the payload after release, without a timing delay.
    let payload = fractal_runtime::NativeLaunchGate::prepare(&root.path().join("payload"), "payload").unwrap();
    let mut launch = gate.wrap(&payload.wrap(&fractal_runtime::Launch::Shell("exit 0".into())));
    let declared = BTreeMap::from([(start_jobs::OPERATION_ENV.into(), operation.into())]);
    let mut environment = member_environment(&declared, &launch_executable().unwrap(), None).unwrap();
    set_lane(&mut environment, None);
    if let fractal_runtime::Launch::Argv(argv) = &mut launch {
        argv[0] = fractal_runtime::resolve_executable(&argv[0], &environment).unwrap()
            .to_string_lossy().into_owned();
    }
    let generation = runtime.exec.spawn("held-native", &launch, root.path(), &environment).unwrap();
    struct Cleanup<'a>(&'a NativeRuntime, &'a str);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            self.0.exec.stop_if("held-native", Some(self.1)).unwrap();
        }
    }
    let _cleanup = Cleanup(&runtime, &generation.generation_id);
    let ordinary = runtime.observe_exec("held-native").unwrap().unwrap();
    assert_eq!(ordinary.status, "starting", "native exec log: {:?}",
        runtime.read_exec_log("held-native").unwrap());
    let physical = runtime.observe_native_exec("held-native", true).unwrap().unwrap();
    assert_eq!(physical.status, "running");
    assert_eq!(physical.incarnation_id.as_deref(), Some(generation.generation_id.as_str()));
    fractal_runtime::LaunchRelease::release(Box::new(gate), &generation.generation_id).unwrap();
    assert_eq!(runtime.observe_exec("held-native").unwrap().unwrap().status, "running");
    drop(payload);
}

#[derive(Clone, Copy)]
enum Registration {
    Accept,
    Revoke,
    Supersede,
    NotRunning,
    WriteFailure,
}

#[derive(Default)]
struct LaunchState {
    member: Option<MemberSpec>,
    physical: Option<RuntimeObservation>,
    registration_reads: usize,
    released: bool,
    cancelled: bool,
    ordinary_observation: bool,
    stopped: Vec<(String, Option<String>)>,
}

struct OneShotRuntime {
    store: Arc<Store>,
    state: Arc<Mutex<LaunchState>>,
    registration: Registration,
    agent: String,
    step: String,
}

impl OneShotRuntime {
    fn allocate(&self, member: &MemberSpec) -> Box<dyn fractal_runtime::LaunchRelease> {
        assert!(member.driver.is_none());
        let mut state = self.state.lock().unwrap();
        assert!(state.member.is_none(), "one-shot must not relaunch");
        state.member = Some(member.clone());
        state.physical = Some(RuntimeObservation {
            runtime_id: member.runtime_id.clone(), terminal: member.terminal,
            status: "running".into(), exit_code: None,
            incarnation_id: Some("held-one-shot-incarnation".into()),
        });
        Box::new(OneShotCommand {
            store: self.store.clone(), state: self.state.clone(), agent: self.agent.clone(),
            step: self.step.clone(), released: false,
        })
    }
}

struct OneShotCommand {
    store: Arc<Store>,
    state: Arc<Mutex<LaunchState>>,
    agent: String,
    step: String,
    released: bool,
}

impl fractal_runtime::LaunchRelease for OneShotCommand {
    fn release(mut self: Box<Self>, incarnation: &str) -> Result<()> {
        {
            let state = self.state.lock().unwrap();
            assert!(!state.ordinary_observation, "ordinary observation must not publish authority");
            assert_eq!(state.physical.as_ref().unwrap().incarnation_id.as_deref(), Some(incarnation));
        }
        // This is the command's first action, not a subsequent planner observation.
        anyhow::ensure!(
            self.store.work_incarnation_live(&self.agent, incarnation)?,
            "the command's exact incarnation has no published running work authority"
        );
        let actual = self.store.latest_actual_value(&self.agent)?.unwrap();
        assert_eq!(actual_field(&actual, "status").and_then(Value::as_str), Some("running"));
        assert_eq!(actual_field(&actual, "incarnation_id").and_then(Value::as_str), Some(incarnation));
        for action in ["claim", "complete"] {
            let work = self.store.work_action(&self.step, action, &crate::model::WorkRequest {
                wait_ms: None,
                actor: Some(self.agent.clone()),
                incarnation: Some(incarnation.into()),
                summary: (action == "complete").then(|| "one-shot completed immediately".into()),
                reason: None,
                evidence: Vec::new(),
                idempotency_key: format!("one-shot-{action}"),
            })?;
            if action == "claim" {
                assert_eq!(work.claim_incarnation.as_deref(), Some(incarnation));
            } else {
                assert!(work.worker_reported);
                assert_eq!(work.status, "verifying");
            }
        }
        self.released = true;
        let mut state = self.state.lock().unwrap();
        state.released = true;
        let physical = state.physical.as_mut().unwrap();
        physical.status = "exited".into();
        physical.exit_code = Some(0);
        Ok(())
    }
}

impl Drop for OneShotCommand {
    fn drop(&mut self) {
        if !self.released {
            self.state.lock().unwrap().cancelled = true;
        }
    }
}

impl RuntimeControl for OneShotRuntime {
    fn snapshot_ptys(&self) -> Result<Vec<RuntimeObservation>> {
        let state = self.state.lock().unwrap();
        Ok(if state.ordinary_observation {
            state.physical.iter().cloned().collect()
        } else {
            Vec::new()
        })
    }

    fn observe_exec(&self, _id: &str) -> Result<Option<RuntimeObservation>> {
        Ok(None)
    }

    fn observe_start(&self, _member: &MemberSpec) -> Result<Option<RuntimeObservation>> {
        let (read, operation) = {
            let mut state = self.state.lock().unwrap();
            state.registration_reads += 1;
            (state.registration_reads, state.member.as_ref().unwrap().tags[start_jobs::OPERATION_TAG].clone())
        };
        if read == 2 {
            match self.registration {
                Registration::Accept => {}
                Registration::Revoke => {
                    let request = self.store.append_claim(&ClaimInput {
                        subject: self.agent.clone(), kind: "runtime.action.requested".into(), actor: None,
                        fields: BTreeMap::from([("action".into(), Value::from("terminate"))]),
                        evidence: Vec::new(), expected_subject: None,
                        idempotency_key: Some("refuse-one-shot-registration".into()),
                    })?;
                    self.store.revoke_runtime_start(&operation, &request.id)?;
                }
                Registration::Supersede => {
                    apply_source(&self.store, &format!("version 2\nstop {:?}\n", self.agent), "supersede-held-one-shot");
                }
                Registration::NotRunning => {
                    self.state.lock().unwrap().physical.as_mut().unwrap().status = "starting".into();
                }
                Registration::WriteFailure => {
                    self.store.connection.write().execute_batch(
                        "CREATE TRIGGER refuse_one_shot_registration BEFORE INSERT ON claims
                         WHEN (NEW.kind='runtime.observed' AND json_extract(NEW.body,'$.fields.status')='running')
                           OR (NEW.kind='runtime.action.requested' AND json_extract(NEW.body,'$.fields.action')='terminate')
                         BEGIN SELECT RAISE(ABORT,'fixture registration write refused'); END;"
                    )?;
                }
            }
        }
        Ok(self.state.lock().unwrap().physical.clone())
    }

    fn start(&self, member: &MemberSpec) -> Result<()> {
        // The unfenced backend executes immediately. On the old path this reaches
        // the same command before registration and fails its real liveness check.
        self.allocate(member).release("held-one-shot-incarnation")
    }

    fn start_held(&self, member: &MemberSpec, guard: &dyn Fn() -> Result<()>) -> Result<Option<Box<dyn fractal_runtime::LaunchRelease>>> {
        guard()?;
        Ok(Some(self.allocate(member)))
    }

    fn start_process_identity(&self, member: &MemberSpec, incarnation: &str) -> Result<Option<(u32, u64)>> {
        let state = self.state.lock().unwrap();
        anyhow::ensure!(state.physical.as_ref().is_some_and(|physical| {
            physical.runtime_id == member.runtime_id
                && physical.incarnation_id.as_deref() == Some(incarnation)
        }), "physical identity requires the exact held runtime");
        let pid = std::process::id();
        Ok(Some((pid, fractal_runtime::process_start_token(pid)?)))
    }

    fn start_operation(&self, id: &str, _terminal: bool, incarnation: &str) -> Result<Option<String>> {
        let state = self.state.lock().unwrap();
        Ok(state.member.as_ref().filter(|member| member.runtime_id == id)
            .filter(|_| incarnation == "held-one-shot-incarnation")
            .map(|member| member.tags[start_jobs::OPERATION_TAG].clone()))
    }

    fn start_operation_closed(&self, _member: &MemberSpec) -> Result<bool> {
        Ok(self.state.lock().unwrap().physical.as_ref().is_none_or(|physical| physical.status == "exited"))
    }

    fn stop(&self, id: &str, _terminal: bool, incarnation: Option<&str>) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        let physical = state.physical.as_ref().unwrap();
        assert_eq!(physical.runtime_id, id);
        assert_eq!(physical.incarnation_id.as_deref(), incarnation);
        state.stopped.push((id.into(), incarnation.map(str::to_owned)));
        state.physical = None;
        Ok(())
    }

    fn stop_with_retirement_proof(&self, id: &str, terminal: bool, incarnation: &str) -> Result<bool> {
        self.stop(id, terminal, Some(incarnation))?;
        Ok(true)
    }

    fn kill(&self, _id: &str, _terminal: bool, _incarnation: Option<&str>) -> Result<()> {
        anyhow::bail!("exact stop already proves retirement")
    }
    fn remove(&self, _id: &str, _terminal: bool) -> Result<()> { Ok(()) }
    fn screen(&self, _id: &str) -> Result<String> { Ok(String::new()) }
    fn send_key(&self, _id: &str, _key: &str) -> Result<()> { anyhow::bail!("driverless command needs no terminal input") }
    fn read_exec_log(&self, _id: &str) -> Result<Option<String>> { Ok(None) }
}

fn fixture(registration: Registration) -> (Arc<Store>, Arc<OneShotRuntime>, Reconciler<OneShotRuntime>, String) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, r#"version 2
mission "immediate-one-shot" state="ready" {
  goal "Claim and finish before ordinary observation."
  completion { when "all-steps-exhausted" }
  step "judge" {
    assigned-to "agent/${ST_MISSION_RUN}/judge"
    agent "judge" {
      workspace "/tmp"
      command "true"
      one-shot
    }
  }
}
"#, "immediate-one-shot-mission");
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "immediate-one-shot".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/test".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "immediate-one-shot-run".into(),
    }).unwrap();
    let runtime = Arc::new(OneShotRuntime {
        store: store.clone(), state: Arc::new(Mutex::new(LaunchState::default())), registration,
        agent: format!("agent/{}/judge", run.id), step: run.steps[0].subject.clone(),
    });
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
    (store, runtime, reconciler, run.id)
}

#[test]
fn driverless_step_one_shot_claims_and_completes_before_ordinary_observation() {
    let (store, runtime, reconciler, run) = fixture(Registration::Accept);
    // The test constructor joins each launch worker; these are causal planner passes,
    // not retries or a timed race. The first materializes the step's own assignment.
    reconciler.reconcile_once().unwrap();
    reconciler.reconcile_once().unwrap();
    let step = store.step_run(&runtime.step).unwrap().unwrap();
    assert!(step.worker_reported, "the immediate command did not complete: {:?}",
        store.observations_for(&runtime.agent, "runtime.action.failed").unwrap());
    assert!(matches!(step.status.as_str(), "verifying" | "completed"));
    {
        let mut state = runtime.state.lock().unwrap();
        assert!(state.released);
        assert!(!state.cancelled);
        assert!(!state.ordinary_observation);
        state.ordinary_observation = true;
    }
    // Only now may the planner observe the immediate exit and retire the one-shot.
    reconciler.reconcile_once().unwrap();
    reconciler.reconcile_once().unwrap();
    assert!(store.declaration_ended_by_stop(&runtime.agent).unwrap().is_some());
    let actual = store.latest_actual_value(&runtime.agent).unwrap().unwrap();
    assert_ne!(actual_field(&actual, "status").and_then(Value::as_str), Some("running"));
    assert_eq!(store.mission_run(&run).unwrap().unwrap().steps[0].status, "completed");
}

fn assert_registration_refusal(registration: Registration) {
    let (store, runtime, reconciler, _) = fixture(registration);
    reconciler.reconcile_once().unwrap();
    let registration_result = reconciler.reconcile_once();
    match registration {
        Registration::NotRunning => assert!(format!("{:#}", registration_result.unwrap_err())
            .contains("held native launch is not the current running start operation")),
        Registration::WriteFailure => assert!(format!("{:#}", registration_result.unwrap_err())
            .contains("fixture registration write refused")),
        _ => registration_result.unwrap(),
    }
    let state = runtime.state.lock().unwrap();
    assert!(!state.released, "refused registration must never execute the command");
    assert!(state.cancelled, "held command must be cancelled");
    assert!(state.physical.is_none(), "held physical runtime must not become an orphan");
    assert_eq!(state.stopped.len(), 1);
    assert_eq!(state.stopped[0].1.as_deref(), Some("held-one-shot-incarnation"));
    assert_ne!(store.step_run(&runtime.step).unwrap().unwrap().status, "completed");
    assert!(store.observations_for(&runtime.agent, "runtime.observed").unwrap().iter()
        .all(|claim| claim.body["fields"]["status"] != "running"));
}

#[test]
fn revoked_one_shot_registration_never_releases_and_retires_exact_runtime() {
    assert_registration_refusal(Registration::Revoke);
}

#[test]
fn superseded_one_shot_registration_never_releases_and_retires_exact_runtime() {
    assert_registration_refusal(Registration::Supersede);
}

#[test]
fn nonrunning_one_shot_registration_never_releases_and_retires_exact_runtime() {
    assert_registration_refusal(Registration::NotRunning);
}

#[test]
fn failed_one_shot_registration_write_never_releases_and_retires_exact_runtime() {
    assert_registration_refusal(Registration::WriteFailure);
}
