//! Native connector roles are selected from the seat's exact physical PTY generation.
//! Requester arguments, inherited paths and arbitrary ancestry never nominate a wrapper.
use super::{AppState, NativeDeliveryPeer};
use crate::{MailboxLeaseStore, SeatsStoreCore, mailbox::{Authority, Fence}, model::St3Error};
use fractal_drivers::{claude_process::{self, Process}, native_connector};
use smallclaims::error::internal;
use std::path::PathBuf;

fn refused(reason: impl Into<String>) -> St3Error {
    St3Error::new("native-role-refused", reason)
}

pub fn source_agent_dir(state: &AppState, fence: &Fence) -> PathBuf {
    crate::hooks::claude_agent_dir(&state.state_dir.join("drivers"), &fence.subject, &state.node)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    subject: String,
    runtime_incarnation: String,
    harness: String,
    session: String,
    sequence: u64,
    connector_pid: u32,
}

/// Only the recorded provider's own accepted connection can nominate its spawned connector.
pub async fn bootstrap(
    axum::extract::State(state): axum::extract::State<AppState>,
    peer: Option<axum::Extension<super::LocalPeer>>,
    axum::Json(request): axum::Json<Bootstrap>,
) -> Result<axum::Json<serde_json::Value>, super::ApiError> {
    let peer = peer.ok_or_else(|| super::ApiError::bad(refused("connector registration requires a local provider")))?;
    super::blocking_action(move || {
        let kernel = peer.peer_process.as_ref().ok_or_else(|| refused("provider kernel identity unavailable"))?;
        if kernel.pid() != peer.pid { return Err(refused("provider PID differs from accepted kernel identity")); }
        kernel.validate().map_err(internal)?;
        if !matches!(request.harness.as_str(), "omp" | "pi") { return Err(refused("provider does not register native connectors")); }
        let fence = Fence::new(&request.subject, &request.runtime_incarnation, "delivery");
        let selected = physical_selection(&state, &fence, &request.harness)?;
        let dir = source_agent_dir(&state, &fence);
        let identity = native_connector::incarnation(&dir, selected.3, &request.harness).map_err(internal)?;
        if identity != (request.session.clone(), request.sequence) { return Err(refused("provider ownership changed before connector registration")); }
        kernel.validate().map_err(internal)?;
        // Re-select immediately before recording so a replaced terminal cannot register custody.
        let current = physical_selection(&state, &fence, &request.harness)?;
        if current != selected { return Err(refused("physical wrapper changed during connector registration")); }
        native_connector::register_connector(&dir, peer.pid, request.connector_pid, &request.harness, &request.session, request.sequence).map_err(internal)?;
        Ok(())
    }).await?;
    Ok(axum::Json(serde_json::json!({ "registered": true })))
}

enum Custody {
    Native(native_connector::Witness),
    Claude { session: String, sequence: u64 },
    /// Only the already admitted exact capability survives an unwitnessed generation.
    Retained { owner: Authority, fence: Fence },
}

pub struct Proof {
    state: AppState,
    fence: Fence,
    peer: NativeDeliveryPeer,
    physical: Option<PhysicalProof>,
    source: Option<(String, Vec<u8>)>,
    custody: Custody,
}

struct PhysicalProof {
    observation: fractal_runtime::PtyObservation,
    daemon: Process,
    terminal: Process,
    wrapper: Process,
    bound: bool,
}

fn process(pid: u32) -> Result<Process, St3Error> {
    claude_process::process(pid).map_err(|error| refused(format!("physical process unavailable: {error:#}")))
}

fn select_wrapper(terminal: Process, recorded: Option<Process>, bound: bool) -> Result<Process, St3Error> {
    if !bound { return Ok(terminal); }
    let wrapper = recorded.ok_or_else(|| refused("bound terminal has no recorded native wrapper"))?;
    if wrapper.parent != terminal.pid || wrapper.pid == terminal.pid {
        return Err(refused("recorded wrapper is not the direct physical terminal child"));
    }
    Ok(wrapper)
}

