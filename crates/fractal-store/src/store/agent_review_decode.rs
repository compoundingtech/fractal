use super::agent_review_answer::{AnswerCandidate, RequestBinding, ReviewRequest};
use crate::model::evidence_contract::{EvidenceContract, EvidenceSpec, EvidenceType};
use crate::model::{ClaimInput, ClaimRecord, FractalError};
use serde_json::Value;

fn invalid() -> FractalError {
    FractalError::new(
        "invalid-agent-review-claim",
        "missing or malformed agent review claim",
    )
}
fn text(value: Option<&Value>) -> Result<&str, FractalError> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(invalid)
}
fn binding<'a>(
    get: impl Fn(&str) -> Option<&'a Value>,
    request: &'a str,
) -> Result<RequestBinding<'a>, FractalError> {
    Ok(RequestBinding {
        request,
        operation: text(get("operation"))?,
        revision: text(get("mission_revision"))?,
        definition: text(get("step_definition"))?,
        attempt: get("attempt")
            .and_then(Value::as_u64)
            .and_then(|n| n.try_into().ok())
            .ok_or_else(invalid)?,
    })
}
/// Slice 12 must use this exact key builder and 24-hex operation digest form.
pub(crate) fn request_key(owner: &str, digest: &str) -> String {
    format!("agent-review-request:{owner}:{digest}")
}
pub(crate) struct StoredReview<'a> {
    pub binding: RequestBinding<'a>,
    pub owner: &'a str,
    pub reviewer: &'a str,
    pub contract: EvidenceContract,
    pub key: String,
    pub digest: &'a str,
}
impl StoredReview<'_> {
    pub fn request(&self) -> ReviewRequest<'_> {
        ReviewRequest {
            binding: self.binding.clone(),
            owner: self.owner,
            reviewer: self.reviewer,
            contract: &self.contract,
            idempotency_key: &self.key,
            digest: self.digest,
        }
    }
}
/// Only stored claim data supplies request authority; no caller-supplied binding overrides.
pub(crate) fn stored_request(record: &ClaimRecord) -> Result<StoredReview<'_>, FractalError> {
    let fields = record
        .body
        .get("fields")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let allowed = [
        "owner",
        "reviewer",
        "question",
        "review_targets",
        "decisions",
        "operation",
        "mission_revision",
        "step_definition",
        "attempt",
        "gate_kind",
        "gate",
        "evidence_contract",
    ];
    if record.kind != "gate.requested"
        || record.id.is_empty()
        || fields.keys().any(|key| !allowed.contains(&key.as_str()))
        || text(fields.get("gate_kind"))? != "agent-review"
    {
        return Err(invalid());
    }
    text(fields.get("gate"))?;
    text(fields.get("question"))?;
    if !fields
        .get("review_targets")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().all(|v| text(Some(v)).is_ok()))
        || !fields
            .get("decisions")
            .and_then(Value::as_array)
            .is_some_and(|a| {
                a.iter()
                    .map(Value::as_str)
                    .eq([Some("approved"), Some("rejected")])
            })
    {
        return Err(invalid());
    }
    let owner = text(fields.get("owner"))?;
    let reviewer = text(fields.get("reviewer"))?;
    for subject in [owner, reviewer] {
        fractal_schema::registry()
            .validate_subject(subject)
            .map_err(|_| invalid())?;
    }
    if !reviewer.starts_with("agent/")
        || !["mission-run/", "step-run/", "loop-run/"]
            .iter()
            .any(|prefix| owner.starts_with(prefix))
    {
        return Err(invalid());
    }
    let binding = binding(|key| fields.get(key), &record.id)?;
    let (path, digest) = binding
        .operation
        .strip_prefix("gate-operation/")
        .and_then(|op| op.rsplit_once('/'))
        .ok_or_else(invalid)?;
    if binding.operation != record.subject
        || digest.len() != 24
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        || !path
            .chars()
            .eq(owner.chars().map(|c| if c == '/' { '.' } else { c }))
    {
        return Err(invalid());
    }
    let key = request_key(owner, digest);
    let (operation, _) = smallclaims::store::operation_parts(&record.body).ok_or_else(invalid)?;
    if operation != smallclaims::store::operation_id_for_key(&key) {
        return Err(invalid());
    }
    let specs = fields
        .get("evidence_contract")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let contract = EvidenceContract::try_new(specs.iter().map(|(name, spec)| {
        let spec = spec.as_object().ok_or_else(invalid)?;
        if spec
            .keys()
            .any(|key| !["type", "equals"].contains(&key.as_str()))
        {
            return Err(invalid());
        }
        let evidence_type = match text(spec.get("type"))? {
            "text" => EvidenceType::Text,
            "subject" => EvidenceType::Subject,
            "document" => EvidenceType::Document,
            _ => return Err(invalid()),
        };
        let equals = spec
            .get("equals")
            .map(|v| text(Some(v)).map(str::to_owned))
            .transpose()?;
        Ok((
            name.clone(),
            EvidenceSpec {
                evidence_type,
                equals,
            },
        ))
    }))?;
    Ok(StoredReview {
        binding,
        owner,
        reviewer,
        contract,
        key,
        digest,
    })
}

