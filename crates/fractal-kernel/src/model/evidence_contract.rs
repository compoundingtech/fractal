use super::{FractalError, valid_word};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

// No Serialize/Deserialize: this contract is not a mission publication surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceType {
    Text,
    Subject,
    Document,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSpec {
    pub evidence_type: EvidenceType,
    pub equals: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceContract {
    evidence_contract: BTreeMap<String, EvidenceSpec>,
}

fn invalid(message: impl Into<String>) -> FractalError {
    FractalError::new("invalid-evidence-contract", message)
}

impl EvidenceType {
    fn validate(self, value: &str) -> Result<(), FractalError> {
        if value.trim().is_empty() {
            return Err(invalid("evidence values must not be blank"));
        }
        match self {
            Self::Text => Ok(()),
            Self::Subject => fractal_schema::registry()
                .validate_subject(value)
                .map(|_| ())
                .map_err(|error| invalid(error.to_string())),
            Self::Document => {
                let (name, hash) = value
                    .rsplit_once('@')
                    .ok_or_else(|| invalid("document evidence needs an immutable version"))?;
                smallclaims::store::validate_document_name(name)?;
                fractal_schema::registry()
                    .validate_subject(name)
                    .map_err(|error| invalid(error.to_string()))?;
                if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(invalid("document evidence needs a 64-hex version hash"));
                }
                Ok(())
            }
        }
    }
}

impl EvidenceContract {
    pub fn new(fields: impl IntoIterator<Item = (String, EvidenceSpec)>) -> Result<Self, FractalError> {
        let mut evidence_contract = BTreeMap::new();
        for (name, spec) in fields {
            if !valid_word(&name) {
                return Err(invalid(format!("invalid evidence field name `{name}`")));
            }
            if let Some(equals) = &spec.equals {
                spec.evidence_type.validate(equals)?;
            }
            let entry = evidence_contract.entry(name);
            if let std::collections::btree_map::Entry::Occupied(entry) = entry {
                return Err(invalid(format!(
                    "duplicate evidence field `{}`",
                    entry.key()
                )));
            }
            entry.or_insert(spec);
        }
        if evidence_contract.is_empty() {
            return Err(invalid("an evidence contract needs at least one field"));
        }
        Ok(Self { evidence_contract })
    }

    pub fn fields(&self) -> &BTreeMap<String, EvidenceSpec> {
        &self.evidence_contract
    }

    /// Validate fields.evidence, not the claim-reference evidence envelope.
    pub fn validate_evidence(&self, evidence: &Map<String, Value>) -> Result<(), FractalError> {
        for name in evidence.keys() {
            if !self.evidence_contract.contains_key(name) {
                return Err(invalid(format!("unexpected evidence field `{name}`")));
            }
        }
        for (name, spec) in &self.evidence_contract {
            let value = evidence
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("evidence field `{name}` needs a string")))?;
            spec.evidence_type.validate(value)?;
            if spec.equals.as_deref().is_some_and(|equals| equals != value) {
                return Err(invalid(format!(
                    "evidence field `{name}` does not equal its bound"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(evidence_type: EvidenceType, equals: Option<&str>) -> EvidenceSpec {
        EvidenceSpec {
            evidence_type,
            equals: equals.map(str::to_owned),
        }
    }

    fn contract(evidence_type: EvidenceType, equals: Option<&str>) -> EvidenceContract {
        EvidenceContract::new([("report".into(), spec(evidence_type, equals))]).unwrap()
    }

    fn accepts(contract: &EvidenceContract, evidence: Value) -> bool {
        contract
            .validate_evidence(evidence.as_object().unwrap())
            .is_ok()
    }

    #[test]
    fn evidence_contract_requires_exact_nonblank_string_fields() {
        let contract = contract(EvidenceType::Text, None);
        assert!(accepts(&contract, json!({"report": "Reviewed."})));
        for evidence in [json!({}), json!({"report": "ok", "extra": "no"})] {
            assert!(!accepts(&contract, evidence));
        }
        for value in [
            json!(null),
            json!(false),
            json!(7),
            json!([]),
            json!({}),
            json!(""),
            json!(" \n\t"),
        ] {
            assert!(!accepts(&contract, json!({"report": value})));
        }
    }

    #[test]
    fn evidence_contract_rejects_empty_duplicate_and_invalid_names_and_sorts_fields() {
        assert!(EvidenceContract::new([]).is_err());
        let field = || spec(EvidenceType::Text, None);
        assert!(EvidenceContract::new([("a".into(), field()), ("a".into(), field())]).is_err());
        for name in [
            "",
            "has space",
            "a/b",
            "-bad",
            ".bad",
            "Upper",
            "é",
            &"x".repeat(65),
        ] {
            assert!(EvidenceContract::new([(name.into(), field())]).is_err());
        }
        let contract =
            EvidenceContract::new([("z".into(), field()), ("a".into(), field())]).unwrap();
        assert_eq!(
            contract
                .fields()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
    }

    #[test]
    fn evidence_contract_subjects_use_full_subject_syntax() {
        let contract = contract(EvidenceType::Subject, None);
        for value in [
            "agent/team/reviewer",
            "resource/release-candidate",
            "file/build-node:/srv/report.txt",
            "custom/review/report",
        ] {
            assert!(accepts(&contract, json!({"report": value})), "{value}");
        }
        for value in [
            "reviewer",
            "agent/",
            "agent//reviewer",
            "unknown/report",
            "agent/has space",
            "file/node:relative",
            "file/node:/srv/../secret",
            "${input.subject}",
        ] {
            assert!(!accepts(&contract, json!({"report": value})), "{value}");
        }
    }

    #[test]
    fn evidence_contract_documents_require_immutable_valid_versions() {
        let contract = contract(EvidenceType::Document, None);
        let hash = "a".repeat(64);
        assert!(accepts(
            &contract,
            json!({"report": format!("doc/review/report@{hash}")})
        ));
        for value in [
            "doc/report".into(),
            "doc/report@".into(),
            "doc/report@abc".into(),
            format!("doc/report@{}", "g".repeat(64)),
            format!("resource/report@{hash}"),
            format!("doc/../report@{hash}"),
            format!("doc/report/@{hash}"),
            format!("doc/has space@{hash}"),
        ] {
            assert!(!accepts(&contract, json!({"report": value})), "{value}");
        }
    }

    #[test]
    fn evidence_contract_equals_is_type_validated_and_exact() {
        for (kind, bound) in [
            (EvidenceType::Text, " \n"),
            (EvidenceType::Subject, "not-a-subject"),
            (EvidenceType::Document, "doc/report"),
        ] {
            assert!(EvidenceContract::new([("report".into(), spec(kind, Some(bound)))]).is_err());
        }
        let document = format!("doc/report@{}", "a".repeat(64));
        for (kind, bound) in [
            (EvidenceType::Text, "Reviewed."),
            (EvidenceType::Subject, "resource/candidate"),
            (EvidenceType::Document, document.as_str()),
        ] {
            let contract = contract(kind, Some(bound));
            assert!(accepts(&contract, json!({"report": bound})));
            assert!(!accepts(&contract, json!({"report": format!("{bound} ")})));
        }
    }
}
