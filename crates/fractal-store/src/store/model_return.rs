//! Durable model-return requests use the ordinary runtime restart lifecycle.
use super::*;

pub const OPERATION: &str = "model-return-v1";

fn attempt_count(connection: &Connection, agent: &str, incarnation: &str) -> Result<u64> {
    Ok(connection.query_row(
        "SELECT json_extract(body,'$.fields.model_return_attempt')
         FROM claims INDEXED BY claims_model_return_attempt_index
         WHERE subject=?1 AND json_extract(body,'$.fields.incarnation_id')=?2
           AND kind='runtime.action.requested' AND json_extract(body,'$.fields.operation')='model-return-v1'
         ORDER BY json_extract(body,'$.fields.model_return_attempt') DESC LIMIT 1",
        params![agent, incarnation], |row| row.get(0)).optional()?.unwrap_or(0))
}

impl Store {
    /// A single exact deadline, not a retry poll, when a prior admission spends the budget.
    pub fn model_return_budget_resumes_at(&self, agent: &str) -> Result<Option<u128>> {
        smallclaims::touched::note_read(|| agent.to_owned());
        let latest: Option<i64> = self.connection.read().query_row(
            "SELECT MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims WHERE subject=?1 AND kind='runtime.action.requested' AND json_extract(body,'$.fields.operation')=?2",
            params![agent, OPERATION], |row| row.get(0))?;
        Ok(latest.map(|at| (at.max(0) as u128).saturating_add(3_600_001)))
    }
    /// Counts admitted requests, not detector evaluations, including completed requests.
    pub fn model_return_attempt_count(&self, agent: &str, incarnation: &str) -> Result<u64> {
        smallclaims::touched::note_read(|| agent.to_owned());
        attempt_count(&self.readers.get(), agent, incarnation)
    }

