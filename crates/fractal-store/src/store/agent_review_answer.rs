use crate::model::{FractalError, evidence_contract::EvidenceContract};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestBinding<'a> {
    pub request: &'a str,
    pub operation: &'a str,
    pub revision: &'a str,
    pub definition: &'a str,
    pub attempt: u32,
}
/// Fully expanded request; reviewer is the resolved immutable storage identity.
pub(crate) struct ReviewRequest<'a> {
    pub binding: RequestBinding<'a>,
    pub owner: &'a str,
    pub idempotency_key: &'a str,
    pub digest: &'a str,
    pub reviewer: &'a str,
    pub contract: &'a EvidenceContract,
}
/// The loader supplies the canonical answer-cut assignment (AssignmentMoment::BeforeAnswer),
/// and the resolved responsibility set from slice 4/5, never display names or current state.
pub(crate) struct ReviewPolicy<'a> {
    pub resolved_reviewer: &'a str,
    pub resolved_responsible_agents: &'a BTreeSet<String>,
}
pub(crate) struct AnswerCandidate<'a> {
    pub binding: RequestBinding<'a>,
    pub actor: Option<&'a str>,
    pub reviewer: &'a str,
    pub decision: &'a str,
    pub verdict: &'a str,
    pub reason: Option<&'a str>,
    pub evidence: &'a Map<String, Value>,
}
#[derive(Debug)]
pub(crate) enum Rejection {
    Namespace,
    Assignment,
    Actor,
    NotIndependent,
    Binding,
    Decision,
    Reason,
    Evidence(FractalError),
}