impl Proof {
    pub fn require_publication(&self, publication: &crate::harness_events::Publication) -> Result<(), St3Error> {
        if publication.runtime_incarnation != self.fence.incarnation
            || publication.claim.fields.get("incarnation_id").and_then(serde_json::Value::as_str) != Some(self.fence.incarnation.as_str()) {
            return Err(refused("publication belongs to another runtime"));
        }
        let (_, raw) = self.source.as_ref().ok_or_else(|| refused("publication has no canonical provider state"))?;
        let record: serde_json::Value = serde_json::from_slice(raw).map_err(internal)?;
        let stamp = record["writtenAtMs"].as_u64().ok_or_else(|| refused("provider stamp absent"))?;
        let current = fractal_drivers::harness_state::read_raw_at(raw, None, stamp);
        let fields = &publication.claim.fields;
        if publication.claim.kind == "harness.observed"
            && (fields.get("evidence_incarnation").and_then(serde_json::Value::as_str) != current.evidence_incarnation.as_deref()
                || fields.get("ownership_sequence").and_then(serde_json::Value::as_u64) != current.ownership_sequence) {
            return Err(refused("publication provider session or ownership sequence was superseded"));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), St3Error> {
        self.peer.require_kernel()?;
        if let Some(physical) = &self.physical {
            let current = observe(&self.state, &physical.observation.name)?;
            if current != physical.observation { return Err(refused("the physical runtime generation changed")); }
            for captured in [physical.daemon, physical.terminal, physical.wrapper] {
                if process(captured.pid)? != captured { return Err(refused("physical process generation changed")); }
            }
            let terminal = terminal(&self.state, &current, physical.daemon)?;
            if terminal != physical.terminal || select_wrapper(terminal, Some(physical.wrapper), physical.bound)? != physical.wrapper {
                return Err(refused("the physical wrapper was replaced"));
            }
        }
        let dir = source_agent_dir(&self.state, &self.fence);
        if fractal_drivers::harness_events::read_bound_provider_state(&dir).map_err(internal)? != self.source {
            return Err(refused("provider ownership changed during admission"));
        }
        match &self.custody {
            Custody::Native(witness) => witness.revalidate().map_err(internal),
            Custody::Claude { session, sequence } => claude_process::with_caller(
                &dir, session, *sequence, self.peer.pid, self.physical.as_ref().map(|physical| physical.wrapper),
                if self.peer.archives_inbox { claude_process::Caller::Wrapper } else { claude_process::Caller::Channel },
                |admission| {
                    anyhow::ensure!(admission == claude_process::Admission::Primary, "native role requires witnessed Claude ownership");
                    Ok(())
                },
            ).map_err(internal),
            Custody::Retained { owner, fence } => {
                let present = if owner.provider == "claude" {
                    claude_process::recorded(&dir)
                } else { native_connector::recorded(&dir) }.map_err(internal)?;
                if present { return Err(refused("retained unwitnessed generation gained a launch record")); }
                if self.state.store.mailbox_lease_authority(fence)?.as_ref() != Some(owner) {
                    return Err(refused("retained mailbox capability changed"));
                }
                self.state.store.check_mailbox_custody_fence(fence)?;
                self.state.store.mailbox_session_active(fence, owner)
            }
        }
    }
}

fn observe(state: &AppState, name: &str) -> Result<fractal_runtime::PtyObservation, St3Error> {
    match fractal_runtime::PtyRuntime::new(state.pty_root.clone())
        .with_binary(state.pty_binary.to_string_lossy()).keyed_observation(name).map_err(internal)? {
        fractal_runtime::PtyLookup::Found { observation, identity: pty_core::registry::DaemonIdentityCheck::Match } => Ok(observation),
        _ => Err(refused("the exact physical runtime identity is unavailable")),
    }
}

fn terminal(state: &AppState, physical: &fractal_runtime::PtyObservation, daemon: Process) -> Result<Process, St3Error> {
    let stats = pty_client::query_stats_in_with_timeout(&state.pty_root, &physical.name, pty_client::STATS_TIMEOUT).map_err(internal)?;
    if stats.name != physical.name || stats.created_at != physical.created_at
        || u32::try_from(stats.daemon.pid).ok() != Some(daemon.pid) || !stats.process.alive {
        return Err(refused("the physical terminal generation changed"));
    }
    let pid = stats.process.pid.and_then(|pid| u32::try_from(pid).ok()).filter(|pid| *pid > 1)
        .ok_or_else(|| refused("no live physical terminal child"))?;
    let terminal = process(pid)?;
    if terminal.parent != daemon.pid { return Err(refused("terminal is not the direct runtime child")); }
    Ok(terminal)
}

fn physical_runtime_selection(state: &AppState, fence: &Fence, provider: &str) -> Result<(fractal_runtime::PtyObservation, bool), St3Error> {
    let desired = state.store.desired_subjects_named(std::slice::from_ref(&fence.subject)).map_err(internal)?
        .into_iter().next().ok_or_else(|| refused("seat no longer declared"))?;
    let member = desired.member.ok_or_else(|| refused("seat intentionally stopped"))?;
    if desired.kind == "stop" || member.host != state.node { return Err(refused("seat is stopped or foreign")); }
    if member.driver.as_deref() != Some(provider) { return Err(refused("seat belongs to another provider")); }
    let bound = member.terminal_binding.is_some();
    let incarnation = if let Some(binding) = &member.terminal_binding {
        if binding.agent_incarnation() != fence.incarnation { return Err(refused("bound invocation changed")); }
        let declaration = state.store.desired_subjects_named(std::slice::from_ref(&binding.subject)).map_err(internal)?
            .into_iter().next().ok_or_else(|| refused("bound terminal no longer declared"))?;
        let terminal = declaration.member.ok_or_else(|| refused("bound terminal stopped"))?;
        if declaration.kind == "stop" || terminal.kind != crate::model::MemberKind::Pty || !terminal.terminal
            || terminal.host != state.node || terminal.runtime_id != member.runtime_id {
            return Err(refused("bound terminal is not the declared local PTY"));
        }
        binding.incarnation.as_str()
    } else { &fence.incarnation };
    let physical = observe(state, &member.runtime_id)?;
    if physical.status != "running" || physical.pid.zip(physical.created_at.as_deref())
        .is_none_or(|(pid, stamp)| format!("{pid}:{stamp}") != incarnation)
        || member.terminal_binding.as_ref().is_some_and(|binding| physical.tags.get("st3.subject") != Some(&binding.subject)) {
        return Err(refused("exact physical runtime generation changed"));
    }
    Ok((physical, bound))
}

fn physical_selection(state: &AppState, fence: &Fence, provider: &str) -> Result<(fractal_runtime::PtyObservation, Process, Process, Process, bool), St3Error> {
    let (physical, bound) = physical_runtime_selection(state, fence, provider)?;
    let daemon = process(physical.pid.ok_or_else(|| refused("runtime has no daemon"))?)?;
    let terminal = terminal(state, &physical, daemon)?;
    let dir = source_agent_dir(state, fence);
    let recorded = if provider == "claude" { claude_process::recorded(&dir) } else { native_connector::recorded(&dir) }.map_err(internal)?;
    let recorded_wrapper = if bound && recorded {
        Some(if provider == "claude" { claude_process::wrapper(&dir) } else { native_connector::wrapper(&dir) }.map_err(internal)?)
    } else { None };
    let wrapper = select_wrapper(terminal, recorded_wrapper, bound)?;
    Ok((physical, daemon, terminal, wrapper, bound))
}

pub fn capture(state: &AppState, fence: &Fence, peer: &NativeDeliveryPeer) -> Result<Proof, St3Error> {
    peer.require_kernel()?;
    if peer.agent != fence.subject { return Err(refused("connector belongs to another seat")); }
    let provider = match peer.transport {
        "claude-channel" => "claude", "app-server" => "codex", "opencode-server" => "opencode",
        "omp-channel" => "omp", "pi-channel" => "pi", _ => return Err(refused("unsupported native provider")),
    };
    let dir = source_agent_dir(state, fence);
    let recorded = if provider == "claude" { claude_process::recorded(&dir) } else { native_connector::recorded(&dir) }.map_err(internal)?;
    let physical = if recorded {
        let (observation, daemon, terminal, wrapper, bound) = physical_selection(state, fence, provider)?;
        Some(PhysicalProof { observation, daemon, terminal, wrapper, bound })
    } else { None };
    let source = fractal_drivers::harness_events::read_bound_provider_state(&dir).map_err(internal)?;
    let custody = if !recorded {
        let candidates = if fence.component == "events" {
            [
                state.store.current_mailbox_fence(&fence.subject, &fence.incarnation, "delivery")?,
                state.store.current_mailbox_fence(&fence.subject, &fence.incarnation, "title")?,
            ]
        } else { [Some(fence.clone()), None] };
        let mut retained = None;
        for candidate in candidates.into_iter().flatten() {
            if let Some(owner) = state.store.mailbox_lease_authority(&candidate)?
                && owner.provider == provider && owner.pid == peer.pid
                && process(peer.pid).is_ok_and(|current| current.birth == owner.process_token) {
                retained = Some((candidate, owner));
                break;
            }
        }
        let (retained_fence, owner) = retained.ok_or_else(|| St3Error::new(
            "native-role-unavailable", "native launch witness is absent and no exact admitted lease exists",
        ))?;
        let (runtime, raw) = source.as_ref().ok_or_else(|| refused("retained provider source absent"))?;
        let value: serde_json::Value = serde_json::from_slice(raw).map_err(internal)?;
        let stamp = value["writtenAtMs"].as_u64().ok_or_else(|| refused("retained provider stamp absent"))?;
        let observed = fractal_drivers::harness_state::read_raw_at(raw, None, stamp);
        if runtime != &fence.incarnation || observed.harness.as_deref() != Some(provider)
            || observed.evidence_incarnation.as_deref() != Some(&owner.session)
            || observed.ownership_sequence != Some(owner.sequence)
            || observed.state == fractal_drivers::harness_state::Activity::Unknown
            || observed.state == fractal_drivers::harness_state::Activity::Ended && fence.component != "events" {
            return Err(refused("unwitnessed custody cannot promote its exact admitted session or sequence"));
        }
        Custody::Retained { owner, fence: retained_fence }
    } else {
        let wrapper = physical.as_ref().ok_or_else(|| refused("physical wrapper unavailable"))?.wrapper;
        let identity = source.as_ref().filter(|(runtime, _)| runtime == &fence.incarnation).map(|(_, raw)| {
            let value: serde_json::Value = serde_json::from_slice(raw).map_err(internal)?;
            let stamp = value["writtenAtMs"].as_u64().ok_or_else(|| refused("provider stamp absent"))?;
            let observed = fractal_drivers::harness_state::read_raw_at(raw, None, stamp);
            if observed.harness.as_deref() != Some(provider)
                || observed.state == fractal_drivers::harness_state::Activity::Unknown
                || (observed.state == fractal_drivers::harness_state::Activity::Ended && fence.component != "events") {
                return Err(refused("provider ownership is malformed, foreign or ended"));
            }
            Ok((observed.evidence_incarnation.filter(|value| !value.is_empty()).ok_or_else(|| refused("provider session absent"))?,
                observed.ownership_sequence.filter(|sequence| *sequence > 0).ok_or_else(|| refused("provider sequence absent"))?))
        }).transpose()?;
        if provider == "claude" {
            let (session, sequence) = identity.ok_or_else(|| refused("Claude provider ownership absent"))?;
            Custody::Claude { session, sequence }
        } else {
            let (session, sequence) = match identity {
                Some(identity) => identity,
                None if provider == "codex" && fence.component == "delivery" => native_connector::incarnation(&dir, wrapper, provider).map_err(internal)?,
                None => return Err(refused("provider ownership absent")),
            };
            Custody::Native(native_connector::capture(&dir, wrapper, peer.pid, provider,
                if peer.archives_inbox || fence.component == "events" && peer.pid == wrapper.pid {
                    native_connector::Role::Wrapper
                } else { native_connector::Role::Delivery }, &session, sequence).map_err(internal)?)
        }
    };
    let proof = Proof { state: state.clone(), fence: fence.clone(), peer: peer.clone(), physical, source, custody };
    proof.validate()?;
    Ok(proof)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bound_wrapper_must_be_recorded_direct_terminal_child() {
        let terminal = Process { pid: 11, parent: 10, birth: 1 };
        assert!(select_wrapper(terminal, None, true).is_err());
        assert!(select_wrapper(terminal, Some(Process { pid: 12, parent: 9, birth: 2 }), true).is_err());
        let wrapper = Process { pid: 12, parent: 11, birth: 2 };
        assert_eq!(select_wrapper(terminal, Some(wrapper), true).unwrap(), wrapper);
        assert_eq!(select_wrapper(terminal, Some(wrapper), false).unwrap(), terminal);
    }
    #[test]
    fn classified_peer_without_kernel_proof_cannot_capture_role() {
        let peer = NativeDeliveryPeer { agent: "agent/example/role".into(), transport: "omp-channel", pid: 37, archives_inbox: false, peer_process: None };
        assert!(peer.require_kernel().is_err());
    }
}
