//! The first session slice controls existing native custody; it cannot launch work.
use super::*;
use crate::model::FractalError;
use fractal_peer::light_node::journal::{Command, Journal, Operation, Outcome, Recovery, Selection};
use fractal_peer::light_node::session::Executor;
use crate::store::local_sessions::{LocalSession, SessionCustody};
use crate::store::run_credentials::{ExecutionBinding, local};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::pin::Pin;

#[derive(Serialize, Deserialize)]
#[serde(tag = "native_phase", rename_all = "snake_case")]
enum NativeProof {
    Complete { outcome: Outcome },
    Stop { binding: LocalSession, retired: bool },
}

type RetirementResult = Option<std::result::Result<(), Arc<str>>>;
type RetirementWaits = BTreeMap<(String, String), tokio::sync::watch::Receiver<RetirementResult>>;

/// Own native waiters across transport reconnects, not across control-epoch replacement.
/// Each accepted physical generation has at most one kernel-wait worker; journal capacity
/// bounds unresolved work. Blocking kernel waits do not occupy Tokio's shutdown pool.
pub struct NativeSessionExecutor {
    reconciler: Arc<Reconciler>,
    waits: Mutex<RetirementWaits>,
}
impl NativeSessionExecutor {
    pub fn new(reconciler: Arc<Reconciler>) -> Self { Self { reconciler, waits: Mutex::new(BTreeMap::new()) } }