/// Typed envelope fields originate in ClaimInput, never parallel caller arguments.
pub(crate) fn incoming_answer(input: &ClaimInput) -> Result<AnswerCandidate<'_>, FractalError> {
    let fields = &input.fields;
    let allowed = [
        "request",
        "operation",
        "reviewer",
        "mission_revision",
        "step_definition",
        "attempt",
        "decision",
        "verdict",
        "reason",
        "evidence",
    ];
    let request = text(fields.get("request"))?;
    let binding = binding(|key| fields.get(key), request)?;
    if input.kind != "gate.result"
        || binding.operation != input.subject
        || fields.keys().any(|key| !allowed.contains(&key.as_str()))
        || !input.evidence.iter().any(|id| id == request)
    {
        return Err(invalid());
    }
    let actor = input
        .actor
        .as_deref()
        .filter(|actor| !actor.trim().is_empty())
        .ok_or_else(invalid)?;
    Ok(AnswerCandidate {
        binding,
        actor: Some(actor),
        reviewer: text(fields.get("reviewer"))?,
        decision: text(fields.get("decision"))?,
        verdict: text(fields.get("verdict"))?,
        reason: fields
            .get("reason")
            .map(|v| v.as_str().ok_or_else(invalid))
            .transpose()?,
        evidence: fields
            .get("evidence")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> ClaimRecord {
        let operation = "gate-operation/step-run.example.review/aaaaaaaaaaaaaaaaaaaaaaaa";
        let key = request_key("step-run/example/review", "aaaaaaaaaaaaaaaaaaaaaaaa");
        let body = json!({"fields":{"owner":"step-run/example/review", "reviewer":"agent/example/reviewer",
            "question":"Approve?", "gate":"review", "gate_kind":"agent-review", "review_targets":[],
            "decisions":["approved","rejected"], "operation":operation, "mission_revision":"revision",
            "step_definition":"definition", "attempt":2, "evidence_contract":{"report":{"type":"text","equals":"ok"}}},
            "_operation":{"id":smallclaims::store::operation_id_for_key(&key), "request_digest":"digest"}});
        ClaimRecord {
            id: "request".into(),
            store_index: 1,
            batch_id: "batch".into(),
            subject: operation.into(),
            kind: "gate.requested".into(),
            origin: "example".into(),
            actor: None,
            operation_id: None,
            request_digest: None,
            body,
            predecessors: vec![],
            accepted_at_unix_ms: 1,
        }
    }
    #[test]
    fn agent_decode_uses_only_stored_request_authority() {
        let record = fixture();
        let decoded = stored_request(&record).unwrap();
        assert_eq!(decoded.binding.request, record.id);
        assert_eq!(decoded.binding.operation, record.subject);
        assert_eq!(decoded.reviewer, "agent/example/reviewer");
        assert_eq!(
            decoded.contract.fields()["report"].equals.as_deref(),
            Some("ok")
        );
        assert_eq!(
            decoded.request().idempotency_key,
            request_key(decoded.owner, decoded.digest)
        );
    }
    #[test]
    fn agent_decode_rejects_missing_malformed_and_unknown_fields() {
        let record = fixture();
        for field in record.body["fields"].as_object().unwrap().keys() {
            let mut missing = record.clone();
            missing.body["fields"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(stored_request(&missing).is_err(), "{field}");
            let mut malformed = record.clone();
            malformed.body["fields"][field] = Value::Null;
            assert!(stored_request(&malformed).is_err(), "{field}");
        }
        for (field, value) in [
            ("gate_kind", json!("human")),
            ("owner", json!("resource/candidate")),
            ("reviewer", json!("person/operator")),
            ("attempt", json!(-1)),
            ("attempt", json!(u64::MAX)),
            (
                "operation",
                json!("gate-operation/step-run.example.review/ab"),
            ),
            ("unknown", json!(true)),
            ("decisions", json!(["approved", "changes-requested"])),
        ] {
            let mut bad = record.clone();
            bad.body["fields"][field] = value;
            assert!(stored_request(&bad).is_err(), "{field}");
        }
        for patch in [
            json!({}),
            json!({"report":{"type":"unknown"}}),
            json!({"report":{"type":"text","equals":3}}),
            json!({"report":{"type":"text","extra":true}}),
        ] {
            let mut bad = record.clone();
            bad.body["fields"]["evidence_contract"] = patch;
            assert!(stored_request(&bad).is_err());
        }
    }
    #[test]
    fn agent_decode_request_envelope_and_operation_fail_closed() {
        let record = fixture();
        for value in [
            Value::Null,
            json!({}),
            json!({"id":"op/wrong","request_digest":"digest"}),
        ] {
            let mut bad = record.clone();
            bad.body["_operation"] = value;
            assert!(stored_request(&bad).is_err());
        }
    }
    fn answer(record: &ClaimRecord) -> ClaimInput {
        serde_json::from_value(json!({"subject":record.subject,"kind":"gate.result","actor":"agent/example/reviewer",
            "fields":{"request":record.id,"operation":record.subject,"reviewer":"agent/example/reviewer",
                "mission_revision":"revision","step_definition":"definition","attempt":2,
                "decision":"approved","verdict":"pass","evidence":{"report":"ok"}},"evidence":[record.id]})).unwrap()
    }
    #[test]
    fn agent_review_writer_rejects_empty_non_agentless_snapshot_without_writes() {
        use crate::store::agent_review_transaction as writer;
        use crate::store::*;
        use smallclaims::store::append_claim_record_tx;
        let store = Store::open_memory("node").unwrap();
        let intent = crate::parse_intent(
            "version 2\nagent \"example/reviewer\" { command \"true\"; workspace \"/tmp\"; }",
            "example",
        )
        .unwrap();
        let subjects = &intent.subjects;
        let expected = subjects.keys().map(|s| (s.clone(), vec![])).collect();
        store.apply(&intent, &expected, "declare").unwrap();
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let f = fixture();
        let origin = store.origin();
        let record =
            append_claim_record_tx(&tx, origin, &f.subject, &f.kind, None, &f.body, &[], None)
                .unwrap();
        let mut input = answer(&record);
        input.idempotency_key = Some("empty-owner-guard".into());
        let before = smallclaims::store::current_index_tx(&tx).unwrap();
        let error = writer::append_agent_review_answer_tx(&tx, origin, &input, |_, _, reviewer| {
            Ok(writer::ReviewOwnerSnapshot {
                responsible_agents: Default::default(),
                reviewer: reviewer.into(),
                agentless: false,
            })
        })
        .unwrap_err();
        assert_eq!(error.code, "reviewer-owner-unresolved");
        tx.commit().unwrap();
        let tx = connection.transaction().unwrap();
        assert_eq!(smallclaims::store::current_index_tx(&tx).unwrap(), before);
        let operation = identities::claim_operation(&input).unwrap();
        let outcome = claim_operation_outcome_tx(&tx, operation.as_ref()).unwrap();
        assert!(outcome.is_none());
    }
    #[test]
    fn agent_decode_answer_reuses_the_stored_request_predicate() {
        use super::super::agent_review_answer::{Rejection, ReviewPolicy, validate};
        let record = fixture();
        let decoded = stored_request(&record).unwrap();
        let excluded = Default::default();
        let policy = ReviewPolicy {
            resolved_reviewer: decoded.reviewer,
            resolved_responsible_agents: &excluded,
        };
        for decision in ["approved", "rejected"] {
            let mut input = answer(&record);
            input.fields.insert("decision".into(), json!(decision));
            input.fields.insert(
                "verdict".into(),
                json!(if decision == "approved" {
                    "pass"
                } else {
                    "fail"
                }),
            );
            input.fields.insert("reason".into(), json!("reviewed"));
            validate(
                &decoded.request(),
                &policy,
                &incoming_answer(&input).unwrap(),
            )
            .unwrap();
            input.fields.insert("attempt".into(), json!(3));
            assert!(matches!(
                validate(
                    &decoded.request(),
                    &policy,
                    &incoming_answer(&input).unwrap()
                ),
                Err(Rejection::Binding)
            ));
        }
    }
    #[test]
    fn agent_decode_answer_required_fields_fail_closed() {
        let input = answer(&fixture());
        for field in input.fields.keys() {
            let mut missing = input.clone();
            missing.fields.remove(field);
            assert!(incoming_answer(&missing).is_err(), "{field}");
            for malformed in [Value::Null, json!([]), json!(" ")] {
                let mut bad = input.clone();
                bad.fields.insert(field.clone(), malformed);
                assert!(incoming_answer(&bad).is_err(), "{field}");
            }
        }
        for (field, value) in [
            ("reason", json!(3)),
            ("acted_for", json!("person/operator")),
            ("delegation", json!({"episode":"request"})),
            ("attempt", json!(-1)),
            ("attempt", json!(u64::MAX)),
            ("attempt", json!(1.5)),
        ] {
            let mut bad = input.clone();
            bad.fields.insert(field.into(), value);
            assert!(incoming_answer(&bad).is_err(), "{field}");
        }
    }
    #[test]
    fn agent_decode_answer_envelope_is_not_a_binding_override() {
        let input = answer(&fixture());
        for which in 0..7 {
            let mut bad = input.clone();
            match which {
                0 => bad.kind = "gate.requested".into(),
                1 => bad.subject = "gate-operation/other".into(),
                2 => bad.actor = None,
                3 => bad.actor = Some(" ".into()),
                4 => bad.evidence.clear(),
                5 => bad.evidence = vec!["wrong-request".into()],
                _ => {
                    bad.fields.insert("request".into(), json!("stale"));
                }
            }
            assert!(incoming_answer(&bad).is_err());
        }
        for (field, value) in [
            ("actor", json!(3)),
            ("fields", json!([])),
            ("evidence", json!([3])),
        ] {
            let mut raw = serde_json::to_value(&input).unwrap();
            raw[field] = value;
            assert!(serde_json::from_value::<ClaimInput>(raw).is_err());
        }
    }
}
