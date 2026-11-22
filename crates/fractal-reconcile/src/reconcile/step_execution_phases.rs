//! Native exec time belongs to the executor, not the coordinator's claim/dispatch clock.
//! These durable internal claims deliberately do not extend the frozen client-v0 Work schema.
use super::*;

const KIND: &str = "step.execution-phase";
pub(super) const DEFAULT_DISPATCH_BUDGET_MS: u64 = 3_600_000;
const DEFAULT_OVERALL_BUDGET_MS: u64 = 86_400_000;

fn phase_at(claims: &[crate::model::ClaimRecord], attempt: u32, exec: &str, phase: &str) -> Option<u128> {
    claims.iter().filter_map(|claim| {
        let fields = &claim.body["fields"];
        (fields["attempt"].as_u64() == Some(u64::from(attempt))
            && fields["exec"].as_str() == Some(exec)
            && fields["phase"].as_str() == Some(phase))
            .then(|| fields["at_unix_ms"].as_u64().map(u128::from)).flatten()
    }).min()
}

fn timeout_observed(claims: &[crate::model::ClaimRecord], view: &StepRunView, exec: &str) -> bool {
    claims.iter().any(|claim| {
        let fields = &claim.body["fields"];
        fields["attempt"].as_u64() == Some(u64::from(view.attempt))
            && fields["exec"].as_str() == Some(exec)
            && fields["phase"].as_str() == Some("execution-timeout")
            && fields["timeout_extension_ms"].as_u64().unwrap_or(0) == view.timeout_extension_ms
    })
}

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn record_step_exec_phase(&self, view: &StepRunView, desired: &DesiredSubject, phase: &str,
        at: u128, incarnation: Option<&str>, reason: Option<&str>) -> Result<()> {
        if desired.kind != "exec" || desired.member.as_ref().is_none_or(|member| member.terminal) {
            return Ok(());
        }
        let claims = self.store.claims_for(&view.subject, Some(KIND))?;
        if if phase == "execution-timeout" { timeout_observed(&claims, view, &desired.subject) }
            else { phase_at(&claims, view.attempt, &desired.subject, phase).is_some() } { return Ok(()); }
        let mut fields = BTreeMap::from([
            ("attempt".into(), Value::from(view.attempt)),
            ("phase".into(), Value::from(phase)),
            ("exec".into(), Value::from(desired.subject.clone())),
            ("at_unix_ms".into(), Value::from(u64::try_from(at).unwrap_or(u64::MAX))),
            ("host".into(), Value::from(self.host.clone())),
        ]);
        if let Some(incarnation) = incarnation { fields.insert("incarnation_id".into(), Value::from(incarnation)); }
        if let Some(incarnation) = incarnation
            && let Some(member) = desired.member.as_ref()
            && let Some(operation) = self.runtime.start_operation(&member.runtime_id, false, incarnation)?
        {
            fields.insert("start_operation".into(), Value::from(operation));
        }
        if let Some(reason) = reason { fields.insert("reason".into(), Value::from(reason)); }
        if phase == "execution-timeout" {
            fields.insert("timeout_extension_ms".into(), Value::from(view.timeout_extension_ms));
        }
        self.store.append_claim(&ClaimInput {
            subject: view.subject.clone(), kind: KIND.into(), actor: None, fields,
            evidence: Vec::new(), expected_subject: None,
            idempotency_key: Some(format!("step-phase:{}:{}:{}:{phase}:{}", view.subject, view.attempt, desired.subject,
                if phase == "execution-timeout" { view.timeout_extension_ms } else { 0 })),
        })?;
        Ok(())
    }

    pub(super) fn record_native_exec_phase(&self, desired: &DesiredSubject, phase: &str, at: u128,
        incarnation: Option<&str>) -> Result<()> {
        if desired.kind != "exec" { return Ok(()); }
        let Some(owner) = desired.owner_step.as_deref() else { return Ok(()); };
        let Some(view) = self.store.step_run(owner)? else { return Ok(()); };
        let claims = self.store.claims_for(&view.subject, Some(KIND))?;
        if phase_at(&claims, view.attempt, &desired.subject, "dispatched").is_none() {
            return Ok(());
        }
        self.record_step_exec_phase(&view, desired, phase, at, incarnation, None)
    }

    pub(super) fn record_native_exec_observation(&self, desired: &DesiredSubject, member: &MemberSpec,
        observation: &RuntimeObservation) -> Result<()> {
        if desired.kind != "exec" || member.terminal || member.host != self.host {
            return Ok(());
        }
        let Some(owner) = desired.owner_step.as_deref() else { return Ok(()); };
        let Some(view) = self.store.step_run(owner)? else { return Ok(()); };
        let claims = self.store.claims_for(owner, Some(KIND))?;
        if phase_at(&claims, view.attempt, &desired.subject, "dispatched").is_none() {
            return Ok(());
        }
        let Some(incarnation) = observation.incarnation_id.as_deref() else { return Ok(()); };
        let Some(operation) = self.runtime.start_operation(&member.runtime_id, false, incarnation)? else { return Ok(()); };
        let Some(token) = self.store.selected_desired_token(&desired.subject)? else { return Ok(()); };
        let launched_current = self.store.observations_for(&desired.subject, "runtime.action.succeeded")?
            .iter().any(|claim| {
                let fields = &claim.body["fields"];
                fields["operation"].as_str() == Some(operation.as_str())
                    && fields["incarnation_id"].as_str() == Some(incarnation)
                    && fields["desired_token"].as_str() == Some(token.as_str())
            });
        // A retry can reuse the subject while its previous physical generation is still
        // observed. Do not attribute that generation's start/exit to this new attempt.
        if !launched_current { return Ok(()); }
        if let Some(started) = self.runtime.exec_started_at(&member.runtime_id, incarnation)? {
            self.record_native_exec_phase(desired, "exec-started", started, Some(incarnation))?;
            let payload = self.runtime.exec_payload_state(&member.runtime_id, incarnation)?;
            let exited = match payload {
                Some(fractal_runtime::payload_receipt::PayloadState::Exited { at_unix_ms }) => Some(u128::from(at_unix_ms)),
                _ => self.native_exec_payload_exit(&desired.subject, started)?,
            };
            if let Some(exited) = exited {
                self.record_native_exec_phase(desired, "exec-exited", exited, Some(incarnation))?;
            } else if observation.status == "running"
                && payload == Some(fractal_runtime::payload_receipt::PayloadState::Running)
                && let Some(timeout) = view.timeout_ms
            {
                let now = now_ms();
                let (waited, paused_until) = self.native_exec_wait(&view, started, now)?;
                let budget = u128::from(timeout.saturating_add(view.timeout_extension_ms));
                if now.saturating_sub(started).saturating_sub(waited) >= budget {
                    self.record_step_exec_phase(&view, desired, "execution-timeout", now, Some(incarnation), None)?;
                } else {
                    let deadline = paused_until.unwrap_or_else(|| started.saturating_add(budget).saturating_add(waited));
                    self.arm_deadline_at(&format!("native-executor:{}", desired.subject), deadline,
                        crate::reconcile_telemetry::WakeCause::TimerStepTimeout);
                }
            }
        }
        Ok(())
    }

    fn native_exec_payload_exit(&self, subject: &str, started: u128) -> Result<Option<u128>> {
        Ok(self.store.newest_claim_where(subject, "runtime.observed", |claim| {
            claim.body["fields"]["exec_exited_at_unix_ms"].as_u64()
                .is_some_and(|at| u128::from(at) >= started)
        })?.and_then(|claim| claim.body["fields"]["exec_exited_at_unix_ms"].as_u64().map(u128::from)))
    }

    fn native_exec_wait(&self, view: &StepRunView, started: u128, end: u128) -> Result<(u128, Option<u128>)> {
        let claims = self.store.claims_for(&view.subject, Some("work.progress"))?;
        Ok(fractal_kernel::step_execution::execution_wait(claims.iter().map(|claim|
            (claim.kind.as_str(), &claim.body, claim.accepted_at_unix_ms)), view.attempt, started, end))
    }

    pub(super) fn native_step_execs(&self, view: &StepRunView) -> Result<Vec<DesiredSubject>> {
        Ok(self.store.desired_subjects_for_owner_step(&view.subject)?.into_iter()
            .filter(|desired| desired.kind == "exec"
                && desired.owner_step.as_deref() == Some(view.subject.as_str())
                && desired.member.as_ref().is_some_and(|member| !member.terminal))
            .collect())
    }

    /// None leaves non-native work on its existing worker/wait clock. The executor's durable
    /// start and end bound native execution even if either claim arrives much later.
    pub(super) fn native_step_timed_out(&self, view: &StepRunView, timeout: u64) -> Result<Option<bool>> {
        let execs = self.native_step_execs(view)?;
        if execs.is_empty() { return Ok(None); }
        let claims = self.store.claims_for(&view.subject, Some(KIND))?;
        if execs.iter().any(|desired|
            phase_at(&claims, view.attempt, &desired.subject, "dispatched").is_none())
        {
            // Enrollment is additive; existing attempts keep their original bounded clock.
            return Ok(None);
        }
        for desired in &execs {
            // Every declared exec is enrolled in this attempt.
            if timeout_observed(&claims, view, &desired.subject) {
                return Ok(Some(true));
            }
            let Some(started) = phase_at(&claims, view.attempt, &desired.subject, "exec-started") else { continue; };
            let ended = phase_at(&claims, view.attempt, &desired.subject, "exec-exited")
                .or(self.native_exec_payload_exit(&desired.subject, started)?);
            if let Some(ended) = ended {
                let (waited, _) = self.native_exec_wait(view, started, ended)?;
                if ended.saturating_sub(started).saturating_sub(waited) >= u128::from(timeout) {
                    return Ok(Some(true));
                }
            }
            // Absence of an exit on the coordinator is not proof of a live remote process.
            // Only the executor can report that its physical registry is still running.
        }
        Ok(Some(false))
    }

    /// Dispatch/admission and result replication each get one unextended step budget. The
    /// coordinator also bounds the whole attempt to dispatch + execution + result budgets.
    /// A missing start/result is never evidence that an exec exceeded its execution budget.
    pub(super) fn expire_native_dispatch_budget(&self, view: &StepRunView, timeout: u64) -> Result<bool> {
        let execs = self.native_step_execs(view)?;
        if execs.is_empty() { return Ok(false); }
        let claims = self.store.claims_for(&view.subject, Some(KIND))?;
        let now = now_ms();
        let budget = u128::from(timeout);
        for desired in &execs {
            let Some(dispatched) = phase_at(&claims, view.attempt, &desired.subject, "dispatched") else { continue; };
            let started = phase_at(&claims, view.attempt, &desired.subject, "exec-started");
            let exited = phase_at(&claims, view.attempt, &desired.subject, "exec-exited");
            let identity = claims.iter().find_map(|claim| {
                let fields = &claim.body["fields"];
                (fields["attempt"].as_u64() == Some(u64::from(view.attempt))
                    && fields["exec"].as_str() == Some(desired.subject.as_str())
                    && fields["phase"].as_str() == Some("exec-started"))
                    .then(|| fields["incarnation_id"].as_str().zip(fields["start_operation"].as_str())).flatten()
            });
            let result = self.store.latest_actual_value(&desired.subject)?;
            let matches = |actual: &Value, incarnation: &str, operation: &str| {
                actual_field(actual, "incarnation_id").and_then(Value::as_str) == Some(incarnation)
                    && actual_field(actual, "start_operation").and_then(Value::as_str) == Some(operation)
            };
            let result_present = identity.is_some_and(|(incarnation, operation)| {
                result.as_ref().is_some_and(|actual|
                    actual_field(actual, "status").and_then(Value::as_str) == Some("exited")
                        && matches(actual, incarnation, operation))
            });
            let replicated = identity.is_some_and(|(incarnation, operation)| claims.iter().any(|claim| {
                let fields = &claim.body["fields"];
                fields["attempt"].as_u64() == Some(u64::from(view.attempt))
                    && fields["exec"].as_str() == Some(desired.subject.as_str())
                    && fields["phase"].as_str() == Some("result-replicated")
                    && matches(&claim.body, incarnation, operation)
            }));
            if result_present || replicated {
                if !replicated {
                    let (incarnation, operation) = identity.expect("matching result has a start identity");
                    self.store.append_claim(&ClaimInput {
                        subject: view.subject.clone(), kind: KIND.into(), actor: None,
                        fields: BTreeMap::from([
                            ("attempt".into(), Value::from(view.attempt)), ("phase".into(), Value::from("result-replicated")),
                            ("exec".into(), Value::from(desired.subject.clone())), ("at_unix_ms".into(), Value::from(now as u64)),
                            ("host".into(), Value::from(self.host.clone())), ("incarnation_id".into(), Value::from(incarnation)),
                            ("start_operation".into(), Value::from(operation)),
                        ]),
                        evidence: Vec::new(), expected_subject: None,
                        idempotency_key: Some(format!("step-phase:{}:{}:{}:result-replicated:0", view.subject, view.attempt, desired.subject)),
                    })?;
                }
                // Delivery is finished. Review/product gates own any subsequent waiting.
                // Execution-overrun evaluation remains independent of this closed guard.
                continue;
            }
            let overall_budget = view.timeout_ms.map_or(DEFAULT_OVERALL_BUDGET_MS, |_| timeout.saturating_mul(3))
                .saturating_add(view.timeout_extension_ms);
            let overall = dispatched.saturating_add(u128::from(overall_budget));
            let (deadline, waiting) = match (started, exited) {
                (None, _) => (dispatched.saturating_add(budget).min(overall), "dispatch, replication or executor admission"),
                (_, Some(exited)) => (exited.saturating_add(budget).min(overall), "result replication"),
                (Some(_), None) => (overall, "executor completion or result replication"),
            };
            if now >= deadline {
                // Known execution overrun belongs to the execution fault path, not this guard.
                if view.timeout_ms.is_some() && (timeout_observed(&claims, view, &desired.subject)
                    || if let Some((started, exited)) = started.zip(exited) {
                        let (waited, _) = self.native_exec_wait(view, started, exited)?;
                        exited.saturating_sub(started).saturating_sub(waited)
                            >= budget.saturating_add(u128::from(view.timeout_extension_ms))
                    } else { false }) {
                    continue;
                }
                let reason = format!("dispatch-stalled: `{}` exceeded its {timeout}ms {waiting} budget (overall guard {overall_budget}ms); no execution timeout was observed",
                    desired.subject);
                self.record_step_exec_phase(view, desired, "dispatch-stalled", now, None, Some(&reason))?;
                self.store.set_step_state_unless_terminal(&view.subject, "failed", Some(&reason))?;
                self.store.record_operational_failure(
                    &format!("dispatch-stalled:{}:{}", view.subject, view.attempt),
                    &AttentionRequest {
                        reviewer: "person/operator".into(), title: "A native step's dispatch stalled".into(),
                        reason, severity: "error".into(), targets: vec![view.subject.clone()],
                        actor: RECONCILER_ACTOR.into(),
                        idempotency_key: format!("dispatch-stalled:{}:{}", view.subject, view.attempt),
                    })?;
                return Ok(true);
            }
            self.arm_deadline_at(&format!("native-dispatch:{}:{}", view.subject, desired.subject), deadline,
                crate::reconcile_telemetry::WakeCause::TimerStepTimeout);
        }
        Ok(false)
    }
}