    async fn await_exec_retirement(&self, binding: &LocalSession) -> Result<()> {
        let key = (binding.runtime_id.clone(), binding.native_incarnation.clone());
        let mut completion = {
            let mut waits = self.waits.lock().map_err(|_| anyhow::anyhow!("native retirement wait ownership poisoned"))?;
            if let Some(existing) = waits.get(&key) { existing.clone() }
            else {
                let (done, completion) = tokio::sync::watch::channel(None);
                let exec = self.reconciler.runtime.exec.clone();
                let runtime = binding.runtime_id.clone();
                let incarnation = binding.native_incarnation.clone();
                std::thread::Builder::new().name("native-session-retirement".into()).spawn(move || {
                    let result = (|| -> Result<()> {
                        // Stop acceptance is only a signal. Wait for the leader before
                        // whole-scope cleanup, then retain exact native retirement proof.
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
                return result.map_err(|error| anyhow::anyhow!("native retirement remains unresolved: {error}"));
            }
            completion.changed().await.context("native retirement worker lost its result")?;
        }
    }

    fn stop_proof(&self, command: &Command, binding: LocalSession) -> Result<NativeProof> {
        let reconciler = &self.reconciler;
        reconciler.host_observations.invalidate();
        let retired = stop_local_session_with_retirement_proof(&reconciler.store, reconciler.runtime.as_ref(),
            &binding.subject, &binding.incarnation, &command.execution_id)?;
        reconciler.start_changed(&binding.subject);
        Ok(if binding.custody == SessionCustody::Borrowed {
            NativeProof::Complete { outcome: Outcome::Detached { incarnation: binding.incarnation } }
        } else { NativeProof::Stop { binding, retired } })
    }

    fn finish_stop(&self, command: &Command, binding: LocalSession) -> Result<Outcome> {
        anyhow::ensure!(binding.subject == command.binding.subject && binding.runtime_id == command.binding.runtime
            && binding.incarnation == command.binding.incarnation && binding.custody == SessionCustody::Owned,
            "native stop receipt custody differs");
        let outcome = Outcome::Stopped { incarnation: binding.incarnation.clone() };
        let reconciler = &self.reconciler;
        // Receipt custody is durable before local finalization, but cannot be transmitted
        // or reclaimed until that finalization succeeds. A returned Store error is just
        // as recoverable as a crash, including while the websocket remains connected.
        let proof = NativeProof::Stop { binding, retired: true };
        let encoded = serde_json::to_string(&proof)?;
        let NativeProof::Stop { binding, .. } = proof else { unreachable!("constructed stop proof") };
        let journal = reconciler.home_session_journal.get().context("session journal is not installed")?;
        journal.complete_requiring_local_finalization(&command.execution_id, outcome.clone(), &encoded)?;
        reconciler.store.settle_local_session_custody(&binding.subject, &binding.native_incarnation)?;
        journal.finish_local_finalization(&command.execution_id)?;
        self.waits.lock().map_err(|_| anyhow::anyhow!("native retirement wait ownership poisoned"))?
            .remove(&(binding.runtime_id.clone(), binding.native_incarnation.clone()));
        Ok(outcome)
    }
}

fn execution_binding(selection: &Selection, binding: &LocalSession) -> Result<ExecutionBinding> {
    let revision = hex::encode(Sha256::digest(serde_json::to_vec(&(selection, binding))?));
    Ok(ExecutionBinding {
        version: 1, home: selection.home.clone(), host: selection.host.clone(), subject: binding.subject.clone(),
        runtime: binding.runtime_id.clone(), incarnation: binding.incarnation.clone(), authorization_revision: revision,
        epoch: selection.epoch, run: None,
    })
}

impl<R: RuntimeControl> Reconciler<R> {
    /// Admit only exact, already-owned native sessions, never graph status or a launch.
    pub fn session_bindings(&self, selection: &Selection) -> Result<Vec<ExecutionBinding>> {
        anyhow::ensure!(selection.host == self.host, "session grant names another physical host");
        let mut bindings = Vec::new();
        for subject in &selection.grant.subjects {
            if self.store.local_session(subject)?.is_none() { continue; }
            let binding = match local_session(&self.store, self.runtime.as_ref(), subject, None) {
                Ok(binding) => binding,
                Err(error) if error.is::<LocalSessionUnavailable>() => {
                    tracing::warn!(%subject, %error, "native session is unavailable for home admission");
                    continue;
                }
                Err(error) => return Err(error),
            };
            bindings.push(execution_binding(selection, &binding)?);
        }
        Ok(bindings)
    }

    pub fn install_home_session_journal(&self, journal: Journal) -> Result<()> {
        anyhow::ensure!(journal.selection()?.host == self.host, "session journal names another physical host");
        self.home_session_journal.set(journal).map_err(|_| anyhow::anyhow!("session journal already installed"))?;
        let journal = self.home_session_journal.get().context("session journal installation failed")?;
        for (recovery, outcome) in journal.pending_terminal_results()? {
            let command = recovery.command;
            let Outcome::Stopped { incarnation } = outcome else {
                anyhow::bail!("pending native finalization is not an exact stop receipt");
            };
            let proof = recovery.custody_proof.context("pending stop finalization lost its native custody proof")?;
            let NativeProof::Stop { binding, .. } = serde_json::from_str(&proof)? else {
                anyhow::bail!("pending stop finalization lost its native custody snapshot");
            };
            anyhow::ensure!(command.binding.host == self.host && command.binding.incarnation == incarnation
                && binding.subject == command.binding.subject && binding.runtime_id == command.binding.runtime
                && binding.incarnation == incarnation && binding.custody == SessionCustody::Owned,
                "pending native stop receipt names another custody owner");
            self.store.settle_local_session_custody(&binding.subject, &binding.native_incarnation)?;
            journal.finish_local_finalization(&command.execution_id)?;
        }
        // A new daemon cannot adopt the previous in-memory program VM. Permission is
        // revoked before a fresh attempt, without claiming that the old process exited.
        for prior in journal.reported_executions()? {
            if prior.run.is_some() { journal.revoke_execution(&prior)?; }
        }
        self.refresh_session_bindings()
    }

    /// Replacement grants may offer existing custody only after the old-home barrier drains.
    pub fn refresh_session_bindings(&self) -> Result<()> {
        let journal = self.home_session_journal.get().context("session journal is not installed")?;
        if journal.closing()? { return Ok(()); }
        let selection = journal.selection()?;
        let bindings = self.session_bindings(&selection)?;
        for prior in journal.reported_executions()? {
            if prior.run.is_none() && !bindings.iter().any(|binding| binding == &prior) { journal.revoke_execution(&prior)?; }
        }
        for binding in bindings { let _ = journal.offer_execution(&binding)?; }
        Ok(())
    }

    pub(super) fn record_local_session(&self, binding: &LocalSession) -> Result<()> {
        if let Some(journal) = self.home_session_journal.get() {
            if !journal.closing()? {
                let selection = journal.selection()?;
                if selection.grant.subjects.contains(&binding.subject) {
                    if let Some(prior) = self.store.local_session(&binding.subject)? {
                        if prior != *binding {
                            // Withdraw the old remote permission under its dispatch barrier
                            // BEFORE publishing a different local custody owner.
                            journal.revoke_execution(&execution_binding(&selection, &prior)?)?;
                        }
                    }
                    self.store.bind_local_session(binding)?;
                    if self.store.local_session(&binding.subject)?.as_ref() == Some(binding) {
                        let _ = journal.offer_execution(&execution_binding(&selection, binding)?)?;
                    }
                    return Ok(());
                }
            }
        }
        self.store.bind_local_session(binding)
    }

    pub(super) fn revoke_session_incarnation(&self, subject: &str, incarnation: &str) -> Result<()> {
        if let Some(journal) = self.home_session_journal.get() {
            for binding in journal.reported_executions()?.into_iter().filter(|binding| binding.run.is_none() && binding.subject == subject) {
                if self.store.local_session_at(subject, &binding.incarnation)?.is_some_and(|local| local.native_incarnation == incarnation) {
                    journal.revoke_execution(&binding)?;
                }
            }
        }
        Ok(())
    }

    pub fn session_authority_changed(&self) -> Result<()> {
        self.program_run_authority_changed();
        if let Some(journal) = self.home_session_journal.get() {
            for subject in &journal.selection()?.grant.subjects { self.start_changed(subject); }
        }
        self.refresh_session_bindings()
    }

    fn session_live(&self, command: &Command) -> Result<LocalSession> {
        let binding = local_session(&self.store, self.runtime.as_ref(), &command.binding.subject, Some(&command.binding.incarnation))?;
        anyhow::ensure!(binding.runtime_id == command.binding.runtime, "session runtime binding changed");
        Ok(binding)
    }
}

impl Executor for NativeSessionExecutor {
    fn enter(&self, command: &Command, authority: &rusqlite::Connection) -> Result<String> {
        let reconciler = &self.reconciler;
        anyhow::ensure!(command.binding.host == reconciler.host, "command names another physical host");
        if matches!(command.operation, Operation::Start) {
            return Err(FractalError::new("light-node-start-not-supported", "this native session supports observe and stop only").into());
        }
        local::require_execution(authority, &command.binding)?;
        let binding = match reconciler.session_live(command) {
            Ok(binding) => binding,
            Err(error) if error.is::<LocalSessionUnavailable>() => {
                return Ok(serde_json::to_string(&NativeProof::Complete { outcome: Outcome::Refused { reason: error.to_string() } })?);
            }
            Err(error) => return Err(error),
        };
        let proof = match command.operation {
            Operation::Observe => NativeProof::Complete { outcome: Outcome::Observed { incarnation: binding.incarnation, state: "running".into() } },
            Operation::Stop => self.stop_proof(command, binding)?,
            Operation::Start => unreachable!("remote start was refused before native resolution"),
        };
        Ok(serde_json::to_string(&proof)?)
    }

    fn cancel(&self, _command: &Command, _authority: &rusqlite::Connection, _reason: &str) -> Result<()> {
        // A completed observation cannot be undone; an already-signaled Stop retains its
        // native retirement waiter. Cancellation never invents physical closure.
        Ok(())
    }

    fn recover<'a>(&'a self, recovery: Recovery) -> Pin<Box<dyn Future<Output = Result<Outcome>> + Send + 'a>> {
        Box::pin(async move {
            let reconciler = &self.reconciler;
            let mut recovery = recovery;
            if recovery.custody_proof.is_none() && matches!(recovery.command.operation, Operation::Stop) {
                match reconciler.session_live(&recovery.command) {
                    Ok(_) => {
                        let journal = reconciler.home_session_journal.get().context("session journal is not installed")?;
                        recovery = journal.reconcile_entered_stop(&recovery.command.execution_id, |command, authority| {
                            // This is generation-fenced reconciliation of a retained Stop
                            // intent, not blind dispatch or permission to launch anything.
                            local::require_execution(authority, &command.binding)?;
                            let binding = reconciler.session_live(command)?;
                            Ok(serde_json::to_string(&self.stop_proof(command, binding)?)?)
                        })?;
                    }
                    Err(error) if error.is::<LocalSessionUnavailable>() => {}
                    Err(error) => return Err(error),
                }
            }
            if let Some(proof) = recovery.custody_proof.as_deref() {
                match serde_json::from_str::<NativeProof>(proof)? {
                    NativeProof::Complete { outcome } => {
                        anyhow::ensure!(!matches!(outcome, Outcome::Stopped { .. }), "stop proof lost its native custody snapshot");
                        return Ok(outcome);
                    }
                    NativeProof::Stop { binding, retired } => {
                        anyhow::ensure!(matches!(recovery.command.operation, Operation::Stop), "native stop proof names another operation");
                        if !retired {
                            anyhow::ensure!(!binding.terminal, "terminal retirement proof is unavailable");
                            self.await_exec_retirement(&binding).await?;
                        }
                        return self.finish_stop(&recovery.command, binding);
                    }
                }
            }
            match recovery.command.operation {
                Operation::Start => Err(FractalError::new("light-node-start-not-supported", "remote start is not enabled").into()),
                Operation::Observe => Ok(Outcome::Cancelled { reason: "observation interrupted before its durable result".into() }),
                Operation::Stop => {
                    let live = &recovery.command.binding;
                    if reconciler.store.local_session_detached(&live.subject, &live.incarnation)? {
                        return Ok(Outcome::Detached { incarnation: live.incarnation.clone() });
                    }
                    let binding = reconciler.store.local_session_at(&live.subject, &live.incarnation)?.context("stop custody binding unavailable")?;
                    anyhow::ensure!(binding.runtime_id == live.runtime && binding.custody == SessionCustody::Owned, "stop recovery custody differs");
                    if binding.terminal {
                        let mut matching = reconciler.runtime.snapshot_ptys()?.into_iter().filter(|entry| entry.runtime_id == binding.runtime_id);
                        let observed = matching.next().context("stop custody lacks positive native closure evidence")?;
                        anyhow::ensure!(matching.next().is_none() && observed.status == "exited"
                            && observed.incarnation_id.as_deref() == Some(binding.native_incarnation.as_str()), "stop custody lacks exact native closure evidence");
                        anyhow::ensure!(reconciler.runtime.stop_with_retirement_proof(&binding.runtime_id, true, &binding.native_incarnation)?, "native terminal scope retirement is unproven");
                    } else { self.await_exec_retirement(&binding).await?; }
                    self.finish_stop(&recovery.command, binding)
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "home_session_tests.rs"]
mod tests;
