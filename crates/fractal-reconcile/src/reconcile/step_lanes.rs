//! A step's capacity raises its seat's lane while the seat holds the step, and the lane goes back
//! to the seat's own when it no longer does (docs/st3/seat-lanes.md).
use super::*;
use fractal_runtime::Capacity;
use super::seat_admission_queue::{show_lane, show_memory};

impl<R: RuntimeControl> Reconciler<R> {
    /// Holds `agent`'s live lane at its own, raised by each step it holds that declares a capacity.
    /// A seat without a lane, or a host without lanes, is left as it is.
    pub(super) fn hold_step_lanes(
        &self,
        agent: &str,
        incarnation: &str,
        member: &MemberSpec,
        work: &[crate::model::StepRunView],
    ) -> Result<()> {
        let (Some(_), Some(base)) = (self.seat_admission.lanes, member.capacity) else {
            return Ok(());
        };
        let wanted = self.wanted_lane(agent, base, work)?;
        let status = &self.seat_admission.status;
        {
            let mut state = status.state();
            if wanted == base {
                state.lane_raised.remove(agent);
            } else {
                state.lane_raised.insert(agent.to_owned(), wanted);
            }
        }
        // Each launch starts the slice at the seat's own lane, so what was set before belongs to
        // the incarnation it was set for.
        let applied = status.state().lane_applied.get(agent).cloned();
        if applied.as_ref().is_some_and(|(at, lane)| at == incarnation && *lane == wanted) {
            return Ok(());
        }
        if applied.is_none() && wanted == base {
            status.state().lane_applied.insert(agent.to_owned(), (incarnation.to_owned(), wanted));
            return Ok(());
        }
        if self.runtime.resize_lane(&member.runtime_id, wanted)? {
            eprintln!("st3: {agent}'s lane is now {wanted} for the steps it holds");
        }
        status.state().lane_applied.insert(agent.to_owned(), (incarnation.to_owned(), wanted));
        Ok(())
    }

    /// Why a step that is ready to be delivered must wait, if it must: its seat is one of this
    /// host's laned seats and the lane the step raises it to does not fit beside the host's other
    /// lanes and what its seats outside lanes hold, or the step would lower the seat's lane (a seat
    /// declared outside the step's publication is checked here). Every reason starts `step lane`.
    pub(super) fn step_lane_wait(&self, raise: Option<fractal_runtime::Raise>, agent: Option<&str>) -> Result<Option<String>> {
        let (Some(host), Some(raise), Some(agent)) = (self.seat_admission.lanes, raise, agent) else {
            return Ok(None);
        };
        let Some(base) = self.store.desired_subject_with_writer(agent)?
            .map(|(desired, _)| desired)
            .filter(|desired| desired.kind == "agent")
            .and_then(|desired| desired.member)
            .filter(|member| member.host == self.host)
            .and_then(|member| member.capacity)
        else {
            return Ok(None);
        };
        let lane = match raise.over(base) {
            Ok(lane) => lane,
            Err(error) => return Ok(Some(format!("step lane for {agent}: {error:#}"))),
        };
        let room = host.room;
        if lane.cores > room.cores || lane.memory > room.memory {
            return Ok(Some(format!("step lane {} for {agent} can never fit on this host, which gives lanes {}",
                show_lane(lane), show_lane(room))));
        }
        let now = u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX);
        let held = self.held_lanes(agent, now)?;
        let outside = (host.outside)().unwrap_or(0);
        let free = room.memory.saturating_sub(held).saturating_sub(outside);
        if lane.memory <= free {
            return Ok(None);
        }
        smallclaims::touched::note_due(now_ms() + 30_000);
        Ok(Some(format!(
            "step lane {} for {agent} does not fit: {} free of the {} this host gives lanes (other lanes hold {}, seats outside lanes {})",
            show_lane(lane), show_memory(free), show_memory(room.memory), show_memory(held), show_memory(outside))))
    }

    /// `agent`'s own lane, raised by the steps it holds: a step it claimed, or one delivered to it.
    pub(super) fn wanted_lane(&self, agent: &str, base: Capacity, work: &[crate::model::StepRunView]) -> Result<Capacity> {
        let held = work.iter().filter(|view| holds_step(view, agent)).collect::<Vec<_>>();
        if held.is_empty() {
            return Ok(base);
        }
        let runs = held.iter().map(|view| view.run.clone()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        let specs = self.store.mission_specs_for_runs(&runs)?;
        let mut lane = base;
        for view in held {
            let Some(spec) = specs.get(&view.run)
                .or_else(|| specs.get(view.run.strip_prefix("mission-run/").unwrap_or(&view.run)))
            else { continue };
            let Some(raise) = flatten_mission_steps(spec).into_iter()
                .find(|step| step.spec.path == view.step)
                .and_then(|step| step.spec.capacity)
            else { continue };
            // A step only raises: what it names below the lane leaves the lane as it is.
            lane = Capacity {
                cores: lane.cores.max(raise.cores.unwrap_or(0)),
                memory: lane.memory.max(raise.memory.unwrap_or(0)),
            };
        }
        Ok(lane)
    }
}

/// Whether `agent` holds `view`'s step: it claimed it, or the step is ready and assigned to it,
/// so its wake can arrive any moment.
pub(super) fn holds_step(view: &crate::model::StepRunView, agent: &str) -> bool {
    (view.claimant.as_deref() == Some(agent) && matches!(view.status.as_str(), "claimed" | "working" | "verifying"))
        || (view.status == "ready" && view.assigned_to.as_deref() == Some(agent))
}
