//! Native session custody and immutable home-admitted, graph-free physical launches.
use super::*;
use fractal_peer::light_node::journal::{Command, Journal, Operation, Outcome, Recovery, Selection, StartWorkspaceAdmission, require_start};
use fractal_peer::light_node::session::Executor;
use crate::store::local_sessions::{LocalSession, SessionCustody};
use crate::store::run_credentials::{ExecutionBinding, local};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::pin::Pin;

#[derive(Serialize, Deserialize)]
#[serde(tag = "native_phase", rename_all = "snake_case")]
enum NativeProof {
    Complete { outcome: Outcome },
    Stop { binding: LocalSession, retired: bool },
    Start,
}

type RetirementResult = Option<std::result::Result<(), Arc<str>>>;
type RetirementWaits = BTreeMap<(String, String), tokio::sync::watch::Receiver<RetirementResult>>;

/// Own native waiters across transport reconnects, not across control-epoch replacement.
/// Each accepted physical generation has at most one kernel-wait worker; journal capacity
/// bounds unresolved work. Blocking kernel waits do not occupy Tokio's shutdown pool.
pub struct NativeSessionExecutor {
    reconciler: Arc<Reconciler>,
    waits: Mutex<RetirementWaits>,
    native_watcher: Mutex<Option<notify::RecommendedWatcher>>,
    native_generation: Arc<tokio::sync::watch::Sender<u64>>,
    #[cfg(test)]
    recovery_failures: tokio::sync::watch::Sender<u64>,
}
impl NativeSessionExecutor {
    pub fn new(reconciler: Arc<Reconciler>) -> Self {
        Self { reconciler, waits: Mutex::new(BTreeMap::new()), native_watcher: Mutex::new(None),
            native_generation: Arc::new(tokio::sync::watch::channel(0).0),
            #[cfg(test)]
            recovery_failures: tokio::sync::watch::channel(0).0,
        }
    }

