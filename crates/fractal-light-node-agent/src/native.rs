//! Native Observe/Stop adapter. Transport loss never owns native process lifetime.
use anyhow::{Context, Result};
use fractal_light_node_core::{
    authority::{ExecutionBinding, local},
    custody::{self, LocalSession, SessionCustody},
    journal::{self, Command, Journal, Operation, Outcome, Recovery, Selection},
    session::Executor,
};
use fractal_runtime::{ExecObservation, ExecRuntime, PtyRuntime};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::{Arc, Mutex}};

#[derive(Serialize, Deserialize)]
#[serde(tag = "native_phase", rename_all = "snake_case")]
enum NativeProof {
    Complete { outcome: Outcome },
    Stop { binding: LocalSession, retired: bool },
}

type RetirementResult = Option<std::result::Result<(), Arc<str>>>;
type RetirementWaits = BTreeMap<(String, String), tokio::sync::watch::Receiver<RetirementResult>>;

#[derive(Debug)]
struct Unavailable;
impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("local native custody is absent, detached, ambiguous, or stale")
    }
}
impl std::error::Error for Unavailable {}

/// Kernel waiters are generation keyed, retained across websocket reconnects, and bounded
/// by admitted durable custody. No drop handler stops a healthy process.
pub struct NativeExecutor {
    host: String,
    journal: Journal,
    pty: PtyRuntime,
    exec: ExecRuntime,
    waits: Mutex<RetirementWaits>,
}

impl NativeExecutor {
    /// Repairs receipt finalization before a client is allowed to transmit retained results.
    pub fn new(host: String, journal: Journal, pty: PtyRuntime, exec: ExecRuntime) -> Result<Self> {
        anyhow::ensure!(journal.selection()?.host == host, "journal names another physical host");
        let executor = Self { host, journal, pty, exec, waits: Mutex::new(BTreeMap::new()) };
        executor.repair_finalizations()?;
        Ok(executor)
    }

    /// Explicit private provisioning is the only admission source. Reopening identical
    /// retained metadata is inert, including after Stop; it cannot resurrect settled custody.
    /// Returns false while the configured epoch awaits its paired durable drain.
    pub fn bootstrap(&self, target: &Selection, bindings: &[LocalSession]) -> Result<bool> {
        anyhow::ensure!(target.host == self.host, "configured custody names another physical host");
        let active = self.journal.with_authority_transaction(|authority| {
            let selection = journal::current(authority)?;
            anyhow::ensure!(selection.host == self.host, "journal custody names another physical host");
            if selection != *target || journal::is_closing(authority)? { return Ok(false); }
            for binding in bindings {
                custody::require_provisioning_compatible(authority, binding)?;
                anyhow::ensure!(selection.grant.subjects.contains(&binding.subject),
                    "provisioned native custody is outside the selected grant");
                if let Some(prior) = custody::local_session_at(authority, &binding.subject, &binding.incarnation)? {
                    anyhow::ensure!(prior == *binding, "provisioned custody conflicts with retained exact metadata");
                    continue;
                }
                self.require_native(binding)?;
                custody::bind(authority, binding)?;
            }
            Ok(true)
        })?;
        if active { self.refresh_bindings()?; }
        Ok(active)
    }

    /// Must be called on journal authority changes, not by an idle polling timer.
    pub fn refresh_bindings(&self) -> Result<()> {
        if self.journal.closing()? { return Ok(()); }
        let selection = self.journal.selection()?;
        anyhow::ensure!(selection.host == self.host, "selection names another physical host");
        let bindings = self.journal.with_authority(|authority| {
            let mut bindings = Vec::new();
            for subject in &selection.grant.subjects {
                let Some(binding) = custody::local_session(authority, subject)? else { continue; };
                match self.require_native(&binding) {
                    Ok(()) => bindings.push(execution_binding(&selection, &binding)?),
                    Err(error) if error.is::<Unavailable>() => {},
                    Err(error) => return Err(error),
                }
            }
            Ok(bindings)
        })?;
        for prior in self.journal.reported_executions()? {
            if !bindings.contains(&prior) { self.journal.revoke_execution(&prior)?; }
        }
        for binding in bindings { let _ = self.journal.offer_execution(&binding)?; }
        Ok(())
    }

