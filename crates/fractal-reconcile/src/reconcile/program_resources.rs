//! Effect resource calls lower to the same host membership and execution adapters as data steps.
use super::*;
use crate::store::resource_membership::{Admission, Coverage, Grant, LaneRequest, Membership, Owner};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Acquire {
    name: String,
    host: String,
    #[serde(default = "one")]
    max_members: u32,
    capacity: Option<fractal_runtime::Capacity>,
    coverage: Coverage,
    admission_timeout: Option<String>,
}
fn one() -> u32 { 1 }

#[derive(Default, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct CommandOptions {
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    timeout: Option<String>,
    priority: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Execute {
    membership: Grant,
    argv: Vec<String>,
    #[serde(default)]
    options: CommandOptions,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Child {
    membership: Grant,
    mission: String,
    input: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Release { membership: Grant }

fn failure(code: &str, message: impl Into<String>) -> Value {
    serde_json::json!({"_tag":"Failure","error":{"_tag":"ResourceError","code":code,"message":message.into()}})
}
fn success(value: impl serde::Serialize) -> Value {
    serde_json::json!({"_tag":"Success","value":value})
}

fn execution_id(request: &program::Request) -> String {
    format!("program-resource-{}", hex::encode(sha2::Sha256::digest(request.idempotency_key().as_bytes())))
}

/// Bind the durable request key to the caller's payload before any physical preparation.
/// Authoritative attempt env is intentionally not hashed: credentials rotate on host replay.
fn command_execution_id(base: &str, command: &Execute) -> Result<String> {
    let payload = serde_json::to_vec(&(&command.argv, &command.options))?;
    Ok(format!("{base}-{}", hex::encode(sha2::Sha256::digest(payload))))
}

fn command_root<'a>(
    roots: &'a [crate::store::resource_membership::ExecutionRoot],
    base: &str,
    expected: &str,
) -> Result<Option<&'a crate::store::resource_membership::ExecutionRoot>> {
    let prefix = format!("{base}-");
    let root = roots.iter().find(|root| root.runtime_id == base || root.runtime_id.starts_with(&prefix));
    anyhow::ensure!(root.is_none_or(|root| root.runtime_id == expected),
        "resource command payload changed for its durable key; execution custody retained");
    Ok(root)
}

/// Match spawnEffect: command options are defaults below the attempt's authoritative `run.env`.
fn command_environment(
    mut options: BTreeMap<String, String>,
    attempt: BTreeMap<String, String>,
    priority: Option<&str>,
) -> BTreeMap<String, String> {
    options.insert("ST_SEAT_PRIORITY".into(), priority.unwrap_or("normal").into());
    options.extend(attempt);
    options
}

impl<R: RuntimeControl> Reconciler<R> {
    /// A single authority epoch for this daemon's host, never one per program or membership.
    pub(super) fn ensure_resource_authority(&self) -> Result<()> {
        let initialized = self.resource_authority.get_or_init(|| {
            (|| -> Result<()> {
                self.store.start_resource_authority(&self.host, &uuid::Uuid::now_v7().to_string())?;
                self.runtime.recover_resource_lanes(&self.store, &self.host, u64::try_from(now_ms()).unwrap_or(u64::MAX))?;
                Ok(())
            })().map_err(|error| format!("resource authority recovery failed: {error:#}"))
        });
        initialized.as_ref().map_err(|reason| anyhow::anyhow!("{reason}"))?;
        Ok(())
    }

    fn program_membership(&self, run: &MissionRunView, request: &program::Request, grant: &Grant) -> Result<Membership> {
        let record = self.store.resource_membership_for_grant(grant)?;
        anyhow::ensure!(record.request.host == self.host, "resource membership does not belong to this execution host");
        let owns = record.request.owner.run == run.subject
            && record.request.owner.step == request.step && record.request.owner.attempt == request.attempt;
        let inherits = self.store.resource_context_for_run(&run.subject)?
            .is_some_and(|context| context.grant.as_ref() == Some(grant));
        anyhow::ensure!(owns || inherits, "resource membership belongs to another run, step or attempt");
        anyhow::ensure!(record.grant.as_ref() == Some(grant), "resource membership generation is stale");
        Ok(record)
    }

    fn resource_changed(&self, membership: &LaneRequest) {
        self.incremental.touch(&crate::store::resource_membership::dependency(&membership.host, &membership.name));
        self.incremental.touch("seat-admission");
        self.signal_changed();
    }

