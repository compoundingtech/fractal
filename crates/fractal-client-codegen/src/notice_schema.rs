//! Dependency-closed notice schema traversal; unsupported constructs fail closed.
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub fn closure(schema: &Value) -> Result<BTreeMap<String, Value>> {
    let defs = schema["$defs"].as_object().context("schema definitions")?;
    let mut closed = BTreeMap::new();
    for name in defs.keys().filter(|name| name.starts_with("Notice")) {
        visit(name, defs, &mut closed, &mut BTreeSet::new())?;
    }
    ensure!(!closed.is_empty(), "missing notice definitions");
    Ok(closed)
}

fn visit(
    name: &str,
    defs: &serde_json::Map<String, Value>,
    closed: &mut BTreeMap<String, Value>,
    active: &mut BTreeSet<String>,
) -> Result<()> {
    ensure!(!active.contains(name), "cyclic notice reference: {name}");
    if closed.contains_key(name) {
        return Ok(());
    }
    let node = defs
        .get(name)
        .with_context(|| format!("missing reference {name}"))?;
    active.insert(name.to_owned());
    walk(node, defs, closed, active)?;
    active.remove(name);
    closed.insert(name.to_owned(), node.clone());
    Ok(())
}

fn walk(
    node: &Value,
    defs: &serde_json::Map<String, Value>,
    closed: &mut BTreeMap<String, Value>,
    active: &mut BTreeSet<String>,
) -> Result<()> {
    let object = node
        .as_object()
        .context("notice schema must be an object")?;
    let annotations = [
        "description",
        "examples",
        "x-st-codec",
        "x-st-ref",
        "x-st-brand",
    ];
    let keywords: &[&str] = if node.get("$ref").is_some() {
        &["$ref"]
    } else if node.get("oneOf").is_some() {
        &["oneOf"]
    } else {
        match node["type"].as_str() {
            Some("object") => &["type", "properties", "required", "additionalProperties"],
            Some("array") => &["type", "items", "minItems", "maxItems", "uniqueItems"],
            Some("string") => &["type", "format", "pattern", "minLength", "maxLength"],
            Some("boolean" | "null") => &["type"],
            None if node.get("const").is_some() => &["const"],
            None if node.get("enum").is_some() => &["enum"],
            _ => bail!("unsupported notice shape: {node}"),
        }
    };
    for key in object.keys() {
        ensure!(
            annotations.contains(&key.as_str()) || keywords.contains(&key.as_str()),
            "unsupported notice keyword: {key}"
        );
    }
    if let Some(reference) = node.get("$ref") {
        visit(
            reference
                .as_str()
                .and_then(|s| s.strip_prefix("#/$defs/"))
                .context("nonlocal notice reference")?,
            defs,
            closed,
            active,
        )?;
    } else if let Some(branches) = node.get("oneOf") {
        let branches = branches.as_array().context("oneOf array")?;
        ensure!(!branches.is_empty(), "empty notice union");
        for branch in branches {
            walk(branch, defs, closed, active)?;
        }
    } else {
        match node["type"].as_str() {
            Some("object") => {
                ensure!(
                    node["additionalProperties"] == false,
                    "notice objects must be closed"
                );
                let properties = node["properties"]
                    .as_object()
                    .context("object properties")?;
                for required in node["required"].as_array().context("required array")? {
                    ensure!(
                        properties.contains_key(required.as_str().context("required name")?),
                        "missing required property"
                    );
                }
                for child in properties.values() {
                    walk(child, defs, closed, active)?;
                }
            }
            Some("array") => walk(&node["items"], defs, closed, active)?,
            None if node.get("const").is_some() => ensure!(
                node["const"].is_string() || node["const"].is_u64(),
                "unsupported constant"
            ),
            None if node.get("enum").is_some() => {
                let values = node["enum"].as_array().context("enum array")?;
                ensure!(
                    !values.is_empty() && values.iter().all(Value::is_string),
                    "unsupported enum"
                );
            }
            _ => (),
        }
        if let Some(format) = node.get("format") {
            ensure!(
                ["date-time", "uri-reference"].contains(&format.as_str().unwrap_or("")),
                "unsupported notice format"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn notice_contract_closure() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../docs/st3/client-v0/schemas/client-v0.schema.json"
        ))
        .unwrap();
        let closed = closure(&schema).unwrap();
        assert!(closed.contains_key("Revision"));
        assert!(closed.contains_key("NoticeSnapshot"));
        assert!(!closed.contains_key("Resource"));
    }
    #[test]
    fn notice_dependency_closure_and_rejection() {
        let mut schema = json!({"$defs": {"NoticeRoot": {"type":"array", "items":{"$ref":"#/$defs/Nested"}}, "Nested":{"type":"string"}}});
        assert!(closure(&schema).unwrap().contains_key("Nested"));
        for unsupported in [
            json!({"allOf": []}),
            json!({"type":"string", "minimum": 1}),
            json!({"type":"object", "additionalProperties":true}),
            json!({"enum":[1]}),
        ] {
            schema["$defs"]["Nested"] = unsupported;
            assert!(closure(&schema).is_err());
        }
        schema["$defs"].as_object_mut().unwrap().remove("Nested");
        assert!(
            closure(&schema)
                .unwrap_err()
                .to_string()
                .contains("missing reference")
        );
        schema["$defs"]["Nested"] = json!({"$ref":"#/$defs/NoticeRoot"});
        assert!(closure(&schema).unwrap_err().to_string().contains("cyclic"));
    }
}
