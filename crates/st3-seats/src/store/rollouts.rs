//! The part of `store::rollouts` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::rollout::{Operation, Policy, Selection};
pub use st3_missions::store::rollouts::*;
pub use st3_missions::store::rollouts::{intake_held, message_allowed, retiring_ask_live};

pub trait RolloutsStore {
    /// The seat's current intake hold, if a rollout holds its independent mail.
    fn rollout_intake_hold(&self, subject: &str) -> Result<Option<Operation>, St3Error>;
    /// `rollout_message_allowed` for a message to the seat whose `rollout_intake_hold` is `hold`.
    fn rollout_message_allowed_under(
            &self,
            hold: Option<&Operation>,
            message: &MessageView,
        ) -> Result<bool, St3Error>;
    /// Capture publication and incarnation fences atomically with the durable request.
    #[allow(clippy::too_many_arguments)]
    fn request_rollout(
            &self,
            subject: &str,
            expected_token: &str,
            old: &crate::model::MemberSpec,
            incarnation: &str,
            actor: &str,
            policy: &Policy,
            key: &str,
        ) -> Result<ClaimRecord, St3Error>;
}

impl RolloutsStore for Store {

    fn rollout_intake_hold(&self, subject: &str) -> Result<Option<Operation>, St3Error> {
        intake_hold(&self.readers.get(), subject)
    }

    fn rollout_message_allowed_under(
            &self,
            hold: Option<&Operation>,
            message: &MessageView,
        ) -> Result<bool, St3Error> {
        message_allowed_under(&self.readers.get(), hold, message)
    }

