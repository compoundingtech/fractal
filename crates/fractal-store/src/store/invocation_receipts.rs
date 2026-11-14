//! Native launch receipts share the runtime-start writer fence, not its disposable local log.
use super::*;
pub use fractal_schema::invocation::{FallbackEvidence, FreshnessEvidence, InvocationPhase, InvocationReceipt};

/// Live owners are daemon-local; their W3C correlation is retained in durable receipts.
/// Dropping the store closes host segments without rewriting historical correlation.
#[derive(Default)]
struct InvocationTraces(Mutex<HashMap<String, crate::otel::AttemptTrace>>);

impl Store {
    pub fn start_invocation_trace(&self, invocation: &str, agent: &str,
        links: &[crate::otel::AttemptTraceContext]) -> crate::otel::AttemptTraceContext {
        let owners = self.extension::<InvocationTraces>();
        let mut owners = owners.0.lock().unwrap_or_else(PoisonError::into_inner);
        owners.entry(invocation.into()).or_insert_with(|| crate::otel::AttemptTrace::start(
            "st.agent.invocation",
            vec![
                opentelemetry::KeyValue::new("span.label", "agent invocation"),
                opentelemetry::KeyValue::new("st.invocation.id", invocation.to_owned()),
                opentelemetry::KeyValue::new("st.agent", agent.to_owned()),
            ], links)).correlation.clone()
    }

    pub fn finish_invocation_trace(&self, invocation: &str, failed: bool) {
        let owners = self.extension::<InvocationTraces>();
        let Some(owner) = owners.0.lock().unwrap_or_else(PoisonError::into_inner).remove(invocation)
            else { return; };
        if failed { owner.failed(); }
    }
}

/// Construct from the admitted snapshot, never from a later desired declaration.
pub fn native_receipt(operation: &str, agent: &str, prepared: &Value, phase: InvocationPhase,
    result: Option<&Value>) -> Result<Option<InvocationReceipt>> {
    if prepared.pointer("/subject/kind").and_then(Value::as_str) != Some("agent") {
        return Ok(None);
    }
    let member = &prepared["launch_member"];
    let environment = &member["environment"];
    let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let env = |name: &str| text(&environment[name]);
    let unknown = Value::Null;
    let result = result.unwrap_or(&unknown);
    let resumed = ["ST3_NATIVE_RESUME_SESSION", "FRACTAL_NATIVE_RESUME_SESSION",
        "ST3_NATIVE_CONTINUE_SESSION", "FRACTAL_NATIVE_CONTINUE_SESSION"]
        .iter().any(|name| env(name).is_some());
    let requested_model = member.pointer("/launch/value").and_then(Value::as_array)
        .and_then(|argv| argv.windows(2).find(|pair| pair[0] == "--model")
            .and_then(|pair| text(&pair[1])));
    let trace_context = prepared.get("trace_context").filter(|context| !context.is_null()).cloned();
    let incarnation = matches!(result["verification"].as_str(),
        Some("operation-incarnation" | "operation-ended" | "operation-retired"))
        .then(|| text(&result["incarnation"])).flatten();
    Ok(Some(InvocationReceipt {
        invocation_id: format!("invocation/{operation}"), operation_id: operation.into(), phase,
        agent: agent.into(), event_key: None,
        owner_run: text(&prepared["subject"]["owner_run"])
            .or_else(|| env("ST_MISSION_RUN")).or_else(|| env("FRACTAL_MISSION_RUN")),
        owner_step: text(&prepared["subject"]["owner_step"])
            .or_else(|| env("ST_STEP_RUN")).or_else(|| env("FRACTAL_STEP_RUN")),
        owner_attempt: env("ST_ATTEMPT").or_else(|| env("FRACTAL_ATTEMPT"))
            .and_then(|attempt| attempt.parse().ok()),
        parent_invocation_id: env("FRACTAL_PARENT_INVOCATION_ID"),
        incarnation, requested_model, harness: text(&member["driver"]), harness_version: None,
        launch_input_digest: text(&result["launch_contract"]["prompt"]),
        provider_binary_digest: text(&result["launch_contract"]["binary"]),
        prompt_digest: None, transcript_ref: None, trace_context,
        freshness: FreshnessEvidence { requested_fresh: prepared.pointer("/request/body/fields/action")
                .and_then(Value::as_str).filter(|action| *action == "fresh-context").map(|_| true),
            resume_selected: Some(resumed), observed_fresh: None,
            source: Some("admitted-launch-selection".into()) },
        fallback: None, error: text(&result["error"]),
        evidence: ["selected_token", "start_request", "control_request"]
            .iter().filter_map(|name| text(&prepared[*name])).collect(),
    }))
}

