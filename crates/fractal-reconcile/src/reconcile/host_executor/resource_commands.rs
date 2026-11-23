//! Resource commands retain the native membership/root journal as their custody authority.
//! Program/shell planning selects commands; only this boundary invokes physical mutations.
use super::*;
use crate::store::resource_membership::{LaneRequest, Owner};

/// Invalidates even on native refusal or partial failure, before a caller can publish its wake.
struct PhysicalMutation<'a>(&'a host_observations::HostObservations);
impl<'a> PhysicalMutation<'a> {
    fn begin(observations: &'a host_observations::HostObservations) -> Self {
        observations.invalidate();
        Self(observations)
    }
}
impl Drop for PhysicalMutation<'_> {
    fn drop(&mut self) { self.0.invalidate(); }
}

/// The member contains its immutable LaneRequest; native admission binds it to the durable
/// membership/root generation before release. The source-specific intent is rechecked at
/// that same physical release guard, not just when the graph planned the command.
pub(in crate::reconcile) enum StartCommand<'a> {
    Program { member: &'a MemberSpec, step: &'a str, attempt: u32 },
    Shell { member: &'a MemberSpec, execution: &'a str, owner: &'a Owner },
}
impl StartCommand<'_> {
    pub(in crate::reconcile) fn execute<R: RuntimeControl>(
        self, store: &Store, runtime: &R, observations: &host_observations::HostObservations,
    ) -> Result<()> {
        let _mutation = PhysicalMutation::begin(observations);
        let member = match &self {
            Self::Program { member, .. } | Self::Shell { member, .. } => *member,
        };
        runtime.start_resource_guarded(store, member, None, &|| {
            match &self {
                Self::Program { step, attempt, .. } => {
                    let current = store.step_run(step)?.context("program step disappeared before payload release")?;
                    anyhow::ensure!(current.attempt == *attempt && program::RUNNING_STATES.contains(&current.status.as_str()),
                        "program attempt was cancelled before payload release");
                }
                Self::Shell { execution, owner, .. } => {
                    anyhow::ensure!(!store.shell_resource(execution)?.cancel, "shell request cancelled before payload release");
                    store.shell_resource_owner_live(owner)?;
                }
            }
            Ok(())
        })
    }
}

/// Exact LaneRequest ownership remains fenced by the native custody journal. A root-only
/// retirement never releases its parent's charged membership. Shell retirement preserves the
/// existing asynchronous native worker and wakes only after invalidating physical evidence.
pub(in crate::reconcile) enum RetireCommand<'a> {
    Lane { request: &'a LaneRequest },
    Execution { request: &'a LaneRequest, runtime_id: &'a str },
    Shell {
        request: &'a LaneRequest,
        execution: Option<&'a str>,
        notify: &'a Arc<Notify>,
        events: &'a watch::Sender<u64>,
    },
}
impl RetireCommand<'_> {
    /// `true` is positive lane retirement proof, never merely an accepted execution signal.
    pub(in crate::reconcile) fn execute<R: RuntimeControl>(
        self, store: &Arc<Store>, runtime: &R, observations: &Arc<host_observations::HostObservations>,
    ) -> Result<bool> {
        let _mutation = PhysicalMutation::begin(observations);
        match self {
            Self::Lane { request } => runtime.retire_resource_lane(store, request),
            Self::Execution { request, runtime_id } => {
                runtime.retire_resource_exec(store, request, runtime_id)?;
                Ok(false)
            }
            Self::Shell { request, execution, notify, events } => {
                let observations = observations.clone();
                let notify = notify.clone();
                let events = events.clone();
                runtime.retire_shell_resource_lane(store.clone(), request, execution, Arc::new(move || {
                    observations.invalidate();
                    signal_changed(&notify, &events);
                }))
            }
        }
    }
}

/// Signals target the exact persisted execution generation. They do not prove retirement;
/// existing native exit watches and custody sweeps still supply that evidence.
pub(in crate::reconcile) enum SignalCommand<'a> {
    Cancel { runtime_id: &'a str, generation: &'a str },
    Kill { runtime_id: &'a str, generation: &'a str },
}
impl SignalCommand<'_> {
    pub(in crate::reconcile) fn execute<R: RuntimeControl>(
        self, runtime: &R, observations: &host_observations::HostObservations,
    ) -> Result<()> {
        let _mutation = PhysicalMutation::begin(observations);
        match self {
            Self::Cancel { runtime_id, generation } => runtime.cancel_resource_exec(runtime_id, generation),
            Self::Kill { runtime_id, generation } => runtime.kill_resource_exec(runtime_id, generation),
        }
    }
}

/// The planner establishes the host's new durable authority epoch before native recovery.
/// Native recovery retains the existing predecessor-owner and process-birth checks.
pub(in crate::reconcile) struct RecoverCommand<'a> {
    pub host: &'a str,
    pub now_ms: u64,
}
impl RecoverCommand<'_> {
    pub(in crate::reconcile) fn execute<R: RuntimeControl>(
        self, store: &Store, runtime: &R, observations: &host_observations::HostObservations,
    ) -> Result<usize> {
        let _mutation = PhysicalMutation::begin(observations);
        runtime.recover_resource_lanes(store, self.host, self.now_ms)
    }
}
