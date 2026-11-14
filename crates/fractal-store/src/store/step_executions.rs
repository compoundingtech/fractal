//! Durable invocation identity and response attribution, independent of local timeline retention.
use super::*;
use fractal_kernel::{StepExecution, StepExecutionUsage};

/// Accumulate only responses already admitted by the generic response deduplication/pricing path.
pub(super) fn accumulate_response(
    connection: &Connection, execution: &str, model: &str, account: &str, host: &str,
    buckets: [u64; 9], at: u128,
) -> Result<()> {
    connection.execute(
        "INSERT INTO local_step_execution_spend(execution_id,model,account,host,input_tokens,output_tokens,
         cache_write_tokens,cache_write_1h_tokens,cached_tokens,total_tokens,cost_microusd,
         reported_cost_microusd,unpriced_tokens,observed_at_unix_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(execution_id,model,account,host) DO UPDATE SET
         input_tokens=input_tokens+excluded.input_tokens,output_tokens=output_tokens+excluded.output_tokens,
         cache_write_tokens=cache_write_tokens+excluded.cache_write_tokens,
         cache_write_1h_tokens=cache_write_1h_tokens+excluded.cache_write_1h_tokens,
         cached_tokens=cached_tokens+excluded.cached_tokens,total_tokens=total_tokens+excluded.total_tokens,
         cost_microusd=cost_microusd+excluded.cost_microusd,
         reported_cost_microusd=reported_cost_microusd+excluded.reported_cost_microusd,
         unpriced_tokens=unpriced_tokens+excluded.unpriced_tokens,observed_at_unix_ms=excluded.observed_at_unix_ms",
        params![execution,model,account,host,buckets[0],buckets[1],buckets[2],buckets[3],buckets[4],
            buckets[5],buckets[6],buckets[7],buckets[8],at.min(i64::MAX as u128) as i64],
    )?;
    Ok(())
}
/// Capture declaration and actual propagated tracing context at the work-claim boundary.
/// This is deliberately not run while receiving/replaying historical claims.
pub(super) fn snapshot_claim(connection: &Connection, actor: Option<&str>, body: &mut Value) -> Result<()> {
    if let Some(actor) = actor {
        let declared: Option<String> = connection.query_row(
            // Canonical agent bodies carry `harness { model \"...\" }` children, not a
            // flattened model property. An undeclared harness default remains unknown.
            "SELECT json_extract(m.value,'$.arguments[0]')
             FROM desired d, json_each(d.body,'$.children') h, json_each(h.value,'$.children') m
             WHERE d.subject=?1 AND json_extract(h.value,'$.name')='harness'
               AND json_extract(m.value,'$.name')='model'
               AND json_type(m.value,'$.arguments[0]')='text' LIMIT 1",
            [actor], |row| row.get(0),
        ).optional()?;
        if let Some(model) = declared {
            body["fields"]["declared_model"] = json!(model);
        }
    }
    use opentelemetry::trace::TraceContextExt as _;
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;
    let context = tracing::Span::current().context();
    let span = context.span();
    if span.span_context().is_valid() {
        body["fields"]["trace_id"] = json!(span.span_context().trace_id().to_string());
    }
    Ok(())
}