    pub fn watch_native_custody(&self) -> Result<()> {
        let mut slot = self.native_watcher.lock().map_err(|_| anyhow::anyhow!("native custody watcher poisoned"))?;
        if slot.is_some() { return Ok(()); }
        let changed = self.reconciler.home_session_journal.get()
            .context("native custody watcher requires installed session journal")?.authority_changed.clone();
        let recovery_changed = self.reconciler.home_session_journal.get()
            .context("native custody watcher requires installed session journal")?.changed.clone();
        let reconciler = self.reconciler.clone();
        let generation = self.native_generation.clone();
        let mut previous = native_recovery_evidence(&reconciler)?;
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return; };
            // Reading registry evidence emits Access events on Linux. Re-reading on
            // those events feeds the watcher itself, starving real native mutations.
            if matches!(event.kind, notify::EventKind::Access(_)) { return; }
            // Registry notifications include repeated writes. Only changed physical
            // incarnation/liveness/provenance can reopen a failed Start recovery.
            if let Ok(current) = native_recovery_evidence(&reconciler)
                && current != previous {
                    previous = current;
                    generation.send_modify(|value| *value = value.saturating_add(1));
                    // SeatClient waits on command progress, separately from the
                    // native authority revalidator. Wake it for physical evidence
                    // even when that revalidator has no SQL mutation to perform.
                    recovery_changed.notify_one();
                }
            changed.notify_one();
        })?;
        for directory in &self.reconciler.runtime.observation_paths {
            std::fs::create_dir_all(directory)?;
            watcher.watch(directory, notify::RecursiveMode::Recursive)?;
        }
        *slot = Some(watcher);
        Ok(())
    }

    async fn await_exec_retirement(&self, binding: &LocalSession) -> Result<()> {
        let key = (binding.runtime_id.clone(), binding.native_incarnation.clone());
        let mut completion = {
            let mut waits = self.waits.lock().map_err(|_| anyhow::anyhow!("native retirement wait ownership poisoned"))?;
            if let Some(existing) = waits.get(&key) { existing.clone() }
            else {
                let (done, completion) = tokio::sync::watch::channel(None);
                let exec = self.reconciler.runtime.exec.clone();
                let runtime = binding.runtime_id.clone();
                let incarnation = binding.native_incarnation.clone();
                std::thread::Builder::new().name("native-session-retirement".into()).spawn(move || {
                    let result = (|| -> Result<()> {
                        // Stop acceptance is only a signal. Wait for the leader before
                        // whole-scope cleanup, then retain exact native retirement proof.
                        exec.wait_for_exit(&runtime, &incarnation)?;
                        exec.retire_if(&runtime, &incarnation)?;
                        Ok(())
                    })().map_err(|error| Arc::<str>::from(format!("{error:#}")));
                    done.send_replace(Some(result));
                })?;
                waits.insert(key, completion.clone());
                completion
            }
        };
        loop {
            if let Some(result) = completion.borrow_and_update().clone() {
                result.map_err(|error| anyhow::anyhow!("native retirement remains unresolved: {error}"))?;
                if !self.reconciler.store.local_session_retirement_verified(binding)? {
                    self.reconciler.store.verify_local_session_retirement(binding)?;
                }
                return Ok(());
            }
            completion.changed().await.context("native retirement worker lost its result")?;
        }
    }

    fn stop_proof(&self, command: &Command, binding: LocalSession) -> Result<NativeProof> {
        let reconciler = &self.reconciler;
        reconciler.host_observations.invalidate();
        let retired = stop_local_session_with_retirement_proof(&reconciler.store, reconciler.runtime.as_ref(),
            &binding.subject, &binding.incarnation, &command.execution_id)?;
        reconciler.start_changed(&binding.subject);
        if let Some(journal) = reconciler.home_session_journal.get() {
            journal.authority_changed.notify_one();
        }
        Ok(if binding.custody == SessionCustody::Borrowed {
            NativeProof::Complete { outcome: Outcome::Detached { incarnation: binding.incarnation } }
        } else { NativeProof::Stop { binding, retired } })
    }

    fn finish_stop(&self, command: &Command, binding: LocalSession) -> Result<Outcome> {
        anyhow::ensure!(binding.subject == command.binding.subject && binding.runtime_id == command.binding.runtime
            && binding.incarnation == command.binding.incarnation && binding.custody == SessionCustody::Owned,
            "native stop receipt custody differs");
        let outcome = Outcome::Stopped { incarnation: binding.incarnation.clone() };
        let reconciler = &self.reconciler;
        // Receipt custody is durable before local finalization, but cannot be transmitted
        // or reclaimed until that finalization succeeds. A returned Store error is just
        // as recoverable as a crash, including while the websocket remains connected.
        let proof = NativeProof::Stop { binding, retired: true };
        let encoded = serde_json::to_string(&proof)?;
        let NativeProof::Stop { binding, .. } = proof else { unreachable!("constructed stop proof") };
        let journal = reconciler.home_session_journal.get().context("session journal is not installed")?;
        journal.complete_requiring_local_finalization(&command.execution_id, outcome.clone(), &encoded)?;
        if let Some(operation) = &binding.start_operation
            && let Some(operation) = reconciler.store.remote_runtime_start(operation)? {
                reconciler.retire_remote_start_resources(&operation, Some(&binding.native_incarnation))?;
                reconciler.store.verify_remote_runtime_start_cleanup(&operation.id)?;
            }
        reconciler.store.settle_local_session_custody(&binding.subject, &binding.native_incarnation)?;
        if let Some(operation) = &binding.start_operation {
            reconciler.store.settle_remote_runtime_start(operation)?;
        }
        journal.finish_local_finalization(&command.execution_id)?;
        self.waits.lock().map_err(|_| anyhow::anyhow!("native retirement wait ownership poisoned"))?
            .remove(&(binding.runtime_id.clone(), binding.native_incarnation.clone()));
        Ok(outcome)
    }

    fn enter_start(&self, command: &Command, authority: &rusqlite::Connection) -> Result<NativeProof> {
        let Operation::Start { authorization } = &command.operation else { unreachable!("Start dispatch") };
        require_start(authority, command)?;
        self.watch_native_custody()?;
        let reconciler = &self.reconciler;
        anyhow::ensure!(authorization.member.host == reconciler.host
            && authorization.launch_member.host == reconciler.host
            && authorization.member.runtime_id == command.binding.runtime
            && authorization.launch_member.runtime_id == command.binding.runtime,
            "Start payload names another physical runtime");
        // An existing intent is recovery, never permission to repeat a backend call.
        if let Some(prior) = reconciler.store.remote_runtime_start(&command.execution_id)? {
            anyhow::ensure!(prior.body["remote_start"] == serde_json::to_value(authorization)?,
                "native Start operation payload changed");
            return Ok(NativeProof::Start);
        }
        let identity = match &authorization.workspace_admission {
            StartWorkspaceAdmission::NonGit => start_jobs::WorkspaceAdmission::NonGit,
            StartWorkspaceAdmission::Git { owner, repository } =>
                start_jobs::WorkspaceAdmission::Git { owner: owner.into(), repository: repository.into() },
        };
        let checkout = authorization.checkout.as_ref().map(|checkout| Checkout {
            repository: checkout.repository.clone().into(), base: checkout.base.clone(),
            branch: checkout.branch.clone(), remove_at_run_end: checkout.remove_at_run_end,
        });
        let _permit = reconciler.store.admit_launch()?.context("native launch is frozen")?;
        let _lock = reconciler.store.try_runtime_start_lock(&command.binding.subject)?
            .context("native subject launch custody is busy")?;
        let owner = command.execution_id.clone();
        let operation = crate::store::RuntimeStartOperation {
            id: command.execution_id.clone(), subject: command.binding.subject.clone(), owner: owner.clone(),
            body: serde_json::json!({"remote_start": authorization}),
            revoked_by: None, finished: false, result: None, applied: false,
            cleanup_verified: false, dispatched: false, receipted: false,
        };
        anyhow::ensure!(reconciler.store.admit_remote_runtime_start(&operation, start_jobs::MAX_START_JOBS)?,
            "native launch admission capacity or subject custody is busy");
        anyhow::ensure!(reconciler.remote_seat_lane_fits(&command.execution_id, &authorization.launch_member)?,
            "native agent lane does not fit the physical host envelope");
        if let Some(request) = resource_lanes::request(&authorization.launch_member)? {
            anyhow::ensure!(request.host == reconciler.host, "finite Start names another resource host");
            reconciler.ensure_resource_authority()?;
            let fit = reconciler.remote_resource_lane_fit(&request)?;
            let admission = reconciler.store.request_native_resource_membership(&command.execution_id, &request, fit,
                u64::try_from(now_ms()).unwrap_or(u64::MAX))?;
            anyhow::ensure!(matches!(&admission, crate::store::resource_membership::Admission::Admitted(_)),
                "native finite resource membership was not admitted: {admission:?}");
        }
        let workspace = Path::new(&authorization.member.workspace);
        let _preparation_lease = if !workspace.is_dir() {
            require_start(authority, command)?;
            match (&checkout, &identity) {
                (Some(checkout), start_jobs::WorkspaceAdmission::Git { owner, repository }) => {
                    let lease = reconciler.worktree_activation_lease(owner, repository)?;
                    checkout.create_leased(workspace, &lease)?;
                    Some(lease)
                }
                (Some(_), start_jobs::WorkspaceAdmission::NonGit) =>
                    anyhow::bail!("checkout preparation requires immutable Git workspace admission"),
                (None, identity) => {
                    anyhow::ensure!(authorization.member.workspace_create,
                        "workspace does not exist and creation was not authorized");
                    let lease = match identity {
                        start_jobs::WorkspaceAdmission::Git { owner, repository } =>
                            Some(reconciler.worktree_activation_lease(owner, repository)?),
                        start_jobs::WorkspaceAdmission::NonGit => None,
                    };
                    std::fs::create_dir_all(workspace)?;
                    lease
                }
            }
        } else { None };
        let _activation = reconciler.admit_physical_workspace(&authorization.member, &identity, checkout.as_ref())?;
        anyhow::ensure!(!Path::new(&authorization.member.cwd).is_absolute() || Path::new(&authorization.member.cwd).is_dir(),
            "native Start working directory does not exist");
        if let Some(observed) = reconciler.runtime.observe_start(&authorization.member)? {
            anyhow::ensure!(observed.incarnation_id == authorization.expected_incarnation
                && authorization.expected_incarnation.is_some(),
                "Start cannot adopt or restart a preexisting native incarnation");
        } else {
            anyhow::ensure!(authorization.expected_incarnation.is_none(),
                "Start predecessor incarnation is unavailable");
        }
        let mut launch_member = authorization.launch_member.clone();
        crate::environment::normalize_aliases(&mut launch_member.environment);
        overlay_runtime_environment(&command.binding.subject, &mut launch_member.environment,
            &reconciler.runtime_environment);
        crate::environment::remove_aliases(&mut launch_member.environment, "ST3_ACCOUNT");
        if let Some(credentials) = &authorization.credentials {
            anyhow::ensure!(authorization.member.driver.as_deref() == Some(credentials.driver.as_str()),
                "Start credential driver differs from immutable member");
            let variable = crate::accounts::login_environment_name(&credentials.driver)
                .context("Start credential driver has no login directory")?;
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let directory = crate::accounts::expand_login(&credentials.login, home.as_deref())
                .context("Start credential login selector cannot be expanded")?;
            anyhow::ensure!(directory.is_dir(), "Start credential login directory is unavailable");
            crate::environment::set(&mut launch_member.environment, variable, directory.to_string_lossy().into_owned());
            if let Some(account) = &credentials.account {
                crate::environment::set(&mut launch_member.environment, "ST3_ACCOUNT", account.clone());
            }
        }
        crate::environment::remove_aliases(&mut launch_member.environment, crate::suspension::CONTINUE_ENV);
        crate::environment::remove_aliases(&mut launch_member.environment, crate::suspension::CONTINUE_PATH_ENV);
        if let Some(continuation) = &authorization.continuation {
            crate::environment::set(&mut launch_member.environment, crate::suspension::CONTINUE_ENV, continuation.session.clone());
            if let Some(path) = &continuation.path {
                crate::environment::set(&mut launch_member.environment, crate::suspension::CONTINUE_PATH_ENV, path.clone());
            }
        }
        crate::native_seed::omit_seed_for_native_resume(&mut launch_member);
        let kind = command.binding.subject.split('/').next().context("Start subject has no kind")?;
        reconciler.decorate_local_launch(&command.binding.subject, kind, command.binding.run.is_some(),
            &authorization.member, &mut launch_member)?;
        launch_member.tags.insert(start_jobs::OPERATION_TAG.into(), command.execution_id.clone());
        crate::environment::set(&mut launch_member.environment, start_jobs::OPERATION_ENV, command.execution_id.clone());
        crate::environment::set(&mut launch_member.environment, crate::suspension::LAUNCH_DESIRED_TOKEN_ENV,
            authorization.desired_token.clone());
        if let Some(render) = &authorization.render {
            let (writes, _) = crate::render::prepare_render_with_documents(
                &render.declaration, Path::new(&authorization.member.workspace),
                &|_| require_start(authority, command),
                &|name, hash| Ok(render.documents.get(&format!("{name}@{hash}")).cloned()))?;
            require_start(authority, command)?;
            crate::render::commit_transaction(&writes)?;
        }
        super::host_executor::StartCommand::dispatch_remote(command, authority, &reconciler.store,
            reconciler.runtime.as_ref(), &reconciler.host_observations, &owner, &launch_member)?;
        Ok(NativeProof::Start)
    }

    async fn recover_start(&self, recovery: &Recovery) -> Result<Outcome> {
        let command = &recovery.command;
        let Operation::Start { authorization } = &command.operation else { unreachable!("Start recovery") };
        self.watch_native_custody()?;
        let reconciler = &self.reconciler;
        let Some(mut operation) = reconciler.store.remote_runtime_start(&command.execution_id)? else {
            // The barrier was entered, but native admission did not create an operation.
            if let Some(reason) = &recovery.cancellation_reason {
                return Ok(Outcome::Cancelled { reason: reason.clone() });
            }
            if let Some(proof) = &recovery.custody_proof
                && let NativeProof::Complete { outcome: Outcome::Refused { reason } } = serde_json::from_str(proof)? {
                return Ok(Outcome::Refused { reason });
            }
            return Ok(Outcome::Refused { reason: "native Start was not admitted".into() });
        };
        anyhow::ensure!(operation.body["remote_start"] == serde_json::to_value(authorization)?,
            "retained native authorization differs");
        let cancelled = recovery.cancellation_reason.as_ref().or(operation.revoked_by.as_ref());
        if !operation.dispatched {
            reconciler.retire_remote_start_resources(&operation, None)?;
            reconciler.store.settle_remote_runtime_start(&operation.id)?;
            return Ok(match cancelled {
                Some(reason) => Outcome::Cancelled { reason: reason.clone() },
                None => Outcome::Refused { reason: operation.result.as_ref()
                    .and_then(|result| result["error"].as_str())
                    .unwrap_or("native backend was never entered").into() },
            });
        }
        let mut result = operation.result.clone().unwrap_or_else(|| serde_json::json!({}));
        if result["verification"] != "operation-incarnation" {
            // Reopening an entered unknown may discover positive native provenance,
            // but a missing process is deliberately not a nonlaunch witness.
            if let Some(observed) = reconciler.runtime.observe_start(&authorization.launch_member)?
                && let Some(incarnation) = observed.incarnation_id
                && reconciler.runtime.start_operation(&authorization.member.runtime_id, authorization.member.terminal, &incarnation)?
                    .as_deref() == Some(operation.id.as_str())
            {
                result["incarnation"] = Value::from(incarnation);
                result["verification"] = Value::from("operation-incarnation");
                reconciler.store.finish_remote_runtime_start(&operation.id, &result)?;
                operation.result = Some(result.clone());
            }
            if result["verification"] != "operation-incarnation" && result["producer_closed"] != true {
                // Revisit only exact native producer evidence, never repeat dispatch.
                // The immutable authorization predates the operation tag added at entry.
                let mut launch_member = authorization.launch_member.clone();
                launch_member.tags.insert(start_jobs::OPERATION_TAG.into(), operation.id.clone());
                if reconciler.runtime.start_operation_closed(&launch_member)? {
                    result["producer_closed"] = Value::from(true);
                    result["verification"] = Value::from("producer-closed");
                    result["phase"] = Value::from("failed");
                    reconciler.store.finish_remote_runtime_start(&operation.id, &result)?;
                    operation.result = Some(result.clone());
                }
            }
        }
        let Some(incarnation) = result["incarnation"].as_str()
            .filter(|_| result["verification"] == "operation-incarnation") else {
            if result["producer_closed"] == true {
                reconciler.retire_remote_start_resources(&operation, None)?;
                reconciler.store.settle_remote_runtime_start(&operation.id)?;
                return Ok(Outcome::Refused { reason: result["error"].as_str().unwrap_or("native launch refused").into() });
            }
            anyhow::bail!("entered native Start retains unresolved producer custody: operation={}, result={result}",
                operation.id);
        };
        let binding = LocalSession {
            subject: command.binding.subject.clone(), runtime_id: authorization.member.runtime_id.clone(),
            incarnation: incarnation.into(), native_incarnation: incarnation.into(),
            terminal: authorization.member.terminal, driver: authorization.member.driver.clone(),
            lifecycle: authorization.member.lifecycle.clone(), custody: SessionCustody::Owned,
            start_operation: Some(operation.id.clone()), invocation: None,
        };
        // Durable custody must exist before exposing actual native success to home.
        if reconciler.store.local_session_at(&binding.subject, &binding.incarnation)?.as_ref() != Some(&binding) {
            reconciler.store.bind_local_session(&binding)?;
        }
        if let Some(reason) = cancelled {
            reconciler.store.revoke_remote_runtime_start(&operation.id, reason)?;
            if !reconciler.store.local_session_retirement_verified(&binding)? {
                let retired = reconciler.runtime.stop_with_retirement_proof(
                    &binding.runtime_id, binding.terminal, &binding.native_incarnation)?;
                if retired { reconciler.store.verify_local_session_retirement(&binding)?; }
                if !retired {
                    anyhow::ensure!(!binding.terminal, "native terminal producer retirement remains unresolved");
                    self.await_exec_retirement(&binding).await?;
                }
            }
            reconciler.retire_remote_start_resources(&operation, Some(&binding.native_incarnation))?;
            reconciler.store.verify_remote_runtime_start_cleanup(&operation.id)?;
            reconciler.store.settle_local_session_custody(&binding.subject, &binding.native_incarnation)?;
            reconciler.store.settle_remote_runtime_start(&operation.id)?;
            self.waits.lock().map_err(|_| anyhow::anyhow!("native retirement wait ownership poisoned"))?
                .remove(&(binding.runtime_id.clone(), binding.native_incarnation.clone()));
            return Ok(Outcome::Stopped { incarnation: binding.incarnation });
        }
        reconciler.refresh_session_bindings()?;
        Ok(Outcome::Started { incarnation: binding.incarnation })
    }

}