/// Immutable lifecycle slot: (agent, invocation_id, phase, event_key). Replays keep the
/// original fact; later observations append with their own source claim identity.
pub fn append_receipt_tx(tx: &Transaction<'_>, origin: &str, receipt: &InvocationReceipt) -> Result<()> {
    let fields = serde_json::to_value(receipt)?;
    let fields_map = serde_json::from_value(fields.clone())?;
    fractal_schema::registry().validate_claim(&receipt.agent, fractal_schema::invocation::KIND, &fields_map)?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='harness.invocation'
         AND json_extract(body,'$.fields.invocation_id')=?2 AND json_extract(body,'$.fields.phase')=?3
         AND json_extract(body,'$.fields.event_key') IS ?4)",
        params![receipt.agent, receipt.invocation_id, fields["phase"].as_str(), receipt.event_key], |row| row.get(0))?;
    if !exists {
        // Receipts are ordinary claims on the agent's timeline: keep the subject's causal
        // chain connected so acknowledgement walks through a receipt reach its predecessors.
        let predecessors: Vec<String> = latest_claim_id_tx(tx, &receipt.agent)?
            .into_iter().collect();
        append_claim_tx(tx, origin, &receipt.agent, fractal_schema::invocation::KIND, None,
            &json!({"fields":fields,"evidence":receipt.evidence}), &predecessors, None)?;
    }
    Ok(())
}

pub fn append_native_receipt_tx(tx: &Transaction<'_>, origin: &str, operation: &str,
    agent: &str, prepared: &Value, phase: InvocationPhase, result: Option<&Value>) -> Result<()> {
    if let Some(receipt) = native_receipt(operation, agent, prepared, phase, result)? {
        append_receipt_tx(tx, origin, &receipt)?;
    }
    Ok(())
}