    fn require_native(&self, binding: &LocalSession) -> Result<()> {
        if binding.terminal {
            let snapshot = self.pty.snapshot()?;
            let mut matches = snapshot.iter().filter(|item| item.name == binding.runtime_id);
            let item = matches.next().ok_or(Unavailable)?;
            anyhow::ensure!(matches.next().is_none() && item.status == "running"
                && item.pid.zip(item.created_at.as_ref()).is_some_and(|(pid, at)|
                    format!("{pid}:{at}") == binding.native_incarnation), Unavailable);
            let fractal_runtime::PtyLookup::Found { observation, identity } =
                self.pty.keyed_observation(&binding.runtime_id)? else { return Err(Unavailable.into()); };
            anyhow::ensure!(identity == pty_core::registry::DaemonIdentityCheck::Match
                && observation == *item, Unavailable);
            self.pty.protect_servers(&snapshot)?;
            if let Some(operation) = &binding.start_operation {
                anyhow::ensure!(item.tags.get("st3.start-operation") == Some(operation), Unavailable);
            }
        } else {
            let Some(ExecObservation::Running(generation)) = self.exec.observe(&binding.runtime_id)? else {
                return Err(Unavailable.into());
            };
            anyhow::ensure!(generation.generation_id == binding.native_incarnation
                && binding.start_operation.as_ref().is_none_or(|operation|
                    generation.start_operation.as_ref() == Some(operation)), Unavailable);
        }
        Ok(())
    }

    fn require_authority(&self, authority: &Connection, command: &Command) -> Result<()> {
        let selection = journal::current(authority)?;
        anyhow::ensure!(!journal::is_closing(authority)? && command.binding.host == self.host
            && command.binding.host == selection.host && command.binding.home == selection.home
            && command.binding.epoch == selection.epoch && command.binding.run.is_none()
            && selection.grant.subjects.contains(&command.binding.subject)
            && selection.grant.operations.contains(&command.operation.kind()),
            "native command is outside the current open epoch grant");
        local::require_acknowledged_execution(authority, &command.binding)?;
        Ok(())
    }

    fn live(&self, authority: &Connection, command: &Command) -> Result<LocalSession> {
        let binding = custody::local_session(authority, &command.binding.subject)?.ok_or(Unavailable)?;
        anyhow::ensure!(binding.incarnation == command.binding.incarnation
            && binding.runtime_id == command.binding.runtime, Unavailable);
        self.require_native(&binding)?;
        Ok(binding)
    }

    fn stop_proof(&self, authority: &Connection, command: &Command, binding: LocalSession) -> Result<NativeProof> {
        if binding.custody == SessionCustody::Borrowed {
            custody::detach(authority, &binding.subject, &binding.incarnation, &command.execution_id)?;
            return Ok(NativeProof::Complete { outcome: Outcome::Detached { incarnation: binding.incarnation } });
        }
        let retired = if binding.terminal {
            // The canonical runtime fences both native incarnation and registry generation,
            // verifies daemon/tree retirement and ends the native work scope before return.
            self.pty.stop_if(&binding.runtime_id, Some(&binding.native_incarnation))?;
            custody::verify_retirement(authority, &binding)?;
            true
        } else {
            self.exec.stop_if(&binding.runtime_id, Some(&binding.native_incarnation))?;
            false
        };
        Ok(NativeProof::Stop { binding, retired })
    }