/// No acted_for/delegation input: only the exact reviewer can answer.
/// Agent reviewer changes require a fresh revision/request, never a person-rename repair.
pub(crate) fn validate(
    request: &ReviewRequest<'_>,
    policy: &ReviewPolicy<'_>,
    answer: &AnswerCandidate<'_>,
) -> Result<(), Rejection> {
    let digest = request
        .idempotency_key
        .strip_prefix("agent-review-request:")
        .and_then(|key| key.strip_prefix(request.owner))
        .and_then(|key| key.strip_prefix(':'));
    if digest != Some(request.digest) || request.digest.is_empty() {
        return Err(Rejection::Namespace);
    }
    if request.reviewer != policy.resolved_reviewer || answer.reviewer != request.reviewer {
        return Err(Rejection::Assignment);
    }
    if answer.actor != Some(policy.resolved_reviewer) {
        return Err(Rejection::Actor);
    }
    if policy
        .resolved_responsible_agents
        .contains(policy.resolved_reviewer)
    {
        return Err(Rejection::NotIndependent);
    }
    if answer.binding != request.binding {
        return Err(Rejection::Binding);
    }
    match (answer.decision, answer.verdict) {
        ("approved", "pass") => {}
        ("rejected", "fail") => {
            if answer.reason.is_none_or(|reason| reason.trim().is_empty()) {
                return Err(Rejection::Reason);
            }
        }
        _ => return Err(Rejection::Decision),
    }
    request
        .contract
        .validate_evidence(answer.evidence)
        .map_err(Rejection::Evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::evidence_contract::{EvidenceSpec, EvidenceType};
    use serde_json::json;

    const REVIEWER: &str = "agent/example/reviewer";
    fn contract() -> EvidenceContract {
        EvidenceContract::new([(
            "report".into(),
            EvidenceSpec {
                evidence_type: EvidenceType::Text,
                equals: Some("reviewed".into()),
            },
        )])
        .unwrap()
    }
    const BINDING: RequestBinding<'static> = RequestBinding {
        request: "request",
        operation: "gate-operation/example",
        revision: "revision",
        definition: "definition",
        attempt: 2,
    };
    fn request(contract: &EvidenceContract) -> ReviewRequest<'_> {
        ReviewRequest {
            binding: BINDING,
            owner: "step-run/example/review",
            idempotency_key: "agent-review-request:step-run/example/review:digest",
            digest: "digest",
            reviewer: REVIEWER,
            contract,
        }
    }
    fn answer(evidence: &Map<String, Value>) -> AnswerCandidate<'_> {
        AnswerCandidate {
            binding: BINDING,
            actor: Some(REVIEWER),
            reviewer: REVIEWER,
            decision: "approved",
            verdict: "pass",
            reason: None,
            evidence,
        }
    }
    fn check(
        request: &ReviewRequest<'_>,
        excluded: &BTreeSet<String>,
        answer: &AnswerCandidate<'_>,
    ) -> Result<(), Rejection> {
        let policy = ReviewPolicy {
            resolved_reviewer: REVIEWER,
            resolved_responsible_agents: excluded,
        };
        validate(request, &policy, answer)
    }
    fn reject(result: Result<(), Rejection>, expected: Rejection) {
        assert_eq!(
            std::mem::discriminant(&result.unwrap_err()),
            std::mem::discriminant(&expected)
        );
    }
    #[test]
    fn agent_answer_authority_bindings_and_namespace() {
        let (contract, evidence, excluded) = (contract(), Map::new(), BTreeSet::new());
        let mut request = request(&contract);
        for actor in [
            None,
            Some("agent/example/delegate"),
            Some("reviewer"),
            Some("person/operator"),
        ] {
            let mut answer = answer(&evidence);
            answer.actor = actor; // An acted_for claim cannot change this exact-actor check.
            reject(check(&request, &excluded, &answer), Rejection::Actor);
        }
        for field in 0..5 {
            let mut answer = answer(&evidence);
            match field {
                0 => answer.binding.request = "stale",
                1 => answer.binding.operation = "gate-operation/other",
                2 => answer.binding.revision = "stale",
                3 => answer.binding.definition = "stale",
                _ => answer.binding.attempt += 1,
            }
            reject(check(&request, &excluded, &answer), Rejection::Binding);
        }
        let mut answer = answer(&evidence);
        answer.reviewer = "agent/example/alias";
        reject(check(&request, &excluded, &answer), Rejection::Assignment);
        answer.reviewer = REVIEWER;
        request.reviewer = "agent/example/replacement";
        reject(check(&request, &excluded, &answer), Rejection::Assignment);
        request.reviewer = REVIEWER;
        for key in [
            "review-request:step-run/example/review:digest",
            "agent-review-request:other:digest",
            "agent-review-request:step-run/example/review:stale",
        ] {
            request.idempotency_key = key;
            reject(check(&request, &excluded, &answer), Rejection::Namespace);
        }
        request.idempotency_key = "agent-review-request:step-run/example/review:digest";
        // The pure policy consumes the combined resolved owner/requester/worker exclusion set.
        let excluded = BTreeSet::from([REVIEWER.into()]);
        reject(
            check(&request, &excluded, &answer),
            Rejection::NotIndependent,
        );
    }
    #[test]
    fn agent_answer_decisions_and_evidence_validator_wiring() {
        let (contract, excluded) = (contract(), BTreeSet::new());
        let request = request(&contract);
        let evidence = json!({"report":"reviewed"}).as_object().unwrap().clone();
        for (decision, verdict, reason, expected) in [
            ("approved", "pass", None, None),
            ("approved", "pass", Some(""), None),
            ("approved", "pass", Some("yes"), None),
            ("rejected", "fail", Some("no"), None),
            ("rejected", "fail", None, Some(Rejection::Reason)),
            ("rejected", "fail", Some(" \n"), Some(Rejection::Reason)),
            ("approved", "fail", None, Some(Rejection::Decision)),
            ("rejected", "pass", Some("no"), Some(Rejection::Decision)),
            (
                "changes-requested",
                "feedback",
                Some("revise"),
                Some(Rejection::Decision),
            ),
        ] {
            let mut answer = answer(&evidence);
            (answer.decision, answer.verdict, answer.reason) = (decision, verdict, reason);
            let result = check(&request, &excluded, &answer);
            if let Some(expected) = expected {
                reject(result, expected);
            } else {
                result.unwrap();
            }
        }
        let missing = Map::new();
        for decision in ["approved", "rejected"] {
            let mut answer = answer(&missing);
            answer.decision = decision;
            answer.verdict = if decision == "approved" {
                "pass"
            } else {
                "fail"
            };
            answer.reason = Some("reviewed");
            let Err(Rejection::Evidence(error)) = check(&request, &excluded, &answer) else {
                panic!("missing evidence accepted");
            };
            let expected = contract.validate_evidence(&missing).unwrap_err();
            assert_eq!(
                (error.code, error.message),
                (expected.code, expected.message)
            );
        }
    }
    #[test]
    fn agent_answer_resolved_alias_responsibility_collides() {
        use crate::store::identities::{IdentityKind, IdentityStore, RenameRequest};
        let store = crate::store::Store::open_memory("example").unwrap();
        let source = "version 2\nagent \"example/reviewer\" { command \"true\"; }";
        let intent = crate::parse_intent(source, "example").unwrap();
        let expected = [(REVIEWER.into(), vec![])].into();
        store.apply(&intent, &expected, "declare-reviewer").unwrap();
        let alias = "agent/example/alias";
        let rename = RenameRequest {
            from: REVIEWER.into(),
            to: alias.into(),
            actor: "person/operator".into(),
            reason: "Clarify responsibility".into(),
            expected_naming_revision: None,
            expected_declaration: None,
            idempotency_key: "rename-reviewer".into(),
        };
        store
            .rename_identity(IdentityKind::Agent, &rename, &[])
            .unwrap();
        let resolved = store.resolve_identity(IdentityKind::Agent, alias).unwrap();
        assert_eq!(resolved.identity, REVIEWER);
        let excluded = BTreeSet::from([resolved.identity]);
        let (contract, evidence) = (contract(), Map::new());
        reject(
            check(&request(&contract), &excluded, &answer(&evidence)),
            Rejection::NotIndependent,
        );
    }
}
