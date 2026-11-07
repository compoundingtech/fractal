//! Declaration-local admission metadata: validation only, never enforcement.
use crate::declaration_syntax::{
    ensure_bare, ensure_no_children, ensure_no_properties, reject_unknown_children, unique_child,
};
use crate::model::{DeclaredAdmission, DeclaredBudgets, DeclaredLane, DeclaredPriority, FractalError};
use kdl::{KdlDocument, KdlNode, KdlValue};
use serde::de::DeserializeOwned;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const INTEGER: &str = "must be an integer from 1 to 9007199254740991";
const SIZE: &str = "must be a positive size such as 8G or 512M";
const BUDGETS: [(&str, &str); 4] = [
    ("cpu-seconds", "cpu_seconds"),
    ("memory-max", "memory_max"),
    ("disk-max", "disk_max"),
    ("token-budget", "token_budget"),
];

#[derive(Clone, Copy)]
struct Scope<'a>(&'a str, &'a str);

impl Scope<'_> {
    fn error(self, field: &str, requirement: &str) -> FractalError {
        FractalError::new(
            "invalid-declared-admission",
            format!("{} '{}' {field} {requirement}", self.0, self.1),
        )
        .with_detail("subject", self.1)
        .with_detail("field", field)
    }

    fn number(self, field: &str, value: Option<u64>) -> Result<u64, FractalError> {
        value
            .filter(|v| (1..=MAX_SAFE_INTEGER).contains(v))
            .ok_or_else(|| self.error(field, INTEGER))
    }

    fn size(self, field: &str, value: Option<&str>) -> Result<(), FractalError> {
        let value = value.ok_or_else(|| self.error(field, SIZE))?;
        let bytes =
            fractal_runtime::parse_lane_memory(value).map_err(|_| self.error(field, SIZE))?;
        if bytes == 0 {
            return Err(self.error(field, SIZE));
        }
        Ok(())
    }

    fn validate(
        self,
        priority: Option<&DeclaredPriority>,
        lane: Option<&DeclaredLane>,
        budgets: Option<&DeclaredBudgets>,
    ) -> Result<(), FractalError> {
        if matches!(priority, Some(DeclaredPriority::Unknown(_))) {
            return Err(self.error("declared_priority", "must be background or normal"));
        }
        if matches!(lane, Some(DeclaredLane::Unknown(_))) {
            return Err(self.error("declared_lane", "must be check or build"));
        }
        if let Some(b) = budgets {
            for (field, value) in [
                ("declared_budgets.cpu_seconds", b.cpu_seconds),
                ("declared_budgets.token_budget", b.token_budget),
            ] {
                if let Some(value) = value {
                    self.number(field, Some(value))?;
                }
            }
            for (field, value) in [
                ("declared_budgets.memory_max", b.memory_max.as_deref()),
                ("declared_budgets.disk_max", b.disk_max.as_deref()),
            ] {
                if let Some(value) = value {
                    self.size(field, Some(value))?;
                }
            }
        }
        Ok(())
    }
}

fn scalar(node: &KdlNode) -> Result<&KdlValue, FractalError> {
    ensure_no_properties(node)?;
    ensure_no_children(node)?;
    let name = node.name().value();
    let [entry] = node.entries() else {
        return Err(FractalError::new(
            "wrong-argument-count",
            format!("child `{name}` needs one scalar"),
        ));
    };
    if entry.ty().is_some() {
        return Err(FractalError::new(
            "unexpected-type",
            format!("child `{name}` cannot have an argument type annotation"),
        ));
    }
    Ok(entry.value())
}

fn string<T: DeserializeOwned>(node: &KdlNode) -> Result<T, FractalError> {
    let value = scalar(node)?.as_string().ok_or_else(|| {
        FractalError::new(
            "expected-string",
            format!("child `{}` needs one string", node.name().value()),
        )
    })?;
    // String and the open string enums accept every string; author validation follows.
    Ok(
        T::deserialize(serde::de::value::StrDeserializer::<serde::de::value::Error>::new(value))
            .expect("string or open string enum"),
    )
}

fn integer(node: &KdlNode, scope: Scope<'_>, field: &str) -> Result<u64, FractalError> {
    let KdlValue::Integer(value) = scalar(node)? else {
        return Err(FractalError::new(
            "expected-integer",
            format!("child `{}` needs one integer", node.name().value()),
        ));
    };
    scope.number(field, u64::try_from(*value).ok())
}