/// Resolve every historical ownership candidate, never the latest agent-wide claim.
/// Concurrent live claims on different steps are ambiguous and remain unknown.
pub(super) fn response_execution(connection: &Connection, agent: &str, incarnation: &str, at: u128)
    -> Result<Option<(String, String)>>
{
    if incarnation.is_empty() { return Ok(None); }
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h3a0be1d87c44 owner=fractal_store::store::step_executions TODO=trace-statement-and-value-representation */ &format!(
        "SELECT claimed.id, claimed.subject FROM claims claimed INDEXED BY claims_execution_actor_index
         WHERE claimed.actor=?1 AND claimed.kind='work.claimed'
           AND json_extract(claimed.body,'$.fields.claimant')=?1
           AND json_extract(claimed.body,'$.fields.claim_incarnation')=?2
           AND json_type(claimed.body,'$.fields.attempt')='integer'
           AND CAST(claimed.accepted_at_unix_ms AS INTEGER)<=?3
           AND NOT EXISTS(SELECT 1 FROM claims later WHERE later.subject=claimed.subject
             AND later.kind IN ('work.claimed','work.submitted','work.failed','work.released','step-run.retried','step-run.state')
             AND CAST(later.accepted_at_unix_ms AS INTEGER)<=?3 AND {}
             AND (later.kind<>'step-run.state' OR json_extract(later.body,'$.fields.status') IN ('completed','failed','cancelled','ready'))
             AND (later.kind='work.claimed' OR COALESCE(json_extract(later.body,'$.fields.attempt'),
                 json_extract(claimed.body,'$.fields.attempt'))>=json_extract(claimed.body,'$.fields.attempt')))
           AND NOT EXISTS(SELECT 1 FROM claims exited WHERE exited.subject=?1 AND exited.kind='runtime.observed'
             AND json_extract(exited.body,'$.fields.incarnation_id')=?2
             AND json_type(exited.body,'$.fields.exit_evidence')='object'
             AND CAST(exited.accepted_at_unix_ms AS INTEGER)<=?3
             AND CAST(exited.accepted_at_unix_ms AS INTEGER)>=CAST(claimed.accepted_at_unix_ms AS INTEGER))
         ", smallclaims::store::canonical::after_sql("later", "claimed")))?;
    let mut rows = statement.query(params![agent, incarnation, at.min(i64::MAX as u128) as i64])?;
    let mut candidate = None;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        if !execution_lease_at(connection, &id, at)?.is_some_and(|expiry| expiry > at) { continue; }
        if candidate.is_some() { return Ok(None); }
        candidate = Some((id, row.get(1)?));
    }
    Ok(candidate)
}

