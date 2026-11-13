//! Native work turns are not process launches. Reminders share a readiness/ownership
//! episode; a retry, released lease, or different incarnation gets another identity.
//! No process environment is consulted, and freshness stays unknown across resume.
use super::*;
use super::invocation_receipts::{append_receipt_tx, InvocationPhase, InvocationReceipt};

pub const TAG_PREFIX: &str = "fractal-invocation:";

fn operation_id(run: &str, step: &str, attempt: u32, epoch: u32, agent: &str,
    incarnation: &str, prior_claim: Option<&str>) -> String {
    let identity = json!([run, step, attempt, epoch, agent, incarnation, prior_claim]);
    format!("turn-{}", hex::encode(Sha256::digest(identity.to_string().as_bytes())))
}

fn receipt_tx(tx: &Transaction<'_>, run: &str, step: &str, attempt: u32, epoch: u32,
    agent: &str, incarnation: &str) -> Result<InvocationReceipt> {
    let prior = latest_claim_of_kind_tx(tx, step, "work.claimed")?;
    let operation = operation_id(run, step, attempt, epoch, agent, incarnation,
        prior.as_ref().map(|claim| claim.id.as_str()));
    // Only exact verified launch provenance establishes parentage. Never use the
    // latest launch, desired declaration, or an inherited invocation environment.
    let parent: Option<String> = tx.query_row(
        "SELECT json_extract(body,'$.fields.invocation_id') FROM claims
         WHERE subject=?1 AND kind='harness.invocation'
         AND json_extract(body,'$.fields.incarnation')=?2
         AND json_extract(body,'$.fields.operation_id') NOT LIKE 'turn-%'
         ORDER BY store_index DESC LIMIT 1", params![agent, incarnation], |row| row.get(0)).optional()?;
    let trace: Option<String> = tx.query_row(
        "SELECT json_extract(body,'$.fields.trace_context') FROM claims
         WHERE subject=?1 AND kind='harness.invocation'
         AND json_extract(body,'$.fields.invocation_id')=?2
         AND json_type(body,'$.fields.trace_context')='object'
         ORDER BY store_index LIMIT 1",
        params![agent, format!("invocation/{operation}")], |row| row.get(0)).optional()?;
    Ok(serde_json::from_value(json!({
        "invocation_id":format!("invocation/{operation}"), "operation_id":operation,
        "phase":"observed", "agent":agent, "owner_run":run, "owner_step":step,
        "owner_attempt":attempt, "incarnation":incarnation, "parent_invocation_id":parent,
        "trace_context":trace.map(|trace| serde_json::from_str::<Value>(&trace)).transpose()?,
        "freshness":{"source":"native-work-turn"}
    }))?)
}

impl Store {
    /// Persist before publishing the wake. Event keys bind every reminder message to
    /// the same turn, without treating a reminder as another provider context.
    pub fn record_work_turn_wake(&self, step: &StepRunView, agent: &str,
        incarnation: &str, message: &str) -> Result<String> {
        self.connection.batched(|tx| -> Result<String> {
            let existing: Option<String> = tx.query_row(
                "SELECT json_extract(body,'$.fields.invocation_id') FROM claims
                 WHERE subject=?1 AND kind='harness.invocation'
                 AND json_extract(body,'$.fields.event_key')=?2
                 AND json_extract(body,'$.fields.incarnation')=?3
                 ORDER BY store_index LIMIT 1",
                params![agent, message, incarnation], |row| row.get(0)).optional()?;
            if let Some(existing) = existing { return Ok(existing); }
            let mut receipt = receipt_tx(tx, &step.run, &step.subject, step.attempt,
                step.readiness_epoch, agent, incarnation)?;
            receipt.event_key = Some(message.into());
            if receipt.trace_context.is_none() {
                let links = receipt.parent_invocation_id.as_deref()
                    .map(|parent| self.invocation_receipts(parent))
                    .transpose()?.unwrap_or_default().into_iter()
                    .find_map(|receipt| receipt.trace_context
                        .and_then(|context| serde_json::from_value(context).ok()))
                    .into_iter().collect::<Vec<_>>();
                receipt.trace_context = Some(serde_json::to_value(
                    self.start_invocation_trace(&receipt.invocation_id, agent, &links))?);
            }
            append_receipt_tx(tx, self.origin(), &receipt)?;
            Ok(receipt.invocation_id)
        }).map_err(|error| anyhow::anyhow!("{error}"))?
    }
}

/// Called inside the native work mutation fence before its immutable receipt is
/// appended. Caller-chosen invocation labels are removed, not trusted.
pub fn bind_claim_tx(tx: &Transaction<'_>, origin: &str, subject: &str,
    actor: Option<&str>, body: &mut Value) -> Result<Option<InvocationReceipt>> {
    let Some(fields) = body.get_mut("fields").and_then(Value::as_object_mut) else { return Ok(None); };
    fields.remove("invocation_id");
    let (Some(agent), Some(incarnation), Some(attempt), Some(epoch)) = (
        actor.filter(|actor| actor.starts_with("agent/")),
        fields.get("claim_incarnation").and_then(Value::as_str).filter(|s| !s.is_empty()),
        fields.get("attempt").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        fields.get("readiness_epoch").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
    ) else { return Ok(None); };
    if fields.get("claimant").and_then(Value::as_str) != Some(agent) { return Ok(None); }
    let run: Option<String> = tx.query_row("SELECT run_id FROM step_runs WHERE subject=?1",
        [subject], |row| row.get(0)).optional()?;
    let Some(run) = run else { return Ok(None); };
    let mut receipt = receipt_tx(tx, &run, subject, attempt, epoch, agent, incarnation)?;
    // An unexpired same-owner claim is a repeated read/claim, not a new native turn.
    if let Some(prior) = latest_claim_of_kind_tx(tx, subject, "work.claimed")? {
        let previous = &prior.body["fields"];
        if previous["claimant"] == agent && previous["claim_incarnation"] == incarnation
            && previous["attempt"] == attempt && previous["readiness_epoch"] == epoch
            && previous["claim_expires_at_unix_ms"].as_u64().is_some_and(|expiry| u128::from(expiry) > now_ms())
            && let Some(id) = previous["invocation_id"].as_str()
        {
            receipt.invocation_id = id.into();
            receipt.operation_id = id.strip_prefix("invocation/").unwrap_or(id).into();
        }
    }
    receipt.phase = InvocationPhase::Started;
    fields.insert("invocation_id".into(), json!(receipt.invocation_id));
    append_receipt_tx(tx, origin, &receipt)?;
    Ok(Some(receipt))
}

/// The claim's own immutable ID, rather than a mutable current step projection,
/// preserves the historical response association after release/retry/reclaim.
pub fn record_claim_tx(tx: &Transaction<'_>, origin: &str, claim: &ClaimRecord,
    mut receipt: InvocationReceipt) -> Result<()> {
    receipt.phase = InvocationPhase::Observed;
    receipt.event_key = Some(claim.id.clone());
    receipt.evidence.push(claim.id.clone());
    append_receipt_tx(tx, origin, &receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reminders_share_identity_but_ownership_episodes_do_not() {
        let id = operation_id("run", "step", 1, 2, "agent/a", "inc", None);
        assert_eq!(id, operation_id("run", "step", 1, 2, "agent/a", "inc", None));
        for other in [
            operation_id("other", "step", 1, 2, "agent/a", "inc", None),
            operation_id("run", "other", 1, 2, "agent/a", "inc", None),
            operation_id("run", "step", 2, 2, "agent/a", "inc", None),
            operation_id("run", "step", 1, 3, "agent/a", "inc", None),
            operation_id("run", "step", 1, 2, "agent/b", "inc", None),
            operation_id("run", "step", 1, 2, "agent/a", "new-inc", None),
            operation_id("run", "step", 1, 2, "agent/a", "inc", Some("prior-claim")),
        ] { assert_ne!(id, other); }
    }

    #[test]
    fn message_events_keep_history_and_unknown_freshness() {
        let store = Store::open_memory("node").unwrap();
        store.connection.batched(|tx| -> Result<()> {
            let mut first = receipt_tx(tx, "run", "step-run/r/g/build", 1, 0,
                "agent/node.worker", "inc-one")?;
            let mut reminder = first.clone();
            first.event_key = Some("message/first".into());
            reminder.event_key = Some("message/reminder".into());
            append_receipt_tx(tx, "node", &first)?;
            append_receipt_tx(tx, "node", &first)?;
            append_receipt_tx(tx, "node", &reminder)?;
            let other = receipt_tx(tx, "run", "step-run/r/g/build", 2, 0,
                "agent/node.worker", "inc-two")?;
            append_receipt_tx(tx, "node", &other)?;
            assert_ne!(first.invocation_id, other.invocation_id);
            assert_eq!(first.freshness.observed_fresh, None);
            assert_eq!(first.parent_invocation_id, None);
            Ok(())
        }).unwrap().unwrap();
        let receipts = store.agent_invocation_receipts("agent/node.worker").unwrap();
        assert_eq!(receipts.len(), 3);
        assert_eq!(receipts[0].invocation_id, receipts[1].invocation_id);
        assert_eq!(receipts[0].event_key.as_deref(), Some("message/first"));
        assert_eq!(receipts[1].event_key.as_deref(), Some("message/reminder"));
        assert_ne!(receipts[0].invocation_id, receipts[2].invocation_id);
    }

    #[test]
    fn exact_incarnation_parentage_does_not_exclude_parented_launches() {
        let store = Store::open_memory("node").unwrap();
        store.connection.batched(|tx| -> Result<()> {
            let launch: InvocationReceipt = serde_json::from_value(json!({
                "invocation_id":"invocation/launch", "operation_id":"launch",
                "phase":"started", "agent":"agent/node.worker",
                "incarnation":"inc-one", "parent_invocation_id":"invocation/ancestor"
            }))?;
            append_receipt_tx(tx, "node", &launch)?;
            let same = receipt_tx(tx, "run", "step-run/r/g/build", 1, 0,
                "agent/node.worker", "inc-one")?;
            assert_eq!(same.parent_invocation_id.as_deref(), Some("invocation/launch"));
            let different = receipt_tx(tx, "run", "step-run/r/g/build", 1, 0,
                "agent/node.worker", "inc-two")?;
            assert_eq!(different.parent_invocation_id, None);
            let mut turn = same.clone();
            turn.event_key = Some("message/wake".into());
            append_receipt_tx(tx, "node", &turn)?;
            let next = receipt_tx(tx, "run", "step-run/r/g/next", 1, 0,
                "agent/node.worker", "inc-one")?;
            assert_eq!(next.parent_invocation_id.as_deref(), Some("invocation/launch"));
            Ok(())
        }).unwrap().unwrap();
    }

    #[test]
    fn unbound_claim_cannot_supply_its_own_invocation_label() {
        let store = Store::open_memory("node").unwrap();
        store.connection.batched(|tx| -> Result<()> {
            let mut body = json!({"fields":{"invocation_id":"invocation/forged","attempt":1}});
            assert!(bind_claim_tx(tx, "node", "step-run/r/g/build", None, &mut body)?.is_none());
            assert!(body["fields"].get("invocation_id").is_none());
            Ok(())
        }).unwrap().unwrap();
    }
}