    fn retire_program_membership(&self, record: &Membership) -> Result<bool> {
        let mut retirement_errors = Vec::new();
        for child in &record.child_runs {
            smallclaims::touched::note_read(|| child.clone());
            let cancellation = (|| -> Result<()> {
                if !matches!(self.store.mission_run_status(child)?.as_deref(), None | Some("completed" | "failed" | "cancelled")) {
                    self.store.request_mission_run_cancellation(child, "its resource membership scope closed")?;
                }
                Ok(())
            })();
            if let Err(error) = cancellation { retirement_errors.push(format!("{child}: {error:#}")); }
        }
        // A child cancellation failure must not prevent cleanup of the remaining execution roots.
        let retired = match self.runtime.retire_resource_lane(&self.store, &record.request) {
            Ok(retired) => retired,
            Err(error) => { retirement_errors.push(format!("{error:#}")); false }
        };
        let current = self.store.resource_membership_record(&record.request.id, &record.request.owner)?;
        if retired || current.state != record.state { self.resource_changed(&record.request); }
        anyhow::ensure!(retirement_errors.is_empty(), "resource retirement failed: {}", retirement_errors.join("; "));
        anyhow::ensure!(current.state != "uncertain", "resource retirement is uncertain; host custody retained");
        Ok(retired)
    }

    pub(super) fn cancel_invalid_program_resources(&self) -> Result<()> {
        for record in self.store.resource_memberships_for_host(&self.host)? {
            if !record.request.owner.invocation.as_deref().is_some_and(|id| id.starts_with("program:"))
                || matches!(record.state.as_str(), "retired" | "refused") { continue; }
            let active = self.store.step_run(&record.request.owner.step)?.is_some_and(|step|
                step.attempt == record.request.owner.attempt && program::RUNNING_STATES.contains(&step.status.as_str()));
            let live = self.program_engine.get().is_some_and(|engine|
                engine.resource_owner_live(&record.request.owner.step, record.request.owner.attempt));
            if !active || !live { self.retire_program_membership(&record)?; }
        }
        Ok(())
    }

    pub(super) fn settle_program_resource(&self, engine: &program::ProgramEngine, run: &MissionRunView,
        request: &program::Request, operation: &str, payload: &Value, cancelled: bool) -> Result<bool>
    {
        self.ensure_resource_authority()?;
        match self.apply_program_resource(engine, run, request, operation, payload, cancelled) {
            Ok(changed) => Ok(changed),
            Err(error) => {
                engine.resolve_resource(request, failure("resource-operation-refused", format!("{error:#}")))?;
                Ok(true)
            }
        }
    }

