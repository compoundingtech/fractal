//! Lane bursts: a laned seat's command raises its seat's lane for its own run, served in the order
//! they asked as the host's room for lanes allows (docs/st3/seat-lanes.md, "Bursts").
use super::*;
use fractal_runtime::Capacity;
use super::seat_admission_queue::{show_lane, show_memory};

impl<R: RuntimeControl> Reconciler<R> {
    /// Drops the bursts whose holder stopped renewing, then grants waiting ones in the order they
    /// asked while they fit. A seat whose burst was granted, changed or dropped has its wake judged
    /// again (`lane-burst:{agent}`), which sets its live lane.
    pub(super) fn reconcile_lane_bursts(&self) -> Result<()> {
        let status = &self.seat_admission.status;
        let now = u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX);
        {
            let mut state = status.state();
            if state.lane_bursts.is_empty() && state.burst_touched.is_empty() && state.lane_found.is_empty() {
                return Ok(());
            }
            // A raise found at start that no holder's renewal restored within a lease comes down.
            if now_ms() >= self.found_lanes_kept_until() {
                for (agent, lane) in std::mem::take(&mut state.lane_found) {
                    eprintln!("st3: WARN {agent}'s lane was raised to {} when this daemon started and no burst holder renewed it: it goes back to what its steps hold",
                        show_lane(lane));
                    state.burst_touched.insert(agent);
                }
            }
        }
        for agent in status.burst_expire(now) {
            eprintln!("st3: WARN {agent}'s lane burst lapsed: its holder stopped renewing it");
        }
        let waiting = status.state().lane_bursts.iter().filter(|burst| !burst.granted && burst.refused.is_none())
            .map(|burst| (burst.subject.clone(), burst.capacity)).collect::<Vec<_>>();
        let mut first = None::<String>;
        for (agent, asked) in waiting {
            let judged = self.judge_lane_burst(&agent, asked, first.as_deref(), now)?;
            let mut state = status.state();
            let Some(burst) = state.lane_bursts.iter_mut().find(|burst| burst.subject == agent) else { continue };
            match judged {
                BurstJudged::Granted(lane) => {
                    burst.granted = true;
                    burst.wait = None;
                    // Counted at once, so another burst or step judged in this pass does not take the same room.
                    let raised = state.lane_raised.get(&agent).copied().unwrap_or(lane);
                    state.lane_raised.insert(agent.clone(), Capacity {
                        cores: raised.cores.max(lane.cores),
                        memory: raised.memory.max(lane.memory),
                    });
                    state.burst_touched.insert(agent.clone());
                    eprintln!("st3: {agent}'s lane burst {} is granted", show_lane(lane));
                }
                BurstJudged::Waits(reason) => {
                    burst.wait = Some(reason);
                    first.get_or_insert(agent);
                    smallclaims::touched::note_due(now_ms() + BURST_RETRY_MS);
                }
                BurstJudged::Refused(reason) => burst.refused = Some(reason),
            }
        }
        let touched = std::mem::take(&mut status.state().burst_touched);
        for agent in touched {
            self.incremental.touch(&format!("lane-burst:{agent}"));
        }
        Ok(())
    }

    /// Once, before this daemon judges any room: each of its laned seats' slices, read for a raise
    /// it has no record of. A daemon that restarted has none of the bursts the one before it
    /// granted, while their slices stay raised and their holders' runs go on in them. Such a raise
    /// counts as held at once (`held_lanes`, the burst reserve), so no start or other burst is given
    /// its room, and the lane stays raised until the holder's renewal restores the grant
    /// (`burst_ask`) or one burst lease after this daemon started.
    pub(super) fn find_raised_lanes(&self) -> Result<()> {
        if self.seat_admission.lanes.is_none() || self.seat_admission.status.state().lanes_found {
            return Ok(());
        }
        let mut found = false;
        for desired in self.store.desired_subjects()? {
            let Some(member) = desired.member.as_ref().filter(|member| member.host == self.host) else { continue };
            let Some(base) = member.capacity.filter(|_| desired.kind == "agent") else { continue };
            let Some(live) = self.runtime.live_lane(&member.runtime_id)
                .filter(|live| live.cores > base.cores || live.memory > base.memory)
            else { continue };
            let mut state = self.seat_admission.status.state();
            // A renewal served before this pass already restored it (burst_ask).
            if state.lane_bursts.iter().any(|burst| burst.subject == desired.subject) {
                continue;
            }
            state.lane_found.insert(desired.subject.clone(), live);
            let raised = state.lane_raised.get(&desired.subject).copied().unwrap_or(base);
            state.lane_raised.insert(desired.subject.clone(), Capacity {
                cores: raised.cores.max(live.cores),
                memory: raised.memory.max(live.memory),
            });
            eprintln!("st3: {}'s lane is raised to {} above its own: it counts as held and stays raised for a burst holder's renewal",
                desired.subject, show_lane(live));
            found = true;
        }
        self.seat_admission.status.state().lanes_found = true;
        if found {
            smallclaims::touched::note_due(self.found_lanes_kept_until());
        }
        Ok(())
    }

    /// When a raise this daemon found at start stops being kept for its holder: one burst lease
    /// after it started, the longest the daemon before it would have kept the grant unrenewed.
    fn found_lanes_kept_until(&self) -> u128 {
        self.started_at_unix_ms + u128::from(fractal_runtime::seat_admission_status::BURST_LEASE_SECS) * 1000
    }

    /// Whether `agent`'s burst to `asked` can be held now, beside this host's other lanes and what
    /// its seats outside lanes hold. Bursts wait in order: one behind `first` waits for it.
    fn judge_lane_burst(&self, agent: &str, asked: Capacity, first: Option<&str>, now: u64) -> Result<BurstJudged> {
        let Some(host) = self.seat_admission.lanes else {
            return Ok(BurstJudged::Refused("lanes are off on this host".into()));
        };
        let Some(base) = self.store.desired_subject_with_writer(agent)?
            .map(|(desired, _)| desired)
            .filter(|desired| desired.kind == "agent")
            .and_then(|desired| desired.member)
            .filter(|member| member.host == self.host)
            .and_then(|member| member.capacity)
        else {
            return Ok(BurstJudged::Refused(format!("{agent} holds no lane on this host")));
        };
        let lane = Capacity { cores: base.cores.max(asked.cores), memory: base.memory.max(asked.memory) };
        let room = host.room;
        if lane.cores > room.cores || lane.memory > room.memory {
            return Ok(BurstJudged::Refused(format!("burst lane {} for {agent} can never fit on this host, which gives lanes {}",
                show_lane(lane), show_lane(room))));
        }
        if let Some(first) = first {
            return Ok(BurstJudged::Waits(format!("burst lane {} for {agent} waits behind {first}, first in line for room", show_lane(lane))));
        }
        let outside = (host.outside)().unwrap_or(0);
        // A seat that has waited long to start is served before more bursts: the host drains to
        // fit for it, as it does for the first seat in line. One that would not fit even once
        // every raise were given back, beside the room kept for a burst, gains nothing from the
        // wait, so bursts go on beside it.
        if let Some((seat, waited)) = self.seat_admission.status.aged_lane_head(now, BURST_YIELDS_TO_START_AFTER_SECS)
            .filter(|(seat, _)| seat != agent)
            && let Some(seat_lane) = self.store.desired_subject_with_writer(&seat)?
                .and_then(|(desired, _)| desired.member)
                .filter(|member| member.host == self.host)
                .and_then(|member| member.capacity)
            && seat_lane.memory <= room.memory.saturating_sub(self.held_base_lanes(&seat, now)?).saturating_sub(outside)
                .saturating_sub(self.burst_reserve(&seat, seat_lane)?.full)
        {
            return Ok(BurstJudged::Waits(format!(
                "burst lane {} for {agent} waits behind {seat}, which has waited {} min to start", show_lane(lane), waited / 60)));
        }
        let held = self.held_lanes(agent, now)?;
        let free = room.memory.saturating_sub(held).saturating_sub(outside);
        if lane.memory <= free {
            return Ok(BurstJudged::Granted(lane));
        }
        Ok(BurstJudged::Waits(format!(
            "burst lane {} for {agent} does not fit: {} free of the {} this host gives lanes (other lanes hold {}, seats outside lanes {})",
            show_lane(lane), show_memory(free), show_memory(room.memory), show_memory(held), show_memory(outside))))
    }
}

/// How soon a waiting burst is judged again without a claim to wake it: what seats outside lanes
/// hold changes without one.
const BURST_RETRY_MS: u128 = 5_000;

/// How long the first seat in line for room waits before new bursts wait behind it.
const BURST_YIELDS_TO_START_AFTER_SECS: u64 = 15 * 60;

enum BurstJudged {
    Granted(Capacity),
    Waits(String),
    Refused(String),
}
