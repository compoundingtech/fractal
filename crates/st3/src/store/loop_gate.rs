//! Round gate admission uses the same ready work projection as wake delivery.
use super::*;

impl Store {
    pub(crate) fn is_loop_gate(&self, subject: &DesiredSubject) -> Result<bool> {
        if subject.kind != "agent" || !subject.subject.ends_with("/gate") {
            return Ok(false);
        }
        let Some(owner) = subject.owner_run.as_deref() else {
            return Ok(false);
        };
        Ok(self.mission_run(owner)?.is_some_and(|run| {
            run.mission.starts_with("__st3/")
                && run.mission.contains("/loop/")
                && run.mission.ends_with("/round")
        }))
    }

    /// `None` means that no current, claimable work authorizes a gate launch.
    /// Other members retain their existing lifecycle and do not use this method.
    pub(crate) fn loop_gate_ready_at(&self, subject: &DesiredSubject) -> Result<Option<u128>> {
        let mut ready_at = None;
        for work in self.work_for_reconcile(&subject.subject)? {
            if work.status != "ready" || work.claimant.is_some() {
                continue;
            }
            let at = self
                .latest_claim(&work.subject, Some("step-run.state"))?
                .filter(|claim| {
                    claim.body.pointer("/fields/status").and_then(Value::as_str) == Some("ready")
                })
                .map_or(work.updated_at_unix_ms, |claim| claim.accepted_at_unix_ms);
            ready_at = Some(ready_at.map_or(at, |prior: u128| prior.min(at)));
        }
        Ok(ready_at)
    }
}