/// Parse strict KDL; retain empty-block presence until the publication compatibility gate.
pub fn parse(
    document: &KdlDocument,
    kind: &str,
    subject: &str,
) -> Result<DeclaredAdmission, FractalError> {
    let scope = Scope(kind, subject);
    let priority = unique_child(document, "declared-priority")?
        .map(string)
        .transpose()?;
    let lane = unique_child(document, "declared-lane")?
        .map(string)
        .transpose()?;
    let budgets = unique_child(document, "declared-budgets")?
        .map(|node| {
            ensure_bare(node)?;
            let mut budgets = DeclaredBudgets::default();
            if let Some(body) = node.children() {
                reject_unknown_children(
                    body,
                    &BUDGETS.map(|(name, _)| name),
                    "declared-budgets",
                    subject,
                )?;
                let number = |name, field| {
                    unique_child(body, name)?
                        .map(|n| integer(n, scope, field))
                        .transpose()
                };
                budgets.cpu_seconds = number("cpu-seconds", "declared_budgets.cpu_seconds")?;
                budgets.token_budget = number("token-budget", "declared_budgets.token_budget")?;
                budgets.memory_max = unique_child(body, "memory-max")?.map(string).transpose()?;
                budgets.disk_max = unique_child(body, "disk-max")?.map(string).transpose()?;
            }
            Ok::<_, FractalError>(budgets)
        })
        .transpose()?;
    scope.validate(priority.as_ref(), lane.as_ref(), budgets.as_ref())?;
    Ok(DeclaredAdmission {
        priority,
        lane,
        budgets,
    })
}