    #[allow(clippy::too_many_arguments)]
    fn request_rollout(
            &self,
            subject: &str,
            expected_token: &str,
            old: &crate::model::MemberSpec,
            incarnation: &str,
            actor: &str,
            policy: &Policy,
            key: &str,
        ) -> Result<ClaimRecord, St3Error> {
        policy
            .validate()
            .map_err(|e| St3Error::new("invalid-rollout-policy", e.to_string()))?;
        if crate::suspension::current(self, subject)
            .map_err(internal)?
            .is_some_and(|suspension| suspension.holds_seat())
        {
            return Err(St3Error::new(
                "rollout-suspended",
                "the seat is suspended; resume it before requesting a rollout",
            ));
        }
        self.connection.batched(|tx| -> Result<ClaimRecord, St3Error> {
            if let Some(prior) = tx.query_row(
                "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims
                 WHERE subject=?1 AND kind='runtime.action.requested' AND origin=?2 AND json_extract(body,'$.fields.operation')=?3",
                params![subject,self.origin(),key], claim_from_row).optional().map_err(internal)? {
                let fields = &prior.body["fields"]["rollout"];
                if prior.actor.as_deref() != Some(actor) || fields["desired_token"] != expected_token
                    || fields["old_incarnation"] != incarnation || fields["policy"] != json!(policy) {
                    return Err(St3Error::new("idempotency-key-mismatch", "rollout request key already names another request"));
                }
                return Ok(prior);
            }
            let selected = selection(tx, subject)?.ok_or_else(|| St3Error::new("rollout-not-enabled", "publish an owned set with --rollout when-idle or a rollout manual seat first"))?;
            if selected.desired_token != expected_token {
                return Err(St3Error::new("stale-rollout-target", "selected declaration changed; read it again"));
            }
            if let Some(member) = selected.desired.member.as_ref() {
                crate::native_resume::rollout_support(member)
                    .map_err(|refusal| St3Error::new(refusal.code, refusal.reason))?;
            }
            let actual = latest_actual(tx, subject).map_err(internal)?.unwrap_or(Value::Null);
            let prior = operation(tx,subject)?;
            let ended = matches!(actual["status"].as_str(),Some("stopped"|"exited"|"vanished"));
            let carry = prior.as_ref().filter(|o| o.native_session_id.is_some()
                && (o.old_incarnation == incarnation || o.replacement_incarnation.as_deref() == Some(incarnation)));
            // A pending manual publication can outlive its incumbent. Capture its exact
            // native binding in this transaction; an explicit request still requires positive
            // runtime exit before replacement and never falls back to a fresh conversation.
            let ended_binding = if selected.manual && ended && carry.is_none() {
                tx.query_row(/* sqlite-large-value: allow-for-now lv-hc5b7020e20a0 owner=st3_seats::store::rollouts TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT body FROM claims WHERE subject=?1 AND kind='harness.session-file'
                    AND json_extract(body,'$.fields.incarnation_id')=?2 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
                    params![subject,incarnation], |row| row.get::<_,String>(0)).optional().map_err(internal)?
                    .map(|body| serde_json::from_str::<Value>(&body)).transpose().map_err(internal)?
            } else { None };
            let ended_binding = ended_binding.as_ref().map(|body| &body["fields"])
                .filter(|fields| fields["harness"].as_str() == old.driver.as_deref());
            let ended_session = ended_binding.and_then(|fields| fields["session_id"].as_str())
                .filter(|session| !session.trim().is_empty());
            if actual["incarnation_id"].as_str() != Some(incarnation) || (actual["status"] != "running" && !(ended && (carry.is_some() || ended_session.is_some()))) {
                return Err(St3Error::new("stale-rollout-incarnation", "rollout needs the exact running incarnation"));
            }
            if old.host != self.origin() || selected.desired.member.as_ref().is_some_and(|new| new.host != old.host || new.driver != old.driver)
                || !matches!(old.driver.as_deref(), Some("claude" | "codex" | "pi" | "omp" | "opencode")) {
                return Err(St3Error::new("unsupported-rollout", "rollout requires a supported native harness on its current host"));
            }
            let now = now_ms();
            let mut asks = tx.prepare_cached("SELECT step.subject,step.attempt FROM step_runs step
                JOIN claims ask ON json_extract(ask.body,'$.fields.origin_step')=step.subject
                WHERE step.status IN ('waiting-person','ready') AND ask.kind='work.person-asked' AND ask.actor=?1
                    AND json_extract(ask.body,'$.fields.origin_attempt')=step.attempt").map_err(internal)?;
            let allowed_work = asks.query_map([subject],|row|Ok((row.get::<_,String>(0)?,row.get::<_,u32>(1)?))).map_err(internal)?
                .collect::<Result<BTreeMap<_,_>,_>>().map_err(internal)?;
            let operation = Operation { id: String::new(), set: selected.set, receipt: selected.receipt,
                source: selected.source, desired_token: selected.desired_token, target: selected.target,
                policy: policy.clone(), publication_policy: selected.policy, publication_manual: selected.manual, old_incarnation: incarnation.into(),
                old_member: old.clone(), deadline_unix_ms: now.saturating_add(policy.deadline_ms.into()),
                requested_by: Some(actor.into()), requested_at_unix_ms: now, phase: if ended { "stopping" } else { "draining" }.into(),
                phase_at_unix_ms: now, drain_ack: None, start_attempted: false, allowed_work, native_session_id: carry.and_then(|o|o.native_session_id.clone()).or_else(|| ended_session.map(str::to_owned)),
                native_path: carry.and_then(|o|o.native_path.clone()).or_else(|| ended_binding.and_then(|f|f["path"].as_str()).map(str::to_owned)),
                native_account: carry.and_then(|o|o.native_account.clone()).or_else(|| ended_binding.and_then(|f|f["account_ref"].as_str()).map(str::to_owned)), replacement_incarnation: None,
                forced: false, blocking: Vec::new(), reason: None };
            let claim = append_claim_tx(tx, self.origin(), subject, "runtime.action.requested", Some(actor),
                &json!({"fields":{"action":"rollout","operation":key,"rollout":operation},"evidence":[expected_token]}),
                &[], None).map_err(internal)?;
            Ok(claim)
        }).map_err(|e| St3Error::new("internal", e))?
    }
}

