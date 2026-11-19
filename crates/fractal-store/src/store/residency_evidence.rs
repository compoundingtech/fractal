//! Residency evidence is admitted and appended under the same graph writer transaction.
use super::*;

const FEATURE: declared_admission::FleetFeature = declared_admission::FleetFeature {
    keys: &["idle_residency"],
    code: "idle-residency-feature-barrier",
    carries: "automatic idle suspension and wake",
};

/// Use the replicated claim-operation identity and request digest, not the receipt cache key.
/// This keeps transaction-local residency writers interchangeable with normal claim retries.
#[allow(clippy::too_many_arguments)]
pub(super) fn append_residency_claim_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
    body: &Value,
    predecessors: &[String],
    forced_batch: Option<&str>,
    key: &str,
) -> Result<ClaimRecord> {
    let input = ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: actor.map(str::to_owned),
        fields: serde_json::from_value(body.get("fields").cloned().unwrap_or_else(|| json!({})))?,
        evidence: serde_json::from_value(body.get("evidence").cloned().unwrap_or_else(|| json!([])))?,
        expected_subject: None,
        idempotency_key: Some(key.into()),
    };
    append_input_tx(transaction, origin, &input, body, predecessors, forced_batch)
}

fn append_input_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    input: &ClaimInput,
    body: &Value,
    predecessors: &[String],
    forced_batch: Option<&str>,
) -> Result<ClaimRecord> {
    validate_claim_input(input)?;
    let operation = identities::claim_operation(input)?;
    if let Some(record) = claim_operation_outcome_tx(transaction, operation.as_ref())? {
        return Ok(record);
    }
    for evidence in &input.evidence {
        if !checkpoint::claim_or_tombstone_exists(transaction, evidence)? {
            return Err(FractalError::new("missing-evidence", format!("evidence claim `{evidence}` is not stored")).into());
        }
    }
    let (id, request_digest) = operation.expect("a residency receipt always has an idempotency key");
    let mut body = body.clone();
    body["_operation"] = json!({"id":id,"request_digest":request_digest});
    let record = append_claim_tx(transaction, origin, &input.subject, &input.kind,
        input.actor.as_deref(), &body, predecessors, forced_batch)?;
    if !register_operation_tx(transaction, &record)? {
        return Err(FractalError::new("idempotency-conflict", "residency operation conflicts").into());
    }
    Ok(record)
}

