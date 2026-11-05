//! Canonical gate nodes for declaration reads. Environment values never leave this renderer.
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use crate::model::{GateSpec, St3Error};

pub(crate) fn node(name: &str, args: impl IntoIterator<Item = KdlValue>) -> KdlNode {
    let mut node = KdlNode::new(name);
    node.entries_mut().extend(args.into_iter().map(KdlEntry::new));
    node
}

pub(crate) fn text(name: &str, value: &str) -> KdlNode {
    node(name, [value.into()])
}

pub(crate) fn block(mut node: KdlNode, children: Vec<KdlNode>) -> KdlNode {
    let mut body = KdlDocument::new();
    body.nodes_mut().extend(children);
    node.set_children(body);
    node
}

pub(crate) fn property(node: &mut KdlNode, name: &str, value: impl Into<KdlValue>) {
    node.entries_mut().push(KdlEntry::new_prop(name, value));
}

pub(crate) fn duration(name: &str, ms: u64) -> KdlNode {
    text(name, &format!("{ms}ms"))
}

pub(crate) fn scalar(value: &serde_json::Value) -> Result<KdlValue, St3Error> {
    use serde_json::Value;
    Ok(match value {
        Value::Null => KdlValue::Null,
        Value::Bool(v) => (*v).into(),
        Value::String(v) => v.as_str().into(),
        Value::Number(v) => {
            if let Some(v) = v.as_i64() { i128::from(v).into() }
            else if let Some(v) = v.as_u64() { i128::from(v).into() }
            else { KdlValue::Float(v.as_f64().ok_or_else(|| unsupported("number"))?) }
        }
        Value::Array(_) | Value::Object(_) => return Err(unsupported("non-scalar predicate value")),
    })
}

fn unsupported(field: &str) -> St3Error {
    St3Error::new("unrenderable-mission", format!("canonical KDL cannot represent {field}"))
}

/// Render every gate variant. Unsupported structured predicate values fail closed rather than
/// disappearing or being coerced to strings. The result is normalized KDL, not authored bytes.
pub fn render_gate_node(gate: &GateSpec) -> Result<KdlNode, St3Error> {
    use GateSpec::*;
    let mut root = text("gate", crate::gate_parse::gate_name(gate));
    let children = match gate {
        Exists { subject, .. } => vec![text("exists", subject)],
        Empty { subject, .. } => vec![text("empty", subject)],
        Has { subject, text: value, .. } | Lacks { subject, text: value, .. } => {
            vec![node(if matches!(gate, Has { .. }) { "has" } else { "lacks" },
                [subject.as_str().into(), value.as_str().into()])]
        }
        Field { path, subject, operator, value, .. } => vec![node("field", [
            path.as_str().into(), subject.as_str().into(), operator.as_str().into(), scalar(value)?,
        ])],
        Every { path, subject, fields, .. } | NotEvery { path, subject, fields, .. } => {
            let predicates = fields.iter().map(|field| Ok(node("field", [
                field.path.as_str().into(), field.operator.as_str().into(), scalar(&field.value)?,
            ]))).collect::<Result<Vec<_>, St3Error>>()?;
            vec![block(node(if matches!(gate, Every { .. }) { "every" } else { "not-every" },
                [path.as_str().into(), subject.as_str().into()]), predicates)]
        }
        Deadline { duration_ms, .. } => vec![duration("deadline", *duration_ms)],
        Mechanical { command, host, workspace, environment, time_limit_ms, .. }
        | Llm { prompt: command, host, workspace, environment, time_limit_ms, .. } => {
            let mut children = vec![text("host", host), text("workspace", workspace)];
            children.push(block(node("env", []), environment.keys()
                .map(|name| text(name, "<redacted>")).collect()));
            children.push(duration("time-limit", *time_limit_ms));
            if let Llm { model, tools, token_budget, .. } = gate {
                property(&mut root, "type", "llm");
                children.extend([text("model", model), node("tools", tools.iter().map(|s| s.as_str().into())),
                    node("token-budget", [i128::from(*token_budget).into()]), text("prompt", command)]);
            } else {
                children.push(text("exec", command));
            }
            children
        }
        Human { reviewer, mode, question, review_targets, .. } => {
            property(&mut root, "type", "human");
            if let Some(mode) = mode { property(&mut root, "mode", mode.as_str()); }
            let mut children = vec![text("reviewer", reviewer)];
            children.extend(question.iter().map(|q| text("question", q)));
            children.extend(review_targets.iter().map(|target| text("review", target)));
            children
        }
    };
    Ok(block(root, children))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate_parse::parse_gate;

    #[test]
    fn canonical_gate_variants_round_trip_and_redact() {
        let sources = [
            r#"gate "exists" { exists "file/node:/test"; }"#,
            r#"gate "empty" { empty "mission-run/test/run"; }"#,
            r#"gate "has" { has "file/node:/test" "line\n\"quoted\""; }"#,
            r#"gate "lacks" { lacks "doc/test" "missing"; }"#,
            r#"gate "field" { field "state" "file/node:/test" "is" #null; }"#,
            r#"gate "every" { every "items" "file/node:/test" { field "ok" "is" #true; }; }"#,
            r#"gate "not-every" { not-every "items" "file/node:/test" { field "n" "is" 42; }; }"#,
            r#"gate "deadline" { deadline "123ms"; }"#,
            r#"gate "exec" { exec "true"; host "node"; workspace "."; env { TOKEN "secret"; }; }"#,
            r#"gate "llm" type="llm" { model "model"; host "node"; workspace "."; tools "shell"; env { TOKEN "secret"; }; token-budget 12; time-limit "1s"; prompt "check"; }"#,
            r#"gate "human" type="human" mode="feedback" { reviewer "person/test"; question "ready?"; review "file/node:/test"; }"#,
        ];
        for source in sources {
            let document: KdlDocument = source.parse().unwrap();
            let mut expected = parse_gate(&document.nodes()[0], "node").unwrap();
            if let GateSpec::Mechanical { environment, .. } | GateSpec::Llm { environment, .. } = &mut expected {
                for value in environment.values_mut() { *value = "<redacted>".into(); }
            }
            let rendered = render_gate_node(&parse_gate(&document.nodes()[0], "node").unwrap()).unwrap();
            assert!(!rendered.to_string().contains("secret"));
            assert_eq!(parse_gate(&rendered, "node").unwrap(), expected);
            assert_eq!(render_gate_node(&expected).unwrap(), rendered);
        }
    }

    #[test]
    fn structured_predicate_values_are_not_silently_lost() {
        let gate = GateSpec::Field { name: "structured".into(), path: "data".into(),
            subject: "file/node:/test".into(), operator: "is".into(), value: serde_json::json!([1, 2]) };
        assert!(render_gate_node(&gate).is_err());
        assert_eq!(scalar(&serde_json::json!(u64::MAX)).unwrap(), KdlValue::Integer(i128::from(u64::MAX)));
    }
}
