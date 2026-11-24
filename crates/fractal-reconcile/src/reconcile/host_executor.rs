//! Physical command custody inside the full node. Graph planning and result adoption stay outside.
//!
//! The existing runtime-start journal is the admission/receipt authority. Commands are recovered
//! from those same bytes, including legacy workspace admission, rather than a second journal.
use super::*;
use super::start_jobs::PreparedStart;
use crate::RuntimeStartsStore;
use crate::store::{RuntimeStartAdmission, RuntimeStartOperation};

pub(super) mod resource_commands;

const ENVIRONMENT_CAPTURE_ATTEMPTS: u32 = 3;

/// Immutable launch snapshot. Only the planner may select a different desired state or request.
/// The worker owner is the current custody owner, including after queued-operation recovery.
pub(super) struct StartCommand {
    id: String,
    owner: String,
    prepared: PreparedStart,
}


pub(super) enum StartReceipt {
    /// Capture did not enter a backend. The planner owns the next injected-clock deadline.
    CaptureRetry,
    /// The result is already durable; callers may now notify graph adoption.
    Persisted,
}

impl StartCommand {
    /// Admission and request receipts remain one transaction in the existing custody store.
    pub(super) fn admit(
        store: &Store,
        mut operation: RuntimeStartOperation,
        limit: usize,
    ) -> Result<Option<RuntimeStartOperation>> {
        match store.admit_runtime_start(&mut operation, limit)? {
            RuntimeStartAdmission::SeatBusy | RuntimeStartAdmission::Superseded => {
                Ok(None)
            }
            RuntimeStartAdmission::Admitted | RuntimeStartAdmission::CapacityBusy => {
                Ok(Some(operation))
            }
        }
    }

    pub(super) fn recovered(operation: &RuntimeStartOperation, owner: String) -> Result<Self> {
        Ok(Self {
            id: operation.id.clone(),
            owner,
            prepared: serde_json::from_value(operation.body.clone())?,
        })
    }

    pub(super) fn prepared(&self) -> &PreparedStart {
        &self.prepared
    }