    fn apply_program_resource(&self, engine: &program::ProgramEngine, run: &MissionRunView,
        request: &program::Request, operation: &str, payload: &Value, cancelled: bool) -> Result<bool>
    {
        let now = u64::try_from(now_ms()).unwrap_or(u64::MAX);
        match operation {
            "resource.lane.acquire" => {
                let options: Acquire = serde_json::from_value(payload.clone())?;
                anyhow::ensure!(options.host == self.host, "the resource execution host must be this daemon's host");
                let lane = LaneRequest {
                    id: request.idempotency_key(), name: options.name, host: options.host,
                    max_members: options.max_members, capacity: options.capacity, coverage: options.coverage,
                    admission_timeout_ms: options.admission_timeout.as_deref()
                        .map(|duration| crate::declaration_syntax::parse_duration(duration, true)).transpose()?,
                    owner: Owner { run: run.subject.clone(), step: request.step.clone(), attempt: request.attempt,
                        invocation: Some(format!("program:{}", request.key)) },
                };
                let fit = self.resource_lane_fit(&lane.id, lane.capacity)?;
                let admission = self.store.request_resource_membership(&lane, fit, now)?;
                if cancelled {
                    self.store.withdraw_resource_membership(&lane.id, &lane.owner, now)?;
                    let record = self.store.resource_membership_record(&lane.id, &lane.owner)?;
                    if !self.retire_program_membership(&record)? { return Ok(false); }
                    engine.resolve_resource(request, failure("cancelled", "Resource acquisition was withdrawn."))?;
                    return Ok(true);
                }
                match admission {
                    Admission::Admitted(grant) => {
                        self.resource_changed(&lane);
                        engine.resolve_resource(request, success(grant))?;
                        Ok(true)
                    }
                    Admission::Refused { code, reason } => {
                        self.resource_changed(&lane);
                        engine.resolve_resource(request, failure(code.as_str(), reason))?;
                        Ok(true)
                    }
                    Admission::Queued { .. } => {
                        if let Some(remaining) = self.store.resource_admission_remaining(&lane.id, &lane.owner, now)? {
                            self.arm_restart(&format!("program-admission:{}", lane.id), now_ms().saturating_add(u128::from(remaining)));
                        }
                        Ok(false)
                    }
                }
            }
            "resource.lane.release" => {
                let release: Release = serde_json::from_value(payload.clone())?;
                // Release remains idempotent after retirement; authorization is still exact.
                let record = self.store.resource_membership_for_grant(&release.membership)?;
                anyhow::ensure!(record.request.host == self.host, "resource membership belongs to another execution host");
                anyhow::ensure!(record.request.owner.run == run.subject && record.request.owner.step == request.step
                    && record.request.owner.attempt == request.attempt && record.grant.as_ref() == Some(&release.membership),
                    "release does not own the exact resource generation");
                if !self.retire_program_membership(&record)? { return Ok(false); }
                engine.resolve_resource(request, success(Value::Null))?;
                Ok(true)
            }
            "resource.lane.exec" => {
                let command: Execute = serde_json::from_value(payload.clone())?;
                let record = self.program_membership(run, request, &command.membership)?;
                if cancelled {
                    let base = execution_id(request);
                    let prefix = format!("{base}-");
                    for root in record.roots.iter().filter(|root| root.runtime_id == base || root.runtime_id.starts_with(&prefix)) {
                        self.runtime.retire_resource_exec(&self.store, &record.request, &root.runtime_id)?;
                        self.resource_changed(&record.request);
                    }
                    engine.resolve_resource(request, failure("cancelled", "Resource execution was cancelled."))?;
                    return Ok(true);
                }
                self.execute_program_resource(engine, run, request, record, command)
            }
            "resource.lane.run" => {
                let child: Child = serde_json::from_value(payload.clone())?;
                let record = self.program_membership(run, request, &child.membership)?;
                let child_id = self.store.mission_run_subject_for_idempotency_key(&request.idempotency_key());
                let child_id = self.store.mission_run_supersession_successor(&child_id)?;
                if cancelled {
                    smallclaims::touched::note_read(|| child_id.clone());
                    if !matches!(self.store.mission_run_status(&child_id)?.as_deref(), None | Some("completed" | "failed" | "cancelled")) {
                        self.store.request_mission_run_cancellation(&child_id, "its lane.run effect was interrupted")?;
                        return Ok(false);
                    }
                    engine.resolve_resource(request, failure("cancelled", "Linked resource execution was cancelled."))?;
                    return Ok(true);
                }
                anyhow::ensure!(matches!(self.store.resource_membership(&record.request.id, &record.request.owner, now)?,
                    Admission::Admitted(current) if current == child.membership), "resource scope is not live");
                let Some(status) = self.store.mission_run_status(&child_id)? else {
                    for holder in self.store.active_mission_runs_for_mission(&child.mission)? {
                        smallclaims::touched::note_read(|| holder.subject.clone());
                    }
                    // The shared creation boundary binds the child before it becomes runnable.
                    match self.start_program_child(run, &request.step, &child.mission, &child.input, request.idempotency_key(), Some(&record)) {
                        Ok(_) => {}
                        Err(error) if error.code == "mission-run-capacity" => return Ok(false),
                        Err(error) => return Err(error.into()),
                    }
                    self.resource_changed(&record.request);
                    return Ok(true);
                };
                self.store.attach_resource_child(&child.membership, &record.request.owner, &child_id)?;
                smallclaims::touched::note_read(|| child_id.clone());
                if !matches!(status.as_str(), "completed" | "failed" | "cancelled") { return Ok(false); }
                engine.resolve_resource(request, self.program_child_exit(&child_id, &status)?)?;
                Ok(true)
            }
            _ => anyhow::bail!("unsupported resource operation"),
        }
    }

