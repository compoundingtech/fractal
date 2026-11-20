//! Effect resource calls lower to the same host membership and execution adapters as data steps.
use super::*;
use crate::store::resource_membership::{Admission, Coverage, Grant, LaneRequest, Membership, Owner};
use serde::Deserialize;
pub(super) const RETRY_RETIREMENT_FAULT_SCOPE: &str = "program/retry-resource-retirement";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Acquire {
    name: String,
    host: String,
    profile: Option<String>,
    disk_path: Option<String>,
    #[serde(default = "one")]
    max_members: u32,
    capacity: Option<fractal_runtime::Capacity>,
    coverage: Coverage,
    admission_timeout: Option<String>,
    #[serde(default)]
    class: fractal_runtime::RequestClass,
    disk: Option<fractal_runtime::DiskRequest>,
    io: Option<fractal_runtime::IoClass>,
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
                // Preserve released shell outcomes before generic predecessor recovery can
                // remove their cgroups or turn their memberships into admission refusals.
                self.prepare_shell_resource_recovery()?;
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
        record.request.class.ensure_supported()?;
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
        record.request.class.ensure_supported()?;
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

    /// Retire, never adopt, an earlier attempt's scopes before admitting the next attempt.
    /// Compare logical step paths as operator retry can reopen the run in a new generation.
    /// Inherited parent scopes are intentionally excluded: their owner is another run/step.
    pub(super) fn retire_previous_step_resources(
        &self,
        run: &str,
        view: &crate::model::StepRunView,
    ) -> Result<bool> {
        let mut ready = true;
        for record in self.store.resource_memberships_for_host(&self.host)? {
            let owner = &record.request.owner;
            if owner.run != run
                || (owner.step == view.subject && owner.attempt == view.attempt)
                || matches!(record.state.as_str(), "retired" | "refused" | "withdrawn")
            {
                continue;
            }
            let Some(previous) = self.store.step_run(&owner.step)? else { continue; };
            if previous.step != view.step { continue; }
            if let Err(error) = record.request.class.ensure_supported() {
                tracing::warn!(membership = %record.request.id, %error, "deferring unsupported previous-attempt resource retirement until upgrade");
                ready = false;
                continue;
            }
            self.ensure_resource_authority()?;
            // Record the dependency even while asynchronous child/root retirement is pending.
            smallclaims::touched::note_read(||
                crate::store::resource_membership::dependency(&record.request.host, &record.request.name));
            ready &= self.retire_program_membership(&record)?;
        }
        Ok(ready)
    }

    pub(super) fn cancel_invalid_program_resources(&self) -> Result<()> {
        for record in self.store.resource_memberships_for_host(&self.host)? {
            if !record.request.owner.invocation.as_deref().is_some_and(|id| id.starts_with("program:"))
                || matches!(record.state.as_str(), "retired" | "refused") { continue; }
            if let Err(error) = record.request.class.ensure_supported() {
                tracing::warn!(membership = %record.request.id, %error, "deferring unsupported program resource cancellation until upgrade");
                continue;
            }
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
            "agent.turn" => self.settle_program_turn(engine, run, request, payload, cancelled),
            "resource.lane.acquire" => {
                if !cancelled {
                    let view = self.store.step_run(&request.step)?.context("resource owner step missing")?;
                    anyhow::ensure!(view.attempt == request.attempt
                        && program::RUNNING_STATES.contains(&view.status.as_str()),
                        "resource owner left its attempt");
                    if !self.retire_previous_step_resources(&run.subject, &view)? { return Ok(false); }
                }
                let options: Acquire = serde_json::from_value(payload.clone())?;
                anyhow::ensure!(options.host == self.host, "the resource execution host must be this daemon's host");
                let mut lane = LaneRequest {
                    id: request.idempotency_key(), name: options.name, host: options.host,
                    max_members: options.max_members, capacity: options.capacity, coverage: options.coverage,
                    class: options.class, disk: options.disk, io: options.io,
                    profile: options.profile, profile_estimate: None, disk_path: options.disk_path,
                    admission_timeout_ms: options.admission_timeout.as_deref()
                        .map(|duration| crate::declaration_syntax::parse_duration(duration, true)).transpose()?,
                    owner: Owner { run: run.subject.clone(), step: request.step.clone(), attempt: request.attempt,
                        invocation: Some(format!("program:{}", request.key)) },
                };
                self.resolve_resource_profile(&mut lane)?;
                let fit = self.resource_lane_fit(&lane)?;
                let admission = self.store.request_resource_membership(&lane, fit, now)?;
                if !matches!(&admission, Admission::Queued { .. }) {
                    self.seat_admission.status.state().lane_waits.retain(|wait| wait.subject != lane.id);
                }
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
            // Keep the durable resource request pending, just like a queued seat start.
            let Some(_launch_permit) = self.admit_incarnation_launch()? else {
                return Ok(false);
            };
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
            self.runtime.start_resource_guarded(&self.store, &member, None, &|| {
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
    fn keyed_acquisition_decodes_declaration_but_never_caller_estimates() {
        let payload = serde_json::json!({
            "name": "checks", "host": "node", "profile": "check.unit",
            "diskPath": "/workspace", "maxMembers": 3, "coverage": "enforced", "class": "check"
        });
        let options: Acquire = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(options.profile.as_deref(), Some("check.unit"));
        assert_eq!(options.disk_path.as_deref(), Some("/workspace"));
        assert_eq!(options.max_members, 3);
        assert!(options.capacity.is_none());
        let mut injected = payload;
        injected["profileEstimate"] = serde_json::json!({"seedRevision":"caller"});
        assert!(serde_json::from_value::<Acquire>(injected).is_err());
    }

    fn fit() -> Fit {
        Fit { room: Some(fractal_runtime::Capacity { cores: 32, memory: 128 << 30 }), ..Fit::default() }
    }

    #[test]
    fn retry_retires_queued_and_admitted_attempts_before_readmission() {
        for retry in ["automatic", "operator", "operator-reopen"] {
            for queued in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let store = Arc::new(Store::open_memory("node").unwrap());
                let bundle = store.put_blob(b"export default {}").unwrap();
                super::super::tests::apply_source(&store, &format!(r#"version 2
mission "retry-lane" state="ready" {{
  goal "Keep retry attempts out of the failed attempt's lane."
  step "work" {{ program "/unused.mjs" sha256="{bundle}" }}
}}"#), "retry-lane-source");
                let run = store.create_mission_run(&crate::model::MissionRunRequest {
                    mission: "retry-lane".into(), revision: None, workspace: root.path().display().to_string(),
                    requester: Some("person/operator".into()), mode: None, inputs: BTreeMap::new(),
                    idempotency_key: "retry-lane-run".into(),
                }).unwrap();
                let view = &run.steps[0];
                let runtime = Arc::new(NativeRuntime::new(root.path(), None, Path::new("unused-pty")));
                let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
                reconciler.ensure_resource_authority().unwrap();
                let old = LaneRequest {
                    profile: None, profile_estimate: None, disk_path: None,
                    id: "failed-attempt".into(), name: "heavy-gate".into(), host: "node".into(),
                    max_members: 1, capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
                    coverage: Coverage::Logical, admission_timeout_ms: None,
                    class: fractal_runtime::RequestClass::Agent, disk: None, io: None,
                    owner: Owner { run: run.subject.clone(), step: view.subject.clone(), attempt: view.attempt,
                        invocation: Some("program:heavy-gate".into()) },
                };
                let mut blocker = old.clone();
                blocker.id = "other-run".into();
                blocker.owner.run = "mission-run/other".into();
                if queued {
                    assert!(matches!(store.request_resource_membership(&blocker, fit(), 1).unwrap(),
                        Admission::Admitted(_)));
                }
                let admission = store.request_resource_membership(&old, fit(), 2).unwrap();
                assert_eq!(matches!(admission, Admission::Queued { .. }), queued);
                store.set_step_state(&view.subject, "failed", Some("transient failure")).unwrap();
                let retried = if retry == "automatic" {
                    assert!(store.retry_step(&view.subject, "automatic retry", 0).unwrap());
                    store.mission_run(&run.subject).unwrap().unwrap()
                } else {
                    if retry == "operator-reopen" {
                        store.set_mission_run_state(&run.subject, "failed", "terminal", Some("step failed")).unwrap();
                    }
                    store.retry_failed_step(&view.subject, "person/operator", "retry", "operator-retry").unwrap()
                };
                let next = &retried.steps[0];
                assert!(next.subject != view.subject || next.attempt != view.attempt);
                assert!(reconciler.retire_previous_step_resources(&retried.subject, next).unwrap());
                let retired = store.resource_membership_record(&old.id, &old.owner).unwrap();
                assert_eq!(retired.state, "retired", "{retry}");
                if queued {
                    let record = store.resource_membership_record(&blocker.id, &blocker.owner).unwrap();
                    assert!(reconciler.retire_program_membership(&record).unwrap());
                }
                let mut new = old.clone();
                new.id = "retry-attempt".into();
                new.owner.step = next.subject.clone();
                new.owner.attempt = next.attempt;
                assert!(matches!(store.request_resource_membership(&new, fit(), 3).unwrap(),
                    Admission::Admitted(_)), "{retry}: max-members=1 must admit after retirement");
                assert!(matches!(store.request_resource_membership(&old, fit(), 4).unwrap(),
                    Admission::Refused { .. }), "failed attempt must never rejoin the queue");
                let live = store.resource_memberships_for_host("node").unwrap().into_iter()
                    .filter(|record| matches!(record.state.as_str(), "admitted" | "launched" | "retiring" | "uncertain"))
                    .count();
                assert_eq!(live, 1, "{retry}: old and retried attempt must never overlap");
            }
        }
    }



    #[test]
    fn retry_dispatch_waits_for_previous_scope_children_to_exit() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        let bundle = store.put_blob(b"export default {}").unwrap();
        let source = format!(r#"version 2
mission "retry-parent" state="ready" {{
  goal "Retain custody until child termination."
  step "work" {{ program "/unused.mjs" sha256="{bundle}" }}
}}
mission "retry-child" state="ready" {{
  goal "Own work until cancellation settles."
  step "work" {{ agentless }}
}}"#);
        super::super::tests::apply_source(&store, &source, "retry-child-source");
        let start = |mission: &str| store.create_mission_run(&crate::model::MissionRunRequest {
            mission: mission.into(), revision: None, workspace: root.path().display().to_string(),
            requester: Some("person/operator".into()), mode: None, inputs: BTreeMap::new(),
            idempotency_key: mission.into(),
        }).unwrap();
        let parent = start("retry-parent");
        let runtime = Arc::new(NativeRuntime::new(root.path(), None, Path::new("unused-pty")));
        let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
        reconciler.ensure_resource_authority().unwrap();
        let old = LaneRequest {
            profile: None, profile_estimate: None, disk_path: None,
            id: "parent-scope".into(), name: "heavy-gate".into(), host: "node".into(), max_members: 1,
            capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
            coverage: Coverage::Logical, admission_timeout_ms: None,
            class: fractal_runtime::RequestClass::Agent, disk: None, io: None,
            owner: Owner { run: parent.subject.clone(), step: parent.steps[0].subject.clone(), attempt: 1,
                invocation: Some("program:scope".into()) },
        };
        let Admission::Admitted(grant) = store.request_resource_membership(&old, fit(), 1).unwrap()
            else { panic!("parent must admit"); };
        let child_subject = store.mission_run_subject_for_idempotency_key("retry-child");
        store.attach_resource_child(&grant, &old.owner, &child_subject).unwrap();
        let child = start("retry-child");
        store.set_step_state(&parent.steps[0].subject, "failed", Some("transient")).unwrap();
        assert!(store.retry_step(&parent.steps[0].subject, "retry", 0).unwrap());
        let retried = store.mission_run(&parent.subject).unwrap().unwrap();
        let mission = store.mission_spec("retry-parent", None).unwrap().unwrap();
        let step = RuntimeStep { spec: &mission.steps["work"], dependency_prefix: String::new(),
            parent: None, mission_capacity: None };
        let (result, reads) = smallclaims::touched::record(|| reconciler.dispatch_program(
            &retried, &step, &retried.steps[0], step.spec.program.as_ref().unwrap()));
        assert!(result.unwrap(), "the retirement wait must publish its blocked state");
        let blocked = store.step_run(&retried.steps[0].subject).unwrap().unwrap();
        assert_eq!(blocked.status, "blocked");
        let reason = blocked.blocked_reason.as_deref().unwrap();
        assert!(reason.contains("previous attempt resource retirement"));
        assert!(reason.contains("cancel the mission run or release/recover the retained resource custody"));
        let attention = store.operational_attention_items(None, now_ms() + FAULT_ATTENTION_AFTER_MS + 1).unwrap();
        assert!(attention.iter().any(|item| item.subject == blocked.subject && item.detail.contains(reason)),
            "an indefinite resource retirement wait must name the recovery path in attention");
        assert!(reconciler.program_engine.get().is_none(), "retry must not reach the program host");
        assert!(reads.contains(&child.subject), "child completion must wake the retry barrier");
        assert_eq!(store.resource_membership_for_grant(&grant).unwrap().state, "retiring");
        let mut next = old.clone();
        next.id = "next-scope".into();
        next.owner.attempt = retried.steps[0].attempt;
        assert!(matches!(store.request_resource_membership(&next, fit(), 2).unwrap(),
            Admission::Queued { .. }));
        store.set_mission_run_state(&child.subject, "cancelled", "terminal", Some("process exited")).unwrap();
        assert!(reconciler.retire_previous_step_resources(&retried.subject, &retried.steps[0]).unwrap());
        assert_eq!(store.resource_membership_for_grant(&grant).unwrap().state, "retired");
        assert!(matches!(store.request_resource_membership(&next, fit(), 3).unwrap(),
            Admission::Admitted(_)));
    }

    #[test]
    fn retry_resource_wait_expires_finally_even_if_custody_retires_before_the_next_dispatch() {
        struct Clock;
        impl Drop for Clock {
            fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
        }
        let _clock = Clock;
        const START: u128 = 1_800_000_000_000;
        for retire_before_dispatch in [false, true] {
            smallclaims::store::set_thread_clock(Some(START));
            let root = tempfile::tempdir().unwrap();
            let store = Arc::new(Store::open_memory("node").unwrap());
            store.set_write_clock_at(START).unwrap();
            let bundle = store.put_blob(b"export default {}").unwrap();
            super::super::tests::apply_source(&store, &format!(r#"version 2
mission "retry-window" state="ready" {{
  goal "Expire while old custody is retained."
  step "work" {{
    program "/unused.mjs" sha256="{bundle}"
    retry {{ attempts 3; within "1s"; }}
  }}
}}
mission "window-child" state="ready" {{
  goal "Retain custody until cancellation settles."
  step "work" {{ agentless }}
}}"#), "retry-window-source");
            let run = store.create_mission_run(&crate::model::MissionRunRequest {
                mission: "retry-window".into(), revision: None, workspace: root.path().display().to_string(),
                requester: Some("person/operator".into()), mode: None, inputs: BTreeMap::new(),
                idempotency_key: "retry-window-run".into(),
            }).unwrap();
            let subject = &run.steps[0].subject;
            store.set_step_state(subject, "ready", None).unwrap();
            let runtime = Arc::new(NativeRuntime::new(root.path(), None, Path::new("unused-pty")));
            let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
            reconciler.ensure_resource_authority().unwrap();
            let old = LaneRequest {
                profile: None, profile_estimate: None, disk_path: None,
                id: "uncertain-scope".into(), name: "heavy-gate".into(), host: "node".into(), max_members: 1,
                capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
                coverage: Coverage::Logical, admission_timeout_ms: None,
                class: fractal_runtime::RequestClass::Agent, disk: None, io: None,
                owner: Owner { run: run.subject.clone(), step: subject.clone(), attempt: 1,
                    invocation: Some("program:scope".into()) },
            };
            let Admission::Admitted(grant) = store.request_resource_membership(&old, fit(), START as u64).unwrap()
                else { panic!("first attempt must admit"); };
            let child = if retire_before_dispatch {
                let child = store.mission_run_subject_for_idempotency_key("window-child");
                store.attach_resource_child(&grant, &old.owner, &child).unwrap();
                Some(store.create_mission_run(&crate::model::MissionRunRequest {
                    mission: "window-child".into(), revision: None, workspace: root.path().display().to_string(),
                    requester: Some("person/operator".into()), mode: None, inputs: BTreeMap::new(),
                    idempotency_key: "window-child".into(),
                }).unwrap())
            } else {
                // Interrupted preparation retains uncertain custody indefinitely.
                store.prepare_resource_membership(&grant, "unknown-root").unwrap();
                None
            };
            store.set_step_state(subject, "failed", Some("transient")).unwrap();
            assert!(store.retry_step(subject, "retry", 0).unwrap());
            let run = store.mission_run(&run.subject).unwrap().unwrap();
            let mission = store.mission_spec("retry-window", None).unwrap().unwrap();
            let step = RuntimeStep { spec: &mission.steps["work"], dependency_prefix: String::new(),
                parent: None, mission_capacity: None };
            let (result, due) = smallclaims::touched::record_due(|| reconciler.dispatch_program(
                &run, &step, &run.steps[0], step.spec.program.as_ref().unwrap()));
            assert!(result.unwrap());
            assert_eq!(due, Some(START + 1_001), "window expiry must wake even without resource events");
            let blocked = store.step_run(subject).unwrap().unwrap();
            assert_eq!(blocked.status, "blocked");
            let reason = blocked.blocked_reason.as_deref().unwrap();
            assert!(reason.contains("cancel the mission run or release/recover"));
            if let Some(child) = child {
                // Child completion races with the policy deadline: even successful
                // retirement must not admit this expired retry.
                store.set_mission_run_state(&child.subject, "cancelled", "terminal", Some("process exited")).unwrap();
            } else {
                assert!(reason.contains("uncertain"));
            }
            smallclaims::store::set_thread_clock(Some(START + 1_001));
            store.set_write_clock_at(START + 1_001).unwrap();
            let run = store.mission_run(&run.subject).unwrap().unwrap();
            assert!(reconciler.dispatch_program(
                &run, &step, &run.steps[0], step.spec.program.as_ref().unwrap()).unwrap());
            let expired = store.step_run(subject).unwrap().unwrap();
            assert_eq!(expired.status, "failed");
            assert!(expired.blocked_reason.as_deref().unwrap().contains("retry policy window expired"));
            assert!(reconciler.failed_step_is_final(&run, step.spec, &expired).unwrap());
            assert!(reconciler.program_engine.get().is_none(), "an expired retry must never dispatch");
            assert!(store.reconcile_fault(subject, RETRY_RETIREMENT_FAULT_SCOPE).unwrap().is_none());
            assert_eq!(store.resource_membership_for_grant(&grant).unwrap().state,
                if retire_before_dispatch { "retired" } else { "uncertain" });
        }
    }

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
            profile: None, profile_estimate: None, disk_path: None,
            id: "scope-holder".into(), name: "child-scope".into(), host: "node".into(),
            max_members: 1, capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
            coverage: Coverage::Logical, admission_timeout_ms: None,
            class: fractal_runtime::RequestClass::Agent, disk: None, io: None,
            owner: Owner { run: "mission-run/parent".into(), step: "step-run/parent/work".into(),
                attempt: 1, invocation: None },
        };
        let Admission::Admitted(grant) = store.request_resource_membership(&lane, fit(), 1).unwrap() else {
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
        let mut future = record.clone();
        future.request.class = fractal_runtime::RequestClass::Unknown("future-class".into());
        let child_status = store.mission_run_status(&child).unwrap();
        let index = store.index().unwrap();
        assert!(reconciler.retire_program_membership(&future).unwrap_err().to_string().contains("unsupported resource class future-class"));
        assert_eq!(store.mission_run_status(&child).unwrap(), child_status);
        assert_eq!(store.index().unwrap(), index, "unsupported-class retirement must append no child cancellation claims");
        let (result, reads) = smallclaims::touched::record(|| reconciler.retire_program_membership(&record));
        assert!(!result.unwrap(), "normal asynchronous cancellation must not refuse scope release");
        assert!(reads.contains(&child));
        let pending = store.resource_membership_for_grant(&grant).unwrap();
        assert_eq!(pending.state, "retiring");
        assert_eq!(pending.retired_at, None);
        let mut waiter = lane.clone();
        waiter.id = "scope-waiter".into();
        assert!(matches!(store.request_resource_membership(&waiter, fit(), 2).unwrap(), Admission::Queued { .. }));
        reconciler.incremental.observe(&store).unwrap();
        reconciler.incremental.evaluated("scope-release", reads, None);
        assert!(!reconciler.incremental.needs("scope-release", now_ms()));
        for _ in 0..3 { reconciler.reconcile_once().unwrap(); }
        assert_eq!(store.mission_run_status(&child).unwrap().as_deref(), Some("cancelled"));
        reconciler.incremental.observe(&store).unwrap();
        assert!(reconciler.incremental.needs("scope-release", now_ms()), "terminal child must wake pending release");
        assert!(reconciler.retire_program_membership(&pending).unwrap());
        assert_eq!(store.resource_membership_for_grant(&grant).unwrap().state, "retired");
        assert!(matches!(store.request_resource_membership(&waiter, fit(), 3).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn scope_release_refuses_unknown_child_without_releasing_custody() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        store.start_resource_authority("node", "epoch").unwrap();
        let lane = LaneRequest {
            profile: None, profile_estimate: None, disk_path: None,
            id: "unknown-scope".into(), name: "child-scope".into(), host: "node".into(),
            max_members: 1, capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
            coverage: Coverage::Logical, admission_timeout_ms: None,
            class: fractal_runtime::RequestClass::Agent, disk: None, io: None,
            owner: Owner { run: "mission-run/parent".into(), step: "step-run/parent/work".into(),
                attempt: 1, invocation: None },
        };
        let Admission::Admitted(grant) = store.request_resource_membership(&lane, fit(), 1).unwrap() else {
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
        assert!(matches!(store.request_resource_membership(&waiter, fit(), 2).unwrap(), Admission::Queued { .. }));
    }
}