type NativeRecoveryEvidence = (
    Vec<RuntimeObservation>,
    HashMap<String, (String, String)>,
    BTreeMap<String, fractal_runtime::ExecObservation>,
);

/// Compare physical facts, not registry file timestamps or local SQL revisions.
fn native_recovery_evidence(reconciler: &Reconciler) -> Result<NativeRecoveryEvidence> {
    let mut snapshot = reconciler.runtime.snapshot_ptys_with_provenance()?;
    snapshot.observations.sort_by(|left, right| left.runtime_id.cmp(&right.runtime_id));
    let mut executions = BTreeMap::new();
    // Enumerate the native exec registry itself, not SQL's operation list: settling
    // a local row must never manufacture a new recovery generation.
    let directory = &reconciler.runtime.observation_paths[1];
    if directory.is_dir() {
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().is_none_or(|extension| extension != "json") { continue; }
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else { continue; };
            if let Some(observed) = reconciler.runtime.exec.observe(id)? {
                executions.insert(id.to_owned(), observed);
            }
        }
    }
    Ok((snapshot.observations, snapshot.start_operations, executions))
}

fn execution_binding(selection: &Selection, binding: &LocalSession) -> Result<ExecutionBinding> {
    let revision = hex::encode(Sha256::digest(serde_json::to_vec(&(selection, binding))?));
    Ok(ExecutionBinding {
        version: 1, home: selection.home.clone(), host: selection.host.clone(), subject: binding.subject.clone(),
        runtime: binding.runtime_id.clone(), incarnation: binding.incarnation.clone(), authorization_revision: revision,
        epoch: selection.epoch, run: None,
    })
}