    fn execute_program_resource(&self, engine: &program::ProgramEngine, run: &MissionRunView,
        request: &program::Request, record: Membership, command: Execute) -> Result<bool>
    {
        anyhow::ensure!(!command.argv.is_empty(), "resource command argv must contain a program");
        anyhow::ensure!(command.options.priority.as_deref().is_none_or(|priority| matches!(priority, "background")), "invalid command priority");
        anyhow::ensure!(matches!(self.store.resource_membership(&record.request.id, &record.request.owner,
            u64::try_from(now_ms()).unwrap_or(u64::MAX))?, Admission::Admitted(current) if current == command.membership),
            "resource scope is not live");
        let timeout = command.options.timeout.as_deref()
            .map(|duration| crate::declaration_syntax::parse_duration(duration, true)).transpose()?;
        let base = execution_id(request);
        let runtime_id = command_execution_id(&base, &command)?;
        let root = command_root(&record.roots, &base, &runtime_id)?;
        if root.is_none() {
            let environment = command_environment(
                command.options.env,
                engine.attempt_environment(&request.step, request.attempt)?,
                command.options.priority.as_deref(),
            );
            let mut argv = command.argv;
            if cfg!(target_os = "macos") && command.options.priority.as_deref() == Some("background") {
                argv.splice(0..0, ["/usr/sbin/taskpolicy".into(), "-b".into()]);
            }
            let member = MemberSpec {
                kind: MemberKind::Exec, host: self.host.clone(), runtime_id: runtime_id.clone(),
                workspace: run.workspace.clone(), workspace_create: false,
                cwd: command.options.cwd.unwrap_or_else(|| run.workspace.clone()), terminal: false,
                launch: LaunchSpec::Argv(argv), environment, tags: BTreeMap::new(), display_name: None,
                lifecycle: MemberLifecycle::Service, terminal_binding: None, one_shot: false,
                restart: RestartType::Never, restart_intensity: RestartIntensity::default(),
                shutdown_timeout_ms: 5_000, driver: None, terminal_size: None, capacity: None,
                declared_priority: None, declared_lane: None, declared_budgets: None,
            };
            let mut member = member;
            resource_lanes::set_request(&mut member, &record.request)?;
            self.runtime.start_resource_guarded(&self.store, &member, &|| {
                let current = self.store.step_run(&request.step)?.context("program step disappeared before payload release")?;
                anyhow::ensure!(current.attempt == request.attempt && program::RUNNING_STATES.contains(&current.status.as_str()),
                    "program attempt was cancelled before payload release");
                Ok(())
            })?;
            self.resource_changed(&record.request);
            return Ok(true);
        }
        let root = root.expect("root tested above");
        let generation = root.execution.as_ref().context("execution preparation is uncertain; custody retained")?;
        engine.watch_resource_once(request, &generation.generation_id, || {
            let incremental = self.incremental.clone();
            let notify = self.notify.clone();
            let event_notify = self.event_notify.clone();
            let step = request.step.clone();
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                incremental.touch(&step);
                signal_changed(&notify, &event_notify);
            });
            self.runtime.watch_resource_exec(&runtime_id, &generation.generation_id, wake)
        })?;
        let observation = self.runtime.observe_exec(&runtime_id)?.context("resource execution record disappeared")?;
        anyhow::ensure!(observation.incarnation_id.as_deref() == Some(generation.generation_id.as_str()), "resource execution generation changed");
        if observation.status == "running" {
            if let Some(timeout) = timeout {
                let elapsed = self.store.resource_execution_elapsed(&record.request.id, &record.request.owner,
                    &runtime_id, u64::try_from(now_ms()).unwrap_or(u64::MAX))?
                    .context("resource execution has no payload release clock")?;
                if elapsed >= timeout {
                    self.runtime.retire_resource_exec(&self.store, &record.request, &runtime_id)?;
                    self.resource_changed(&record.request);
                    engine.resolve_resource(request, failure("execution-timeout", "Resource command exceeded its payload budget."))?;
                    return Ok(true);
                }
                self.arm_restart(&format!("program-execution:{runtime_id}"), now_ms().saturating_add(u128::from(timeout.saturating_sub(elapsed))));
            }
            return Ok(false);
        }
        anyhow::ensure!(observation.status == "exited", "resource execution state is indeterminate; custody retained");
        let (stdout, stderr) = self.runtime.resource_exec_output(&runtime_id, &generation.generation_id)?;
        let signal = self.runtime.resource_exec_signal(&runtime_id, &generation.generation_id)?.map(|signal| match signal {
            libc::SIGTERM => "SIGTERM".to_owned(),
            libc::SIGKILL => "SIGKILL".to_owned(),
            other => other.to_string(),
        });
        self.store.end_resource_execution(&command.membership, &runtime_id, u64::try_from(now_ms()).unwrap_or(u64::MAX))?;
        engine.resolve_resource(request, success(serde_json::json!({
            "code": observation.exit_code.unwrap_or(128), "signal": signal, "stdout": stdout, "stderr": stderr
        })))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource_membership::Fit;


    #[test]
    fn native_command_adoption_rejects_changed_payload_for_the_same_key() {
        let command = |argv: &str, cwd: &str, value: &str| -> Execute {
            serde_json::from_value(serde_json::json!({
                "membership": {
                    "membershipId": "scope", "generation": "generation", "authorityEpoch": "epoch", "coverage": "logical"
                },
                "argv": ["sh", "-c", argv],
                "options": { "cwd": cwd, "env": { "VALUE": value } }
            })).unwrap()
        };
        let original = command("printf first", "/workspace", "first");
        let id = command_execution_id("request-key", &original).unwrap();
        let roots = vec![crate::store::resource_membership::ExecutionRoot {
            runtime_id: id.clone(), execution: None,
            payload_released_at: None, execution_ended_at: None,
        }];
        assert!(command_root(&roots, "request-key", &id).unwrap().is_some());
        for changed in [
            command("printf second", "/workspace", "first"),
            command("printf first", "/other", "first"),
            command("printf first", "/workspace", "second"),
        ] {
            let changed_id = command_execution_id("request-key", &changed).unwrap();
            assert!(command_root(&roots, "request-key", &changed_id).unwrap_err()
                .to_string().contains("payload changed"));
        }
        assert!(command_root(&roots, "other-key", "other-key-payload").unwrap().is_none());
    }
    #[test]
    fn native_command_inherits_authoritative_attempt_environment() {
        let attempt = BTreeMap::from([
            ("ST_MISSION".into(), "resource-mission".into()),
            ("ST_WORKSPACE".into(), "/workspace/resource".into()),
            ("ST_STEP".into(), "work".into()),
            ("ST_ATTEMPT".into(), "3".into()),
            ("ST_MISSION_RUN".into(), "run-id".into()),
            ("ST_STEP_RUN".into(), "step-run/resource/work".into()),
            ("ST_LOOP_ROUND".into(), "2".into()),
            ("ST_LOOP_FEEDBACK".into(), "retry with the recorded feedback".into()),
            ("ST_LOOP_ITEM_ID".into(), "item-4".into()),
            ("ST_CANDIDATE_INDEX".into(), "1".into()),
            ("CUSTOM_ATTEMPT_ENV".into(), "attempt-value".into()),
            ("TRACEPARENT".into(), "00-11111111111111111111111111111111-2222222222222222-01".into()),
            ("TRACESTATE".into(), "vendor=attempt".into()),
            (crate::store::run_credentials::RUN_CREDENTIAL_ENV.into(), "attempt-credential".into()),
        ]);
        let environment = command_environment(BTreeMap::new(), attempt.clone(), None);
        for (key, value) in &attempt {
            assert_eq!(environment.get(key), Some(value), "{key} must come from the attempt");
        }
        assert_eq!(environment["ST_SEAT_PRIORITY"], "normal");

        // A custom command value survives only when run.env does not define that key.
        // Identity, credential, loop and tracing values all obey the same spawn precedence.
        let mut options: BTreeMap<_, _> = attempt.keys()
            .map(|key| (key.clone(), "command-override".to_owned())).collect();
        options.insert("CUSTOM_COMMAND_ENV".into(), "custom-value".into());
        let environment = command_environment(options, attempt.clone(), Some("background"));
        for (key, value) in &attempt {
            assert_eq!(environment.get(key), Some(value), "{key} must not be overridden by command options");
        }
        assert_eq!(environment["CUSTOM_COMMAND_ENV"], "custom-value");
        assert_eq!(environment["ST_SEAT_PRIORITY"], "background");

        let mut attempt_priority = attempt;
        attempt_priority.insert("ST_SEAT_PRIORITY".into(), "normal".into());
        let environment = command_environment(BTreeMap::new(), attempt_priority, Some("background"));
        assert_eq!(environment["ST_SEAT_PRIORITY"], "normal", "run.env has the final word, as in spawnEffect");
    }

    #[test]
    fn scope_release_awaits_child_cancellation_and_reacts_to_terminal_run() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        super::super::tests::apply_source(&store, r#"version 2
mission "resource-child" state="ready" {
  goal "Stop when the enclosing scope closes."
  step "work" { agentless }
}"#, "resource-child-source");
        let runtime = Arc::new(NativeRuntime::new(root.path(), None, Path::new("unused-pty")));
        let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
        reconciler.ensure_resource_authority().unwrap();
        let lane = LaneRequest {
            id: "scope-holder".into(), name: "child-scope".into(), host: "node".into(),
            max_members: 1, capacity: None, coverage: Coverage::Logical, admission_timeout_ms: None,
            owner: Owner { run: "mission-run/parent".into(), step: "step-run/parent/work".into(),
                attempt: 1, invocation: None },
        };
        let Admission::Admitted(grant) = store.request_resource_membership(&lane, Fit::default(), 1).unwrap() else {
            panic!("scope must acquire custody");
        };
        let child = store.mission_run_subject_for_idempotency_key("scope-child");
        store.attach_resource_child(&grant, &lane.owner, &child).unwrap();
        store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "resource-child".into(), revision: None, workspace: root.path().display().to_string(),
            requester: Some("person/operator".into()), mode: None, inputs: BTreeMap::new(),
            idempotency_key: "scope-child".into(),
        }).unwrap();
        let record = store.resource_membership_for_grant(&grant).unwrap();
        let (result, reads) = smallclaims::touched::record(|| reconciler.retire_program_membership(&record));
        assert!(!result.unwrap(), "normal asynchronous cancellation must not refuse scope release");
        assert!(reads.contains(&child));
        let pending = store.resource_membership_for_grant(&grant).unwrap();
        assert_eq!(pending.state, "retiring");
        assert_eq!(pending.retired_at, None);
        let mut waiter = lane.clone();
        waiter.id = "scope-waiter".into();
        assert!(matches!(store.request_resource_membership(&waiter, Fit::default(), 2).unwrap(), Admission::Queued { .. }));
        reconciler.incremental.observe(&store).unwrap();
        reconciler.incremental.evaluated("scope-release", reads, None);
        assert!(!reconciler.incremental.needs("scope-release", now_ms()));
        for _ in 0..3 { reconciler.reconcile_once().unwrap(); }
        assert_eq!(store.mission_run_status(&child).unwrap().as_deref(), Some("cancelled"));
        reconciler.incremental.observe(&store).unwrap();
        assert!(reconciler.incremental.needs("scope-release", now_ms()), "terminal child must wake pending release");
        assert!(reconciler.retire_program_membership(&pending).unwrap());
        assert_eq!(store.resource_membership_for_grant(&grant).unwrap().state, "retired");
        assert!(matches!(store.request_resource_membership(&waiter, Fit::default(), 3).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn scope_release_refuses_unknown_child_without_releasing_custody() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        store.start_resource_authority("node", "epoch").unwrap();
        let lane = LaneRequest {
            id: "unknown-scope".into(), name: "child-scope".into(), host: "node".into(),
            max_members: 1, capacity: None, coverage: Coverage::Logical, admission_timeout_ms: None,
            owner: Owner { run: "mission-run/parent".into(), step: "step-run/parent/work".into(),
                attempt: 1, invocation: None },
        };
        let Admission::Admitted(grant) = store.request_resource_membership(&lane, Fit::default(), 1).unwrap() else {
            panic!("scope must acquire custody");
        };
        store.attach_resource_child(&grant, &lane.owner, "mission-run/missing-child").unwrap();
        let runtime = Arc::new(NativeRuntime::new(root.path(), None, Path::new("unused-pty")));
        let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
        let record = store.resource_membership_for_grant(&grant).unwrap();
        assert!(reconciler.retire_program_membership(&record).is_err());
        let uncertain = store.resource_membership_for_grant(&grant).unwrap();
        assert_eq!(uncertain.state, "uncertain");
        assert_eq!(uncertain.retired_at, None);
        let mut waiter = lane;
        waiter.id = "blocked-scope".into();
        assert!(matches!(store.request_resource_membership(&waiter, Fit::default(), 2).unwrap(), Admission::Queued { .. }));
    }
}
