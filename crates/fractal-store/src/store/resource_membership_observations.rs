//! In-memory span ownership mirrors committed host custody; SQLite remains authoritative.
use super::{Membership, Store};
use crate::otel::{LaneIdentity, LaneOutcome, LanePhase, LanePhaseKind, LaneTelemetry};
use std::collections::BTreeMap;
use std::time::Duration;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use opentelemetry::trace::TraceContextExt as _;

#[derive(Default)]
struct Active {
    admission: Option<LanePhase>,
    executions: BTreeMap<String, LanePhase>,
    release: Option<LanePhase>,
    links: Option<opentelemetry::Context>,
}

#[derive(Default)]
pub(in crate::store) struct Observations {
    telemetry: LaneTelemetry,
    active: BTreeMap<String, Active>,
}

fn links() -> Option<opentelemetry::Context> {
    let context = tracing::Span::current().context();
    context.span().span_context().is_valid().then_some(context)
}

impl Observations {
    fn transition(&mut self, member: &Membership, previous: Option<&str>, now_ms: u64, recovered: bool) {
        let terminal = matches!(member.state.as_str(), "retired" | "refused");
        if terminal && !self.active.contains_key(&member.request.id)
            && previous.is_some_and(|state| matches!(state, "retired" | "refused")) { return; }
        if terminal && recovered { return; }
        let identity = LaneIdentity {
            lane: member.request.name.clone(), membership: member.request.id.clone(),
            run: member.request.owner.run.clone(), step: member.request.owner.step.clone(),
            attempt: u64::from(member.request.owner.attempt),
            generation: member.grant.as_ref().map_or_else(String::new, |grant| grant.generation.clone()),
            invocation: member.request.owner.invocation.clone(),
        };
        if !self.active.contains_key(&member.request.id) {
            self.active.insert(member.request.id.clone(), Active { links: links(), ..Default::default() });
        }
        let active = self.active.get_mut(&member.request.id).expect("observation was just installed");
        let correlation = active.links.as_slice();
        let make_phase = |phase, at: u64| self.telemetry.phase(phase, &identity, correlation)
            .rebase(Duration::from_millis(now_ms.saturating_sub(at)), recovered || member.clock_uncertain || now_ms < at);
        if active.admission.is_none() && (member.state == "queued" || previous.is_none() || previous == Some("queued")) {
            active.admission = Some(make_phase(LanePhaseKind::Admission, member.queued_at));
        }
        if member.state != "queued" && let Some(mut phase) = active.admission.take() {
            if let Some(grant) = &member.grant { phase.set_generation(&grant.generation); }
            let outcome = if member.state == "refused" {
                if member.refusal_code == Some(super::RefusalCode::AdmissionExpired) { LaneOutcome::Expired } else { LaneOutcome::Refused }
            } else if member.grant.is_some() { LaneOutcome::Succeeded } else { LaneOutcome::Cancelled };
            phase.finish(outcome);
        }
        for root in &member.roots {
            if let Some(at) = root.payload_released_at
                && root.execution_ended_at.is_none() && !terminal && !active.executions.contains_key(&root.runtime_id)
            {
                let mut phase = make_phase(LanePhaseKind::Execution, at);
                if let Some(execution) = &root.execution { phase.set_generation(&execution.generation_id); }
                active.executions.insert(root.runtime_id.clone(), phase);
            }
            if (root.execution_ended_at.is_some() || terminal)
                && let Some(phase) = active.executions.remove(&root.runtime_id)
            {
                phase.finish(if root.execution_ended_at.is_some() { LaneOutcome::Succeeded } else { LaneOutcome::Uncertain });
            }
        }
        if matches!(member.state.as_str(), "retiring" | "uncertain") && active.release.is_none() {
            active.release = Some(make_phase(LanePhaseKind::Release, member.retirement_requested_at.unwrap_or(now_ms)));
        }
        if member.state == "uncertain" && previous != Some("uncertain") {
            tracing::warn!(st.resource.lane = %member.request.name, st.resource.membership = %member.request.id,
                st.run = %member.request.owner.run, st.step = %member.request.owner.step,
                st.attempt = member.request.owner.attempt, "resource retirement uncertain; occupancy retained");
        }
        if terminal {
            if let Some(phase) = active.release.take() { phase.finish(LaneOutcome::Succeeded); }
            self.active.remove(&member.request.id);
        }
    }
}

