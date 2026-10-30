//! The shared syntax of declaration KDL: reading a node's arguments, properties and children,
//! and validating the names, subjects and maps they carry. Intent parsing (`graph`), mission
//! parsing (`mission`) and gate parsing (`gate_parse`) all read declarations through these.
use std::collections::{BTreeMap, HashSet};

use kdl::{KdlDocument, KdlNode, KdlValue};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::model::St3Error;

/// The context variables st sets for mission steps and gates (`mission`).
pub const VARIABLES: &[&str] = &[
    "ST_MISSION",
    "ST_MISSION_REVISION",
    "ST_MISSION_RUN",
    "ST_RUN_GENERATION",
    "ST_ROOT_MISSION_RUN",
    "ST_ROOT_MISSION_RUN_ID",
    "ST_WORKSPACE",
    "ST_REQUESTER",
    "ST_STEP",
    "ST_STEP_RUN",
    "ST_ATTEMPT",
    "ST_ASSIGNEE",
    "ST_PARENT_STEP_RUN",
    "ST_GATE",
    "ST_AGENT",
    "ST_LOOP_ROUND",
    "ST_LOOP_FEEDBACK",
    "ST_LOOP_ITEM_ID",
    "ST_CANDIDATE_INDEX",
    "loop.round",
    "loop.feedback",
    "loop.item.id",
    "candidate.index",
    "PATH",
];

pub fn is_reserved_context_name(name: &str) -> bool {
    (VARIABLES.contains(&name) && name != "PATH") || name == "ST3_SUBJECT"
}

pub fn child_integer(document: &KdlDocument, name: &str) -> Result<Option<i128>, St3Error> {
    let Some(node) = unique_child(document, name)? else {
        return Ok(None);
    };
    ensure_no_properties(node)?;
    ensure_no_children(node)?;
    let [entry] = node.entries() else {
        return Err(St3Error::new(
            "wrong-argument-count",
            format!("child `{name}` needs one integer"),
        ));
    };
    match entry.value() {
        KdlValue::Integer(value) => Ok(Some(*value)),
        _ => Err(St3Error::new(
            "expected-integer",
            format!("child `{name}` needs one integer"),
        )),
    }
}

pub fn child_string(document: &KdlDocument, name: &str) -> Result<Option<String>, St3Error> {
    unique_child(document, name)?.map(one_string).transpose()
}

pub fn child_strings(document: &KdlDocument, name: &str) -> Result<Option<Vec<String>>, St3Error> {
    unique_child(document, name)?
        .map(positional_strings)
        .transpose()
}

pub fn ensure_bare(node: &KdlNode) -> Result<(), St3Error> {
    reject_type(node)?;
    if !node.entries().is_empty() {
        return Err(St3Error::new(
            "unexpected-value",
            format!("node `{}` cannot have values", node.name().value()),
        ));
    }
    Ok(())
}

pub fn ensure_no_children(node: &KdlNode) -> Result<(), St3Error> {
    if node.children().is_some() {
        Err(St3Error::new(
            "unexpected-children",
            format!("node `{}` cannot have children", node.name().value()),
        ))
    } else {
        Ok(())
    }
}

pub fn ensure_no_properties(node: &KdlNode) -> Result<(), St3Error> {
    ensure_only_properties(node, &[])
}

pub fn ensure_only_properties(node: &KdlNode, allowed: &[&str]) -> Result<(), St3Error> {
    reject_type(node)?;
    let mut seen = HashSet::new();
    for entry in node.entries().iter().filter(|entry| entry.name().is_some()) {
        let name = entry.name().expect("filtered").value();
        if !allowed.contains(&name) {
            return Err(St3Error::new(
                "unknown-property",
                format!(
                    "node `{}` does not accept property `{name}`",
                    node.name().value()
                ),
            ));
        }
        if !seen.insert(name) {
            return Err(St3Error::new(
                "duplicate-property",
                format!("node `{}` repeats property `{name}`", node.name().value()),
            ));
        }
    }
    Ok(())
}

pub fn first_string(node: &KdlNode) -> Result<String, St3Error> {
    positional_values(node)
        .first()
        .ok_or_else(|| {
            St3Error::new(
                "missing-argument",
                format!("node `{}` needs a name", node.name().value()),
            )
        })
        .and_then(|value| value_string(value))
}