    /// Budget, host exclusion, and request insertion share the graph writer transaction.
    /// The normal operation envelope supplies replay identity across daemon restarts.
    pub fn admit_model_return(&self, input: &ClaimInput, now: u128) -> Result<ClaimRecord, FractalError> {
        smallclaims::touched::note_read(|| "kind:runtime.action.requested".into());
        smallclaims::touched::note_read(|| "kind:runtime.action.succeeded".into());
        smallclaims::touched::note_read(|| "kind:runtime.action.failed".into());
        let host = input.fields.get("host").and_then(Value::as_str).unwrap_or("");
        let admission = |connection: &Connection| -> Result<(), FractalError> {
            let token = current_desired_row(connection, &input.subject).map_err(internal)?.map(|row| row.claim_id);
            if token.as_deref() != input.evidence.first().map(String::as_str) {
                return Err(FractalError::new("model-return-stale", "the declaration changed before model return admission"));
            }
            let spent: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='runtime.action.requested' AND json_extract(body,'$.fields.operation')=?2 AND CAST(accepted_at_unix_ms AS INTEGER)>=?3)",
                params![input.subject, OPERATION, now.saturating_sub(3_600_000).min(i64::MAX as u128) as i64], |row| row.get(0)).map_err(internal)?;
            if spent { return Err(FractalError::new("model-return-budget", "the agent already admitted a model return within the last hour")); }
            let incarnation = input.fields.get("incarnation_id").and_then(Value::as_str).unwrap_or("");
            let next = attempt_count(connection, &input.subject, incarnation).map_err(internal)?.checked_add(1);
            if next.is_none() || input.fields.get("model_return_attempt").and_then(Value::as_u64) != next {
                return Err(FractalError::new("model-return-stale", "another model return advanced the durable admission generation"));
            }
            let mut requests = connection.prepare_cached(
                "SELECT id FROM claims WHERE kind='runtime.action.requested' AND json_extract(body,'$.fields.operation')=?1 AND json_extract(body,'$.fields.host')=?2"
            ).map_err(internal)?;
            let mut rows = requests.query(params![OPERATION, host]).map_err(internal)?;
            while let Some(row) = rows.next().map_err(internal)? {
                let request: String = row.get(0).map_err(internal)?;
                let completion = operation_id_for_key(&format!("agent-restart-completed:{request}"));
                let completed: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM operations WHERE id=?1 AND state='active')",
                    [&completion], |row| row.get(0)).map_err(internal)?;
                if !completed { return Err(FractalError::new("model-return-host-busy", "another model return is in progress on this host")); }
            }
            Ok(())
        };
        append_claim_with_commit_context(&self.graph, input, None, None, None, None,
            ClaimCommitContext { admission: Some(&admission), ..Default::default() }).map(|(claim, _)| claim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Store { initialize(Store::open_memory("node").unwrap()) }
    fn initialize(store: Store) -> Store {
        let source = "version 2\nagent \"first\" { workspace \"/tmp\"; harness \"omp\" {} }\nagent \"second\" { workspace \"/tmp\"; harness \"omp\" {} }\n";
        let intent = crate::graph::parse_internal_intent(source, "node").unwrap();
        let plan = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
        store.apply(&intent, &plan.subject_tokens, "return-test").unwrap();
        store
    }
    fn request(store: &Store, agent: &str, incarnation: &str, key: &str) -> ClaimInput {
        ClaimInput {
            subject: agent.into(), kind: "runtime.action.requested".into(), actor: Some("agent/st3/reconciler".into()),
            fields: BTreeMap::from([
                ("action".into(), json!("restart-when-idle")),
                ("operation".into(), json!(OPERATION)),
                ("host".into(), json!("node")),
                ("incarnation_id".into(), json!(incarnation)),
                ("model_return_attempt".into(), json!(store.model_return_attempt_count(agent, incarnation).unwrap() + 1)),
                ("reason".into(), json!("transient fallback at a settled boundary")),
            ]),
            evidence: vec![store.selected_desired_token(agent).unwrap().unwrap()],
            expected_subject: None, idempotency_key: Some(key.into()),
        }
    }
    #[test]
    fn durable_admission_replays_excludes_host_and_preserves_cross_incarnation_budget() {
        let store = fixture();
        let now = now_ms();
        let first = request(&store, "agent/first", "one", "return-one");
        let claim = store.admit_model_return(&first, now).unwrap();
        assert_eq!(store.admit_model_return(&first, now).unwrap().id, claim.id);
        assert_eq!(store.model_return_attempt_count("agent/first", "one").unwrap(), 1);
        assert_eq!(store.model_return_attempt_count("agent/first", "two").unwrap(), 0);
        let second = request(&store, "agent/second", "one", "return-two");
        assert_eq!(store.admit_model_return(&second, now).unwrap_err().code, "model-return-host-busy");
        store.append_claim(&ClaimInput {
            subject: "agent/first".into(), kind: "runtime.action.succeeded".into(), actor: Some("agent/st3/reconciler".into()),
            fields: BTreeMap::from([("action".into(), json!("restart")), ("reason".into(), json!("cancelled stale return"))]),
            evidence: vec![claim.id.clone()], expected_subject: None,
            idempotency_key: Some(format!("agent-restart-completed:{}", claim.id)),
        }).unwrap();
        let next = request(&store, "agent/first", "two", "return-three");
        assert_eq!(store.admit_model_return(&next, now).unwrap_err().code, "model-return-budget");
        assert!(store.admit_model_return(&second, now).is_ok());
        assert_eq!(store.model_return_attempt_count("agent/second", "one").unwrap(), 1);
    }
    #[test]
    fn declaration_fence_rejects_stale_request() {
        let store = fixture();
        let mut input = request(&store, "agent/first", "one", "stale-return");
        input.evidence = vec!["stale-declaration".into()];
        assert_eq!(store.admit_model_return(&input, now_ms()).unwrap_err().code, "model-return-stale");
        assert_eq!(store.model_return_attempt_count("agent/first", "one").unwrap(), 0);
    }
    #[test]
    fn reopening_store_preserves_request_identity_and_hourly_budget() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.db");
        let store = initialize(Store::open(&path, "node").unwrap());
        let input = request(&store, "agent/first", "one", "persistent-return");
        let now = now_ms();
        let id = store.admit_model_return(&input, now).unwrap().id;
        drop(store);
        let store = Store::open(&path, "node").unwrap();
        assert_eq!(store.admit_model_return(&input, now).unwrap().id, id);
        assert_eq!(store.model_return_attempt_count("agent/first", "one").unwrap(), 1);
        let next = request(&store, "agent/first", "two", "after-restart");
        assert_eq!(store.admit_model_return(&next, now).unwrap_err().code, "model-return-budget");
        let other = request(&store, "agent/second", "two", "host-after-restart");
        assert_eq!(store.admit_model_return(&other, now).unwrap_err().code, "model-return-host-busy");
    }
    #[test]
    fn writer_rejects_counter_skips_and_stale_generations_without_counting_them() {
        let store = fixture();
        let mut input = request(&store, "agent/first", "one", "counter-skip");
        input.fields.insert("model_return_attempt".into(), json!(2));
        assert_eq!(store.admit_model_return(&input, now_ms()).unwrap_err().code, "model-return-stale");
        assert_eq!(store.model_return_attempt_count("agent/first", "one").unwrap(), 0);
        input.fields.insert("model_return_attempt".into(), json!(1));
        let claim = store.admit_model_return(&input, now_ms()).unwrap();
        assert_eq!(claim.body["fields"]["model_return_attempt"], 1);
        assert_eq!(store.model_return_attempt_count("agent/first", "one").unwrap(), 1);
        store.append_claim(&ClaimInput {
            subject: "agent/first".into(), kind: "runtime.action.succeeded".into(), actor: Some("agent/st3/reconciler".into()),
            fields: BTreeMap::from([("action".into(), json!("restart")), ("reason".into(), json!("cancelled"))]),
            evidence: vec![claim.id.clone()], expected_subject: None,
            idempotency_key: Some(format!("agent-restart-completed:{}", claim.id)),
        }).unwrap();
        input.idempotency_key = Some("stale-counter-generation".into());
        let after_budget = claim.accepted_at_unix_ms + 3_600_001;
        assert_eq!(store.admit_model_return(&input, after_budget).unwrap_err().code, "model-return-stale");
        assert_eq!(store.model_return_attempt_count("agent/first", "one").unwrap(), 1);
        input.fields.insert("model_return_attempt".into(), json!(2));
        assert!(store.admit_model_return(&input, after_budget).is_ok());
        assert_eq!(store.model_return_attempt_count("agent/first", "one").unwrap(), 2);
    }
}
