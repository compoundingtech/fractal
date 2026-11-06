use super::agent_review_answer::{RequestBinding, ReviewRequest};
use crate::model::evidence_contract::{EvidenceContract, EvidenceSpec, EvidenceType};
use crate::model::{ClaimRecord, FractalError};
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
}
