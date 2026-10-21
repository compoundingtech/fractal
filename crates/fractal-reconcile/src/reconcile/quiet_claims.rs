//! The quiet deadline of a held step (see `store::quiet_claims`). Each held step with no open
//! declared wait has one deadline: its holder's last progress plus its run's stalled-after. The
//! run's evaluation is due then, so a quiet fleet does no work to watch for quiet steps.
//!
//! At the deadline the holder gets a `step-quiet` fault: record progress, declare the wait, ask,
//! or release. Thirty minutes later, with no answer, the step is stalled: a `step-stalled` fault
//! goes to the run's report-to, else its requester when that is an agent. Both are delivered as
//! faults are, once per quiet period, and any progress ends them.
use super::*;
use crate::store::{STALLED_GRACE_MS, STEP_QUIET_CONDITION, STEP_STALLED_CONDITION, quiet_episode};

impl<R: RuntimeControl> Reconciler<R> {
    /// Raise the quiet or stalled fault `view` is due, or arm its next deadline.
    pub(super) fn check_quiet_claim(
        &self,
        run: &MissionRunView,
        view: &StepRunView,
    ) -> Result<bool> {
        let Some(holder) = view.claimant.as_deref() else {
            return Ok(false);
        };
        if self.store.declared_wait(&view.subject, view.attempt)?.is_some() {
            return Ok(false);
        }
        let Some(since) = self.store.quiet_since(&view.subject, view.attempt)? else {
            return Ok(false);
        };
        let report = self.run_report(&run.subject)?;
        let stalled_after = u128::from(
            report
                .as_ref()
                .map_or(crate::mission::DEFAULT_STALLED_AFTER_MS, |report| {
                    report.stalled_after_ms
                }),
        );
        let quiet_at = since.saturating_add(stalled_after);
        let stalled_at = quiet_at.saturating_add(STALLED_GRACE_MS);
        let now = now_ms();
        let key = format!("quiet-claim:{}", view.subject);
        if now < quiet_at {
            self.arm_deadline_at(
                &key,
                quiet_at,
                crate::reconcile_telemetry::WakeCause::TimerStepTimeout,
            );
            return Ok(false);
        }
        let step = &view.subject;
        let title = view.title.as_deref().unwrap_or(&view.step);
        let quiet_for = super::waits::describe_age(stalled_after);
        let mut changed = self.raise_quiet_fault(
            STEP_QUIET_CONDITION,
            view,
            since,
            &AttentionRequest {
                reviewer: holder.to_owned(),
                title: format!("No progress on {title}"),
                reason: format!(
                    "You hold `{step}` and have recorded no progress on it for {quiet_for}, and it declares no wait. \
                     Record progress with `st work progress {step} --summary TEXT --as {holder}`, \
                     declare what it waits on with `st work wait {step} --on TARGET --as {holder}` \
                     (OWNER/REPO#N, step-run/..., or a time), ask a person with `st work ask`, \
                     or release it with `st work release {step} --as {holder}`. \
                     If nothing moves in 30 minutes, the step shows as stalled and its run's report-to is told."
                ),
                severity: "warning".into(),
                targets: vec![step.clone()],
                actor: RECONCILER_ACTOR.into(),
                idempotency_key: format!("step-quiet:{step}:{}:{since}", view.attempt),
            },
        )?;
        if now < stalled_at {
            self.arm_deadline_at(
                &key,
                stalled_at,
                crate::reconcile_telemetry::WakeCause::TimerStepTimeout,
            );
            return Ok(changed);
        }
        let reviewer = report
            .map(|report| report.to)
            .or_else(|| Some(run.requester.clone()).filter(|agent| agent.starts_with("agent/")))
            .unwrap_or_else(|| "person/operator".to_owned());
        let stalled_for = super::waits::describe_age(stalled_after + STALLED_GRACE_MS);
        changed |= self.raise_quiet_fault(
            STEP_STALLED_CONDITION,
            view,
            since,
            &AttentionRequest {
                reviewer,
                title: format!("Stalled: {title}"),
                reason: format!(
                    "`{holder}` holds `{step}` in `{}` and has recorded no progress on it for {stalled_for}, \
                     declared no wait, and did not answer its fault 30 minutes ago. \
                     Look at it with `st work show {step}`: ask the holder, restart it with `st agents restart {holder}`, \
                     or have the work released.",
                    run.subject
                ),
                severity: "error".into(),
                targets: vec![step.clone()],
                actor: RECONCILER_ACTOR.into(),
                idempotency_key: format!("step-stalled:{step}:{}:{since}", view.attempt),
            },
        )?;
        Ok(changed)
    }

    fn raise_quiet_fault(
        &self,
        condition: &str,
        view: &StepRunView,
        since: u128,
        request: &AttentionRequest,
    ) -> Result<bool> {
        let episode = quiet_episode(condition, &view.subject, view.attempt, since);
        // One set guards every per-step fault episode this daemon raised.
        if !self
            .step_timeout_faults
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(episode.clone())
        {
            return Ok(false);
        }
        self.store.record_quiet_fault(condition, &episode, request)?;
        Ok(true)
    }
}