pub(super) fn require_start_helper<R: RuntimeControl>(
    store: &Store, runtime: &R, authorization: &fractal_peer::light_node::journal::StartAuthorization,
) -> Result<()> {
    if let Some(helper) = &authorization.helper {
        // Use logical custody to resolve a borrowed launcher, then let local_session
        // verify its actual native generation. No graph observation can substitute.
        local_session(store, runtime, &helper.subject, Some(&helper.lifetime))
            .context("Start helper launcher custody has ended")?;
    }
    Ok(())
}

impl<R: RuntimeControl> Reconciler<R> {
    /// Native custody, including entered producers with no visible registry entry,
    /// holds its immutable envelope until positive physical cleanup.
    fn remote_held_lanes(&self, candidate: &str, candidate_lane: Option<fractal_runtime::Capacity>)
        -> Result<(u64, seat_admission_queue::BurstReserve)>
    {
        let mut held = 0_u64;
        let mut smallest = candidate_lane.map(|lane| lane.memory);
        let mut burst_raise = 0_u64;
        let mut accounted = BTreeSet::new();
        let mut ordinary = BTreeMap::<String, fractal_runtime::Capacity>::new();
        for (operation, member) in self.store.unretired_native_start_members()? {
            if member.host != self.host { continue; }
            // Finite reservations are charged once by the membership accountant,
            // never a second time through the runtime's live lane.
            if resource_lanes::request(&member)?.is_some() {
                accounted.insert(member.runtime_id);
                continue;
            }
            // Only this operation is excluded, never an unresolved predecessor
            // merely because it has the same logical subject or runtime name.
            if operation == candidate { continue; }
            if member.kind != crate::model::MemberKind::Agent { continue; }
            let Some(base) = member.capacity else { continue; };
            ordinary.entry(member.runtime_id).and_modify(|prior| {
                prior.memory = prior.memory.max(base.memory);
                prior.cores = prior.cores.max(base.cores);
            }).or_insert(base);
        }
        for (runtime, base) in ordinary {
            if !accounted.insert(runtime.clone()) { continue; }
            let live = self.runtime.live_lane(&runtime).unwrap_or(base);
            held = held.checked_add(base.memory.max(live.memory)).context("native lane capacity overflow")?;
            smallest = Some(smallest.map_or(base.memory, |memory| memory.min(base.memory)));
            burst_raise = burst_raise.max(live.memory.saturating_sub(base.memory));
        }
        for observed in self.runtime.snapshot_ptys()? {
            if !accounted.insert(observed.runtime_id.clone()) { continue; }
            if let Some(lane) = self.runtime.live_lane(&observed.runtime_id) {
                held = held.checked_add(lane.memory).context("native lane capacity overflow")?;
                smallest = Some(smallest.map_or(lane.memory, |memory| memory.min(lane.memory)));
            }
        }
        let reserve = self.seat_admission.lanes.and_then(|host| host.burst_lane.filter(|burst|
            burst.cores <= host.room.cores && burst.memory <= host.room.memory))
            .map_or(seat_admission_queue::BurstReserve { full: 0, kept: 0 },
                |burst| seat_admission_queue::reserved_burst(smallest, burst.memory, burst_raise));
        Ok((held, reserve))
    }