/// Lease evidence for one exact ownership episode, as known at response time.
/// Missing expiry is unknown. Never borrow a newer attempt/incarnation's lease,
/// a future renewal, or the already-mutated step projection.
pub(super) fn execution_lease_at(connection: &Connection, claim: &str, at: u128)
    -> Result<Option<u128>> {
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hb150610e49d0 owner=fractal_store::store::step_executions TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
        "SELECT claims.body,claims.accepted_at_unix_ms FROM claims
         JOIN claims claimed ON claimed.id=?1
         WHERE claims.subject=claimed.subject
           AND claims.kind IN ('work.claimed','work.renewed','work.progress','work.extended')
           AND json_extract(claims.body,'$.fields.attempt')=json_extract(claimed.body,'$.fields.attempt')
           AND json_extract(claims.body,'$.fields.claimant')=json_extract(claimed.body,'$.fields.claimant')
           AND json_extract(claims.body,'$.fields.claim_incarnation')=json_extract(claimed.body,'$.fields.claim_incarnation')
           AND CAST(claims.accepted_at_unix_ms AS INTEGER)<=?2
           AND (claims.id=claimed.id OR {})
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
        smallclaims::store::canonical::after_sql("claims", "claimed"))))?;
    let reading: Option<(String, String)> = statement.query_row(
        /* sqlite-large-value: allow-for-now lv-h988f5ad3eefe owner=fractal_store::store::step_executions TODO=trace-statement-and-value-representation */
        params![claim, at.min(i64::MAX as u128) as i64], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let Some((body, effective_at)) = reading else { return Ok(None); };
    let body: Value = serde_json::from_str(&body)?;
    let effective_at = effective_at.parse::<u128>()?;
    let mut expiry = body["fields"]["claim_expires_at_unix_ms"].as_u64().map(u128::from);
    let quiet: Option<(String, String)> = connection.query_row(
        "SELECT renewal.lease_expires_at_unix_ms,renewal.updated_at_unix_ms
         FROM local_work_lease_renewals renewal JOIN claims claimed ON claimed.id=?1
         WHERE renewal.subject=claimed.subject
           AND renewal.attempt=json_extract(claimed.body,'$.fields.attempt')
           AND renewal.lease_owner=json_extract(claimed.body,'$.fields.claimant')
           AND renewal.lease_incarnation=json_extract(claimed.body,'$.fields.claim_incarnation')
           AND CAST(renewal.updated_at_unix_ms AS INTEGER)>=CAST(claimed.accepted_at_unix_ms AS INTEGER)
           AND CAST(renewal.updated_at_unix_ms AS INTEGER)<=?2",
        params![claim, at.min(i64::MAX as u128) as i64], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    if let Some((quiet_expiry, updated)) = quiet
        && updated.parse::<u128>()? >= effective_at {
        expiry = Some(quiet_expiry.parse()?);
    }
    Ok(expiry)
}

/// Join only immutable work-claim receipts or a unique verified launch incarnation.
/// A caller's invocation label and the current declaration are never authorities.
pub(super) fn response_invocation(connection: &Connection, agent: &str, incarnation: &str,
    execution: Option<&str>) -> Result<Option<fractal_schema::invocation::InvocationReceipt>> {
    if incarnation.is_empty() { return Ok(None); }
    let mut statement = connection.prepare(
        "SELECT body FROM claims WHERE subject=?1 AND kind='harness.invocation'
         AND json_extract(body,'$.fields.incarnation')=?2
         AND ((?3 IS NOT NULL AND json_extract(body,'$.fields.event_key')=?3)
           OR (?3 IS NULL AND json_extract(body,'$.fields.operation_id') NOT LIKE 'turn-%'))")?;
    let rows = statement.query_map(params![agent, incarnation, execution], |row| row.get::<_, String>(0))?;
    let mut matched: Option<fractal_schema::invocation::InvocationReceipt> = None;
    for row in rows {
        let body: Value = serde_json::from_str(&row?)?;
        let receipt: fractal_schema::invocation::InvocationReceipt = serde_json::from_value(body["fields"].clone())?;
        if matched.as_ref().is_some_and(|prior| prior.invocation_id != receipt.invocation_id
            || prior.owner_attempt != receipt.owner_attempt || prior.owner_step != receipt.owner_step
            || prior.owner_run != receipt.owner_run) { return Ok(None); }
        matched = Some(receipt);
    }
    Ok(matched)
}

impl Store {
    /// Every durable agent invocation of this step, including failed/no-usage invocations.
    /// Usage comes from replicated response rollups plus this host's held latest readings;
    /// neither trimmed timeline entries nor the seat's current declaration are consulted.
    pub fn step_executions(&self, step_subject: &str) -> Result<Vec<StepExecution>> {
        smallclaims::touched::note_read(|| step_subject.to_owned());
        let connection = self.readers.get();
        let mut statement = connection.prepare(&canonical_sql(
            "SELECT id, kind, actor, body, accepted_at_unix_ms FROM claims
             WHERE subject=?1 AND (kind LIKE 'work.%' OR kind='step-run.state') ORDER BY CANONICAL_ASC(claims)",
        ))?;
        let claims = statement.query_map([step_subject], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?, row.get::<_, String>(4)?,
        )))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut executions: Vec<StepExecution> = Vec::new();
        for (id, kind, actor, body, at) in claims {
            let body: Value = serde_json::from_str(&body)?;
            let fields = &body["fields"];
            let at = at.parse::<u128>()?;
            let attempt = fields["attempt"].as_u64().map(|n| n as u32);
            if kind == "work.claimed" {
                let agent = fields["claimant"].as_str().or(actor.as_deref());
                let Some(agent) = agent.filter(|agent| agent.starts_with("agent/")) else { continue; };
                // A second claim is another invocation even if attempt, agent and incarnation agree.
                for execution in &mut executions {
                    if execution.ended_at_unix_ms.is_none() {
                        finish(execution, at, "replaced");
                    }
                }
                executions.push(StepExecution {
                    id: id.clone(), step_subject: step_subject.to_owned(), attempt: attempt.unwrap_or(0),
                    agent: agent.to_owned(),
                    incarnation_id: fields["claim_incarnation"].as_str().map(str::to_owned),
                    invocation_id: response_invocation(&connection, agent,
                        fields["claim_incarnation"].as_str().unwrap_or(""), Some(&id))?
                        .map(|receipt| receipt.invocation_id),
                    declared_model: fields["declared_model"].as_str().map(str::to_owned),
                    trace_id: fields["trace_id"].as_str().map(str::to_owned),
                    fallback: fields["fallback"].as_bool(),
                    started_at_unix_ms: at, status: Some("claimed".into()),
                    ..StepExecution::default()
                });
            } else if let Some(execution) = executions.iter_mut().rev().find(|execution|
                execution.ended_at_unix_ms.is_none() && attempt.is_none_or(|attempt| attempt == execution.attempt))
            {
                if matches!(kind.as_str(), "work.submitted" | "work.failed" | "work.released") {
                    finish(execution, at, kind.strip_prefix("work.").unwrap_or(&kind));
                } else if kind == "step-run.state"
                    && let Some(status @ ("completed" | "failed" | "cancelled" | "ready")) = fields["status"].as_str() {
                    finish(execution, at, status);
                }
            }
        }
        for execution in &mut executions {
            let mut statement = connection.prepare(&canonical_sql(
                "SELECT body FROM claims WHERE subject=?1 AND kind='harness.usage'
                   AND json_extract(body,'$.fields.execution_id')=?2 ORDER BY CANONICAL_ASC(claims)",
            ))?;
            let mut readings = statement.query_map(params![execution.agent, execution.id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut pending = connection.prepare(
                "SELECT o.body FROM local_latest_slots s JOIN local_observations o ON o.id=s.pending_local_id
                 WHERE s.subject=?1 AND s.kind='harness.usage'
                   AND json_extract(o.body,'$.fields.execution_id')=?2 ORDER BY o.id",
            )?;
            readings.extend(pending.query_map(params![execution.agent, execution.id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?);
            let mut series = BTreeMap::new();
            for reading in readings {
                let body: Value = serde_json::from_str(&reading)?;
                let fields = &body["fields"];
                if fields["semantics"] != "response_rollup" { continue; }
                if execution.incarnation_id.as_deref()
                    .is_some_and(|incarnation| fields["incarnation_id"].as_str() != Some(incarnation))
                { continue; }
                let key = ["model", "account", "host"].map(|key| fields[key].as_str().unwrap_or("").to_owned());
                series.insert(key, fields.clone());
            }
            if !series.is_empty() {
                let mut usage = StepExecutionUsage::default();
                for (key, fields) in series {
                    if !key[0].is_empty() && key[0] != "unknown" { execution.observed_models.push(key[0].clone()); }
                    macro_rules! add { ($($name:ident),*) => { $(usage.$name = usage.$name.saturating_add(fields[stringify!($name)].as_u64().unwrap_or(0));)* }; }
                    add!(input_tokens, output_tokens, cache_write_tokens, cache_write_1h_tokens, cached_tokens,
                        total_tokens, cost_microusd, reported_cost_microusd, unpriced_tokens, unpriced_responses);
                }
                execution.observed_models.sort();
                execution.observed_models.dedup();
                execution.usage = Some(usage);
            }
            if let Some(incarnation) = &execution.incarnation_id {
                let runtime: Option<(String, String, String)> = connection.query_row(
                    /* sqlite-large-value: allow-for-now lv-h32350d549449 owner=fractal_store::store::step_executions TODO=trace-statement-and-value-representation */
                    &canonical_sql("SELECT id, body, accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='runtime.observed'
                     AND json_extract(body,'$.fields.incarnation_id')=?2
                     AND CAST(accepted_at_unix_ms AS INTEGER)>=?3
                     AND json_type(body,'$.fields.exit_evidence')='object'
                     ORDER BY CANONICAL_ASC(claims) LIMIT 1"),
                    params![execution.agent, incarnation, execution.started_at_unix_ms.min(i64::MAX as u128) as i64],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ).optional()?;
                if let Some((id, body, at)) = runtime {
                    execution.exit_evidence = Some(id);
                    let body: Value = serde_json::from_str(&body)?;
                    if execution.trace_id.is_none() { execution.trace_id = body["fields"]["trace_id"].as_str().map(str::to_owned); }
                    let exit_at = at.parse()?;
                    if execution.ended_at_unix_ms.is_none_or(|ended| exit_at < ended) {
                        finish(execution, exit_at, "exited");
                    }
                }
            }
        }
        Ok(executions)
    }
}

fn finish(execution: &mut StepExecution, at: u128, status: &str) {
    execution.ended_at_unix_ms = Some(at);
    execution.duration_ms = Some(at.saturating_sub(execution.started_at_unix_ms));
    execution.status = Some(status.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: &str = "step-run/example/generation/build";
    const AGENT: &str = "agent/example.worker";

    fn event(store: &Store, subject: &str, kind: &str, at: u128, fields: Value) -> ClaimRecord {
        store.set_write_clock_at(at).unwrap();
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let claim = append_claim_tx(&tx, store.origin(), subject, kind, Some(AGENT),
            &json!({"fields": fields, "evidence": []}), &[], None).unwrap();
        tx.commit().unwrap();
        claim
    }

    fn claim(store: &Store, attempt: u32, incarnation: &str, at: u128) -> ClaimRecord {
        event(store, STEP, "work.claimed", at, json!({
            "attempt": attempt, "claimant": AGENT, "claim_incarnation": incarnation,
            "claim_expires_at_unix_ms": at + 600_000,
            "declared_model": "declared-at-claim", "trace_id": "0123456789abcdef0123456789abcdef",
        }))
    }

    #[test]
    fn response_ownership_ends_at_expiry_before_the_ready_receipt() {
        let store = Store::open_memory("source").unwrap();
        let first = event(&store, STEP, "work.claimed", 0, json!({
            "attempt":1,"claimant":AGENT,"claim_incarnation":"one","claim_expires_at_unix_ms":600,
        }));
        event(&store, STEP, "step-run.state", 700, json!({"attempt":1,"status":"ready"}));
        let connection = store.readers.get();
        assert_eq!(response_execution(&connection, AGENT, "one", 599).unwrap(),
            Some((first.id.clone(), STEP.into())), "late ingestion retains genuine earlier responses");
        assert_eq!(response_execution(&connection, AGENT, "one", 600).unwrap(), None);
        assert_eq!(response_execution(&connection, AGENT, "one", 650).unwrap(), None);
    }

    #[test]
    fn historical_lease_renewals_and_extensions_are_episode_and_time_fenced() {
        let store = Store::open_memory("source").unwrap();
        let first = event(&store, STEP, "work.claimed", 0, json!({
            "attempt":1,"claimant":AGENT,"claim_incarnation":"one","claim_expires_at_unix_ms":600,
        }));
        event(&store, STEP, "work.renewed", 300, json!({
            "attempt":1,"claimant":AGENT,"claim_incarnation":"one","claim_expires_at_unix_ms":900,
        }));
        event(&store, STEP, "work.extended", 800, json!({
            "attempt":1,"claimant":AGENT,"claim_incarnation":"one","claim_expires_at_unix_ms":1200,
        }));
        {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            tx.execute("INSERT INTO local_work_lease_renewals
                (subject,attempt,lease_owner,lease_incarnation,lease_expires_at_unix_ms,updated_at_unix_ms)
                VALUES(?1,1,?2,'one','1500','1100')", params![STEP, AGENT]).unwrap();
            tx.commit().unwrap();
        }
        let connection = store.readers.get();
        assert_eq!(execution_lease_at(&connection, &first.id, 200).unwrap(), Some(600));
        assert_eq!(execution_lease_at(&connection, &first.id, 700).unwrap(), Some(900));
        assert_eq!(execution_lease_at(&connection, &first.id, 1000).unwrap(), Some(1200));
        assert_eq!(execution_lease_at(&connection, &first.id, 1300).unwrap(), Some(1500));
        for at in [700, 1000, 1300] {
            assert_eq!(response_execution(&connection, AGENT, "one", at).unwrap(),
                Some((first.id.clone(), STEP.into())));
        }
        assert_eq!(response_execution(&connection, AGENT, "one", 1500).unwrap(), None);
        drop(connection);
        store.connection.write().execute("UPDATE local_work_lease_renewals SET attempt=2", []).unwrap();
        assert_eq!(response_execution(&store.readers.get(), AGENT, "one", 1300).unwrap(), None,
            "a different attempt's quiet renewal cannot extend the earlier claim");
    }

    #[test]
    fn a_claim_without_lease_evidence_does_not_prove_response_ownership() {
        let store = Store::open_memory("source").unwrap();
        event(&store, STEP, "work.claimed", 100,
            json!({"attempt":1,"claimant":AGENT,"claim_incarnation":"one"}));
        assert_eq!(response_execution(&store.readers.get(), AGENT, "one", 150).unwrap(), None);
    }

    #[test]
    fn overlapping_step_claims_are_unknown_not_latest_wins() {
        let store = Store::open_memory("source").unwrap();
        let first = claim(&store, 1, "one", 100);
        event(&store, "step-run/example/generation/review", "work.claimed", 110,
            json!({"attempt":1,"claimant":AGENT,"claim_incarnation":"one","claim_expires_at_unix_ms":600110}));
        let connection = store.readers.get();
        assert_eq!(response_execution(&connection, AGENT, "one", 105).unwrap(),
            Some((first.id.clone(), STEP.into())));
        assert_eq!(response_execution(&connection, AGENT, "one", 120).unwrap(), None);
        drop(connection);
        event(&store, "step-run/example/generation/review", "work.released", 130, json!({"attempt":1}));
        assert_eq!(response_execution(&store.readers.get(), AGENT, "one", 140).unwrap(),
            Some((first.id, STEP.into())));
    }

    fn rollup(store: &Store, execution: &ClaimRecord, model: &str, total: u64, at: u128) {
        event(store, AGENT, "harness.usage", at, json!({
            "semantics":"response_rollup", "driver":"codex",
            "incarnation_id":execution.body["fields"]["claim_incarnation"],
            "model":model, "execution_id":execution.id, "owner_step":STEP,
            "total_tokens":total, "input_tokens":total, "cost_microusd":total,
        }));
    }

    #[test]
    fn exact_claim_and_message_receipts_join_without_inherited_launch_guesses() {
        let store = Store::open_memory("source").unwrap();
        let first = claim(&store, 1, "one", 100);
        let receipt = |key: &str, operation: &str, attempt: u32| json!({
            "invocation_id":format!("invocation/{operation}"),"operation_id":operation,
            "agent":AGENT,"phase":"observed","incarnation":"one","event_key":key,
            "owner_step":STEP,"owner_attempt":attempt,
        });
        event(&store, AGENT, "harness.invocation", 101, receipt(&first.id, "turn-first", 1));
        event(&store, AGENT, "harness.invocation", 102, receipt("message/wake", "turn-first", 1));
        let connection = store.readers.get();
        let bound = response_invocation(&connection, AGENT, "one", Some(&first.id)).unwrap().unwrap();
        assert_eq!(bound.invocation_id, "invocation/turn-first");
        assert_eq!(bound.owner_attempt, Some(1));
        assert_eq!(response_invocation(&connection, AGENT, "one", Some("message/wake")).unwrap()
            .unwrap().invocation_id, bound.invocation_id);
        assert!(response_invocation(&connection, AGENT, "other", Some(&first.id)).unwrap().is_none());
        assert!(response_invocation(&connection, AGENT, "one", Some("missing")).unwrap().is_none());
        drop(connection);
        assert_eq!(store.step_executions(STEP).unwrap()[0].invocation_id.as_deref(),
            Some("invocation/turn-first"));
        event(&store, AGENT, "harness.invocation", 103, receipt(&first.id, "turn-conflict", 1));
        assert!(response_invocation(&store.readers.get(), AGENT, "one", Some(&first.id)).unwrap().is_none());
    }

    #[test]
    fn launch_join_requires_unique_exact_verified_incarnation() {
        let store = Store::open_memory("source").unwrap();
        for (operation, incarnation) in [("launch", Some("one")), ("unverified", None)] {
            event(&store, AGENT, "harness.invocation", 100, json!({
                "invocation_id":format!("invocation/{operation}"),"operation_id":operation,
                "agent":AGENT,"phase":"observed","incarnation":incarnation,
            }));
        }
        assert_eq!(response_invocation(&store.readers.get(), AGENT, "one", None).unwrap()
            .unwrap().invocation_id, "invocation/launch");
        assert!(response_invocation(&store.readers.get(), AGENT, "", None).unwrap().is_none());
        assert!(response_invocation(&store.readers.get(), AGENT, "other", None).unwrap().is_none());
        event(&store, AGENT, "harness.invocation", 110, json!({
            "invocation_id":"invocation/conflict","operation_id":"conflict",
            "agent":AGENT,"phase":"observed","incarnation":"one",
        }));
        assert!(response_invocation(&store.readers.get(), AGENT, "one", None).unwrap().is_none());
    }
    #[test]
    fn retries_and_reclaims_keep_distinct_invocations_even_without_usage() {
        let store = Store::open_memory("source").unwrap();
        let first = claim(&store, 1, "one", 100);
        event(&store, STEP, "work.released", 150, json!({"attempt":1}));
        let second = claim(&store, 1, "one", 200);
        event(&store, STEP, "work.failed", 250, json!({"attempt":1}));
        let third = claim(&store, 2, "two", 300);
        let executions = store.step_executions(STEP).unwrap();
        assert_eq!(executions.iter().map(|execution| execution.id.as_str()).collect::<Vec<_>>(),
            vec![first.id.as_str(), second.id.as_str(), third.id.as_str()]);
        assert_eq!(executions[0].duration_ms, Some(50));
        assert_eq!(executions[1].status.as_deref(), Some("failed"));
        assert_eq!(executions[2].duration_ms, None);
        assert!(executions.iter().all(|execution| execution.usage.is_none()));
        assert_eq!(executions[2].attempt, 2);
        let connection = store.readers.get();
        assert_eq!(response_execution(&connection, AGENT, "one", 140).unwrap(),
            Some((first.id, STEP.into())));
        assert_eq!(response_execution(&connection, AGENT, "one", 160).unwrap(), None);
        assert_eq!(response_execution(&connection, AGENT, "one", 210).unwrap(),
            Some((second.id, STEP.into())));
    }

    #[test]
    fn historical_response_time_and_incarnation_fence_delayed_observations() {
        let store = Store::open_memory("source").unwrap();
        let first = claim(&store, 1, "one", 100);
        event(&store, STEP, "work.failed", 200, json!({"attempt":1}));
        let second = claim(&store, 2, "two", 300);
        let connection = store.readers.get();
        assert_eq!(response_execution(&connection, AGENT, "one", 150).unwrap(),
            Some((first.id, STEP.into())));
        assert_eq!(response_execution(&connection, AGENT, "one", 250).unwrap(), None);
        assert_eq!(response_execution(&connection, AGENT, "two", 250).unwrap(), None);
        assert_eq!(response_execution(&connection, AGENT, "two", 350).unwrap(),
            Some((second.id, STEP.into())));
        assert_eq!(response_execution(&connection, AGENT, "", 350).unwrap(), None);
    }

    #[test]
    fn durable_rollups_preserve_plural_models_unknown_fallback_and_retry_totals() {
        let store = Store::open_memory("source").unwrap();
        let first = claim(&store, 1, "one", 100);
        rollup(&store, &first, "observed-a", 10, 110);
        rollup(&store, &first, "observed-a", 25, 120);
        rollup(&store, &first, "observed-b", 5, 130);
        event(&store, STEP, "work.failed", 150, json!({"attempt":1}));
        let second = claim(&store, 2, "two", 200);
        rollup(&store, &second, "observed-a", 7, 210);
        let executions = store.step_executions(STEP).unwrap();
        assert_eq!(executions[0].usage.as_ref().unwrap().total_tokens, 30);
        assert_eq!(executions[1].usage.as_ref().unwrap().total_tokens, 7);
        assert_eq!(executions[0].observed_models, ["observed-a", "observed-b"]);
        assert_eq!(executions[0].fallback, None, "model mismatch is not a fallback cause");
        assert_eq!(executions[0].declared_model.as_deref(), Some("declared-at-claim"));
        assert_eq!(executions[0].trace_id.as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(executions[0].exit_evidence, None);
        // This store has no timeline or desired row: history is entirely claim-backed.
        assert_eq!(store.readers.get().query_row(
            "SELECT COUNT(*) FROM local_observations WHERE kind='harness.timeline'", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
    }

    #[test]
    fn out_of_order_receipts_do_not_regress_spend_or_end_a_later_reclaim() {
        use crate::model::ReplicationInventory;
        use crate::store::tests_shared::{exchange_from, receive_and_project};
        let store = Store::open_memory("source").unwrap();
        let delayed = Store::open_memory("delayed").unwrap();
        let first = claim(&store, 1, "one", 100);
        let second = claim(&store, 1, "one", 200);
        // A writer's accepted clock is monotonic. Produce older receipts on another
        // writer, then replicate them after the newer local receipts.
        event(&delayed, STEP, "work.released", 150, json!({"attempt":1}));
        rollup(&delayed, &second, "observed", 10, 220);
        rollup(&store, &second, "observed", 25, 230);
        receive_and_project(&store, delayed.origin(),
            &exchange_from(&delayed, &ReplicationInventory::default()));
        let connection = store.readers.get();
        assert_eq!(response_execution(&connection, AGENT, "one", 210).unwrap(),
            Some((second.id.clone(), STEP.into())));
        drop(connection);
        let executions = store.step_executions(STEP).unwrap();
        assert_eq!(executions[0].id, first.id);
        assert_eq!(executions[1].id, second.id);
        assert_eq!(executions[1].usage.as_ref().unwrap().total_tokens, 25);
        assert_eq!(executions[1].ended_at_unix_ms, None);
    }

    #[test]
    fn legacy_claims_have_unknown_metadata_and_do_not_borrow_current_models() {
        let store = Store::open_memory("source").unwrap();
        event(&store, STEP, "work.claimed", 100,
            json!({"attempt":1,"claimant":AGENT,"claim_incarnation":"old"}));
        let executions = store.step_executions(STEP).unwrap();
        assert_eq!(executions[0].declared_model, None);
        assert_eq!(executions[0].trace_id, None);
        assert_eq!(executions[0].fallback, None);
        assert_eq!(executions[0].usage, None);
    }

    #[test]
    fn claim_snapshot_captures_declared_model_and_only_actual_trace_context() {
        use opentelemetry::trace::TracerProvider as _;
        use tracing_subscriber::prelude::*;
        let store = Store::open_memory("source").unwrap();
        let seed = claim(&store, 1, "one", 100);
        store.connection.write().execute(
            "INSERT INTO desired(subject,kind,revision,claim_id,body) VALUES(?1,'agent','revision',?2,?3)",
            params![AGENT, seed.id, json!({"children":[{"name":"harness","children":[
                {"name":"model","arguments":["historical-declaration"]}
            ]}]}).to_string()],
        ).unwrap();
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("execution-tests")));
        let mut body = json!({"fields":{}});
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("claimed_execution");
            let _entered = span.enter();
            snapshot_claim(&store.readers.get(), Some(AGENT), &mut body).unwrap();
        });
        assert_eq!(body["fields"]["declared_model"], "historical-declaration");
        assert_eq!(body["fields"]["trace_id"].as_str().unwrap().len(), 32);
        assert_ne!(body["fields"]["trace_id"], "00000000000000000000000000000000");
    }

    #[test]
    fn execution_history_survives_replication_without_local_timeline() {
        use crate::model::ReplicationInventory;
        use crate::store::tests_shared::{exchange_from, receive_and_project};
        let source = Store::open_memory("source").unwrap();
        let target = Store::open_memory("target").unwrap();
        let execution = claim(&source, 1, "one", 100);
        rollup(&source, &execution, "observed-a", 42, 110);
        event(&source, STEP, "work.failed", 120, json!({"attempt":1}));
        receive_and_project(&target, source.origin(),
            &exchange_from(&source, &ReplicationInventory::default()));
        assert_eq!(target.step_executions(STEP).unwrap(), source.step_executions(STEP).unwrap());
        assert_eq!(target.step_executions(STEP).unwrap()[0].usage.as_ref().unwrap().total_tokens, 42);
    }

    #[test]
    fn native_exit_ends_unreported_execution_without_inventing_usage() {
        let store = Store::open_memory("source").unwrap();
        claim(&store, 1, "one", 100);
        let exit = event(&store, AGENT, "runtime.observed", 200, json!({
            "status":"failed","incarnation_id":"one","exit_evidence":{"exit_code":1},
        }));
        let executions = store.step_executions(STEP).unwrap();
        assert_eq!(executions[0].exit_evidence.as_deref(), Some(exit.id.as_str()));
        assert_eq!(executions[0].duration_ms, Some(100));
        assert_eq!(executions[0].status.as_deref(), Some("exited"));
        assert_eq!(executions[0].usage, None);
        let connection = store.readers.get();
        assert!(response_execution(&connection, AGENT, "one", 150).unwrap().is_some());
        assert_eq!(response_execution(&connection, AGENT, "one", 250).unwrap(), None);
    }

    #[test]
    fn native_exit_bounds_duration_before_later_cleanup_and_duplicate_observations() {
        for (kind, fields) in [
            ("work.failed", json!({"attempt":1})),
            ("work.released", json!({"attempt":1})),
            ("step-run.state", json!({"attempt":1,"status":"ready"})),
        ] {
            let store = Store::open_memory("source").unwrap();
            claim(&store, 1, "one", 100);
            let exit = event(&store, AGENT, "runtime.observed", 200, json!({
                "status":"failed","incarnation_id":"one","exit_evidence":{"exit_code":1},
            }));
            event(&store, STEP, kind, 700, fields);
            event(&store, AGENT, "runtime.observed", 800, json!({
                "status":"failed","incarnation_id":"one","exit_evidence":{"exit_code":1},
            }));
            let executions = store.step_executions(STEP).unwrap();
            assert_eq!(executions[0].exit_evidence.as_deref(), Some(exit.id.as_str()));
            assert_eq!(executions[0].ended_at_unix_ms, Some(200), "{kind}");
            assert_eq!(executions[0].duration_ms, Some(100), "{kind}");
        }
    }

    #[test]
    fn later_native_exit_does_not_lengthen_a_completed_work_claim() {
        let store = Store::open_memory("source").unwrap();
        claim(&store, 1, "one", 100);
        event(&store, STEP, "work.submitted", 150, json!({"attempt":1}));
        event(&store, AGENT, "runtime.observed", 200, json!({
            "status":"stopped","incarnation_id":"one","exit_evidence":{"exit_code":0},
        }));
        let executions = store.step_executions(STEP).unwrap();
        assert_eq!(executions[0].ended_at_unix_ms, Some(150));
        assert_eq!(executions[0].duration_ms, Some(50));
        assert_eq!(executions[0].status.as_deref(), Some("submitted"));
    }
}
