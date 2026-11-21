//! Native finite work episodes share the standing-seat queue, claims, leases and reports.
//! They do not publish missions or ask the graph scheduler to execute an agent.
use super::*;
use crate::AdhocWorkStore as _;

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn settle_program_turn(
        &self, engine: &program::ProgramEngine, run: &MissionRunView,
        request: &program::Request, payload: &Value, cancelled: bool,
    ) -> Result<bool> {
        let turn: crate::model::NativeTurn = serde_json::from_value(payload.clone())?;
        turn.validate()?;
        let key = request.idempotency_key();
        let subject = crate::store::adhoc_work::native_turn_subject(&key);
        smallclaims::touched::note_read(|| subject.clone());
        let status = self.store.mission_run_status(&subject)?;
        let existed = status.is_some();
        if existed {
            let created = self.store.latest_claim(&subject, Some("mission-run.created"))?
                .context("native turn creation claim is missing")?;
            anyhow::ensure!(created.body["fields"]["native_turn"] == serde_json::to_value(&turn)?
                && created.body["fields"]["parent_step_run"] == request.step
                && created.body["fields"]["parent_attempt"].as_u64() == Some(u64::from(request.attempt))
                && created.actor.as_deref() == Some(run.subject.as_str()),
                "native turn durable key already owns another request");
        }
        if cancelled {
            match status.as_deref() {
                None => {
                    // Never create work merely to cancel it.
                    engine.resolve_resource(request, serde_json::json!({"_tag":"Failure","error":{
                        "_tag":"ResourceError","code":"cancelled","message":"Native turn was cancelled before admission."
                    }}))?;
                    return Ok(true);
                }
                Some("completed" | "failed" | "cancelled") => {}
                Some(_) => {
                    // Wait for the owned episode's actual terminal outcome, not an invented
                    // report. Durable parent linkage also survives loss of the host map.
                    return self.store.request_mission_run_cancellation(&subject, "its native turn scope was cancelled")
                        .map_err(Into::into);
                }
            }
        }
        let inherited = self.store.resource_context_for_run(&run.subject)?;
        if let Some(context) = inherited.as_ref().filter(|_| !existed) {
            let target = self.store.desired_subject_with_writer(&turn.agent)?
                .and_then(|(desired, _)| desired.member)
                .context("native turn target has no standing-seat placement")?;
            anyhow::ensure!(target.host == context.request.host,
                "native turn target host does not match its inherited resource authority host");
            let grant = context.grant.as_ref().context("inherited native turn lane has no grant")?;
            self.store.attach_resource_child(grant, &context.request.owner, &subject)?;
        }
        let episode = match self.store.create_native_turn(run, &request.step, request.attempt, &key, &turn) {
            Ok(episode) => episode,
            Err(error) => {
                if let Some(context) = inherited.as_ref().filter(|_| !existed) {
                    self.store.reject_resource_child(context.grant.as_ref().expect("grant checked"),
                        &context.request.owner, &subject, &error)?;
                }
                return Err(error.into());
            }
        };
        smallclaims::touched::note_read(|| episode.subject.clone());
        if !matches!(episode.status.as_str(), "completed" | "failed" | "cancelled") {
            return Ok(!existed);
        }
        if episode.status != "completed" {
            engine.resolve_resource(request, self.program_child_exit(&episode.subject, &episode.status)?)?;
            return Ok(true);
        }
        let report = self.store.latest_claim(&episode.subject, Some("program.turn.report"))?
            .context("completed native turn has no verified report")?;
        engine.resolve_resource(request, serde_json::json!({"_tag":"Success","value":report.body["fields"]["report"]}))?;
        Ok(true)
    }

    /// The ordinary run reader observes parent and product changes, so no polling or second
    /// scheduler is needed. Only the finite leaf's lifecycle is evaluated here.
    pub(super) fn evaluate_native_turn(&self, run: &MissionRunView) -> Result<bool> {
        if run.phase.starts_with("cleanup-") {
            return self.reconcile_mission_run_cleanup(run);
        }
        let created = self.store.latest_claim(&run.subject, Some("mission-run.created"))?
            .context("native turn creation claim is missing")?;
        let fields = &created.body["fields"];
        let turn: crate::model::NativeTurn = serde_json::from_value(fields["native_turn"].clone())?;
        let parent = fields["parent_step_run"].as_str().context("native turn lacks parent step")?;
        let attempt = fields["parent_attempt"].as_u64().context("native turn lacks parent attempt")?;
        let owner = self.store.step_run(parent)?;
        let live = if let Some(owner) = owner {
            let parent_run = self.store.mission_run(&owner.run)?;
            u64::from(owner.attempt) == attempt
                && program::RUNNING_STATES.contains(&owner.status.as_str())
                && parent_run.is_some_and(|parent_run| parent_run.generation == owner.generation
                    && matches!(parent_run.status.as_str(), "running" | "blocked" | "standing")
                    && parent_run.phase == "normal")
        } else { false };
        if !live {
            return self.store.request_mission_run_cancellation(&run.subject, "its exact native turn parent attempt ended")
                .map_err(Into::into);
        }
        let step = run.steps.first().context("native turn leaf is missing")?;
        if let Some(changed) = self.reconcile_worker_lease(step)? { return Ok(changed); }
        if matches!(step.status.as_str(), "failed" | "cancelled") {
            return self.store.set_mission_run_state(&run.subject, &step.status, "terminal", step.blocked_reason.as_deref());
        }
        if !step.worker_reported { return Ok(false); }
        for product in &turn.products {
            let subject = format!("resource/{}/{}", run.subject, product.path);
            let value = self.subject_value(&subject)?;
            let holds = value.as_ref().is_some_and(|value|
                actual_field(value, "kind").and_then(Value::as_str) == Some(product.kind.as_str())
                && product.fields.iter().all(|(name, expected)| actual_field(value, name) == Some(expected)));
            if !holds {
                let mut fields = product.fields.iter().map(|(key, value)| (key.clone(), value.clone())).collect::<Vec<_>>();
                if !product.fields.contains_key("kind") { fields.push(("kind".into(), Value::String(product.kind.clone()))); }
                return self.notify_missing_product(step, &MissingProduct { subject, fields });
            }
        }
        self.store.complete_native_turn(&run.subject, &self.store.step_executions(&step.subject)?)
    }

    /// Shared leaf ownership transitions; neither a native turn nor graph work can retain an
    /// ended incarnation or an expired lease. Work reports are still verified separately.
    pub(super) fn reconcile_worker_lease(&self, view: &StepRunView) -> Result<Option<bool>> {
        if view.status == "orphaned" {
            return self.store.set_step_state(&view.subject, "ready", Some("the prior worker incarnation ended")).map(Some);
        }
        if matches!(view.status.as_str(), "claimed" | "working" | "verifying" | "blocked") {
            if self.store.work_claim_is_orphaned(view)? {
                return self.store.set_step_state(&view.subject, "orphaned", Some("the exact worker incarnation ended")).map(Some);
            }
            if view.claim_expires_at_unix_ms.is_some_and(|expiry| expiry <= now_ms()) {
                return self.store.set_step_state(&view.subject, "ready", Some("the worker lease expired")).map(Some);
            }
        }
        Ok(None)
    }
}