    fn remote_seat_lane_fits(&self, candidate: &str, member: &MemberSpec) -> Result<bool> {
        if member.kind != crate::model::MemberKind::Agent || resource_lanes::request(member)?.is_some() {
            return Ok(true);
        }
        let (Some(host), Some(lane)) = (self.seat_admission.lanes, member.capacity) else { return Ok(true); };
        let (held, reserve) = self.remote_held_lanes(candidate, Some(lane))?;
        let finite = self.store.held_resource_membership_bytes(&self.host, candidate)?;
        // Same physical fit as ordinary fullnode seats: individual CPU envelope,
        // aggregate memory, outside memory, and both full/remaining burst reserve.
        let outside = (host.outside)(&self.seat_admission.finite_policy.psi_brake).context("outside-lane host observation unavailable")?.outside_bytes;
        let free = host.room.memory.saturating_sub(held).saturating_sub(finite)
            .saturating_sub(outside).saturating_sub(reserve.kept);
        Ok(lane.cores <= host.room.cores && lane.memory <= host.room.memory.saturating_sub(reserve.full)
            && lane.memory <= free)
    }

    fn remote_resource_lane_fit(&self, request: &crate::store::resource_membership::LaneRequest)
        -> Result<crate::store::resource_membership::Fit>
    {
        self.resource_lane_fit_with_accounting(request, &|_| {
            let (held, reserve) = self.remote_held_lanes(&request.id, None)?;
            Ok((held, reserve.kept))
        })
    }