    /// Backend entry is a durable, single-use fence. Replayed commands cannot spawn twice,
    /// and revocation before entry closes the producer without touching a physical runtime.
    pub(super) fn dispatch<R: RuntimeControl>(
        self,
        store: &Store,
        runtime: &R,
        observations: &host_observations::HostObservations,
        timeouts: u32,
    ) -> Result<StartReceipt> {
        let Self { id, owner, prepared } = self;
        let subject = &prepared.subject.subject;
        let guard = || -> Result<()> {
            anyhow::ensure!(!store.runtime_start_revoked(&id)?, "start operation was revoked");
            anyhow::ensure!(store.runtime_start_intent_current(&id)?,
                "restart lifecycle intent changed before launch");
            store.owned_desired_guard(&prepared.subject)?;
            anyhow::ensure!(store.helper_launcher_alive(subject)?,
                "the helper's launcher run has ended");
            anyhow::ensure!(store.selected_desired_token(subject)? == prepared.selected_token,
                "declaration changed before launch");
            anyhow::ensure!(prepared.requests_retained(store)?,
                "an original start request is no longer retained unchanged");
            if let Some(rollout) = prepared.member.environment.get(crate::rollout::OPERATION_ENV) {
                anyhow::ensure!(store.rollout(subject)?.is_some_and(|o| o.id == *rollout && o.phase == "starting"),
                    "rollout changed before launch");
            }
            Ok(())
        };
        let mut backend_entered = false;
        let mut contract = None;
        let mut runtime_before_start = None;
        let result = caught(|| {
            guard()?;
            observations.invalidate();
            if prepared.subject.kind == "agent" {
                let measured = runtime.launch_contract(store, &prepared.subject, &prepared.launch_member)?;
                store.record_invocation_launch_inputs(&id, &measured)?;
                contract = Some(measured);
            }
            if prepared.subject.kind == "agent" && prepared.member.driver.is_none() {
                runtime_before_start = store.latest_claim(subject, Some("runtime.observed"))?
                    .map(|claim| claim.id);
            }
            store.enter_runtime_start_backend(&id, &owner)?;
            backend_entered = true;
            let operation = fractal_runtime::LaunchOperation(id.clone());
            if resource_lanes::request(&prepared.launch_member)?.is_some() {
                runtime.start_resource_guarded(store, &prepared.launch_member, Some(&operation), &guard)?;
            } else {
                runtime.start_guarded(&prepared.launch_member, Some(&operation), &guard)?;
            }
            let (observation, observation_error) = match runtime.observe_start(&prepared.launch_member) {
                Ok(observation) => (observation, None),
                Err(error) => (None, Some(format!("{error:#}"))),
            };
            let incarnation = observation.and_then(|o| o.incarnation_id);
            let (verified, verification_error) = match incarnation.as_deref() {
                Some(incarnation) => match runtime.start_operation(&prepared.member.runtime_id, prepared.member.terminal, incarnation) {
                    Ok(operation) => (operation.as_deref() == Some(id.as_str()), None),
                    Err(error) => (false, Some(format!("{error:#}"))),
                },
                None => (false, observation_error),
            };
            Ok((incarnation, verified, verification_error))
        });
        observations.invalidate();
        if let Err(error) = &result
            && !backend_entered
            && fractal_runtime::is_shell_startup_timeout(error)
            && timeouts + 1 < ENVIRONMENT_CAPTURE_ATTEMPTS
        {
            return Ok(StartReceipt::CaptureRetry);
        }
        let result = result.map_err(|error| {
            if !backend_entered && fractal_runtime::is_shell_startup_timeout(&error) {
                error.context(format!("environment capture timed out after {} attempts", timeouts + 1))
            } else {
                error
            }
        });
        let mut result = match result {
            Ok((incarnation, verified, error)) => serde_json::json!({"incarnation": if verified {incarnation.clone()} else {None}, "candidate_incarnation": incarnation, "verification": if verified {"operation-incarnation"} else {"launch-accepted-unverified"}, "verification_error": error}),
            Err(error) => {
                let rejected = error.downcast_ref::<fractal_runtime::StartRejected>()
                    .is_some_and(|proof| proof.operation.as_deref() == Some(id.as_str()));
                let (closed, verification_error) = if !backend_entered || rejected { (true, None) }
                    else { match runtime.start_operation_closed(&prepared.launch_member) {
                        Ok(closed) => (closed, None),
                        Err(error) => (false, Some(format!("{error:#}"))),
                    }};
                serde_json::json!({"error": format!("{error:#}"), "producer_closed": closed,
                    "closure_kind": if !backend_entered { "before-backend" } else if rejected { "native-rejection" } else { "positive-producer-proof" },
                    "verification": if closed { "producer-closed" } else { "producer-open-unverified" },
                    "verification_error": verification_error})
            },
        };
        if result["verification"] == "operation-incarnation"
            && prepared.subject.kind == "agent" && prepared.member.driver.is_none()
            && let Some(incarnation) = result["incarnation"].as_str()
            && let Ok(Some((pid, token))) =
                runtime.start_process_identity(&prepared.launch_member, incarnation)
        {
            let fields = member_fields(&prepared.member, "starting", Some(incarnation), false);
            result["runtime_pid"] = Value::from(pid);
            result["runtime_pid_start_token"] = Value::from(token.to_string());
            result["runtime_start_fields"] = serde_json::json!(fields);
            result["runtime_before_start"] = serde_json::json!(runtime_before_start);
        }
        result["launch_contract"] = serde_json::json!(contract);
        result["operation_id"] = Value::from(id.clone());
        result["subject"] = Value::from(subject.clone());
        result["origin"] = Value::from(store.origin());
        result["owner"] = Value::from(owner);
        result["phase"] = Value::from(if result["error"].is_string() { "failed" } else if result["verification"] == "operation-incarnation" { "completed" } else { "unknown" });
        result["expected_incarnation"] = serde_json::json!(prepared.expected_incarnation);
        result["runtime_kind"] = Value::from(if prepared.member.terminal { "pty" } else { "exec" });
        result["desired_token"] = Value::from(prepared.desired_token);
        result["runtime_id"] = Value::from(prepared.member.runtime_id);
        result["terminal"] = Value::from(prepared.member.terminal);
        result["start_request"] = Value::from(prepared.start_request);
        result["control_request"] = serde_json::json!(prepared.control_request);
        if result["verification"] == "operation-incarnation"
            && let Some(incarnation) = result["incarnation"].as_str()
        {
            store.bind_local_session(&crate::store::local_sessions::LocalSession {
                subject: subject.clone(),
                runtime_id: result["runtime_id"].as_str().context("start runtime ID missing")?.into(),
                incarnation: incarnation.into(),
                native_incarnation: incarnation.into(),
                terminal: prepared.member.terminal,
                driver: prepared.member.driver.clone(),
                lifecycle: prepared.member.lifecycle.clone(),
                custody: crate::store::local_sessions::SessionCustody::Owned,
                start_operation: Some(id.clone()),
                invocation: None,
            })?;
        }
        store.finish_runtime_start(&id, &result)?;
        Ok(StartReceipt::Persisted)
    }
}

/// The planner admits a durable action request before handing physical control to the host.
/// `incarnation` is the exact observed process/PTY identity, never a PID-only substitute.
pub(super) struct StopCommand<'a> {
    request: &'a crate::model::ClaimRecord,
    runtime_id: &'a str,
    terminal: bool,
    incarnation: Option<&'a str>,
    action: StopAction,
}

pub(super) enum StopAction {
    Terminate { retirement_proof: bool },
    Kill,
}

pub(super) struct StopReceipt<'a> {
    pub request: &'a str,
    pub retirement_verified: bool,
}

impl<'a> StopCommand<'a> {
    pub(super) fn new(
        request: &'a crate::model::ClaimRecord,
        runtime_id: &'a str,
        terminal: bool,
        incarnation: Option<&'a str>,
        action: StopAction,
    ) -> Self {
        Self { request, runtime_id, terminal, incarnation, action }
    }

    /// Signal-only completion is not retirement proof. Only the native exact-generation
    /// retirement path may produce that proof; the graph retains its existing receipt writer.
    pub(super) fn dispatch<R: RuntimeControl>(
        self,
        runtime: &R,
        observations: &host_observations::HostObservations,
    ) -> Result<StopReceipt<'a>> {
        observations.invalidate();
        let result = (|| -> Result<bool> {
            Ok(match self.action {
            StopAction::Terminate { retirement_proof: true } => {
                runtime.stop_with_retirement_proof(
                    self.runtime_id, self.terminal,
                    self.incarnation.context("retirement requires an exact incarnation")?,
                )?
            }
            StopAction::Terminate { retirement_proof: false } => {
                runtime.stop(self.runtime_id, self.terminal, self.incarnation)?;
                false
            }
            StopAction::Kill => {
                runtime.kill(self.runtime_id, self.terminal, self.incarnation)?;
                false
            }
            })
        })();
        observations.invalidate();
        let retirement_verified = result?;
        Ok(StopReceipt { request: &self.request.id, retirement_verified })
    }
}
