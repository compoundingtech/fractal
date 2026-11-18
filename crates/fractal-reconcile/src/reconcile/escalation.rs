//! Policy deadlines share the run reporter's incremental evaluation and native delivery receipts.
use super::*;
use crate::model::{EscalationAction, EscalationClock, EscalationOverride, EscalationSpec, PersonAskRequest};
use crate::store::escalation::EscalationEpisode;
use serde_json::json;

fn policies<'a>(scope: &'a EscalationOverride, inherited: &'a [EscalationSpec]) -> &'a [EscalationSpec] {
    match scope {
        EscalationOverride::Inherit => inherited,
        EscalationOverride::Disable => &[],
        EscalationOverride::Replace { policies } => policies,
    }
}

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn escalate_run(&self, run: &MissionRunView, mission: &MissionSpec, now: u128) -> Result<Option<u128>> {
        let declared = policies(&mission.escalation, &[]);
        let mut due = None;
        let run_state = match run.status.as_str() { "standing" => "running", "blocked" => "waiting", other => other };
        let mut has_policy = !declared.is_empty();
        for policy in declared {
            if policy.state.as_str() != run_state { continue; }
            let progress = policy.clock == EscalationClock::SinceProgress;
            let mut episode = self.store.escalation_episode(&run.subject, run_state, run.created_at_unix_ms, progress)?;
            if progress {
                // Child/held work is the run's progress, not its lease renewals.
                let activity = self.run_report_activity(run)?;
                if activity > episode.since {
                    episode.key = format!("{}:progress:{activity}", episode.key);
                    episode.since = activity;
                }
            }
            due = [due, self.deliver_escalation(&run.subject, policy, &episode, now)?].into_iter().flatten().min();
        }
        for step in &run.steps {
            let Some(spec) = crate::mission::find_step(mission, &step.step) else { continue; };
            let step_policies = policies(&spec.escalation, declared);
            has_policy |= !step_policies.is_empty();
            let state = crate::client_values::client_work_state(&step.status);
            for policy in step_policies {
                if policy.state.as_str() != state { continue; }
                let episode = self.store.escalation_episode(&step.subject, state, step.created_at_unix_ms,
                    policy.clock == EscalationClock::SinceProgress)?;
                due = [due, self.deliver_escalation(&step.subject, policy, &episode, now)?].into_iter().flatten().min();
            }
        }
        // Register before the evaluator can end the run. On restart terminal ladders are loaded
        // alongside active runs, and are removed only after their final native delivery.
        self.store.retain_escalation_run(&run.id, &self.host,
            has_policy && (!matches!(run.status.as_str(), "completed" | "failed" | "cancelled") || due.is_some()))?;
        Ok(due)
    }

    fn deliver_escalation(&self, subject: &str, policy: &EscalationSpec, episode: &EscalationEpisode, now: u128) -> Result<Option<u128>> {
        for (stage, declaration) in policy.stages.iter().enumerate() {
            let at = episode.since.saturating_add(u128::from(declaration.after_ms));
            if now < at { return Ok(Some(at)); }
            // JSON tuples avoid delimiter collisions in user-supplied policy IDs. The source
            // subject scopes inheritance, and the durable transition/progress receipt names the
            // episode. Native delivery commits its own stable identity atomically.
            let key = format!("mission-escalation-v1:{}", serde_json::to_string(&(subject, &policy.id, stage, &episode.key))?);
            let title = format!("Escalation {} stage {}: {subject}", policy.id, stage + 1);
            let reason = format!("{subject} remains {}. Policy {} stage {} is overdue after {}ms ({:?}). Inspect with `fractal missions show` or `fractal work show {subject}`.",
                policy.state.as_str(), policy.id, stage + 1, declaration.after_ms, policy.clock);
            match &declaration.action {
                EscalationAction::Notify(to) => {
                    let digest = hex::encode(sha2::Sha256::digest(key.as_bytes()));
                    let message = format!("message/{}", &digest[..16]);
                    if self.store.message_sent(&message)? { continue; }
                    crate::store::message_routing::send(&self.store, &ClaimInput {
                        subject: message, kind: "message.sent".into(), actor: Some("daemon/runtime".into()),
                        fields: BTreeMap::from([
                            ("from".into(), json!("daemon/runtime")), ("to".into(), json!(to)),
                            ("content".into(), json!(reason)), ("title".into(), json!(title)),
                            ("status".into(), json!("sent")), ("in_reply_to".into(), Value::Null),
                            ("tags".into(), json!(["mission-escalation-v1", subject])),
                        ]), evidence: Vec::new(), expected_subject: None, idempotency_key: Some(key),
                    })?;
                }
                EscalationAction::Ask(person) => {
                    // Independent native asks also work for terminal runs and unclaimed steps;
                    // they do not mutate the work state whose clock they describe.
                    self.store.ask_person(&PersonAskRequest {
                        person: person.clone(), title, reason, actor: "daemon/escalation".into(),
                        step: None, new_run: None, incarnation: None, request: None,
                        deadline_ms: None, default_answer: None, priority: None,
                        legacy_request: None, idempotency_key: key,
                    })?;
                }
            }
        }
        Ok(None)
    }
}