    /// The sealed subject owns either the membership or one inherited execution root.
    /// Never infer membership ownership from the inherited (unchanged) lane request.
    pub(super) fn retire_remote_start_resources(
        &self, operation: &crate::store::RuntimeStartOperation, native_incarnation: Option<&str>,
    ) -> Result<()> {
        let authorization: fractal_peer::light_node::journal::StartAuthorization =
            serde_json::from_value(operation.body["remote_start"].clone())?;
        let Some(request) = resource_lanes::request(&authorization.launch_member)? else { return Ok(()); };
        let Some(record) = self.store.resource_memberships_for_host(&self.host)?.into_iter()
            .find(|record| record.request == request) else {
                anyhow::ensure!(!operation.dispatched, "dispatched native finite Start lost resource custody");
                return Ok(());
            };
        if request.owner.invocation.as_deref() == Some(operation.subject.as_str()) {
            anyhow::ensure!(self.runtime.retire_resource_lane(&self.store, &request)?,
                "native finite Start producer custody remains unresolved");
            return Ok(());
        }
        let runtime_id = &authorization.launch_member.runtime_id;
        let Some(root) = record.roots.iter().find(|root| root.runtime_id == *runtime_id) else {
            // Before dispatch there was no acquisition; a backend refusal may also
            // positively reject its preparation. Neither grants parent cancellation.
            anyhow::ensure!(!operation.dispatched
                || operation.result.as_ref().is_some_and(|result| result["producer_closed"] == true),
                "inherited native Start has no positive root closure");
            return Ok(());
        };
        let execution = root.execution.as_ref().context("inherited native preparation retains unresolved producer custody")?;
        let incarnation = native_incarnation.context("inherited native root lacks exact retirement evidence")?;
        anyhow::ensure!(execution.generation_id == incarnation
            && operation.result.as_ref().is_some_and(|result|
                result["verification"] == "operation-incarnation" && result["incarnation"].as_str() == Some(incarnation)),
            "inherited native retirement names another execution generation");
        let binding = self.store.local_session_at(&operation.subject, incarnation)?
            .context("inherited native retirement lost local custody")?;
        anyhow::ensure!(binding.runtime_id == *runtime_id && binding.native_incarnation == incarnation
            && binding.start_operation.as_deref() == Some(operation.id.as_str())
            && self.store.local_session_retirement_verified(&binding)?,
            "inherited native root retirement is not positively verified");
        let grant = record.grant.as_ref().context("inherited native root lost membership grant")?;
        self.store.end_resource_execution(grant, runtime_id, u64::try_from(now_ms()).unwrap_or(u64::MAX))?;
        Ok(())
    }

    /// Admit only exact, already-owned native sessions, never graph status or a launch.
    pub fn session_bindings(&self, selection: &Selection) -> Result<Vec<ExecutionBinding>> {
        anyhow::ensure!(selection.host == self.host, "session grant names another physical host");
        let mut bindings = Vec::new();
        for subject in &selection.grant.subjects {
            if self.store.local_session(subject)?.is_none() { continue; }
            let binding = match local_session(&self.store, self.runtime.as_ref(), subject, None) {
                Ok(binding) => binding,
                Err(error) if error.is::<LocalSessionUnavailable>() => {
                    tracing::warn!(%subject, %error, "native session is unavailable for home admission");
                    continue;
                }
                Err(error) => return Err(error),
            };
            bindings.push(execution_binding(selection, &binding)?);
        }
        Ok(bindings)
    }

