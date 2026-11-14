//! Declaration-local admission metadata: validation only, never enforcement.
use crate::declaration_syntax::{
    ensure_bare, ensure_no_children, ensure_no_properties, reject_unknown_children, unique_child,
};
use crate::model::{
    DeclaredAdmission, DeclaredAttentionBudget, DeclaredBudgets, DeclaredLane, DeclaredPriority,
    FractalError, MissionSpec, NormalizedIntent,
};
use kdl::{KdlDocument, KdlNode, KdlValue};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const INTEGER: &str = "must be an integer from 1 to 9007199254740991";
const SIZE: &str = "must be a positive size such as 8G or 512M";
const SCALAR: &str = "must have exactly one scalar without properties, annotations or children";
const FIELDS: [(&str, &str); 3] = [
    ("declared-priority", "declared_priority"),
    ("declared-lane", "declared_lane"),
    ("declared-budgets", "declared_budgets"),
];
const BUDGETS: [(&str, &str); 5] = [
    ("cpu-seconds", "cpu_seconds"),
    ("memory-max", "memory_max"),
    ("disk-max", "disk_max"),
    ("token-budget", "token_budget"),
    ("attention", "attention"),
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
            if !b.attention.is_empty() && self.0 != "mission" {
                return Err(self.error("declared_budgets.attention", "is allowed on missions only"));
            }
            let mut people = std::collections::BTreeSet::new();
            for entry in &b.attention {
                let field = format!("declared_budgets.attention[{}]", entry.person);
                if !crate::model::person_subject(&entry.person)
                    || fractal_schema::registry().validate_subject(&entry.person).is_err()
                {
                    return Err(self.error(&field, "must name a native person subject"));
                }
                if !people.insert(&entry.person) {
                    return Err(self.error(&field, "must occur once per person"));
                }
                self.number(&format!("{field}.asks"), Some(entry.asks))?;
            }
            for (field, value) in [
                ("declared_budgets.cpu_seconds", b.cpu_seconds),
                ("declared_budgets.token_budget", b.token_budget),
            ] {
                if let Some(value) = value {
                    if self.0 == "agent" {
                        return Err(self.error(
                            field,
                            if field == "declared_budgets.cpu_seconds" {
                                "cpu-seconds is allowed on missions only"
                            } else {
                                "token-budget is allowed on missions only"
                            },
                        ));
                    }
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

    fn decode<T: DeserializeOwned>(
        self,
        value: &Value,
        field: &str,
        requirement: &str,
    ) -> Result<Option<T>, FractalError> {
        value
            .get(field)
            .map(|v| T::deserialize(v).map_err(|_| self.error(field, requirement)))
            .transpose()
    }

    fn fields(self, value: &Value) -> Result<DeclaredAdmission, FractalError> {
        let priority = self.decode(value, "declared_priority", "must be background or normal")?;
        let lane = self.decode(value, "declared_lane", "must be check or build")?;
        let budgets = value
            .get("declared_budgets")
            .map(|value| {
                let object = value
                    .as_object()
                    .ok_or_else(|| self.error("declared_budgets", "must be an object"))?;
                for (key, value) in object {
                    let field = format!("declared_budgets.{key}");
                    match key.as_str() {
                        "cpu_seconds" | "token_budget" => {
                            self.number(&field, value.as_u64())?;
                        }
                        "memory_max" | "disk_max" => self.size(&field, value.as_str())?,
                        "attention" => {
                            let entries = value.as_array().ok_or_else(|| self.error(&field, "must be an array"))?;
                            for entry in entries {
                                let item = format!("{field}[{}]", entry.get("person").and_then(Value::as_str).unwrap_or("?"));
                                let object = entry.as_object().ok_or_else(|| self.error(&field, "must contain objects"))?;
                                if object.len() != 2 || !object.contains_key("person") || !object.contains_key("asks")
                                    || !entry["person"].is_string()
                                {
                                    return Err(self.error(&item, "must contain only person and asks"));
                                }
                                self.number(&format!("{item}.asks"), entry["asks"].as_u64())?;
                            }
                        }
                        _ => return Err(self.error(&field, "is not a recognized declared budget")),
                    }
                }
                Ok::<_, FractalError>(
                    DeclaredBudgets::deserialize(value).expect("validated budget fields"),
                )
            })
            .transpose()?;
        self.validate(priority.as_ref(), lane.as_ref(), budgets.as_ref())?;
        Ok(DeclaredAdmission {
            priority,
            lane,
            budgets: budgets.filter(|b| !b.is_empty()),
        })
    }

    fn consistent(
        self,
        typed: &DeclaredAdmission,
        canonical: &DeclaredAdmission,
    ) -> Result<(), FractalError> {
        for (field, agrees) in [
            ("declared_priority", typed.priority == canonical.priority),
            ("declared_lane", typed.lane == canonical.lane),
            ("declared_budgets", typed.budgets == canonical.budgets),
        ] {
            if !agrees {
                return Err(self.error(
                    field,
                    "must agree between the typed member and canonical declaration",
                ));
            }
        }
        Ok(())
    }

    fn desired(self, value: &Value) -> Result<DeclaredAdmission, FractalError> {
        self.fields(&Value::Object(self.children(value, false)?))
    }

    // Canonical root children and budget leaves share duplicate and shape validation.
    fn children(self, value: &Value, budget: bool) -> Result<Map<String, Value>, FractalError> {
        let mut fields = Map::new();
        for child in value
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = child.get("name").and_then(Value::as_str).unwrap_or("");
            if budget && name == "attention" {
                let field = "declared_budgets.attention";
                let args = child.get("arguments").and_then(Value::as_array);
                let properties = child.get("properties").and_then(Value::as_object);
                if args.is_none_or(|args| args.len() != 1 || !args[0].is_string())
                    || properties.is_none_or(|p| p.len() != 1 || !p.contains_key("asks"))
                    || child.get("type").is_some()
                    || child.get("children").is_some()
                {
                    return Err(self.error(field, "needs one person and only asks, without children or annotations"));
                }
                fields.entry("attention").or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut().expect("attention array").push(serde_json::json!({
                        "person": args.expect("validated arguments")[0],
                        "asks": properties.expect("validated properties")["asks"],
                    }));
                continue;
            }
            let rules = if budget {
                BUDGETS.as_slice()
            } else {
                FIELDS.as_slice()
            };
            let key = rules
                .iter()
                .find(|(syntax, _)| *syntax == name)
                .map(|(_, key)| *key);
            let Some(key) = key else {
                if budget {
                    return Err(self.error(
                        &format!("declared_budgets.{}", name.replace('-', "_")),
                        "is not a recognized declared budget",
                    ));
                }
                continue;
            };
            let field = if budget {
                format!("declared_budgets.{key}")
            } else {
                key.into()
            };
            if fields.contains_key(key) {
                return Err(self.error(&field, "must occur once"));
            }
            let decorated = child
                .get("properties")
                .is_some_and(|v| v.as_object().is_none_or(|v| !v.is_empty()))
                || child.get("type").is_some();
            if decorated {
                return Err(self.error(&field, SCALAR));
            }
            let args = child.get("arguments").and_then(Value::as_array);
            let value = if !budget && key == "declared_budgets" {
                if args.is_some_and(|v| !v.is_empty()) {
                    return Err(self.error(&field, "must be a block without arguments"));
                }
                Value::Object(self.children(child, true)?)
            } else {
                let children = child.get("children");
                let has_children =
                    children.is_some_and(|v| v.as_array().map_or(budget, |v| !v.is_empty()));
                if args.is_none_or(|v| v.len() != 1) || has_children {
                    return Err(self.error(&field, SCALAR));
                }
                args.expect("validated arguments")[0].clone()
            };
            fields.insert(key.into(), value);
        }
        Ok(fields)
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
                for entry in body.nodes().iter().filter(|n| n.name().value() == "attention") {
                    let field = "declared_budgets.attention";
                    let person = entry.entries().iter().find(|e| e.name().is_none());
                    let asks = entry.entries().iter().find(|e| e.name().is_some());
                    let (Some(person), Some(asks)) = (person, asks) else {
                        return Err(scope.error(field, "needs one person and asks"));
                    };
                    if entry.ty().is_some() || entry.children().is_some() || entry.entries().len() != 2
                        || person.ty().is_some() || asks.ty().is_some()
                        || asks.name().map(|n| n.value()) != Some("asks")
                    {
                        return Err(scope.error(field, "needs one person and only asks, without children or annotations"));
                    }
                    let person = person.value().as_string().ok_or_else(|| scope.error(field, "needs one person string"))?;
                    let asks_field = format!("{field}[{person}].asks");
                    let asks = match asks.value() {
                        KdlValue::Integer(value) => scope.number(&asks_field, u64::try_from(*value).ok())?,
                        _ => return Err(scope.error(&asks_field, INTEGER)),
                    };
                    budgets.attention.push(DeclaredAttentionBudget { person: person.into(), asks });
                }
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

pub fn validate_publication(intent: &NormalizedIntent) -> Result<(), FractalError> {
    for desired in intent.subjects.values() {
        let scope = Scope(&desired.kind, &desired.subject);
        let canonical = (desired.kind == "agent")
            .then(|| scope.desired(&desired.desired))
            .transpose()?;
        if let Some(member) = &desired.member {
            let (priority, lane, budgets) = (
                member.declared_priority.as_ref(),
                member.declared_lane.as_ref(),
                member.declared_budgets.as_ref(),
            );
            scope.validate(priority, lane, budgets)?;
            if let Some(canonical) = &canonical {
                let typed =
                    DeclaredAdmission::from_declaration(priority, lane, budgets).unwrap_or_default();
                scope.consistent(&typed, canonical)?;
            }
        }
    }
    for mission in intent.missions.values() {
        validate_mission(mission)?;
    }
    Ok(())
}

fn validate_mission(mission: &MissionSpec) -> Result<(), FractalError> {
    Scope("mission", &mission.subject).validate(
        mission.declared_priority.as_ref(),
        mission.declared_lane.as_ref(),
        mission.declared_budgets.as_ref(),
    )?;
    let source = |source: Option<&str>| {
        source
            .map(|s| validate_declaration_source(s, &mission.subject))
            .transpose()
    };
    source(mission.declarations_kdl.as_deref())?;
    for step in mission.steps.values() {
        source(step.declarations_kdl.as_deref())?;
        let nested = std::iter::once(step.nested_mission.as_deref()).chain(
            step.loop_spec.iter().flat_map(|s| {
                [
                    Some(s.round.as_ref()),
                    s.on_keep.as_deref(),
                    s.on_discard.as_deref(),
                ]
            }),
        );
        for nested in nested.flatten() {
            validate_mission(nested)?;
        }
    }
    Ok(())
}

fn validate_declaration_source(source: &str, owner: &str) -> Result<(), FractalError> {
    fn walk(document: &KdlDocument) -> Result<(), FractalError> {
        for node in document.nodes() {
            if let Some(children) = node.children() {
                let kind = node.name().value();
                if matches!(kind, "agent" | "mission" | "step")
                    && children.nodes().iter().any(|child| {
                        FIELDS.iter().any(|(name, _)| child.name().value() == *name)
                    })
                {
                    let name = node
                        .entries()
                        .iter()
                        .find(|e| e.name().is_none())
                        .and_then(|e| e.value().as_string())
                        .unwrap_or("");
                    let prefix = format!("{kind}/");
                    let subject = if name.starts_with(&prefix) {
                        name.to_owned()
                    } else {
                        format!("{prefix}{name}")
                    };
                    parse(children, kind, &subject)?;
                }
                walk(children)?;
            }
        }
        Ok(())
    }
    let document = source.parse::<KdlDocument>().map_err(|e| {
        Scope("mission", owner).error(
            "declared_admission",
            &format!("must contain valid declaration KDL: {e}"),
        )
    })?;
    walk(&document)
}

/// Persisted declaration locations, including envelopes that may coexist with empty decoys.
pub fn claim_payload_candidates(body: &Value) -> impl Iterator<Item = &Value> {
    [
        Some(body),
        body.get("fields"),
        body.get("body"),
        body.get("fields").and_then(|fields| fields.get("body")),
    ]
    .into_iter()
    .flatten()
}

/// Validate new claim input before typed decoding can discard nulls or unknown keys.
pub fn validate_claim(kind: &str, subject: &str, body: &Value) -> Result<(), FractalError> {
    for payload in claim_payload_candidates(body) {
        match kind {
            "intent.desired" => {
                let scope = Scope(
                    payload
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("agent"),
                    subject,
                );
                let canonical = (payload.get("kind").and_then(Value::as_str) == Some("agent"))
                    .then(|| scope.desired(payload.get("desired").unwrap_or(&Value::Null)))
                    .transpose()?;
                if let Some(member) = payload.get("member").filter(|v| !v.is_null()) {
                    let typed = scope.fields(member)?;
                    if let Some(canonical) = &canonical {
                        scope.consistent(&typed, canonical)?;
                    }
                }
            }
            "mission.published" => validate_mission_json(subject, payload)?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_mission_json(owner: &str, value: &Value) -> Result<(), FractalError> {
    let subject = value
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or(owner);
    Scope("mission", subject).fields(value)?;
    let source = |node: &Value| {
        node.get("declarations_kdl")
            .and_then(Value::as_str)
            .map(|s| validate_declaration_source(s, subject))
            .transpose()
    };
    source(value)?;
    for step in value
        .get("steps")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|s| s.values())
    {
        Scope("step", step.get("path").and_then(Value::as_str).unwrap_or(subject)).fields(step)?;
        source(step)?;
        let nested = std::iter::once(step.get("nested_mission")).chain(
            ["round", "on_keep", "on_discard"]
                .map(|key| step.get("loop_spec").and_then(|s| s.get(key))),
        );
        for nested in nested.flatten().filter(|v| !v.is_null()) {
            validate_mission_json(subject, nested)?;
        }
    }
    Ok(())
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
                parse(
                    &format!("declared-budgets {{ {child} {value}; }}")
                        .parse::<KdlDocument>()
                        .unwrap(),
                    "mission",
                    "mission/example",
                )
                .unwrap();
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
    fn agent_run_budgets_are_mission_only() {
        for (child, field) in [
            ("token-budget", "declared_budgets.token_budget"),
            ("cpu-seconds", "declared_budgets.cpu_seconds"),
        ] {
            let error =
                parse_fields(&format!("declared-budgets {{ {child} 300; }}")).unwrap_err();
            assert_eq!(error.code, "invalid-declared-admission");
            assert_eq!(error.details["subject"], "agent/example");
            assert_eq!(error.details["field"], field);
            assert!(
                error
                    .message
                    .ends_with(&format!("{child} is allowed on missions only"))
            );
        }
        let parsed =
            parse_fields("declared-budgets { memory-max \"8G\"; disk-max \"20G\"; }").unwrap();
        let budgets = parsed.budgets.unwrap();
        assert_eq!(budgets.memory_max.as_deref(), Some("8G"));
        assert_eq!(budgets.disk_max.as_deref(), Some("20G"));
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
        let parsed = parse(
            &"declared-priority \"background\"\ndeclared-lane \"build\"\ndeclared-budgets { cpu-seconds 300; memory-max \"0008G\"; disk-max \"20G\"; token-budget 100000; }".parse::<KdlDocument>().unwrap(),
            "mission",
            "mission/example",
        )
        .unwrap();
        let json = serde_json::to_value(&parsed).unwrap();
        assert_eq!(json["budgets"]["memory_max"], "0008G");
        assert_eq!(
            serde_json::from_value::<DeclaredAdmission>(json).unwrap(),
            parsed
        );
    }
}

#[cfg(test)]
mod claim_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claim_json_rejects_null_wrong_types_and_unknown_budget_keys() {
        for (fields, field) in [
            (json!({"declared_priority": null}), "declared_priority"),
            (json!({"declared_priority": "urgent"}), "declared_priority"),
            (json!({"declared_lane": 1}), "declared_lane"),
            (json!({"declared_lane": null}), "declared_lane"),
            (json!({"declared_budgets": null}), "declared_budgets"),
            (json!({"declared_budgets": []}), "declared_budgets"),
            (
                json!({"declared_budgets": {"cpu_seconds": null}}),
                "declared_budgets.cpu_seconds",
            ),
            (
                json!({"declared_budgets": {"cpu_seconds": 1.5}}),
                "declared_budgets.cpu_seconds",
            ),
            (
                json!({"declared_budgets": {"token_budget": "1"}}),
                "declared_budgets.token_budget",
            ),
            (
                json!({"declared_budgets": {"memory_max": 8}}),
                "declared_budgets.memory_max",
            ),
            (
                json!({"declared_budgets": {"disk_max": null}}),
                "declared_budgets.disk_max",
            ),
            (
                json!({"declared_budgets": {"extra": 1}}),
                "declared_budgets.extra",
            ),
        ] {
            for (kind, subject, body) in [
                (
                    "intent.desired",
                    "agent/test",
                    json!({"fields": {"body": {"kind": "agent", "member": fields.clone()}}}),
                ),
                (
                    "mission.published",
                    "mission/test",
                    json!({"fields": {"body": fields}}),
                ),
            ] {
                let error = validate_claim(kind, subject, &body).unwrap_err();
                assert_eq!(error.code, "invalid-declared-admission");
                assert_eq!(error.details["subject"], subject);
                assert_eq!(error.details["field"], field);
            }
        }
    }

    #[test]
    fn nested_mission_claim_fields_are_checked_before_decoding() {
        for nested in [
            json!({"nested_mission": {"subject": "mission/nested", "declared_lane": null}}),
            json!({"loop_spec": {"round": {"subject": "mission/nested", "declared_lane": null}}}),
            json!({"loop_spec": {"on_keep": {"subject": "mission/nested", "declared_lane": null}}}),
            json!({"loop_spec": {"on_discard": {"subject": "mission/nested", "declared_lane": null}}}),
        ] {
            let body = json!({"fields": {"body": {"steps": {"work": nested}}}});
            let error = validate_claim("mission.published", "mission/test", &body).unwrap_err();
            assert_eq!(error.details["subject"], "mission/nested");
            assert_eq!(error.details["field"], "declared_lane");
        }
    }

    #[test]
    fn canonical_desired_claim_admission_is_validated_without_member() {
        for child in [
            json!({"name": "declared-priority", "arguments": ["urgent"]}),
            json!({"name": "declared-priority", "arguments": [null]}),
            json!({"name": "declared-lane", "arguments": ["check", "build"]}),
            json!({"name": "declared-budgets", "arguments": [1]}),
            json!({"name": "declared-budgets", "children": [{"name": "disk-max", "arguments": ["16777216T"]}]}),
            json!({"name": "declared-budgets", "children": [{"name": "extra", "arguments": [1]}]}),
            json!({"name": "declared-budgets", "children": [{"name": "cpu-seconds", "arguments": [1], "properties": {"extra": 1}}]}),
        ] {
            let body =
                json!({"fields": {"body": {"kind": "agent", "desired": {"children": [child]}}}});
            assert_eq!(
                validate_claim("intent.desired", "agent/test", &body)
                    .unwrap_err()
                    .code,
                "invalid-declared-admission"
            );
        }
        let body = json!({"fields": {"body": {"kind": "agent", "desired": {"children": [
            {"name": "declared-priority", "arguments": ["normal"]},
            {"name": "declared-lane", "arguments": ["check"]},
            {"name": "declared-budgets", "children": [{"name": "memory-max", "arguments": ["8G"]}]}
        ]}}}});
        validate_claim("intent.desired", "agent/test", &body).unwrap();
    }

    #[test]
    fn agent_run_budgets_are_refused_in_canonical_and_typed_claims() {
        for (child, key) in [
            ("token-budget", "token_budget"),
            ("cpu-seconds", "cpu_seconds"),
        ] {
            for body in [
                json!({"kind": "agent", "desired": {"children": [
                    {"name": "declared-budgets", "children": [
                        {"name": child, "arguments": [300]}
                    ]}
                ]}}),
                json!({"kind": "agent", "member": {"declared_budgets": {key: 300}}}),
            ] {
                let error = validate_claim("intent.desired", "agent/test", &body).unwrap_err();
                assert_eq!(error.code, "invalid-declared-admission");
                assert_eq!(error.details["subject"], "agent/test");
                assert_eq!(error.details["field"], format!("declared_budgets.{key}"));
                assert!(error.message.ends_with("is allowed on missions only"));
            }
        }
    }

    #[test]
    fn old_payloads_are_not_newly_decoded_or_rejected() {
        for (kind, body) in [
            (
                "intent.desired",
                json!({"fields": {"body": {"member": {"old_unknown": 1}}}}),
            ),
            (
                "mission.published",
                json!({"fields": {"body": {"old_unknown": 1}}}),
            ),
            ("other", json!({"declared_budgets": null})),
        ] {
            validate_claim(kind, "subject/test", &body).unwrap();
        }
    }
}

#[cfg(test)]
mod deferred_claim_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deferred_agent_admission_is_checked_at_mission_and_step_scope() {
        for source in [
            r#"agent "worker" { declared-priority "urgent"; }"#,
            r#"host "node" { agent "worker" { declared-budgets { cpu-seconds 0; }; }; }"#,
            r#"agent "${ST_MISSION_RUN}/worker" { declared-lane "future"; }"#,
            r#"agent "worker" { "declared\u{2d}priority" "urgent"; }"#,
            r#"agent "worker" { declared-budgets { token-budget 300; }; }"#,
            r#"host "node" { agent "worker" { declared-budgets { cpu-seconds 300; }; }; }"#,
            r#"agent "worker" { declared-budgets { attention "person/reviewer" asks=3; }; }"#,
        ] {
            for mission in [
                json!({"declarations_kdl": source}),
                json!({"steps": {"work": {"declarations_kdl": source}}}),
            ] {
                let error = validate_claim(
                    "mission.published",
                    "mission/test",
                    &json!({"fields": {"body": mission}}),
                )
                .unwrap_err();
                assert_eq!(error.code, "invalid-declared-admission");
                assert!(
                    error.details["subject"]
                        .as_str()
                        .unwrap()
                        .starts_with("agent/")
                );
            }
        }
    }

    #[test]
    fn admission_only_deferred_validation_leaves_unrelated_authoring_alone() {
        validate_declaration_source(
            r#"resource "example" { "declared\u{2d}priority" "unrelated"; }"#,
            "mission/test",
        ).unwrap();
        validate_declaration_source(
            r#"agent "worker" { unrelated-existing-field "anything"; declared-priority "normal"; declared-budgets { memory-max "8G"; }; }"#,
            "mission/test",
        ).unwrap();
    }
}

#[cfg(test)]
mod consistency_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn direct_claim_member_and_canonical_metadata_must_agree() {
        let raw = json!({"children": [{"name": "declared-priority", "arguments": ["normal"]}]});
        for (member, desired) in [
            (json!({"declared_priority": "normal"}), json!({})),
            (json!({}), raw.clone()),
        ] {
            let body = json!({"fields": {"body": {"kind": "agent", "member": member, "desired": desired}}});
            let error = validate_claim("intent.desired", "agent/test", &body).unwrap_err();
            assert_eq!(error.code, "invalid-declared-admission");
            assert_eq!(error.details["field"], "declared_priority");
        }
        for body in [
            json!({"kind": "agent", "member": {"declared_priority": "normal"}, "desired": raw.clone()}),
            json!({"kind": "agent", "member": null, "desired": raw}),
            json!({"kind": "agent", "member": {"declared_budgets": {}}, "desired": {}}),
            json!({"kind": "agent", "member": {}, "desired": {"children": [{"name": "declared-budgets"}]}}),
        ] {
            validate_claim("intent.desired", "agent/test", &body).unwrap();
        }
    }

    #[test]
    fn every_persisted_candidate_is_validated_despite_empty_decoys() {
        for priority in ["normal", "urgent"] {
            for (kind, payload) in [
                ("intent.desired", json!({"kind": "agent", "desired": {"children": [
                    {"name": "declared-priority", "arguments": [priority]}
                ]}})),
                ("intent.desired", json!({"kind": "resource", "member": {"declared_priority": priority}})),
                ("mission.published", json!({"declared_priority": priority})),
            ] {
                for body in [
                    payload.clone(),
                    json!({"fields": payload, "body": {}}),
                    json!({"body": payload, "fields": {}}),
                    json!({"fields": {"body": payload}, "body": {}}),
                ] {
                    let result = validate_claim(kind, "subject/test", &body);
                    if priority == "normal" {
                        result.unwrap();
                    } else {
                        assert_eq!(result.unwrap_err().code, "invalid-declared-admission");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod attention_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attention_syntax_and_scope() {
        let good = r#"declared-budgets { attention "person/reviewer" asks=1; attention "person/a" asks=9007199254740991; }"#;
        let document: KdlDocument = good.parse().unwrap();
        let budgets = parse(&document, "mission", "mission/test").unwrap().budgets.unwrap();
        assert_eq!(serde_json::to_value(&budgets).unwrap(), json!({"attention": [
            {"person":"person/reviewer", "asks":1}, {"person":"person/a", "asks":MAX_SAFE_INTEGER}
        ]}));
        assert_eq!(DeclaredBudgets::deserialize(serde_json::to_value(&budgets).unwrap()).unwrap(), budgets);
        for kind in ["agent", "step"] {
            let error = parse(&document, kind, "test/owner").unwrap_err();
            assert_eq!(error.code, "invalid-declared-admission");
            assert_eq!(error.details["subject"], "test/owner");
            assert_eq!(error.details["field"], "declared_budgets.attention");
        }
        for child in [
            r#"attention "person/a""#, r#"attention "person/a" asks=#null"#,
            r#"attention "person/a" asks="1""#, r#"attention "person/a" asks=1.5"#,
            r#"attention "person/a" asks=0"#, r#"attention "person/a" asks=-1"#,
            r#"attention "person/a" asks=9007199254740992"#,
            r#"attention #null asks=1"#, r#"attention 1 asks=1"#,
            r#"attention "person/a" "extra" asks=1"#, r#"attention "person/a" asks=1 extra=2"#,
            r#"attention "person/a" asks=1 {}"#, r#"(meta)attention "person/a" asks=1"#,
            r#"attention (meta)"person/a" asks=1"#, r#"attention "person/a" asks=(meta)1"#,
            r#"attention "agent/a" asks=1"#, r#"attention "person/a/b" asks=1"#,
            r#"attention "person/" asks=1"#, r#"attention "person/a b" asks=1"#,
            r#"attention "person/a" asks=1; attention "person/a" asks=2"#,
        ] {
            let document = format!("declared-budgets {{ {child}; }}").parse().unwrap();
            let error = parse(&document, "mission", "mission/test").unwrap_err();
            assert_eq!(error.code, "invalid-declared-admission", "{child}");
            assert_eq!(error.details["subject"], "mission/test");
            assert!(error.details["field"].as_str().unwrap().starts_with("declared_budgets.attention"));
        }
    }

    #[test]
    fn attention_claim_candidates_refuse_before_decoding() {
        for attention in [
            Value::Null, json!({}), json!([null]), json!([{"person":"person/a"}]),
            json!([{"person":"person/a", "asks":null}]), json!([{"person":"person/a", "asks":0}]),
            json!([{"person":"person/a", "asks":1, "extra":2}]),
            json!([{"person":"person/a", "asks":1}, {"person":"person/a", "asks":2}]),
        ] {
            let payload = json!({"declared_budgets":{"attention":attention}});
            for body in [payload.clone(), json!({"fields":payload,"body":{}}),
                json!({"body":payload,"fields":{}}), json!({"fields":{"body":payload},"body":{}})] {
                let error = validate_claim("mission.published", "mission/test", &body).unwrap_err();
                assert_eq!(error.code, "invalid-declared-admission");
                assert_eq!(error.details["subject"], "mission/test");
                assert!(error.details["field"].as_str().unwrap().starts_with("declared_budgets.attention"));
            }
        }
        let attention = json!({"name":"attention","arguments":["person/a"],"properties":{"asks":1}});
        for payload in [
            json!({"kind":"agent","member":{"declared_budgets":{"attention":[{"person":"person/a","asks":1}]}}}),
            json!({"kind":"agent","desired":{"children":[{"name":"declared-budgets","children":[attention]}]}}),
        ] {
            for body in [payload.clone(), json!({"fields":payload,"body":{}}),
                json!({"body":payload,"fields":{}}), json!({"fields":{"body":payload},"body":{}})] {
                let error = validate_claim("intent.desired", "agent/test", &body).unwrap_err();
                assert_eq!(error.code, "invalid-declared-admission");
                assert_eq!(error.details["subject"], "agent/test");
                assert_eq!(error.details["field"], "declared_budgets.attention");
            }
        }
        for value in [json!({"attention":null}), json!({"future_budget":1})] {
            assert!(DeclaredBudgets::deserialize(value).is_err());
        }
    }
    #[test]
    fn nested_attention_and_deferred_agents_follow_all_mission_paths() {
        let good = json!({"declared_budgets":{"attention":[{"person":"person/a","asks":1}]}});
        let deferred = r#"agent "worker" { declared-budgets { attention "person/a" asks=1; }; }"#;
        for mission in [good.clone(), json!({"declarations_kdl":deferred}),
            json!({"steps":{"work":{"declarations_kdl":deferred}}})] {
            for step in [
                json!({"nested_mission":mission}),
                json!({"loop_spec":{"round":mission}}),
                json!({"loop_spec":{"on_keep":mission}}),
                json!({"loop_spec":{"on_discard":mission}}),
            ] {
                let result = validate_claim("mission.published", "mission/test", &json!({"steps":{"work":step}}));
                if mission == good { result.unwrap(); } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.code, "invalid-declared-admission");
                    assert_eq!(error.details["subject"], "agent/worker");
                    assert_eq!(error.details["field"], "declared_budgets.attention");
                }
            }
        }
    }
}