impl Store {
    /// Append evidence only while every active member advertises its reader support.
    /// A reader preflight is diagnostic only: withdrawal serializes with this writer gate.
    pub fn append_residency_evidence(&self, input: &ClaimInput) -> Result<bool, FractalError> {
        self.validate_claim_input(input)?;
        self.connection.batched(|transaction| -> Result<bool, FractalError> {
            if declared_admission::validate_fleet(transaction, &self.origin, &FEATURE).is_err() {
                return Ok(false);
            }
            // A retire writer may have ended this sleep after the runtime read its evidence.
            // The first committed end is authoritative even when later end evidence differs.
            if input.kind == "residency.sleep-ended"
                && latest_claim_of_kind_tx(transaction, &input.subject, &input.kind)?.is_some()
            {
                return Ok(false);
            }
            let operation = identities::claim_operation(input)?;
            if claim_operation_outcome_tx(transaction, operation.as_ref())?.is_some() {
                return Ok(false);
            }
            let predecessor = latest_claim_id_tx(transaction, &input.subject).map_err(internal)?;
            if input.expected_subject.as_ref().is_some_and(|expected| expected != &predecessor) {
                return Err(FractalError::new("stale-subject", &input.subject));
            }
            if input.idempotency_key.is_none() {
                return Err(FractalError::new("invalid-idempotency-key", "residency evidence requires an idempotency key"));
            }
            append_input_tx(transaction, &self.origin, input,
                &json!({"fields":input.fields,"evidence":input.evidence}),
                &predecessor.into_iter().collect::<Vec<_>>(), None).map_err(claim_append_error)?;
            Ok(true)
        }).map_err(internal)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::directive_notes::tests_shared_delivery::{claim, enable};

    fn started(store: &Store, supported: bool) {
        claim(store, "daemon/alder", "daemon.started", None,
            json!({"status":"running","features":{"idle_residency":u8::from(supported)}}));
        store.replication_snapshot().unwrap();
    }

    #[test]
    fn transaction_helper_shares_claim_retry_identity() {
        let store = Store::open_memory("alder").unwrap();
        let input = input();
        let first = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder",
            &input.subject, &input.kind, None,
            &json!({"fields":input.fields,"evidence":input.evidence}),
            &[], None, input.idempotency_key.as_deref().unwrap())).unwrap().unwrap();
        let (replayed, inserted) = store.append_claim_outcome(&input).unwrap();
        assert!(!inserted);
        assert_eq!(replayed.id, first.id);
        let mut changed = input;
        changed.fields.insert("observed_at".into(), json!(101));
        let error = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder",
            &changed.subject, &changed.kind, None,
            &json!({"fields":changed.fields,"evidence":changed.evidence}),
            &[], None, changed.idempotency_key.as_deref().unwrap())).unwrap().unwrap_err();
        assert_eq!(claim_append_error(error).code, "idempotency-mismatch");
    }

    #[test]
    fn concurrent_sleep_end_keeps_first_writer_end() {
        let store = Store::open_memory("alder").unwrap();
        enable(&store);
        started(&store, true);
        let mut input = input();
        input.subject = "residency/alder/sleep/first-end".into();
        input.kind = "residency.sleep-ended".into();
        input.fields = serde_json::from_value(json!({
            "host":"alder","suspended_at":100,"ended_at":200,"short_sleep":0,"end_class":"retire"
        })).unwrap();
        assert!(store.latest_claim(&input.subject, Some(&input.kind)).unwrap().is_none());
        store.connection.batched(|tx| append_residency_claim_tx(tx, "alder",
            &input.subject, &input.kind, None, &json!({"fields":input.fields,"evidence":[]}),
            &[], None, input.idempotency_key.as_deref().unwrap())).unwrap().unwrap();
        input.fields.insert("ended_at".into(), json!(300));
        input.fields.insert("end_class".into(), json!("stop"));
        assert!(!store.append_residency_evidence(&input).unwrap());
        let claims = store.claims_for(&input.subject, Some(&input.kind)).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].body["fields"]["ended_at"], 200);
    }

    fn input() -> ClaimInput {
        ClaimInput {
            subject: "residency/alder/outcome/withdrawal".into(), kind: "residency.outcome".into(), actor: None,
            fields: serde_json::from_value(json!({"host":"alder","observed_at":100,"resume_failed":0,"useless":1,"reason":"no-turn"})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: Some("residency-withdrawal".into()),
        }
    }

    #[test]
    fn withdrawal_after_reader_preflight_prevents_writer_append() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let writer = Store::open(&path, "alder").unwrap();
        enable(&writer);
        started(&writer, true);
        let publisher = Store::open(&path, "alder").unwrap();
        publisher.idle_residency_enabled().unwrap();

        // Deterministically order withdrawal after the successful preflight and before append.
        started(&writer, false);
        let input = input();
        assert!(!publisher.append_residency_evidence(&input).unwrap());
        assert!(publisher.claims_for(&input.subject, Some(&input.kind)).unwrap().is_empty());
        let operation = identities::claim_operation(&input).unwrap();
        publisher.connection.batched(|tx| {
            assert!(claim_operation_outcome_tx(tx, operation.as_ref()).unwrap().is_none());
            Ok::<_, FractalError>(())
        }).unwrap().unwrap();

        started(&writer, true);
        assert!(publisher.append_residency_evidence(&input).unwrap());
        assert!(!publisher.append_residency_evidence(&input).unwrap());
        let (_, inserted) = publisher.append_claim_outcome(&input).unwrap();
        assert!(!inserted, "normal claim retries share the operation identity and digest");
        let mut changed = input.clone();
        changed.fields.insert("observed_at".into(), json!(101));
        assert_eq!(publisher.append_residency_evidence(&changed).unwrap_err().code, "idempotency-mismatch");
        assert_eq!(publisher.claims_for(&input.subject, Some(&input.kind)).unwrap().len(), 1);
    }
}
