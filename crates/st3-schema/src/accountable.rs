//! A root's declared trace principal. This fact never grants runtime authority.
use serde_json::Value;

use crate::{ValidationError, error};

fn invalid(message: &str) -> ValidationError {
    error("invalid-accountable-root", message)
}

pub fn person(value: &str) -> bool {
    value.strip_prefix("person/").is_some_and(|name| {
        !name.is_empty()
            && name != ".."
            && crate::registry().validate_subject(value).is_ok()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    })
}

/// Read the canonical KDL child, rejecting malformed raw declarations as well as KDL.
pub fn from_desired(desired: &Value) -> Result<Option<&str>, ValidationError> {
    let Some(children) = desired.get("children").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut nodes = children.iter().filter(|node| node["name"] == "accountable");
    let Some(node) = nodes.next() else {
        return Ok(None);
    };
    if nodes.next().is_some()
        || node["arguments"]
            .as_array()
            .is_none_or(|args| args.len() != 1)
        || node
            .get("properties")
            .is_some_and(|v| v.as_object().is_none_or(|p| !p.is_empty()))
        || node
            .get("children")
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|c| !c.is_empty()))
    {
        return Err(invalid(
            "accountable needs exactly one person argument and no properties or children",
        ));
    }
    let value = node["arguments"][0]
        .as_str()
        .filter(|value| person(value))
        .ok_or_else(|| invalid("accountable must name one concrete person/NAME"))?;
    Ok(Some(value))
}

/// The schema fields envelope fences older members; every copy must agree with KDL.
pub fn validate_claim(
    subject: &str,
    kind: &str,
    body: &Value,
    actor: Option<&str>,
) -> Result<(), ValidationError> {
    if kind != "intent.desired" {
        return Ok(());
    }
    let declared = from_desired(&body["desired"])?;
    let stored = body.get("accountable");
    let fields = body.get("fields");
    let field_person = fields.and_then(|f| f.get("accountable"));
    let field_child = fields
        .map(|f| from_desired(&f["desired"]))
        .transpose()?
        .flatten();
    if declared.is_none() && stored.is_none() && field_person.is_none() && field_child.is_none() {
        return Ok(());
    }
    if field_person != stored
        || field_child != declared
        || fields.is_none_or(|f| f["desired"] != body["desired"] || f["kind"] != body["kind"])
    {
        return Err(invalid(
            "accountable roots need matching schema fields and a complete declaration body",
        ));
    }
    if stored.and_then(Value::as_str) != declared || declared.is_none() {
        return Err(invalid(
            "the accountable claim field must match its canonical declaration",
        ));
    }
    if !subject.starts_with("agent/")
        || body["subject"] != subject
        || body["kind"] != "agent"
        || body.get("owner_run").is_some_and(|v| !v.is_null())
        || body.get("owner_generation").is_some_and(|v| !v.is_null())
        || body.get("owner_step").is_some_and(|v| !v.is_null())
        || body["desired"]["name"] != "agent"
    {
        return Err(invalid(
            "accountable is valid only on a top-level agent root",
        ));
    }
    if actor != declared {
        return Err(error(
            "accountable-root-forbidden",
            "only the named accountable person may author this root; delegated declaration is not supported",
        ));
    }
    Ok(())
}