/// Author validation is separate from the open enums used by historical readers.
pub fn validate(
    kind: &str,
    subject: &str,
    priority: Option<&DeclaredPriority>,
    lane: Option<&DeclaredLane>,
    budgets: Option<&DeclaredBudgets>,
) -> Result<(), FractalError> {
    Scope(kind, subject).validate(priority, lane, budgets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fields(source: &str) -> Result<DeclaredAdmission, FractalError> {
        parse(
            &source.parse::<KdlDocument>().unwrap(),
            "agent",
            "agent/example",
        )
    }

    #[test]
    fn strict_syntax_table() {
        for (source, code) in [
            (
                "declared-priority \"normal\"\ndeclared-priority \"normal\"",
                "duplicate-child",
            ),
            (
                "declared-lane \"check\"\ndeclared-lane \"build\"",
                "duplicate-child",
            ),
            (
                "declared-budgets {}\ndeclared-budgets {}",
                "duplicate-child",
            ),
            ("declared-priority", "wrong-argument-count"),
            (
                "declared-priority \"normal\" \"background\"",
                "wrong-argument-count",
            ),
            ("declared-lane #null", "expected-string"),
            ("declared-priority 1", "expected-string"),
            ("declared-lane \"check\" extra=\"x\"", "unknown-property"),
            ("declared-lane \"check\" {}", "unexpected-children"),
            ("(meta)declared-priority \"normal\"", "unexpected-type"),
            ("declared-priority (meta)\"normal\"", "unexpected-type"),
            ("declared-budgets 1", "unexpected-value"),
            ("declared-budgets extra=\"x\" {}", "unexpected-value"),
            ("(meta)declared-budgets {}", "unexpected-type"),
            ("declared-budgets { unknown 1; }", "unknown-child"),
            (
                "declared-budgets { cpu-seconds 1; cpu-seconds 2; }",
                "duplicate-child",
            ),
            (
                "declared-budgets { token-budget 1; token-budget 2; }",
                "duplicate-child",
            ),
            (
                "declared-budgets { memory-max \"1\"; memory-max \"2\"; }",
                "duplicate-child",
            ),
            (
                "declared-budgets { disk-max \"1\"; disk-max \"2\"; }",
                "duplicate-child",
            ),
            ("declared-budgets { cpu-seconds; }", "wrong-argument-count"),
            (
                "declared-budgets { cpu-seconds 1 2; }",
                "wrong-argument-count",
            ),
            (
                "declared-budgets { cpu-seconds \"1\"; }",
                "expected-integer",
            ),
            ("declared-budgets { token-budget 1.5; }", "expected-integer"),
            (
                "declared-budgets { token-budget #null; }",
                "expected-integer",
            ),
            ("declared-budgets { memory-max 1; }", "expected-string"),
            (
                "declared-budgets { disk-max \"8G\" {}; }",
                "unexpected-children",
            ),
            (
                "declared-budgets { cpu-seconds 1 extra=2; }",
                "unknown-property",
            ),
            (
                "declared-budgets { (meta)cpu-seconds 1; }",
                "unexpected-type",
            ),
            (
                "declared-budgets { cpu-seconds (meta)1; }",
                "unexpected-type",
            ),
        ] {
            assert_eq!(parse_fields(source).unwrap_err().code, code, "{source}");
        }
    }

    #[test]
    fn numeric_semantics_and_details_table() {
        for child in ["cpu-seconds", "token-budget"] {
            for value in ["1", "9007199254740991"] {
                parse_fields(&format!("declared-budgets {{ {child} {value}; }}")).unwrap();
            }
            for value in ["0", "-1", "9007199254740992", "18446744073709551616"] {
                let error =
                    parse_fields(&format!("declared-budgets {{ {child} {value}; }}")).unwrap_err();
                let field = format!("declared_budgets.{}", child.replace('-', "_"));
                assert_eq!(error.code, "invalid-declared-admission");
                assert_eq!(error.details["subject"], "agent/example");
                assert_eq!(error.details["field"], field);
                assert_eq!(
                    error.message,
                    format!(
                        "agent 'agent/example' {field} must be an integer from 1 to 9007199254740991"
                    )
                );
            }
        }
        for (source, field) in [
            ("declared-priority \"urgent\"", "declared_priority"),
            ("declared-lane \"other\"", "declared_lane"),
        ] {
            let error = parse_fields(source).unwrap_err();
            assert_eq!(error.code, "invalid-declared-admission");
            assert_eq!(error.details["field"], field);
        }
    }

    #[test]
    fn checked_size_grammar_and_product_bounds_table() {
        for child in ["memory-max", "disk-max"] {
            for value in [
                "1",
                "0001",
                "+1",
                "512M",
                "8G",
                "8g",
                "1K",
                "1T",
                "18446744073709551615",
                "18014398509481983K",
                "17592186044415M",
                "17179869183G",
                "16777215T",
            ] {
                parse_fields(&format!("declared-budgets {{ {child} \"{value}\"; }}")).unwrap();
            }
            for value in [
                "", "0", "0G", "00G", "-1", "1.5G", "8GB", "1P", " 1", "1 ", "１G", "G",
            ] {
                let error = parse_fields(&format!("declared-budgets {{ {child} \"{value}\"; }}"))
                    .unwrap_err();
                assert_eq!(error.code, "invalid-declared-admission", "{value}");
                assert!(
                    error
                        .message
                        .ends_with("must be a positive size such as 8G or 512M"),
                    "{value}"
                );
            }
            for value in [
                "18446744073709551616",
                "18014398509481984K",
                "17592186044416M",
                "17179869184G",
                "16777216T",
                "18446744073709551615T",
            ] {
                let error = parse_fields(&format!("declared-budgets {{ {child} \"{value}\"; }}"))
                    .unwrap_err();
                let field = format!("declared_budgets.{}", child.replace('-', "_"));
                assert_eq!(error.code, "invalid-declared-admission");
                assert_eq!(error.details["field"], field);
                assert_eq!(
                    error.message,
                    format!(
                        "agent 'agent/example' {field} must be a positive size such as 8G or 512M"
                    )
                );
            }
        }
    }

    #[test]
    fn typed_authoring_rejects_future_enums_and_invalid_budgets() {
        assert!(
            validate(
                "mission",
                "mission/test",
                Some(&DeclaredPriority::Unknown("future".into())),
                None,
                None
            )
            .is_err()
        );
        assert!(
            validate(
                "mission",
                "mission/test",
                None,
                Some(&DeclaredLane::Unknown("future".into())),
                None
            )
            .is_err()
        );
        for budgets in [
            DeclaredBudgets {
                cpu_seconds: Some(0),
                ..DeclaredBudgets::default()
            },
            DeclaredBudgets {
                token_budget: Some(MAX_SAFE_INTEGER + 1),
                ..DeclaredBudgets::default()
            },
            DeclaredBudgets {
                memory_max: Some("0".into()),
                ..DeclaredBudgets::default()
            },
            DeclaredBudgets {
                disk_max: Some("16777216T".into()),
                ..DeclaredBudgets::default()
            },
        ] {
            let error =
                validate("mission", "mission/test", None, None, Some(&budgets)).unwrap_err();
            assert_eq!(error.code, "invalid-declared-admission");
            assert_eq!(error.details["subject"], "mission/test");
        }
    }

    #[test]
    fn empty_and_full_admission_roundtrip() {
        assert_eq!(parse_fields("").unwrap(), DeclaredAdmission::default());
        for source in ["declared-budgets", "declared-budgets {}"] {
            let parsed = parse_fields(source).unwrap();
            assert!(parsed.budgets.as_ref().unwrap().is_empty());
            assert!(
                DeclaredAdmission::from_declaration(
                    parsed.priority.as_ref(),
                    parsed.lane.as_ref(),
                    parsed.budgets.as_ref()
                )
                .is_none()
            );
        }
        let parsed = parse_fields("declared-priority \"background\"\ndeclared-lane \"build\"\ndeclared-budgets { cpu-seconds 300; memory-max \"0008G\"; disk-max \"20G\"; token-budget 100000; }").unwrap();
        let json = serde_json::to_value(&parsed).unwrap();
        assert_eq!(json["budgets"]["memory_max"], "0008G");
        assert_eq!(
            serde_json::from_value::<DeclaredAdmission>(json).unwrap(),
            parsed
        );
    }
}