    async fn await_exec_retirement(&self, binding: &LocalSession) -> Result<()> {
        let key = (binding.runtime_id.clone(), binding.native_incarnation.clone());
        let mut completion = {
            let mut waits = self.waits.lock().map_err(|_| anyhow::anyhow!("native retirement wait ownership poisoned"))?;
            if let Some(existing) = waits.get(&key) { existing.clone() } else {
                let (done, completion) = tokio::sync::watch::channel(None);
                let exec = self.exec.clone();
                let runtime = binding.runtime_id.clone();
                let incarnation = binding.native_incarnation.clone();
                std::thread::Builder::new().name("native-session-retirement".into()).spawn(move || {
                    let result = (|| -> Result<()> {
                        exec.wait_for_exit(&runtime, &incarnation)?;
                        exec.retire_if(&runtime, &incarnation)?;
                        Ok(())
                    })().map_err(|error| Arc::<str>::from(format!("{error:#}")));
                    done.send_replace(Some(result));
                })?;
                waits.insert(key, completion.clone());
                completion
            }
        };
        loop {
            if let Some(result) = completion.borrow_and_update().clone() {
                result.map_err(|error| anyhow::anyhow!("native retirement remains unresolved: {error}"))?;
                self.journal.with_authority(|authority| custody::verify_retirement(authority, binding))?;
                return Ok(());
            }
            completion.changed().await.context("native retirement worker lost its result")?;
        }
    }

    fn validate_stop(&self, command: &Command, binding: &LocalSession) -> Result<()> {
        anyhow::ensure!(matches!(command.operation, Operation::Stop) && command.binding.host == self.host
            && binding.subject == command.binding.subject && binding.runtime_id == command.binding.runtime
            && binding.incarnation == command.binding.incarnation && binding.custody == SessionCustody::Owned,
            "native stop receipt custody differs");
        Ok(())
    }

    fn finish_stop(&self, command: &Command, binding: LocalSession) -> Result<Outcome> {
        self.validate_stop(command, &binding)?;
        anyhow::ensure!(self.journal.with_authority(|authority| custody::retirement_verified(authority, &binding))?,
            "native stop receipt lacks retained exact retirement proof");
        let outcome = Outcome::Stopped { incarnation: binding.incarnation.clone() };
        let proof = serde_json::to_string(&NativeProof::Stop { binding: binding.clone(), retired: true })?;
        self.journal.complete_requiring_local_finalization(&command.execution_id, outcome.clone(), &proof)?;
        self.journal.with_authority_transaction(|authority| custody::settle(authority, &binding))?;
        self.journal.finish_local_finalization(&command.execution_id)?;
        self.waits.lock().map_err(|_| anyhow::anyhow!("native retirement wait ownership poisoned"))?
            .remove(&(binding.runtime_id, binding.native_incarnation));
        Ok(outcome)
    }

    fn repair_finalizations(&self) -> Result<()> {
        for (recovery, outcome) in self.journal.pending_terminal_results()? {
            let Outcome::Stopped { incarnation } = outcome else { anyhow::bail!("pending native result is not a stop"); };
            let proof = recovery.custody_proof.context("pending native result lost proof")?;
            let NativeProof::Stop { binding, retired: true } = serde_json::from_str(&proof)? else {
                anyhow::bail!("pending native result lacks positive retirement proof");
            };
            self.validate_stop(&recovery.command, &binding)?;
            anyhow::ensure!(binding.incarnation == incarnation, "pending stop incarnation differs");
            self.journal.with_authority_transaction(|authority| custody::settle(authority, &binding))?;
            self.journal.finish_local_finalization(&recovery.command.execution_id)?;
        }
        Ok(())
    }
}

fn execution_binding(selection: &Selection, binding: &LocalSession) -> Result<ExecutionBinding> {
    Ok(ExecutionBinding {
        version: 1, home: selection.home.clone(), host: selection.host.clone(), subject: binding.subject.clone(),
        runtime: binding.runtime_id.clone(), incarnation: binding.incarnation.clone(), epoch: selection.epoch,
        authorization_revision: hex::encode(Sha256::digest(serde_json::to_vec(&(selection, binding))?)), run: None,
    })
}

