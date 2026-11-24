//! Ordinary host control withdraws retained native Start custody, not a fullnode launch.
use super::*;

impl<R: RuntimeControl> Reconciler<R> {
    pub(in crate::reconcile) fn stop_remote_start(&self, operation: &RuntimeStartOperation) -> Result<()> {
        let authorization: fractal_peer::light_node::journal::StartAuthorization =
            serde_json::from_value(operation.body["remote_start"].clone())?;
        anyhow::ensure!(authorization.member.host == self.host
            && authorization.launch_member.host == self.host,
            "ordinary stop names another native Start host");
        let journal = self.home_session_journal.get().context("native Start stop requires its retained session journal")?;
        // The journal commits withdrawal before the Store callback, even if the
        // callback fails or this host exits. No home connection is needed.
        journal.cancel_or_revoke(&operation.id, "ordinary host stop withdrew native Start".into(),
            |command, _, reason| {
                let Operation::Start { authorization: retained } = &command.operation else {
                    anyhow::bail!("ordinary stop does not name retained native Start custody");
                };
                anyhow::ensure!(command.binding.subject == operation.subject
                    && command.binding.runtime == authorization.member.runtime_id
                    && serde_json::to_value(retained)? == operation.body["remote_start"],
                    "ordinary stop differs from retained native Start custody");
                self.store.revoke_remote_runtime_start(&operation.id, reason)
            })?;
        // Never resolve logical custody from a graph observation or infer closure
        // from absence. Unknown backend entry remains retained for native recovery.
        let Some(binding) = self.store.local_session(&operation.subject)? else { return Ok(()); };
        anyhow::ensure!(binding.start_operation.as_deref() == Some(operation.id.as_str())
            && binding.runtime_id == authorization.member.runtime_id
            && binding.terminal == authorization.member.terminal
            && binding.custody == SessionCustody::Owned
            && binding.incarnation == binding.native_incarnation
            && operation.result.as_ref().is_some_and(|result|
                result["verification"] == "operation-incarnation"
                    && result["incarnation"].as_str() == Some(binding.native_incarnation.as_str())),
            "ordinary stop native Start custody changed");
        if !self.store.local_session_retirement_verified(&binding)? {
            self.host_observations.invalidate();
            let retired = self.runtime.stop_with_retirement_proof(
                &binding.runtime_id, binding.terminal, &binding.native_incarnation)?;
            if retired { self.store.verify_local_session_retirement(&binding)?; }
        }
        // A signal is not completion. Native recovery owns exact exec waits,
        // resource retirement, Store finalization and terminal receipt custody.
        // Keeping this Start pending also prevents graph checkout/stop settlement.
        Ok(())
    }
}