    pub fn install_home_session_journal(&self, journal: Journal) -> Result<()> {
        anyhow::ensure!(journal.selection()?.host == self.host, "session journal names another physical host");
        self.home_session_journal.set(journal).map_err(|_| anyhow::anyhow!("session journal already installed"))?;
        let journal = self.home_session_journal.get().context("session journal installation failed")?;
        for (recovery, outcome) in journal.pending_terminal_results()? {
            let command = recovery.command;
            let Outcome::Stopped { incarnation } = outcome else {
                anyhow::bail!("pending native finalization is not an exact stop receipt");
            };
            let proof = recovery.custody_proof.context("pending stop finalization lost its native custody proof")?;
            let NativeProof::Stop { binding, .. } = serde_json::from_str(&proof)? else {
                anyhow::bail!("pending stop finalization lost its native custody snapshot");
            };
            anyhow::ensure!(command.binding.host == self.host && command.binding.incarnation == incarnation
                && binding.subject == command.binding.subject && binding.runtime_id == command.binding.runtime
                && binding.incarnation == incarnation && binding.custody == SessionCustody::Owned,
                "pending native stop receipt names another custody owner");
            if let Some(operation) = &binding.start_operation
                && let Some(operation) = self.store.remote_runtime_start(operation)? {
                    self.retire_remote_start_resources(&operation, Some(&binding.native_incarnation))?;
                    self.store.verify_remote_runtime_start_cleanup(&operation.id)?;
                }
            self.store.settle_local_session_custody(&binding.subject, &binding.native_incarnation)?;
            if let Some(operation) = &binding.start_operation {
                self.store.settle_remote_runtime_start(operation)?;
            }
            journal.finish_local_finalization(&command.execution_id)?;
        }
        // A new daemon cannot adopt the previous in-memory program VM. Permission is
        // revoked before a fresh attempt, without claiming that the old process exited.
        for prior in journal.reported_executions()? {
            if prior.run.is_some() { journal.revoke_execution(&prior)?; }
        }
        self.refresh_session_bindings()
    }

    /// Replacement grants may offer existing custody only after the old-home barrier drains.
    pub fn refresh_session_bindings(&self) -> Result<()> {
        let journal = self.home_session_journal.get().context("session journal is not installed")?;
        if journal.closing()? { return Ok(()); }
        let selection = journal.selection()?;
        let bindings = self.session_bindings(&selection)?;
        for prior in journal.reported_executions()? {
            if prior.run.is_none() && !bindings.iter().any(|binding| binding == &prior) { journal.revoke_execution(&prior)?; }
        }
        for binding in bindings { let _ = journal.offer_execution(&binding)?; }
        Ok(())
    }

    pub(super) fn record_local_session(&self, binding: &LocalSession) -> Result<()> {
        if let Some(journal) = self.home_session_journal.get()
            && !journal.closing()? {
                let selection = journal.selection()?;
                if selection.grant.subjects.contains(&binding.subject) {
                    if let Some(prior) = self.store.local_session(&binding.subject)?
                        && prior != *binding {
                            // Withdraw the old remote permission under its dispatch barrier
                            // BEFORE publishing a different local custody owner.
                            journal.revoke_execution(&execution_binding(&selection, &prior)?)?;
                        }
                    self.store.bind_local_session(binding)?;
                    if self.store.local_session(&binding.subject)?.as_ref() == Some(binding) {
                        let _ = journal.offer_execution(&execution_binding(&selection, binding)?)?;
                    }
                    return Ok(());
                }
        }
        self.store.bind_local_session(binding)
    }

    pub(super) fn revoke_session_incarnation(&self, subject: &str, incarnation: &str) -> Result<()> {
        if let Some(journal) = self.home_session_journal.get() {
            for binding in journal.reported_executions()?.into_iter().filter(|binding| binding.run.is_none() && binding.subject == subject) {
                if self.store.local_session_at(subject, &binding.incarnation)?.is_some_and(|local| local.native_incarnation == incarnation) {
                    journal.revoke_execution(&binding)?;
                }
            }
        }
        Ok(())
    }

    pub fn session_authority_changed(&self) -> Result<()> {
        self.program_run_authority_changed();
        if let Some(journal) = self.home_session_journal.get() {
            for subject in &journal.selection()?.grant.subjects { self.start_changed(subject); }
            for command in journal.live_start_commands()? {
                let Operation::Start { authorization } = &command.operation else { unreachable!("live Start snapshot") };
                if let Err(error) = require_start_helper(&self.store, self.runtime.as_ref(), authorization) {
                    journal.cancel_or_revoke(&command.execution_id, format!("{error:#}"),
                        |command, _, reason| self.store.revoke_remote_runtime_start(&command.execution_id, reason))?;
                }
            }
        }
        self.refresh_session_bindings()
    }

    fn session_live(&self, command: &Command) -> Result<LocalSession> {
        let binding = local_session(&self.store, self.runtime.as_ref(), &command.binding.subject, Some(&command.binding.incarnation))?;
        anyhow::ensure!(binding.runtime_id == command.binding.runtime, "session runtime binding changed");
        Ok(binding)
    }
}

impl Executor for NativeSessionExecutor {
    fn recovery_generation(&self) -> u64 { *self.native_generation.borrow() }

