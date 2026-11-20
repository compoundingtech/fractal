//! Idle-only automatic returns share the ordinary requested-restart state machine.
use super::*;
use crate::config::model_return::{Config, Overrides};
use crate::store::model_fallback::ModelFallbackCause;

#[derive(Default)]
struct Settings(Mutex<Config>);

fn declaration(desired: &Value) -> Overrides {
    let block = desired["children"].as_array().and_then(|nodes| nodes.iter().find(|node| node["name"] == "model-return"));
    let argument = |name: &str| block.and_then(|block| block["children"].as_array())
        .and_then(|nodes| nodes.iter().find(|node| node["name"] == name))
        .and_then(|node| node["arguments"].as_array()).and_then(|args| args.first());
    Overrides { enabled: argument("enabled").and_then(Value::as_bool), max_drift: argument("max-drift").and_then(Value::as_u64), max_per_hour: argument("max-per-hour").and_then(Value::as_u64) }
}

impl<R: RuntimeControl> Reconciler<R> {
    pub fn set_model_return_policy(&self, config: &Config) -> Result<()> {
        config.validate()?;
        let settings = self.store.extension::<Settings>();
        let mut current = settings.0.lock().unwrap_or_else(PoisonError::into_inner);
        if *current != *config { *current = config.clone(); self.signal_changed(); }
        Ok(())
    }

    /// Rechecked both before requesting and immediately before releasing a queued request.
    fn model_return_eligible(&self, subject: &DesiredSubject, incarnation: &str, now: u128) -> Result<Option<String>> {
        let settings = self.store.extension::<Settings>();
        let policy = settings.0.lock().unwrap_or_else(PoisonError::into_inner).resolve(&subject.subject, &declaration(&subject.desired));
        if !policy.enabled || policy.max_per_hour == 0 { return Ok(None); }
        let Some(declared) = crate::store::model_drift::declared_harness_model(&subject.desired) else { return Ok(None) };
        let Some(live) = self.store.latest_usage_model_observation(&subject.subject, incarnation)? else { return Ok(None) };
        if crate::store::model_drift::model_drift_state(Some(declared), Some(&live), now, 60_000) != crate::store::model_drift::ModelDriftState::Drifted { return Ok(None); }
        let Some(since) = self.store.model_drift_since(&subject.subject, incarnation, declared, &live.model)? else { return Ok(None) };
        if since > now || now - since <= policy.max_drift_ms {
            if since <= now { self.arm_restart(&format!("model-return:{}", subject.subject), since.saturating_add(policy.max_drift_ms).saturating_add(1)); }
            return Ok(None);
        }
        let Some(fallback) = self.store.latest_model_fallback(&subject.subject, incarnation)? else { return Ok(None) };
        if fallback.cause != ModelFallbackCause::Transient
            || fallback.observed_at_ms > now
            || fractal_kernel::pricing::declared_model_matches(&fallback.to, &live.model) != Some(true)
            || fractal_kernel::pricing::declared_model_matches(declared, &fallback.from) != Some(true)
        { return Ok(None); }
        let quiet = crate::restart::fresh_harness(&self.store, &subject.subject, incarnation, now)?
            .as_ref().is_some_and(crate::restart::quiet);
        if !quiet { return Ok(None); }
        Ok(Some(fallback.episode_id))
    }

    pub(super) fn reconcile_model_return(&self, subject: &DesiredSubject, incarnation: &str) -> Result<()> {
        self.reconcile_model_return_at(subject, incarnation, now_ms())
    }

