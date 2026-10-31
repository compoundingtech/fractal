//! Seat admission precedes preparation, durable start claims, and native teardown.
use super::*;

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn admit_seat_start(&self, subject: &DesiredSubject, member: &MemberSpec, request: Option<&crate::model::ClaimRecord>) -> Result<bool> {
        if !matches!(subject.kind.as_str(), "agent" | "exec" | "pty") {
            return Ok(true); // Internal gate runners share the runtime, not seat admission.
        }
        let status = &self.seat_admission.status;
        if status.state().decision.mode == "off" { return Ok(true); }
        smallclaims::touched::note_read(|| "seat-admission".into());
        let replacement = if let Some(operation) = member.environment.get(crate::rollout::OPERATION_ENV) {
            if let Some(rollout) = self.store.rollout(&subject.subject)?.filter(|rollout| rollout.id == *operation)
                && let Some(request) = self.store.claim_by_id(&rollout.id)? {
                self.seat_was_live(&subject.subject, &rollout.old_incarnation, request.store_index)?
            } else { false }
        } else if let Some(request) = request {
            request.body["fields"]["incarnation_id"].as_str().filter(|incarnation| !incarnation.is_empty())
                .map(|incarnation| self.seat_was_live(&subject.subject, incarnation, request.store_index))
                .transpose()?.unwrap_or(false)
        } else { false };
        let pending_count = status.state().pending.len();
        let admitted = status.admit(&subject.subject, self.store.selected_desired_token(&subject.subject)?,
            request.map(|request| request.id.clone()), replacement, u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX));
        if admitted && pending_count != 0 {
            // A later FIFO member may have been evaluated before this head in the same pass.
            self.incremental.touch("seat-admission");
            self.signal_changed();
        }
        if !admitted && status.state().pending.len() != pending_count {
            self.event_notify.send_modify(|generation| *generation = generation.saturating_add(1));
        }
        Ok(admitted)
    }

    // Use the original request's cut, never later exit evidence from the same millisecond.
    fn seat_was_live(&self, subject: &str, incarnation: &str, at_index: u64) -> Result<bool> {
        Ok(self.store.observations_for(subject, "runtime.observed")?.into_iter()
            .filter(|claim| claim.store_index <= at_index)
            .max_by_key(|claim| claim.store_index)
            .is_some_and(|claim| claim.body["fields"]["status"] == "running"
                && claim.body["fields"]["incarnation_id"].as_str() == Some(incarnation)))
    }

    pub(super) fn cancel_invalid_admissions(&self, desired: &[DesiredSubject]) -> Result<()> {
        {
            let mut state = self.seat_admission.status.state();
            state.replacements.retain(|name| desired.iter().any(|subject| subject.subject == *name
                && matches!(subject.kind.as_str(), "agent" | "exec" | "pty")
                && subject.member.as_ref().is_some_and(|member| member.host == self.host)));
        }
        let pending = self.seat_admission.status.state().pending.iter().cloned().collect::<Vec<_>>();
        let mut removed = BTreeSet::new();
        for entry in pending {
            let request_retained = match &entry.request {
                Some(id) => crate::restart::latest_request(&self.store, &entry.subject)?.is_some_and(|request| request.id == *id),
                None => true,
            };
            let selected = desired.iter().find(|subject| subject.subject == entry.subject
                && matches!(subject.kind.as_str(), "agent" | "exec" | "pty")
                && subject.member.as_ref().is_some_and(|member| member.host == self.host));
            let retained = if let Some(selected) = selected {
                self.store.selected_desired_token(&entry.subject)? == entry.token
                    && self.store.owned_desired_guard(selected).is_ok()
                    && self.store.helper_launcher_alive(&entry.subject)?
                    && !crate::suspension::current(&self.store, &entry.subject)?.is_some_and(|state| state.action == "suspend")
                    && request_retained
            } else { false };
            if !retained { removed.insert(entry.subject); }
        }
        if !removed.is_empty() {
            self.seat_admission.status.state().pending.retain(|entry| !removed.contains(&entry.subject));
            self.incremental.touch("seat-admission");
            self.signal_changed();
        }
        Ok(())
    }
}
