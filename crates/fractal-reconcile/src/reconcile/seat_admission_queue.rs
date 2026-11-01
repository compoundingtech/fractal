//! Seat admission precedes preparation, durable start claims, and native teardown.
use super::*;

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn admit_seat_start(&self, subject: &DesiredSubject, member: &MemberSpec, request: Option<&crate::model::ClaimRecord>) -> Result<bool> {
        if !matches!(subject.kind.as_str(), "agent" | "exec" | "pty") {
            return Ok(true); // Internal gate runners share the runtime, not seat admission.
        }
        let lane = match self.lane_fit(subject, member)? {
            LaneFit::Waits => return Ok(false),
            LaneFit::Fits(lane) => Some(lane),
            LaneFit::Unlaned => None,
        };
        let admitted = self.admit_by_envelope(subject, member, request)?;
        // Room is held only for a seat that starts.
        if admitted && let Some(lane) = lane {
            self.seat_admission.status.lane_admitted(&subject.subject, lane, u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX));
            self.incremental.touch("seat-admission");
            self.signal_changed();
        }
        Ok(admitted)
    }

    fn admit_by_envelope(&self, subject: &DesiredSubject, member: &MemberSpec, request: Option<&crate::model::ClaimRecord>) -> Result<bool> {
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

    /// Whether a laned seat's lane fits now: what the host gives lanes, less the lanes of its other
    /// live seats and what its seats outside lanes hold. Seats wait for room in order, so a large
    /// lane is not passed by smaller ones; one larger than the host can ever give waits out of line.
    fn lane_fit(&self, subject: &DesiredSubject, member: &MemberSpec) -> Result<LaneFit> {
        let (Some(host), Some(lane)) = (self.seat_admission.lanes, member.capacity) else {
            return Ok(LaneFit::Unlaned);
        };
        if subject.kind != "agent" {
            return Ok(LaneFit::Unlaned);
        }
        smallclaims::touched::note_read(|| "seat-admission".into());
        let status = &self.seat_admission.status;
        let now = u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX);
        let room = host.room;
        if lane.cores > room.cores || lane.memory > room.memory {
            let reason = format!("lane {} can never fit on this host, which gives lanes {}", show_lane(lane), show_lane(room));
            if status.lane_wait(&subject.subject, reason, false, now) {
                self.event_notify.send_modify(|generation| *generation = generation.saturating_add(1));
            }
            return Ok(LaneFit::Waits);
        }
        let held = self.held_lanes(&subject.subject, now)?;
        let outside = (host.outside)().unwrap_or(0);
        let free = room.memory.saturating_sub(held).saturating_sub(outside);
        let head = status.lane_head(now);
        if lane.memory <= free && head.as_deref().is_none_or(|head| head == subject.subject) {
            return Ok(LaneFit::Fits(lane));
        }
        let room_now = format!(
            "{} free of the {} this host gives lanes (other lanes hold {}, seats outside lanes {})",
            show_memory(free), show_memory(room.memory), show_memory(held), show_memory(outside));
        let reason = match head.filter(|head| *head != subject.subject && lane.memory <= free) {
            Some(head) => format!("lane {} waits behind {head}, first in line for room: {room_now}", show_lane(lane)),
            None => format!("lane {} does not fit: {room_now}", show_lane(lane)),
        };
        if status.lane_wait(&subject.subject, reason, true, now) {
            self.event_notify.send_modify(|generation| *generation = generation.saturating_add(1));
        }
        // What seats outside lanes hold and a start that never ran change without a claim: try
        // again soon.
        smallclaims::touched::note_due(now_ms() + LANE_RETRY_MS);
        Ok(LaneFit::Waits)
    }

    /// The memory of this host's other laned seats that run, or were admitted to start and are not
    /// yet seen running.
    fn held_lanes(&self, candidate: &str, now: u64) -> Result<u64> {
        const STARTING_SECS: u64 = 600;
        let status = &self.seat_admission.status;
        let mut held = 0;
        let mut running = BTreeSet::new();
        for desired in self.store.desired_subjects()? {
            let Some(member) = desired.member.as_ref().filter(|member| member.host == self.host) else { continue };
            let Some(lane) = member.capacity else { continue };
            if desired.kind != "agent" || desired.subject == candidate {
                continue;
            }
            if self.store.latest_observation(&desired.subject, "runtime.observed")?
                .is_some_and(|claim| claim.body["fields"]["status"] == "running")
            {
                running.insert(desired.subject.clone());
                held += lane.memory;
            } else if status.state().lane_starts.get(&desired.subject)
                .is_some_and(|(_, at)| now.saturating_sub(*at) < STARTING_SECS)
            {
                held += lane.memory;
            }
        }
        status.state().lane_starts.retain(|subject, (_, at)| {
            subject == candidate || (!running.contains(subject) && now.saturating_sub(*at) < STARTING_SECS)
        });
        Ok(held)
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
        // A seat that is no longer tried for a start, or no longer this host's laned seat, gives up
        // its place in line, as a pending one does below.
        let now = u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX);
        let waits = self.seat_admission.status.state().lane_waits.clone();
        let mut gone = BTreeSet::new();
        for wait in waits {
            let selected = desired.iter().find(|subject| subject.subject == wait.subject
                && subject.kind == "agent"
                && subject.member.as_ref().is_some_and(|member| member.host == self.host && member.capacity.is_some()));
            let retained = if let Some(selected) = selected {
                now.saturating_sub(wait.checked) <= fractal_runtime::seat_admission_status::LANE_WAIT_STALE_SECS
                    && self.store.owned_desired_guard(selected).is_ok()
                    && self.store.helper_launcher_alive(&wait.subject)?
                    && !crate::suspension::current(&self.store, &wait.subject)?.is_some_and(|state| state.action == "suspend")
            } else { false };
            if !retained { gone.insert(wait.subject); }
        }
        if !gone.is_empty() {
            self.seat_admission.status.state().lane_waits.retain(|wait| !gone.contains(&wait.subject));
            self.incremental.touch("seat-admission");
            self.signal_changed();
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

fn show_lane(lane: fractal_runtime::Capacity) -> String {
    format!("{} cores/{}", lane.cores, show_memory(lane.memory))
}

/// Bytes as the reasons show them: GiB with one decimal.
fn show_memory(bytes: u64) -> String {
    format!("{:.1}G", bytes as f64 / f64::from(1u32 << 30))
}

/// How soon a seat waiting for room is tried again without a claim to wake it.
const LANE_RETRY_MS: u128 = 30_000;

enum LaneFit {
    /// No lane, or lanes are off: admission by fit does not apply.
    Unlaned,
    Fits(fractal_runtime::Capacity),
    Waits,
}