impl Executor for NativeExecutor {
    fn enter(&self, command: &Command, authority: &Connection) -> Result<String> {
        anyhow::ensure!(command.binding.host == self.host, "command names another physical host");
        if matches!(command.operation, Operation::Start) {
            return Err(fractal_wire::Error::new("light-node-start-not-supported", "native agent supports observe and stop only").into());
        }
        self.require_authority(authority, command)?;
        let binding = match self.live(authority, command) {
            Ok(binding) => binding,
            Err(error) if error.is::<Unavailable>() => return Ok(serde_json::to_string(
                &NativeProof::Complete { outcome: Outcome::Refused { reason: error.to_string() } })?),
            Err(error) => return Err(error),
        };
        let proof = match command.operation {
            Operation::Observe => NativeProof::Complete { outcome: Outcome::Observed { incarnation: binding.incarnation, state: "running".into() } },
            Operation::Stop => self.stop_proof(authority, command, binding)?,
            Operation::Start => unreachable!("start rejected before native resolution"),
        };
        Ok(serde_json::to_string(&proof)?)
    }

    fn cancel(&self, _command: &Command, _authority: &Connection, _reason: &str) -> Result<()> {
        // Delivered Stop cannot be undone; cancellation is never physical closure.
        Ok(())
    }

    fn recover<'a>(&'a self, recovery: Recovery) -> Pin<Box<dyn Future<Output = Result<Outcome>> + Send + 'a>> {
        Box::pin(async move {
            let mut recovery = recovery;
            if recovery.custody_proof.is_none() && matches!(recovery.command.operation, Operation::Stop) {
                let live = self.journal.with_authority(|authority| self.live(authority, &recovery.command));
                match live {
                    Ok(_) => recovery = self.journal.reconcile_entered_stop(&recovery.command.execution_id, |command, authority| {
                        self.require_authority(authority, command)?;
                        let binding = self.live(authority, command)?;
                        Ok(serde_json::to_string(&self.stop_proof(authority, command, binding)?)?)
                    })?,
                    Err(error) if error.is::<Unavailable>() => {},
                    Err(error) => return Err(error),
                }
            }
            if let Some(proof) = recovery.custody_proof.as_deref() {
                match serde_json::from_str::<NativeProof>(proof)? {
                    NativeProof::Complete { outcome } => {
                        anyhow::ensure!(!matches!(outcome, Outcome::Stopped { .. }), "stop proof lost native custody");
                        if matches!(outcome, Outcome::Detached { .. }) {
                            self.journal.authority_changed.notify_waiters();
                        }
                        return Ok(outcome);
                    }
                    NativeProof::Stop { binding, retired } => {
                        self.validate_stop(&recovery.command, &binding)?;
                        if !retired {
                            anyhow::ensure!(!binding.terminal, "terminal retirement proof unavailable");
                            self.await_exec_retirement(&binding).await?;
                        }
                        return self.finish_stop(&recovery.command, binding);
                    }
                }
            }
            match recovery.command.operation {
                Operation::Start => Err(fractal_wire::Error::new("light-node-start-not-supported", "native start is not enabled").into()),
                Operation::Observe => Ok(Outcome::Cancelled { reason: "observation interrupted before durable result".into() }),
                Operation::Stop => {
                    let live = &recovery.command.binding;
                    let binding = self.journal.with_authority(|authority| {
                        if custody::detached(authority, &live.subject, &live.incarnation)? { return Ok(None); }
                        let binding = custody::local_session_at(authority, &live.subject, &live.incarnation)?
                            .context("stop custody binding unavailable")?;
                        self.validate_stop(&recovery.command, &binding)?;
                        Ok(Some(binding))
                    })?;
                    let Some(binding) = binding else {
                        self.journal.authority_changed.notify_waiters();
                        return Ok(Outcome::Detached { incarnation: live.incarnation.clone() });
                    };
                    let verified = self.journal.with_authority(|authority| custody::retirement_verified(authority, &binding))?;
                    if !verified {
                        anyhow::ensure!(!binding.terminal, "native terminal scope retirement is unproven");
                        self.await_exec_retirement(&binding).await?;
                    }
                    self.finish_stop(&recovery.command, binding)
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