/// Preserve an observed release without re-running the executable. Admission diagnostics
/// carry their process invocation, not a later standing turn's inherited environment.
pub fn record_diagnostic_tx(tx: &Transaction<'_>, origin: &str, diagnostic: &ClaimRecord) -> Result<()> {
    let fields = &diagnostic.body["fields"];
    if diagnostic.kind != "harness.diagnostic" || fields["code"] != "harness-admission-failed"
        || diagnostic.actor.as_deref() != Some(diagnostic.subject.as_str())
    {
        return Ok(());
    }
    let Some(version) = fields["producer_version"].as_str().filter(|version| !version.is_empty()) else {
        return Ok(());
    };
    let operation = fields["invocation_id"].as_str().and_then(|id| id.strip_prefix("invocation/"));
    let incarnation = fields["incarnation_id"].as_str();
    let start: Option<(String, String, Option<String>)> = tx.query_row(
        "SELECT id,body,result FROM local_runtime_starts WHERE subject=?1 AND
         ((?2 IS NOT NULL AND id=?2) OR (?2 IS NULL AND ?3 IS NOT NULL
          AND json_extract(result,'$.incarnation')=?3
          AND json_extract(result,'$.verification') IN ('operation-incarnation','operation-ended','operation-retired')))
         ORDER BY id DESC LIMIT 1",
        params![diagnostic.subject, operation, incarnation],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    if let Some((id, prepared, result)) = start {
        let prepared = serde_json::from_str(&prepared)?;
        let result: Option<Value> = result.map(|result| serde_json::from_str(&result)).transpose()?;
        if let Some(mut receipt) = native_receipt(&id, &diagnostic.subject, &prepared,
            InvocationPhase::Observed, result.as_ref())? {
            receipt.event_key = Some(diagnostic.id.clone());
            receipt.harness_version = Some(version.into());
            receipt.evidence.push(diagnostic.id.clone());
            append_receipt_tx(tx, origin, &receipt)?;
        }
    }
    Ok(())
}

impl Store {
    pub fn invocation_receipts(&self, invocation_id: &str) -> Result<Vec<InvocationReceipt>> {
        self.read_invocation_receipts(None, Some(invocation_id))
    }

    pub fn agent_invocation_receipts(&self, agent: &str) -> Result<Vec<InvocationReceipt>> {
        self.read_invocation_receipts(Some(agent), None)
    }

    /// Return only the original admitted launch member, never the current declaration.
    pub fn invocation_launch_member(&self, invocation: &str) -> Result<Option<Value>> {
        let Some(operation) = invocation.strip_prefix("invocation/") else { return Ok(None); };
        let member: Option<String> = self.readers.get().query_row(
            "SELECT json_extract(body,'$.launch_member') FROM local_runtime_starts WHERE id=?1", [operation],
            |row| row.get::<_, Option<String>>(0)).optional()?.flatten();
        member.map(|member| serde_json::from_str(&member).map_err(Into::into)).transpose()
    }
    fn read_invocation_receipts(&self, agent: Option<&str>, invocation_id: Option<&str>) -> Result<Vec<InvocationReceipt>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(
            "SELECT json_extract(body,'$.fields') FROM claims WHERE kind='harness.invocation'
             AND (?1 IS NULL OR subject=?1) AND (?2 IS NULL OR json_extract(body,'$.fields.invocation_id')=?2)
             ORDER BY store_index")?;
        statement.query_map(params![agent, invocation_id], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Capture launch inputs before entering the native backend, including launches that fail.
    pub fn record_invocation_launch_inputs(&self, operation: &str,
        contract: &crate::launch_contract::Contract) -> Result<()> {
        self.connection.batched(|tx| -> Result<()> {
            let (agent, prepared): (String, String) = tx.query_row(
                "SELECT subject,body FROM local_runtime_starts WHERE id=?1", [operation],
                |row| Ok((row.get(0)?, row.get(1)?)))?;
            append_native_receipt_tx(tx, self.origin(), operation, &agent,
                &serde_json::from_str(&prepared)?, InvocationPhase::LaunchInputs,
                Some(&json!({"launch_contract":contract})))
        }).map_err(|error| anyhow::anyhow!("{error}"))??;
        Ok(())
    }

    /// Retain a private CAS reference as an additive launch observation, never as provider text.
    pub fn record_invocation_transcript_reference(&self, receipt: &InvocationReceipt,
        artifact: &crate::invocation_artifacts::InvocationArtifact) -> Result<()> {
        anyhow::ensure!(artifact.kind == "transcript" && artifact.invocation_id == receipt.invocation_id
            && artifact.incarnation.is_some() && artifact.incarnation == receipt.incarnation
            && self.invocation_launch_member(&receipt.invocation_id)?.is_some(),
            "transcript reference must bind the exact admitted launch incarnation");
        let mut observed = receipt.clone();
        observed.phase = InvocationPhase::Observed;
        observed.event_key = Some(format!("transcript:{}", artifact.sha256));
        observed.transcript_ref = Some(format!("sha256:{}", artifact.sha256));
        if let Some(binding) = artifact.binding_claim.as_ref() {
            observed.evidence.push(binding.clone());
        }
        self.connection.batched(|tx| append_receipt_tx(tx, self.origin(), &observed))
            .map_err(|error| anyhow::anyhow!("{error}"))??;
        Ok(())
    }
    /// A terminal runtime observation may close only an already verified exact incarnation.
    /// The source observation can be local; the resulting receipt remains durable.
    pub fn record_invocation_runtime_end(&self, observation: &ClaimRecord) -> Result<()> {
        let fields = &observation.body["fields"];
        if observation.kind != "runtime.observed"
            || !matches!(fields["status"].as_str(), Some("exited" | "stopped" | "vanished"))
        {
            return Ok(());
        }
        let Some(incarnation) = fields["incarnation_id"].as_str() else { return Ok(()); };
        let mut invocations = BTreeMap::new();
        for receipt in self.agent_invocation_receipts(&observation.subject)? {
            if receipt.incarnation.as_deref() == Some(incarnation) {
                invocations.insert(receipt.invocation_id.clone(), receipt);
            }
        }
        self.connection.batched(|tx| -> Result<()> {
            for receipt in invocations.values() {
                let mut ended = receipt.clone();
                ended.phase = InvocationPhase::Ended;
                ended.event_key = Some(observation.id.clone());
                ended.evidence.push(observation.id.clone());
                append_receipt_tx(tx, self.origin(), &ended)?;
            }
            Ok(())
        }).map_err(|error| anyhow::anyhow!("{error}"))??;
        let failed = fields["status"] == "vanished"
            || fields["exit_code"].as_i64().is_some_and(|code| code != 0);
        for invocation in invocations.keys() {
            self.finish_invocation_trace(invocation, failed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared() -> Value { json!({"subject":{"kind":"agent","owner_run":"run-a","owner_step":"step-run/a/build"},
        "launch_member":{"driver":"omp","environment":{"ST_ATTEMPT":"3"}},"selected_token":"desired-a"}) }

    #[test]
    fn invocation_correlation_is_durable_and_not_inherited_from_process_environment() {
        let store = Store::open_memory("example").unwrap();
        let correlation = store.start_invocation_trace("invocation/op", "agent/example/worker", &[]);
        let repeated = store.start_invocation_trace("invocation/op", "agent/example/worker", &[]);
        assert_eq!(correlation, repeated);
        let mut body = prepared();
        body["trace_context"] = serde_json::to_value(&correlation).unwrap();
        body["launch_member"]["environment"]["TRACEPARENT"] = json!("stale-environment");
        let receipt = native_receipt("op", "agent/example/worker", &body,
            InvocationPhase::Admitted, None).unwrap().unwrap();
        assert_eq!(receipt.trace_context, Some(serde_json::to_value(&correlation).unwrap()));
        store.connection.batched(|tx| append_receipt_tx(tx, store.origin(), &receipt)).unwrap().unwrap();
        store.finish_invocation_trace("invocation/op", true);
        let retained = store.invocation_receipts("invocation/op").unwrap();
        assert_eq!(retained[0].trace_context, receipt.trace_context);
        assert_ne!(store.start_invocation_trace("invocation/retry", "agent/example/worker",
            std::slice::from_ref(&correlation)).trace_id(), correlation.trace_id());
        let unknown = native_receipt("unknown", "agent/example/worker", &prepared(),
            InvocationPhase::Admitted, None).unwrap().unwrap();
        assert_eq!(unknown.trace_context, None);
    }

    #[test]
    fn runtime_end_closes_only_exact_incarnation_launch_and_turn_receipts() {
        let store = Store::open_memory("node").unwrap();
        let agent = "agent/example/worker";
        for (operation, incarnation) in [("launch", "exact"), ("turn", "exact"), ("other", "other")] {
            let mut receipt = native_receipt(operation, agent, &prepared(), InvocationPhase::Started,
                Some(&json!({"verification":"operation-incarnation","incarnation":incarnation})))
                .unwrap().unwrap();
            if operation == "turn" {
                receipt.parent_invocation_id = Some("invocation/launch".into());
                receipt.event_key = Some("message/exact".into());
            }
            store.connection.batched(|tx| append_receipt_tx(tx, store.origin(), &receipt)).unwrap().unwrap();
        }
        let observation = store.append_claim(&ClaimInput { subject:agent.into(),
            kind:"runtime.observed".into(), actor:None,
            fields:serde_json::from_value(json!({"status":"stopped","incarnation_id":"exact"})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:None }).unwrap();
        for _ in 0..2 { store.record_invocation_runtime_end(&observation).unwrap(); }
        for operation in ["launch", "turn"] {
            let receipts = store.invocation_receipts(&format!("invocation/{operation}")).unwrap();
            let ended = receipts.iter().filter(|receipt| receipt.phase == InvocationPhase::Ended).collect::<Vec<_>>();
            assert_eq!(ended.len(), 1);
            assert!(ended[0].evidence.contains(&observation.id));
        }
        assert!(!store.invocation_receipts("invocation/other").unwrap().iter()
            .any(|receipt| receipt.phase == InvocationPhase::Ended));
    }

    #[test]
    fn startup_failure_never_binds_candidate_or_invents_usage() {
        let receipt = native_receipt("op", "agent/example/worker", &prepared(), InvocationPhase::StartupFailed,
            Some(&json!({"error":"startup failed","candidate_incarnation":"old","verification":"producer-closed"})))
            .unwrap().unwrap();
        assert_eq!(receipt.incarnation, None);
        assert_eq!(receipt.owner_attempt, Some(3));
        assert_eq!(receipt.harness_version, None);
        assert_eq!(receipt.fallback, None);
        assert_eq!(receipt.freshness.observed_fresh, None);
    }

    #[test]
    fn receipts_replay_without_losing_sibling_invocations() {
        let store = Store::open_memory("example").unwrap();
        for op in ["one", "one", "two"] {
            let receipt = native_receipt(op, "agent/example/worker", &prepared(), InvocationPhase::Admitted, None).unwrap().unwrap();
            store.connection.batched(|tx| append_receipt_tx(tx, store.origin(), &receipt)).unwrap().unwrap();
        }
        assert_eq!(store.invocation_receipts("invocation/one").unwrap().len(), 1);
        assert_eq!(store.agent_invocation_receipts("agent/example/worker").unwrap().len(), 2);
    }

    #[test]
    fn launch_input_digest_is_not_an_exact_prompt() {
        let receipt = native_receipt("op", "agent/example/worker", &prepared(), InvocationPhase::LaunchInputs,
            Some(&json!({"launch_contract":{"prompt":"inputs","binary":"provider"}}))).unwrap().unwrap();
        assert_eq!(receipt.launch_input_digest.as_deref(), Some("inputs"));
        assert_eq!(receipt.prompt_digest, None);
        let verified = native_receipt("op", "agent/example/worker", &prepared(), InvocationPhase::Started,
            Some(&json!({"incarnation":"actual","verification":"operation-incarnation"}))).unwrap().unwrap();
        assert_eq!(verified.incarnation.as_deref(), Some("actual"));
    }

    #[test]
    fn launch_selection_does_not_establish_freshness_or_fallback() {
        let mut prepared = prepared();
        prepared["launch_member"]["launch"] = json!({"type":"argv","value":["st3","driver","omp","--","omp","--model","requested"]});
        prepared["launch_member"]["environment"]["ST3_NATIVE_CONTINUE_SESSION"] = json!("existing-session");
        let receipt = native_receipt("op", "agent/example/worker", &prepared,
            InvocationPhase::Admitted, None).unwrap().unwrap();
        assert_eq!(receipt.requested_model.as_deref(), Some("requested"));
        assert_eq!(receipt.freshness.resume_selected, Some(true));
        assert_eq!(receipt.freshness.observed_fresh, None);
        assert_eq!(receipt.fallback, None);
    }

    #[test]
    fn observed_release_is_bound_before_claim_without_binding_a_candidate_incarnation() {
        let store = Store::open_memory("example").unwrap();
        let diagnostic: ClaimRecord = serde_json::from_value(json!({
            "id":"diagnostic/one","store_index":1,"batch_id":"batch/one",
            "subject":"agent/example/worker","kind":"harness.diagnostic","origin":"example",
            "actor":"agent/example/worker","predecessors":[],"accepted_at_unix_ms":1,
            "body":{"fields":{"code":"harness-admission-failed","invocation_id":"invocation/op",
                "incarnation_id":"candidate","producer_version":"18.6.0"}}
        })).unwrap();
        store.connection.batched(|tx| -> Result<()> {
            tx.execute("INSERT INTO local_runtime_starts(id,subject,owner,body) VALUES (?1,?2,?3,?4)",
                params!["op","agent/example/worker","owner",serde_json::to_string(&prepared())?])?;
            record_diagnostic_tx(tx, store.origin(), &diagnostic)?;
            record_diagnostic_tx(tx, store.origin(), &diagnostic)?;
            Ok(())
        }).unwrap().unwrap();
        let receipts = store.invocation_receipts("invocation/op").unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].phase, InvocationPhase::Observed);
        assert_eq!(receipts[0].harness_version.as_deref(), Some("18.6.0"));
        assert_eq!(receipts[0].incarnation, None);
        assert_eq!(receipts[0].fallback, None);
    }
}