impl Store {
    /// Called with the SQLite writer still held, after commit, preserving observation order.
    pub(super) fn observe_resource_transition(&self, connection: &rusqlite::Connection, member: &Membership, previous: Option<&str>, now_ms: u64, recovered: bool) {
        // Timeout anchors are maintained even when every telemetry exporter is disabled.
        if member.state != "queued" { self.finish_resource_admission_clock(&member.request.id); }
        for root in &member.roots {
            if let Some(at) = root.payload_released_at {
                if root.execution_ended_at.is_some() || matches!(member.state.as_str(), "retired" | "refused") {
                    self.finish_resource_execution_clock(&member.request.id, &root.runtime_id);
                } else {
                    self.anchor_resource_execution_clock(&member.request.id, &root.runtime_id, at, now_ms);
                }
            }
        }
        let observations = self.resource_observations.get_or_init(Default::default);
        let Ok(mut observations) = observations.lock() else { return; };
        observations.transition(member, previous, now_ms, recovered);
        if let Err(error) = publish_snapshot(connection, &observations.telemetry, &member.request.host, &member.request.name) {
            tracing::warn!(error = %error, "resource lane snapshot observation failed");
        }
    }

    /// Recreate only live observation segments after recovery; never replay completed samples.
    pub fn observe_recovered_resource_memberships(&self, host: &str, now_ms: u64) -> anyhow::Result<()> {
        let connection = self.connection.write();
        let mut statement = connection.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1 AND state NOT IN ('retired','refused')")?;
        let members = statement.query_map([host], |row| row.get::<_, String>(0))?
            .map(|body| Ok(serde_json::from_str::<Membership>(&body?)?)).collect::<anyhow::Result<Vec<_>>>()?;
        for member in members { self.observe_resource_transition(&connection, &member, Some(&member.state), now_ms, true); }
        let mut statement = connection.prepare("SELECT lane FROM resource_lane_policies_v1 WHERE host=?1")?;
        let lanes = statement.query_map([host], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        let observations = self.resource_observations.get_or_init(Default::default);
        if let Ok(observations) = observations.lock() {
            for lane in lanes { publish_snapshot(&connection, &observations.telemetry, host, &lane)?; }
        }
        Ok(())
    }

    /// Durable hold mutations call this after their transaction commits and writer is released.
    pub fn observe_resource_holds(&self, host: &str, operations: &[String], resolved_age: Option<Duration>) {
        let connection = self.connection.write();
        let result = (|| -> anyhow::Result<()> {
            let mut statement = connection.prepare("SELECT lane FROM resource_lane_policies_v1 WHERE host=?1")?;
            let lanes = statement.query_map([host], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            let observations = self.resource_observations.get_or_init(Default::default);
            let Ok(observations) = observations.lock() else { return Ok(()); };
            for lane in lanes {
                if operations.iter().any(|operation| operation == "*" || operation == &lane) {
                    publish_snapshot(&connection, &observations.telemetry, host, &lane)?;
                    if let Some(age) = resolved_age { observations.telemetry.hold_resolved(&lane, age); }
                }
            }
            Ok(())
        })();
        if let Err(error) = result { tracing::warn!(error = %error, "resource hold observation failed"); }
    }
}

fn publish_snapshot(connection: &rusqlite::Connection, telemetry: &LaneTelemetry, host: &str, lane: &str) -> anyhow::Result<()> {
    let (occupancy, queued): (u64, u64) = connection.query_row(
        "SELECT coalesce(sum(state IN ('admitted','launched','retiring','uncertain')),0), coalesce(sum(state='queued'),0) FROM resource_memberships_v1 WHERE host=?1 AND lane=?2",
        rusqlite::params![host,lane], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let holds = super::super::host_holds::blocking_tx(connection, host, lane)?.len();
    telemetry.snapshot(lane, occupancy, queued, holds as u64);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member() -> Membership {
        serde_json::from_value(serde_json::json!({
            "request": {
                "id": "membership-public-fixture", "name": "executor", "host": "example-host",
                "maxMembers": 1, "capacity": null, "coverage": "logical", "admissionTimeoutMs": null,
                "owner": { "run": "run-public-fixture", "step": "build", "attempt": 1, "invocation": null }
            },
            "grant": null, "state": "queued", "reason": "host held", "queuedAt": 100,
            "admittedAt": null, "payloadReleasedAt": null, "executionEndedAt": null,
            "retiredAt": null, "roots": []
        })).unwrap()
    }

    fn release_root(member: &mut Membership) {
        member.payload_released_at = Some(130);
        member.roots.push(serde_json::from_value(serde_json::json!({
            "runtimeId": "runtime-public-fixture",
            "execution": { "generationId": "generation-public-fixture" },
            "payloadReleasedAt": 130,
            "executionEndedAt": null
        })).unwrap());
    }

    #[test]
    fn queue_and_release_observers_survive_rechecks_and_uncertain_retirement() {
        let mut observations = Observations::default();
        let mut member = member();
        observations.transition(&member, None, 100, false);
        observations.transition(&member, Some("queued"), 110, false);
        assert_eq!(observations.active.len(), 1);
        assert!(observations.active[&member.request.id].admission.is_some());
        member.state = "admitted".into();
        member.admitted_at = Some(120);
        observations.transition(&member, Some("queued"), 120, false);
        assert!(observations.active[&member.request.id].admission.is_none());
        member.state = "launched".into();
        release_root(&mut member);
        observations.transition(&member, Some("admitted"), 130, false);
        assert_eq!(observations.active[&member.request.id].executions.len(), 1);
        member.state = "retiring".into();
        member.retirement_requested_at = Some(140);
        observations.transition(&member, Some("launched"), 140, false);
        assert!(observations.active[&member.request.id].release.is_some());
        assert_eq!(observations.active[&member.request.id].executions.len(), 1);
        member.state = "uncertain".into();
        observations.transition(&member, Some("retiring"), 150, false);
        assert!(observations.active[&member.request.id].release.is_some());
        member.execution_ended_at = Some(160);
        member.roots[0].execution_ended_at = Some(160);
        observations.transition(&member, Some("uncertain"), 160, false);
        assert!(observations.active[&member.request.id].executions.is_empty());
        member.state = "retired".into();
        member.retired_at = Some(170);
        observations.transition(&member, Some("uncertain"), 170, false);
        assert!(observations.active.is_empty());
        observations.transition(&member, Some("retired"), 180, false);
        assert!(observations.active.is_empty());
    }

    #[test]
    fn recovery_recreates_only_live_root_segments() {
        let mut observations = Observations::default();
        let mut member = member();
        assert_eq!(member.retirement_requested_at, None);
        member.state = "retiring".into();
        release_root(&mut member);
        member.retirement_requested_at = Some(140);
        observations.transition(&member, Some("retiring"), 300, true);
        assert!(observations.active[&member.request.id].admission.is_none());
        assert_eq!(observations.active[&member.request.id].executions.len(), 1);
        assert!(observations.active[&member.request.id].release.is_some());
        member.state = "retired".into();
        let mut after_restart = Observations::default();
        after_restart.transition(&member, Some("retired"), 400, true);
        assert!(after_restart.active.is_empty());
    }
}