pub fn json_value(value: &KdlValue) -> Result<Value, St3Error> {
    Ok(match value {
        KdlValue::String(value) => Value::String(value.clone()),
        KdlValue::Integer(value) => {
            let integer = i64::try_from(*value).map_err(|_| {
                St3Error::new(
                    "integer-range",
                    "a KDL integer must fit a signed 64-bit value",
                )
            })?;
            Value::Number(integer.into())
        }
        KdlValue::Float(value) => Value::Number(
            serde_json::Number::from_f64(*value)
                .ok_or_else(|| St3Error::new("invalid-number", "a float must be finite"))?,
        ),
        KdlValue::Bool(value) => Value::Bool(*value),
        KdlValue::Null => Value::Null,
    })
}

pub fn one_string(node: &KdlNode) -> Result<String, St3Error> {
    ensure_no_children(node)?;
    one_string_with_children(node)
}

pub fn one_string_with_children(node: &KdlNode) -> Result<String, St3Error> {
    first_string(node).and_then(|first| {
        if positional_values(node).len() == 1 {
            Ok(first)
        } else {
            Err(St3Error::new(
                "wrong-argument-count",
                format!(
                    "node `{}` needs one string; received `{}`",
                    node.name().value(),
                    node
                ),
            ))
        }
    })
}

pub fn parse_duration(value: &str, positive: bool) -> Result<u64, St3Error> {
    let (number, multiplier) = if let Some(value) = value.strip_suffix("ms") {
        (value, 1)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1_000)
    } else if let Some(value) = value.strip_suffix('m') {
        (value, 60_000)
    } else if let Some(value) = value.strip_suffix('h') {
        (value, 3_600_000)
    } else if let Some(value) = value.strip_suffix('d') {
        (value, 86_400_000)
    } else {
        (value, 1_000)
    };
    let number = number
        .parse::<u64>()
        .map_err(|_| St3Error::new("invalid-duration", format!("invalid duration `{value}`")))?;
    let duration = number
        .checked_mul(multiplier)
        .ok_or_else(|| St3Error::new("duration-range", "duration is too large"))?;
    if positive && duration == 0 {
        return Err(St3Error::new(
            "invalid-duration",
            "a required duration must be positive",
        ));
    }
    Ok(duration)
}

pub fn parse_map_child(
    document: &KdlDocument,
    name: &str,
) -> Result<BTreeMap<String, String>, St3Error> {
    let Some(node) = unique_child(document, name)? else {
        return Ok(BTreeMap::new());
    };
    ensure_bare(node)?;
    let Some(children) = node.children() else {
        return Ok(BTreeMap::new());
    };
    let mut output = BTreeMap::new();
    for child in children.nodes() {
        let value = one_string(child)?;
        let key = child.name().value().to_owned();
        if output.insert(key.clone(), value).is_some() {
            return Err(St3Error::new(
                "duplicate-map-key",
                format!("map key `{key}` repeats"),
            ));
        }
    }
    Ok(output)
}

pub fn placement_host(host: String, default_host: &str) -> String {
    if host == "local" {
        default_host.into()
    } else {
        host
    }
}

pub fn positional_strings(node: &KdlNode) -> Result<Vec<String>, St3Error> {
    ensure_no_properties(node)?;
    ensure_no_children(node)?;
    positional_values(node)
        .into_iter()
        .map(value_string)
        .collect()
}

pub fn positional_values(node: &KdlNode) -> Vec<&KdlValue> {
    node.entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .map(|entry| entry.value())
        .collect()
}

pub fn property_string(node: &KdlNode, property: &str) -> Result<Option<String>, St3Error> {
    node.entries()
        .iter()
        .find(|entry| entry.name().is_some_and(|name| name.value() == property))
        .map(|entry| value_string(entry.value()))
        .transpose()
}

pub fn reject_type(node: &KdlNode) -> Result<(), St3Error> {
    if node.ty().is_some() {
        Err(St3Error::new(
            "unexpected-type",
            format!(
                "node `{}` cannot have a type annotation",
                node.name().value()
            ),
        ))
    } else {
        Ok(())
    }
}

pub fn reject_unknown_children(
    document: &KdlDocument,
    allowed: &[&str],
    kind: &str,
    owner: &str,
) -> Result<(), St3Error> {
    for child in document.nodes() {
        if !allowed.contains(&child.name().value()) {
            return Err(St3Error::new(
                "unknown-child",
                format!(
                    "{kind} `{owner}` does not accept child `{}`",
                    child.name().value()
                ),
            ));
        }
    }
    Ok(())
}

