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
    pub fn finish_work_turn_traces(&self, step: &str, incarnation: &str) -> Result<()> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(
            "SELECT json_extract(body,'$.fields.invocation_id'),json_extract(body,'$.fields.error')
             FROM claims WHERE kind='harness.invocation'
             AND json_extract(body,'$.fields.owner_step')=?1
             AND json_extract(body,'$.fields.incarnation')=?2
             AND json_extract(body,'$.fields.phase')='ended'")?;
        for row in statement.query_map(params![step, incarnation],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))? {
            let (invocation, error) = row?;
            self.finish_invocation_trace(&invocation, error.is_some());
        }
        Ok(())
    }

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
    let at = now_ms();
    // An unexpired same-owner claim is a repeated read/claim, not a new native turn.
    if let Some(prior) = latest_claim_of_kind_tx(tx, subject, "work.claimed")? {
        let previous = &prior.body["fields"];
        if previous["claimant"] == agent && previous["claim_incarnation"] == incarnation
            && previous["attempt"] == attempt && previous["readiness_epoch"] == epoch
            && super::step_executions::execution_lease_at(tx, &prior.id, at)?
                .is_some_and(|expiry| expiry > at)
            && let Some(id) = previous["invocation_id"].as_str()
        {
            receipt.invocation_id = id.into();
            receipt.operation_id = id.strip_prefix("invocation/").unwrap_or(id).into();
            let context: Option<String> = tx.query_row(
                "SELECT json_extract(body,'$.fields.trace_context') FROM claims
                 WHERE subject=?1 AND kind='harness.invocation'
                 AND json_extract(body,'$.fields.invocation_id')=?2
                 AND json_type(body,'$.fields.trace_context')='object'
                 ORDER BY store_index LIMIT 1", params![agent, id], |row| row.get(0)).optional()?;
            receipt.trace_context = context.map(|value| serde_json::from_str(&value)).transpose()?;
        }
    }
    receipt.phase = InvocationPhase::Started;
    fields.insert("invocation_id".into(), json!(receipt.invocation_id));
    if let Some(context) = receipt.trace_context.clone()
        .and_then(|context| serde_json::from_value::<crate::otel::AttemptTraceContext>(context).ok())
    {
        let trace_id = context.trace_id();
        if trace_id != "00000000000000000000000000000000" {
            fields.insert("trace_id".into(), json!(trace_id));
        }
    }
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

/// A turn ends at its fenced work receipt, not when the standing provider process exits.
pub fn record_terminal_tx(tx: &Transaction<'_>, origin: &str, claim: &ClaimRecord) -> Result<()> {
    if !matches!(claim.kind.as_str(), "work.submitted" | "work.failed" | "work.released") {
        return Ok(());
    }
    let Some(start) = latest_claim_of_kind_tx(tx, &claim.subject, "work.claimed")? else { return Ok(()); };
    let fields = &start.body["fields"];
    // The native work writer has already fenced the actor's exact incarnation.
    // Terminal bodies clear lease fields; submission additionally names its incarnation.
    let incarnation = claim.body["fields"]["claim_incarnation"].as_str()
        .or_else(|| claim.body["fields"]["submitted_incarnation"].as_str());
    if start.actor != claim.actor || fields["attempt"] != claim.body["fields"]["attempt"]
        || incarnation.is_some_and(|incarnation| fields["claim_incarnation"] != incarnation)
    { return Ok(()); }
    let Some(invocation) = fields["invocation_id"].as_str() else { return Ok(()); };
    let receipt: Option<String> = tx.query_row(
        "SELECT json_extract(body,'$.fields') FROM claims WHERE kind='harness.invocation'
         AND subject=?1 AND json_extract(body,'$.fields.invocation_id')=?2
         ORDER BY store_index LIMIT 1", params![start.actor, invocation], |row| row.get(0)).optional()?;
    if let Some(receipt) = receipt {
        let mut receipt: InvocationReceipt = serde_json::from_str(&receipt)?;
        receipt.phase = InvocationPhase::Ended;
        receipt.event_key = Some(claim.id.clone());
        receipt.evidence.push(claim.id.clone());
        if claim.kind == "work.failed" {
            receipt.error = claim.body["fields"]["reason"].as_str().map(str::to_owned);
        }
        append_receipt_tx(tx, origin, &receipt)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewed_effective_lease_keeps_the_turn_when_the_original_claim_expired() {
        for quiet in [false, true] {
            let store = Store::open_memory("node").unwrap();
            let now = now_ms();
            let agent = "agent/node.worker";
            let step = "step-run/r/g/build";
            store.set_write_clock_at(now - 700_000).unwrap();
            store.connection.batched(|tx| -> Result<()> {
                tx.execute("INSERT INTO mission_runs
                    (id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,
                     workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
                    VALUES('r','mission/test','rev','g','rev','r','{}','person/test','{}','once','running','execution',?1,?1)",
                    [(now - 700_000).to_string()])?;
                tx.execute("INSERT INTO run_generations
                    (id,run_id,revision,status,actor,reason,created_at_unix_ms,updated_at_unix_ms)
                    VALUES('g','r','rev','running',?1,'test',?2,?2)",
                    params![agent, (now - 700_000).to_string()])?;
                tx.execute("INSERT INTO step_runs
                    (subject,run_id,generation_id,step_path,definition_hash,status,attempt,goals,
                     lease_owner,lease_incarnation,lease_expires_at_unix_ms,created_at_unix_ms,updated_at_unix_ms)
                    VALUES(?1,'r','g','build','hash','working',1,'[]',?2,'one',?3,?4,?4)",
                    params![step, agent, (now - 100_000).to_string(), (now - 700_000).to_string()])?;
                append_claim_tx(tx, "node", step, "work.claimed", Some(agent), &json!({"fields":{
                    "attempt":1,"readiness_epoch":0,"claimant":agent,"claim_incarnation":"one",
                    "claim_expires_at_unix_ms":now - 100_000,"invocation_id":"invocation/turn-original"
                }}), &[], None)?;
                Ok(())
            }).unwrap().unwrap();
            store.set_write_clock_at(now - 400_000).unwrap();
            store.connection.batched(|tx| -> Result<()> {
                if quiet {
                    tx.execute("INSERT INTO local_work_lease_renewals
                        (subject,attempt,lease_owner,lease_incarnation,lease_expires_at_unix_ms,updated_at_unix_ms)
                        VALUES(?1,1,?2,'one',?3,?4)",
                        params![step, agent, (now + 200_000).to_string(), (now - 400_000).to_string()])?;
                } else {
                    append_claim_tx(tx, "node", step, "work.renewed", Some(agent), &json!({"fields":{
                        "attempt":1,"readiness_epoch":0,"claimant":agent,"claim_incarnation":"one",
                        "claim_expires_at_unix_ms":now + 200_000
                    }}), &[], None)?;
                }
                Ok(())
            }).unwrap().unwrap();
            store.set_write_clock_at(now).unwrap();
            store.connection.batched(|tx| -> Result<()> {
                // The work writer has already updated the operational projection before binding.
                tx.execute("UPDATE step_runs SET lease_expires_at_unix_ms=?2 WHERE subject=?1",
                    params![step, (now + 600_000).to_string()])?;
                let mut body = json!({"fields":{"attempt":1,"readiness_epoch":0,"claimant":agent,
                    "claim_incarnation":"one","claim_expires_at_unix_ms":now + 600_000}});
                let receipt = bind_claim_tx(tx, "node", step, Some(agent), &mut body)?.unwrap();
                assert_eq!(receipt.invocation_id, "invocation/turn-original", "quiet={quiet}");
                assert_eq!(body["fields"]["invocation_id"], "invocation/turn-original");
                Ok(())
            }).unwrap().unwrap();
        }
    }

    #[test]
    fn terminal_work_receipt_ends_only_its_exact_turn() {
        let store = Store::open_memory("node").unwrap();
        store.connection.batched(|tx| -> Result<()> {
            let agent = "agent/node.worker";
            let step = "step-run/r/g/build";
            let mut receipt = receipt_tx(tx, "run", step, 1, 0, agent, "one")?;
            receipt.phase = InvocationPhase::Started;
            append_receipt_tx(tx, "node", &receipt)?;
            append_claim_tx(tx, "node", step, "work.claimed", Some(agent),
                &json!({"fields":{"attempt":1,"claimant":agent,"claim_incarnation":"one",
                    "invocation_id":receipt.invocation_id}}), &[], None)?;
            let wrong = append_claim_tx(tx, "node", step, "work.released", Some(agent),
                &json!({"fields":{"attempt":1,"claim_incarnation":"other","reason":"wrong"}}), &[], None)?;
            record_terminal_tx(tx, "node", &wrong)?;
            let exact = append_claim_tx(tx, "node", step, "work.failed", Some(agent),
                &json!({"fields":{"attempt":1,"claim_incarnation":null,"reason":"stopped"}}), &[], None)?;
            record_terminal_tx(tx, "node", &exact)?;
            record_terminal_tx(tx, "node", &exact)?;
            Ok(())
        }).unwrap().unwrap();
        let receipts = store.agent_invocation_receipts("agent/node.worker").unwrap();
        assert_eq!(receipts.len(), 2);
        let end = receipts.iter().find(|receipt| receipt.phase == InvocationPhase::Ended).unwrap();
        assert_eq!(end.incarnation.as_deref(), Some("one"));
        assert_eq!(end.error.as_deref(), Some("stopped"));
        assert_eq!(end.evidence.len(), 1);
    }

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