    fn reconcile_model_return_at(&self, subject: &DesiredSubject, incarnation: &str, now: u128) -> Result<()> {
        if let Some(fallback) = self.store.latest_model_fallback(&subject.subject, incarnation)?
            && fallback.cause != ModelFallbackCause::Transient
        {
            let reason = match fallback.cause {
                ModelFallbackCause::Quota => "quota fallback requires capacity or account remediation; automatic model return is disabled",
                ModelFallbackCause::Auth => "authentication fallback requires credential remediation; automatic model return is disabled",
                _ => "fallback cause is unknown; model drift needs attention and cannot authorize a restart",
            };
            self.record_once(&subject.subject, "runtime.reconcile-decision", BTreeMap::from([
                ("key".into(), Value::String(format!("model-return-held:{incarnation}:{}", fallback.episode_id))),
                ("decision".into(), Value::String("model-return-held".into())),
                ("reason".into(), Value::String(reason.into())),
            ]))?;
            return Ok(());
        }
        let Some(episode) = self.model_return_eligible(subject, incarnation, now)? else { return Ok(()) };
        let Some(token) = self.store.selected_desired_token(&subject.subject)? else { return Ok(()) };
        let Some(member) = subject.member.as_ref() else { return Ok(()) };
        self.store.owned_desired_guard(subject)?;
        // Each admitted request advances a durable generation. A canceled request must not
        // permanently consume its fallback episode's identity after the hourly budget expires.
        let generation = self.store.model_return_attempt_count(&subject.subject, incarnation)?;
        let key = format!("model-return:{}", hex::encode(sha2::Sha256::digest(format!("{}:{incarnation}:{token}:{episode}:{generation}", subject.subject).as_bytes())));
        if self.store.operation_claim(&key)?.is_some() { return Ok(()); }
        if let Some(request) = crate::restart::latest_request(&self.store, &subject.subject)?
            && self.store.operation_claim(&format!("agent-restart-completed:{}", request.id))?.is_none()
        { return Ok(()); }
        let input = ClaimInput {
            subject: subject.subject.clone(), kind: "runtime.action.requested".into(), actor: Some(RECONCILER_ACTOR.into()),
            fields: BTreeMap::from([
                ("action".into(), Value::String(crate::restart::WHEN_IDLE.into())),
                ("operation".into(), Value::String(crate::store::model_return::OPERATION.into())),
                ("model_return_attempt".into(), Value::from(generation.checked_add(1).context("model return attempt counter overflow")?)),
                ("cause".into(), Value::String(episode)),
                ("host".into(), Value::String(self.host.clone())),
                ("runtime_id".into(), Value::String(member.runtime_id.clone())),
                ("terminal".into(), Value::Bool(member.terminal)),
                ("incarnation_id".into(), Value::String(incarnation.into())),
                ("reason".into(), Value::String("fresh continuous model drift exceeded policy duration after a transient fallback; returning at a settled idle boundary".into())),
            ]), evidence: vec![token], expected_subject: None, idempotency_key: Some(key),
        };
        match self.store.admit_model_return(&input, now) {
            Ok(_) => self.signal_changed(),
            Err(error) if error.code == "model-return-budget" => {
                if let Some(deadline) = self.store.model_return_budget_resumes_at(&subject.subject)? {
                    self.arm_restart(&format!("model-return-budget:{}", subject.subject), deadline);
                }
            },
            Err(error) if matches!(error.code, "model-return-host-busy" | "model-return-stale") => {},
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// Cancellation closes the ordinary request, releasing the durable host slot.
    pub(super) fn cancel_stale_model_return(&self, subject: &DesiredSubject, request: &crate::model::ClaimRecord, observation: Option<&RuntimeObservation>) -> Result<bool> {
        if request.body["fields"]["operation"] != crate::store::model_return::OPERATION
            || self.store.operation_claim(&format!("agent-restart-attempt:{}", request.id))?.is_some()
            || self.store.operation_claim(&format!("agent-restart-completed:{}", request.id))?.is_some()
            || self.store.operation_claim(&crate::restart::release_key(request))?.is_some()
        { return Ok(false); }
        let incarnation = request.body["fields"]["incarnation_id"].as_str().unwrap_or("");
        let current = observation.is_some_and(|runtime| runtime.status == "running" && runtime.incarnation_id.as_deref() == Some(incarnation));
        let token = self.store.selected_desired_token(&subject.subject)?;
        let valid = current && token.as_deref() == request.body.pointer("/evidence/0").and_then(Value::as_str)
            && self.model_return_eligible(subject, incarnation, now_ms())?.as_deref() == request.body["fields"]["cause"].as_str();
        if !valid {
            self.retire_restart_request(&subject.subject, request, "model return cancelled: policy, declaration, incarnation, fallback, fresh drift, or settled idle proof changed")?;
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{FakeRuntime, apply_source};
    use serde_json::json;

    struct Fixture {
        store: Arc<Store>,
        reconciler: Reconciler<FakeRuntime>,
        subject: DesiredSubject,
        now: u128,
    }
    impl Fixture {
        fn new() -> Self {
            let store = Arc::new(Store::open_memory("node").unwrap());
            apply_source(&store, "version 2\nagent \"example\" { workspace \"/tmp\"; harness \"omp\" { model \"claude-opus-5-5\"; } }\n", "return-fixture");
            let subject = store.desired_subjects().unwrap().into_iter().find(|subject| subject.subject == "agent/example").unwrap();
            let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
            let fixture = Self { store, reconciler, subject, now: now_ms() };
            fixture.claim("runtime.observed", json!({"status":"running","incarnation_id":"one"}));
            fixture.claim("harness.session-file", json!({"harness":"omp","session_id":"native-one","incarnation_id":"one"}));
            fixture.usage("gpt-6.1-sol", fixture.now);
            fixture.quiet("idle", true, 0, fixture.now);
            fixture.fallback("transient", "fallback-one");
            fixture.drift(fixture.now - 600_001);
            fixture
        }
        fn claim(&self, kind: &str, fields: Value) {
            self.store.append_claim(&ClaimInput {
                subject: self.subject.subject.clone(), kind: kind.into(), actor: Some(self.subject.subject.clone()),
                fields: fields.as_object().unwrap().iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        fn usage(&self, model: &str, at: u128) {
            self.claim("harness.usage", json!({"driver":"omp","model":model,"semantics":"context_occupancy","incarnation_id":"one","observed_at_unix_ms":at as u64}));
        }
        fn quiet(&self, state: &str, quiescent: bool, jobs: u64, at: u128) {
            self.claim("harness.observed", json!({"driver":"omp","state":state,"incarnation_id":"one","observed_at_ms":at as u64,"quiescent":quiescent,"blocking":[],"background_jobs":jobs}));
        }
        fn fallback(&self, cause: &str, event: &str) {
            self.claim("harness.model-fallback.observed", json!({"observation":"selected","incarnation_id":"one","native_session":"native-one","source_event":event,"episode_id":event,"to":"gpt-6.1-sol","observed_at_unix_ms":self.now as u64,"reason":"native-model-selection"}));
            self.claim("harness.model-fallback.observed", json!({"observation":"applied","incarnation_id":"one","native_session":"native-one","source_event":event,"episode_id":event,"from":"claude-opus-5-5","to":"gpt-6.1-sol","cause":cause,"observed_at_unix_ms":self.now as u64,"reason":"native-retry-fallback"}));
        }
        fn drift(&self, since: u128) {
            let episode = model_drift_episode(&self.subject.subject);
            if let Some(failure) = self.store.operational_failure(&episode).unwrap().filter(|failure| failure.status == "pending") {
                self.store.recover_operational_failure(&episode, "new test onset", &format!("reset:{}", failure.request)).unwrap();
            }
            self.store.record_runtime_failure(&episode, &AttentionRequest {
                reviewer: self.subject.subject.clone(), title: MODEL_DRIFT_TITLE.into(),
                reason: ModelDrift { declared:"claude-opus-5-5".into(), live:"gpt-6.1-sol".into(), since_ms:since }.reason(),
                severity:"error".into(), targets:vec![self.subject.subject.clone()], actor:RECONCILER_ACTOR.into(),
                idempotency_key:format!("fixture-drift:{since}:{}", self.store.claims_for(&episode, Some("operational.failure")).unwrap().len()),
            }, MODEL_DRIFT_CONDITION).unwrap();
        }
        fn eligible(&self) -> bool {
            self.reconciler.model_return_eligible(&self.subject, "one", self.now).unwrap().is_some()
        }
    }
    #[test]
    fn requires_strict_duration_fresh_comparable_current_incarnation() {
        let fixture = Fixture::new();
        assert!(fixture.eligible());
        assert!(fixture.reconciler.model_return_eligible(&fixture.subject, "other", fixture.now).unwrap().is_none());
        fixture.drift(fixture.now - 600_000);
        assert!(!fixture.eligible());
        fixture.drift(fixture.now - 600_001);
        fixture.usage("gpt-6.1-sol", fixture.now - 60_001);
        assert!(!fixture.eligible());
        fixture.usage("gpt-6.1-sol", fixture.now + 1);
        assert!(!fixture.eligible());
        fixture.usage("unregistered-model", fixture.now);
        assert!(!fixture.eligible());
        fixture.usage("claude-opus-5-5", fixture.now);
        assert!(!fixture.eligible());
    }
    #[test]
    fn requires_positive_transient_fallback_and_settled_idle() {
        let fixture = Fixture::new();
        for cause in ["quota", "auth", "unknown"] {
            fixture.fallback(cause, &format!("fallback-{cause}"));
            assert!(!fixture.eligible());
        }
        fixture.fallback("transient", "fallback-transient");
        assert!(fixture.eligible());
        for (state, quiescent, jobs, at) in [
            ("working", true, 0, fixture.now),
            ("idle", false, 0, fixture.now),
            ("idle", true, 1, fixture.now),
            ("idle", true, 0, fixture.now - 600_000),
        ] {
            fixture.quiet(state, quiescent, jobs, at);
            assert!(!fixture.eligible());
        }
        fixture.quiet("idle", true, 0, fixture.now);
        assert!(fixture.eligible());
        fixture.claim("harness.model-fallback.observed", json!({"observation":"selected","incarnation_id":"one","native_session":"native-one","source_event":"manual-switch","episode_id":"manual-switch","to":"gpt-6.1-sol","observed_at_unix_ms":fixture.now as u64,"reason":"native-model-selection"}));
        assert!(!fixture.eligible());
    }
    #[test]
    fn policy_off_zero_budget_and_release_revalidation_cancel_without_runtime_actions() {
        let fixture = Fixture::new();
        fixture.reconciler.reconcile_model_return(&fixture.subject, "one").unwrap();
        let request = crate::restart::latest_request(&fixture.store, &fixture.subject.subject).unwrap().unwrap();
        assert_eq!(fixture.store.model_return_attempt_count(&fixture.subject.subject, "one").unwrap(), 1);
        fixture.reconciler.reconcile_model_return(&fixture.subject, "one").unwrap();
        assert_eq!(fixture.store.model_return_attempt_count(&fixture.subject.subject, "one").unwrap(), 1);
        let mut config = Config::default();
        config.fleet.enabled = Some(false);
        fixture.reconciler.set_model_return_policy(&config).unwrap();
        assert!(!fixture.eligible());
        let runtime = RuntimeObservation { runtime_id:fixture.subject.member.as_ref().unwrap().runtime_id.clone(), terminal:true, status:"running".into(), exit_code:None, incarnation_id:Some("one".into()) };
        assert!(fixture.reconciler.cancel_stale_model_return(&fixture.subject, &request, Some(&runtime)).unwrap());
        assert!(fixture.store.operation_claim(&format!("agent-restart-completed:{}", request.id)).unwrap().is_some());
        config.fleet.enabled = Some(true);
        config.fleet.max_per_hour = Some(0);
        fixture.reconciler.set_model_return_policy(&config).unwrap();
        assert!(!fixture.eligible());
    }
    #[test]
    fn canceled_return_can_readmit_same_episode_after_hourly_budget_expires() {
        let fixture = Fixture::new();
        fixture.reconciler.reconcile_model_return_at(&fixture.subject, "one", fixture.now).unwrap();
        let first = crate::restart::latest_request(&fixture.store, &fixture.subject.subject).unwrap().unwrap();
        fixture.quiet("working", false, 0, fixture.now);
        let runtime = RuntimeObservation { runtime_id:fixture.subject.member.as_ref().unwrap().runtime_id.clone(), terminal:true, status:"running".into(), exit_code:None, incarnation_id:Some("one".into()) };
        assert!(fixture.reconciler.cancel_stale_model_return(&fixture.subject, &first, Some(&runtime)).unwrap());
        fixture.quiet("idle", true, 0, fixture.now);
        fixture.reconciler.reconcile_model_return_at(&fixture.subject, "one", fixture.now).unwrap();
        assert_eq!(fixture.store.model_return_attempt_count(&fixture.subject.subject, "one").unwrap(), 1);
        let after_budget = first.accepted_at_unix_ms + 3_600_001;
        fixture.usage("gpt-6.1-sol", after_budget);
        fixture.quiet("idle", true, 0, after_budget);
        fixture.reconciler.reconcile_model_return_at(&fixture.subject, "one", after_budget).unwrap();
        let second = crate::restart::latest_request(&fixture.store, &fixture.subject.subject).unwrap().unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.body["fields"]["cause"], second.body["fields"]["cause"]);
        assert_eq!(fixture.store.model_return_attempt_count(&fixture.subject.subject, "one").unwrap(), 2);
        fixture.reconciler.reconcile_model_return_at(&fixture.subject, "one", after_budget).unwrap();
        assert_eq!(fixture.store.model_return_attempt_count(&fixture.subject.subject, "one").unwrap(), 2);
    }
    #[test]
    fn fallback_requires_declared_origin_current_session_and_unblocked_turn() {
        let fixture = Fixture::new();
        fixture.claim("harness.observed", json!({"driver":"omp","state":"idle","incarnation_id":"one","observed_at_ms":fixture.now as u64,"quiescent":true,"blocking":["turn-in-flight"],"background_jobs":0}));
        assert!(!fixture.eligible());
        fixture.quiet("idle", true, 0, fixture.now);
        fixture.claim("harness.model-fallback.observed", json!({"observation":"selected","incarnation_id":"one","native_session":"native-one","source_event":"manual-origin","episode_id":"manual-origin","to":"gpt-6.2-pro","observed_at_unix_ms":fixture.now as u64,"reason":"native-model-selection"}));
        fixture.claim("harness.model-fallback.observed", json!({"observation":"selected","incarnation_id":"one","native_session":"native-one","source_event":"undeclared-origin","episode_id":"undeclared-origin","to":"gpt-6.1-sol","observed_at_unix_ms":fixture.now as u64,"reason":"native-model-selection"}));
        fixture.claim("harness.model-fallback.observed", json!({"observation":"applied","incarnation_id":"one","native_session":"native-one","source_event":"undeclared-origin","episode_id":"undeclared-origin","from":"gpt-6.2-pro","to":"gpt-6.1-sol","cause":"transient","observed_at_unix_ms":fixture.now as u64,"reason":"native-retry-fallback"}));
        assert!(!fixture.eligible());
        fixture.fallback("transient", "restored-origin");
        assert!(fixture.eligible());
        fixture.claim("harness.session-file", json!({"harness":"omp","session_id":"different-session","incarnation_id":"one"}));
        assert!(!fixture.eligible());
    }
    #[test]
    fn queued_return_cancels_when_incarnation_changes() {
        let fixture = Fixture::new();
        fixture.reconciler.reconcile_model_return(&fixture.subject, "one").unwrap();
        let request = crate::restart::latest_request(&fixture.store, &fixture.subject.subject).unwrap().unwrap();
        let runtime = RuntimeObservation { runtime_id:fixture.subject.member.as_ref().unwrap().runtime_id.clone(), terminal:true, status:"running".into(), exit_code:None, incarnation_id:Some("two".into()) };
        assert!(fixture.reconciler.cancel_stale_model_return(&fixture.subject, &request, Some(&runtime)).unwrap());
        assert!(fixture.store.operation_claim(&format!("agent-restart-completed:{}", request.id)).unwrap().is_some());
    }
    #[test]
    fn declaration_off_is_explicit() {
        let desired = serde_json::json!({"children":[{"name":"model-return","children":[{"name":"enabled","arguments":[false]},{"name":"max-drift","arguments":[900]}]}]});
        let policy = Config::default().resolve("agent/example", &declaration(&desired));
        assert!(!policy.enabled);
        assert_eq!(policy.max_drift_ms, 900_000);
    }
}