pub fn required_child_string(
    document: &KdlDocument,
    child: &str,
    owner: &str,
) -> Result<String, St3Error> {
    child_string(document, child)?
        .ok_or_else(|| St3Error::new("missing-child", format!("`{owner}` needs child `{child}`")))
}

pub fn unique_child<'a>(
    document: &'a KdlDocument,
    name: &str,
) -> Result<Option<&'a KdlNode>, St3Error> {
    let matches = document
        .nodes()
        .iter()
        .filter(|node| node.name().value() == name)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(*node)),
        _ => Err(St3Error::new(
            "duplicate-child",
            format!("child `{name}` repeats"),
        )),
    }
}

pub fn valid_field_path(path: &str) -> bool {
    path.split('.').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic())
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}

pub fn validate_document_ref(value: &str) -> Result<(), St3Error> {
    let (name, hash) = value.rsplit_once('@').unwrap_or((value, ""));
    if name.len() <= 4
        || name.contains("..")
        || name.ends_with('/')
        || (!hash.is_empty()
            && (hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())))
    {
        return Err(St3Error::new(
            "invalid-document-reference",
            format!("invalid document reference `{value}`"),
        ));
    }
    Ok(())
}

pub fn validate_environment_name(name: &str) -> Result<(), St3Error> {
    let valid = name
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !valid {
        return Err(St3Error::new(
            "invalid-environment-name",
            format!("invalid environment name `{name}`"),
        ));
    }
    Ok(())
}

pub fn validate_full_subject(value: &str) -> Result<(), St3Error> {
    if value.contains("${") {
        if value.starts_with("${")
            && value.ends_with('}')
            && value[2..value.len() - 1].chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '.')
            })
        {
            return Ok(());
        }
        let mut concrete = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(start) = rest.find("${") {
            concrete.push_str(&rest[..start]);
            let tail = &rest[start + 2..];
            let Some(end) = tail.find('}') else {
                return Err(St3Error::new(
                    "invalid-variable",
                    "a variable reference has no closing brace",
                ));
            };
            concrete.push('x');
            rest = &tail[end + 1..];
        }
        concrete.push_str(rest);
        return validate_full_subject(&concrete);
    }
    if let Some(rest) = value.strip_prefix("file/") {
        let Some((host, path)) = rest.split_once(':') else {
            return Err(St3Error::new(
                "invalid-file-subject",
                "a file subject needs `file/HOST:/ABSOLUTE-PATH`",
            ));
        };
        validate_name(host, false)?;
        if !path.starts_with('/') || path.contains("/../") || path.ends_with("/..") {
            return Err(St3Error::new(
                "invalid-file-subject",
                "a file subject needs a safe absolute path",
            ));
        }
        return Ok(());
    }
    validate_name(value, true)
}

/// A lane declares an optional entry prefix and an optional approver. Cleanup and revision
/// retire it with `lane "NAME" { stop }`.
pub fn validate_lane(node: &KdlNode) -> Result<(), St3Error> {
    ensure_no_properties(node)?;
    let Some(body) = node.children() else {
        one_string(node)?;
        return Ok(());
    };
    one_string_with_children(node)?;
    if body.nodes().len() == 1 && body.nodes()[0].name().value() == "stop" {
        ensure_bare(&body.nodes()[0])?;
        return Ok(());
    }
    reject_unknown_children(body, &["entries", "approver"], "lane", "lane")?;
    for child in ["entries", "approver"] {
        unique_child(body, child)?;
    }
    if let Some(entries) = child_string(body, "entries")? {
        if !entries.ends_with('/') {
            return Err(St3Error::new(
                "invalid-lane-entries",
                "a lane's `entries` prefix must end with `/`",
            ));
        }
        validate_full_subject(&format!("{entries}entry"))?;
    }
    if let Some(approver) = child_string(body, "approver")? {
        validate_full_subject(&approver)?;
        if !approver.starts_with("person/") {
            return Err(St3Error::new(
                "invalid-lane-approver",
                "a lane's approver must be a `person/` subject",
            ));
        }
    }
    Ok(())
}