    fn enter(&self, command: &Command, authority: &rusqlite::Connection) -> Result<String> {
        let reconciler = &self.reconciler;
        anyhow::ensure!(command.binding.host == reconciler.host, "command names another physical host");
        if matches!(command.operation, Operation::Start { .. }) {
            let proof = match self.enter_start(command, authority) {
                Ok(proof) => proof,
                Err(error) => {
                    let reason = format!("{error:#}");
                    tracing::warn!(execution = %command.execution_id, error = %reason, "native Start entry failed");
                    let operation = reconciler.store.remote_runtime_start(&command.execution_id)
                        .with_context(|| format!("retaining native Start entry error: {reason}"))?;
                    if let Some(operation) = operation {
                        // A returned predispatch failure is positive nonentry evidence,
                        // but resource cleanup must still complete in recover_start.
                        // Do not replace dispatched producer proof with an entry error.
                        if !operation.dispatched {
                            reconciler.store.finish_remote_runtime_start(&operation.id,
                                &serde_json::json!({"phase": "refused-before-backend", "error": reason}))
                                .with_context(|| format!("retaining native Start entry error: {error:#}"))?;
                        }
                        return Err(error);
                    }
                    // No native operation was admitted, hence no resource/producer
                    // custody can need finalization. Retain the actual refusal rather
                    // than losing it when the journal recovers its entry barrier.
                    NativeProof::Complete { outcome: Outcome::Refused { reason } }
                }
            };
            return Ok(serde_json::to_string(&proof)?);
        }
        local::require_execution(authority, &command.binding)?;
        let binding = match reconciler.session_live(command) {
            Ok(binding) => binding,
            Err(error) if error.is::<LocalSessionUnavailable>() => {
                return Ok(serde_json::to_string(&NativeProof::Complete { outcome: Outcome::Refused { reason: error.to_string() } })?);
            }
            Err(error) => return Err(error),
        };
        let proof = match command.operation {
            Operation::Observe => NativeProof::Complete { outcome: Outcome::Observed { incarnation: binding.incarnation, state: "running".into() } },
            Operation::Stop => self.stop_proof(command, binding)?,
            Operation::Start { .. } => unreachable!("Start uses native admission before session resolution"),
        };
        Ok(serde_json::to_string(&proof)?)
    }

    fn cancel(&self, command: &Command, _authority: &rusqlite::Connection, reason: &str) -> Result<()> {
        if matches!(command.operation, Operation::Start { .. }) {
            self.reconciler.store.revoke_remote_runtime_start(&command.execution_id, reason)?;
        }
        Ok(())
    }

    fn recover<'a>(&'a self, recovery: Recovery) -> Pin<Box<dyn Future<Output = Result<Outcome>> + Send + 'a>> {
        Box::pin(async move {
            let reconciler = &self.reconciler;
            let mut recovery = recovery;
            if matches!(recovery.command.operation, Operation::Start { .. }) {
                let result = self.recover_start(&recovery).await;
                #[cfg(test)]
                if result.is_err() {
                    self.recovery_failures.send_modify(|value| *value += 1);
                }
                return result;
            }
            if recovery.custody_proof.is_none() && matches!(recovery.command.operation, Operation::Stop) {
                match reconciler.session_live(&recovery.command) {
                    Ok(_) => {
                        let journal = reconciler.home_session_journal.get().context("session journal is not installed")?;
                        recovery = journal.reconcile_entered_stop(&recovery.command.execution_id, |command, authority| {
                            // This is generation-fenced reconciliation of a retained Stop
                            // intent, not blind dispatch or permission to launch anything.
                            local::require_execution(authority, &command.binding)?;
                            let binding = reconciler.session_live(command)?;
                            Ok(serde_json::to_string(&self.stop_proof(command, binding)?)?)
                        })?;
                    }
                    Err(error) if error.is::<LocalSessionUnavailable>() => {}
                    Err(error) => return Err(error),
                }
            }
            if let Some(proof) = recovery.custody_proof.as_deref() {
                match serde_json::from_str::<NativeProof>(proof)? {
                    NativeProof::Complete { outcome } => {
                        anyhow::ensure!(!matches!(outcome, Outcome::Stopped { .. }), "stop proof lost its native custody snapshot");
                        return Ok(outcome);
                    }
                    NativeProof::Stop { binding, retired } => {
                        anyhow::ensure!(matches!(recovery.command.operation, Operation::Stop), "native stop proof names another operation");
                        if !retired {
                            anyhow::ensure!(!binding.terminal, "terminal retirement proof is unavailable");
                            self.await_exec_retirement(&binding).await?;
                        }
                        return self.finish_stop(&recovery.command, binding);
                    }
                    NativeProof::Start => anyhow::bail!("native Start proof names another operation"),
                }
            }
            match recovery.command.operation {
                Operation::Start { .. } => unreachable!("Start recovery uses native operation provenance"),
                Operation::Observe => Ok(Outcome::Cancelled { reason: "observation interrupted before its durable result".into() }),
                Operation::Stop => {
                    let live = &recovery.command.binding;
                    if reconciler.store.local_session_detached(&live.subject, &live.incarnation)? {
                        return Ok(Outcome::Detached { incarnation: live.incarnation.clone() });
                    }
                    let binding = reconciler.store.local_session_at(&live.subject, &live.incarnation)?.context("stop custody binding unavailable")?;
                    anyhow::ensure!(binding.runtime_id == live.runtime && binding.custody == SessionCustody::Owned, "stop recovery custody differs");
                    if binding.terminal {
                        // A daemon exit/absence is not whole-scope closure, and repeating
                        // native stop on an ended PTY cannot recover the lost result. Consume
                        // only the exact positive proof retained at the physical stop boundary.
                        anyhow::ensure!(reconciler.store.local_session_retirement_verified(&binding)?,
                            "native terminal scope retirement is unproven");
                    } else { self.await_exec_retirement(&binding).await?; }
                    self.finish_stop(&recovery.command, binding)
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "home_session_tests.rs"]
mod tests;

#[path = "home_session_stop.rs"]
mod ordinary_stop;
