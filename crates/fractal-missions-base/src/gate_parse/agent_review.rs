use super::*;
use crate::model::evidence_contract::{EvidenceContract, EvidenceSpec, EvidenceType};

#[derive(Debug, PartialEq)]
struct AgentReview {
    name: String,
    reviewer: String,
    question: String,
    review_targets: Vec<String>,
    evidence_contract: EvidenceContract,
}

fn parse_agent_review(node: &KdlNode) -> Result<AgentReview, FractalError> {
    ensure_only_properties(node, &["type"])?;
    let name = one_string_with_children(node)?;
    let invalid = |message| FractalError::new("invalid-agent-review", message);
    if property_string(node, "type")?.as_deref() != Some("agent-review") {
        return Err(invalid("expected type=\"agent-review\""));
    }
    let body = node
        .children()
        .ok_or_else(|| invalid("agent review needs a body"))?;
    reject_unknown_children(
        body,
        &["reviewer", "question", "review", "evidence"],
        "gate",
        &name,
    )?;
    for child in body.nodes() {
        let properties: &[&str] = if child.name().value() == "evidence" {
            &["type", "equals"]
        } else {
            &[]
        };
        ensure_only_properties(child, properties)?;
    }
    let reviewer = required_child_string(body, "reviewer", &name)?;
    validate_full_subject(&reviewer)?;
    if !reviewer.starts_with("agent/") {
        return Err(invalid("agent review needs a full agent subject"));
    }
    let question = required_child_string(body, "question", &name)?;
    if question.trim().is_empty() {
        return Err(invalid("agent review needs a nonblank question"));
    }
    let mut review_targets = Vec::new();
    let mut fields = Vec::new();
    for child in body.nodes() {
        match child.name().value() {
            "review" => {
                let target = one_string(child)?;
                validate_full_subject(&target)?;
                if review_targets.contains(&target) {
                    return Err(invalid("agent review targets must be unique"));
                }
                review_targets.push(target);
            }
            "evidence" => {
                let field = one_string(child)?;
                let evidence_type = match property_string(child, "type")?.as_deref() {
                    Some("text") => EvidenceType::Text,
                    Some("subject") => EvidenceType::Subject,
                    Some("document") => EvidenceType::Document,
                    _ => return Err(invalid("evidence needs type=text, subject or document")),
                };
                fields.push((
                    field,
                    EvidenceSpec {
                        evidence_type,
                        equals: property_string(child, "equals")?,
                    },
                ));
            }
            _ => {}
        }
    }
    Ok(AgentReview {
        name,
        reviewer,
        question,
        review_targets,
        evidence_contract: EvidenceContract::new(fields)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REQUIRED: &str = "reviewer \"agent/release/reviewer\"; question \"Ready?\"; evidence \"summary\" type=\"text\";";

    fn node(properties: &str, body: &str) -> KdlNode {
        format!("gate \"approval\" type=\"agent-review\" {properties} {{ {body} }}")
            .parse::<KdlDocument>()
            .unwrap()
            .nodes()[0]
            .clone()
    }

    #[test]
    fn agent_review_grammar_valid_and_sorted() {
        let document = format!("doc/release/report@{}", "a".repeat(64));
        let subject = "evidence \"candidate\" type=\"subject\" equals=\"resource/release\";";
        let report = format!("evidence \"report\" type=\"document\" equals=\"{document}\";");
        let context = "review \"resource/context\"; review \"doc/context\";";
        let body = format!("{REQUIRED}{subject}{report}{context}");
        let parsed = parse_agent_review(&node("", &body)).unwrap();
        assert_eq!(parsed.name, "approval");
        assert_eq!(parsed.reviewer, "agent/release/reviewer");
        assert_eq!(parsed.question, "Ready?");
        assert_eq!(parsed.review_targets, ["resource/context", "doc/context"]);
        let body = format!("{report}{subject}{REQUIRED}");
        let reversed = parse_agent_review(&node("", &body)).unwrap();
        assert_eq!(parsed.evidence_contract, reversed.evidence_contract);
        let keys: Vec<_> = parsed.evidence_contract.fields().keys().collect();
        assert_eq!(keys, ["candidate", "report", "summary"]);
        // The grammar creates one required contract, shared by approval and rejection.
        let evidence = json!({
            "summary": "Reviewed", "candidate": "resource/release", "report": document
        });
        let contract = &parsed.evidence_contract;
        assert!(
            contract
                .validate_evidence(evidence.as_object().unwrap())
                .is_ok()
        );
        assert!(
            contract
                .validate_evidence(json!({}).as_object().unwrap())
                .is_err()
        );
        let minimal = parse_agent_review(&node("", REQUIRED)).unwrap();
        assert!(minimal.review_targets.is_empty());
    }

    #[test]
    fn agent_review_grammar_rejects_invalid_shapes() {
        for body in [
            "question \"Ready?\"; evidence \"x\" type=\"text\";",
            "reviewer \"agent/a\"; evidence \"x\" type=\"text\";",
            "reviewer \"agent/a\"; question \"Ready?\";",
            "reviewer \"agent/a\"; question \" \"; evidence \"x\" type=\"text\";",
        ] {
            assert!(parse_agent_review(&node("", body)).is_err(), "{body}");
        }
        for extra in [
            "reviewer \"agent/other\";",
            "question \"Again?\";",
            "unknown \"x\";",
            "mode \"approve\";",
            "tools \"shell\";",
            "exec \"true\";",
            "model \"x\";",
            "evidence \"summary\" type=\"text\";",
            "evidence \"x\";",
            "evidence \"x\" type=\"other\";",
            "evidence \"x\" type=1;",
            "evidence \"x\" type=\"text\" equals=1;",
            "evidence \"x\" type=\"text\" equals=\" \";",
            "evidence \"x\" type=\"subject\" equals=\"not-a-subject\";",
            "evidence \"x\" type=\"document\" equals=\"doc/mutable\";",
            "evidence \"x\" type=\"text\" unknown=\"x\";",
            "evidence \"\" type=\"text\";",
            "review \"bad\";",
            "review \"resource/x\"; review \"resource/x\";",
            "review \"resource/x\" unknown=\"x\";",
            "review \"resource/x\" { nested; };",
            "evidence \"x\" type=\"text\" { nested; };",
        ] {
            let body = format!("{REQUIRED}{extra}");
            assert!(parse_agent_review(&node("", &body)).is_err(), "{extra}");
        }
        for property in [
            "mode=\"approve\"",
            "tools=\"shell\"",
            "exec=\"true\"",
            "unknown=1",
        ] {
            let result = parse_agent_review(&node(property, REQUIRED));
            assert!(result.is_err(), "{property}");
        }
        for body in [
            REQUIRED.replace("agent/release/reviewer", "person/alex"),
            REQUIRED.replace("agent/release/reviewer", "agent/"),
            REQUIRED.replace(
                "reviewer \"agent/release/reviewer\"",
                "reviewer \"agent/a\" extra=1",
            ),
            REQUIRED.replace("question \"Ready?\"", "question \"Ready?\" extra=1"),
        ] {
            assert!(parse_agent_review(&node("", &body)).is_err(), "{body}");
        }
    }

    #[test]
    fn agent_review_grammar_stays_disconnected() {
        let node = node("", REQUIRED);
        assert!(parse_agent_review(&node).is_ok());
        let parsed = parse_gate(&node, "local");
        assert_eq!(parsed.unwrap_err().code, "invalid-gate-type");
        let running = parse_running_gate(&node, "approval".into(), "local");
        assert_eq!(running.unwrap_err().code, "invalid-gate-type");
    }
}