pub fn validate_name(value: &str, full: bool) -> Result<(), St3Error> {
    if value.is_empty() || value.len() > 512 || !value.is_ascii() {
        return Err(St3Error::new(
            "invalid-subject-name",
            format!("invalid subject name `{value}`"),
        ));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._-@/".contains(&byte))
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || value.ends_with('/')
        || value.split('/').any(|part| part.is_empty() || part == "..")
        || (full && !value.contains('/'))
    {
        return Err(St3Error::new(
            "invalid-subject-name",
            format!("invalid subject name `{value}`"),
        ));
    }
    Ok(())
}

pub fn validate_string_map(node: &KdlNode, environment: bool) -> Result<(), St3Error> {
    ensure_bare(node)?;
    let Some(body) = node.children() else {
        return Ok(());
    };
    let mut names = HashSet::new();
    for child in body.nodes() {
        let name = child.name().value();
        if !names.insert(name) {
            return Err(St3Error::new(
                "duplicate-map-key",
                format!("map key `{name}` repeats"),
            ));
        }
        if environment {
            validate_environment_name(name)?;
            if is_reserved_context_name(name) {
                return Err(St3Error::new(
                    "reserved-context-variable",
                    format!("environment cannot override `{name}`"),
                ));
            }
        }
        one_string(child)?;
    }
    Ok(())
}

pub fn value_string(value: &KdlValue) -> Result<String, St3Error> {
    match value {
        KdlValue::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(St3Error::new(
            "expected-string",
            "this value must be a non-empty string",
        )),
    }
}

/// A resource child with a locator names an edge rather than declaring a body.
pub fn is_resource_reference(node: &KdlNode) -> bool {
    node.name().value() == "resource"
        && (node.get("uri").is_some() || node.get("subject").is_some())
}

/// The typed edge a `resource` child of an agent or mission names, validated.
pub fn parse_declared_resource_reference(
    node: &KdlNode,
) -> Result<crate::model::DeclaredResourceReference, St3Error> {
    reject_type(node)?;
    ensure_only_properties(node, &["uri", "subject", "reason"])?;
    ensure_no_children(node)?;
    let name = one_string(node)?;
    validate_name(&name, false)?;
    let uri = property_string(node, "uri")?;
    let subject = property_string(node, "subject")?;
    if uri.is_some() == subject.is_some() {
        return Err(St3Error::new(
            "invalid-resource-reference",
            format!("resource `{name}` needs exactly one of uri=\"...\" or subject=\"resource/...\""),
        ));
    }
    let reason = property_string(node, "reason")?;
    if reason.as_deref().is_some_and(|reason| reason.trim().is_empty()) {
        return Err(St3Error::new(
            "invalid-resource-reason",
            format!("resource `{name}` has an empty reason"),
        ));
    }
    if let Some(subject) = subject {
        let local = subject.strip_prefix("resource/").ok_or_else(|| {
            St3Error::new("invalid-resource-subject", "a resource reference subject must start with resource/")
        })?;
        validate_name(local, false)?;
        return Ok(crate::model::DeclaredResourceReference {
            name,
            subject,
            kind: crate::model::ResourceReferenceKind::Subject,
            reason,
        });
    }
    let uri = uri.expect("exactly one locator was validated");
    validate_resource_uri(&uri)?;
    Ok(crate::model::DeclaredResourceReference {
        name,
        subject: format!("resource/{}", uri_reference_name(&uri)),
        kind: crate::model::ResourceReferenceKind::Uri,
        reason,
    })
}

pub fn uri_reference_name(uri: &str) -> String {
    format!("uri/{}", hex::encode(Sha256::digest(uri.as_bytes())))
}

/// An absolute URI (RFC 3986 `scheme ":" ...`) of any scheme, kept byte for byte.
pub fn validate_resource_uri(uri: &str) -> Result<(), St3Error> {
    let valid = uri.len() <= 4096
        && uri.split_once(':').is_some_and(|(scheme, rest)| {
            !rest.is_empty()
                && scheme.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && scheme
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        })
        && uri.bytes().all(|byte| {
            byte.is_ascii_graphic()
                && !matches!(byte, b'"' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}')
        })
        && uri.match_indices('%').all(|(offset, _)| {
            uri.as_bytes()
                .get(offset + 1..offset + 3)
                .is_some_and(|escape| escape.iter().all(u8::is_ascii_hexdigit))
        });
    if valid {
        Ok(())
    } else {
        Err(St3Error::new(
            "invalid-resource-uri",
            format!("`{uri}` is not an absolute URI; any scheme is accepted"),
        ))
    }
}
