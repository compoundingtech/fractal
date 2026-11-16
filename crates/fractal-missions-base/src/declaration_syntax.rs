//! The shared syntax of declaration KDL: reading a node's arguments, properties and children,
//! and validating the names, subjects and maps they carry. Intent parsing (`graph`), mission
//! parsing (`mission`) and gate parsing (`gate_parse`) all read declarations through these.
use std::collections::{BTreeMap, HashSet};

use kdl::{KdlDocument, KdlNode, KdlValue};
use sha2::{Digest as _, Sha256};

use crate::model::FractalError;

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

// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-09-kdl-fractal-context-names — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
/// Spell each `${FRACTAL_X}` context reference inside `mission` declarations as the registered
/// `${ST_X}` before parsing, as `terminal` is lowered to `pty`. Either spelling publishes the same
/// mission revision, so members that only know the `ST_` names run it unchanged. Escaped `$${…}`
/// text and names that are not context variables stay as written.
pub fn lower_context_references(document: &mut KdlDocument) {
    fn lower(node: &mut KdlNode) {
        for entry in node.entries_mut() {
            let Some(lowered) = entry
                .value()
                .as_string()
                .and_then(registered_references)
            else {
                continue;
            };
            let value = KdlValue::String(lowered);
            if let Some(format) = entry.format_mut() {
                format.value_repr = value.to_string();
            }
            entry.set_value(value);
        }
        if let Some(children) = node.children_mut() {
            children.nodes_mut().iter_mut().for_each(lower);
        }
    }
    for node in document.nodes_mut() {
        if node.name().value() == "mission" {
            lower(node);
        }
    }
}

/// `text` with its `${FRACTAL_X}` context references spelled `${ST_X}`, or `None` when it has none.
fn registered_references(text: &str) -> Option<String> {
    if !text.contains("${FRACTAL_") {
        return None;
    }
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    let mut lowered = false;
    while let Some(start) = rest.find("${") {
        let (before, tail) = (&rest[..start], &rest[start + 2..]);
        output.push_str(before);
        output.push_str("${");
        rest = tail;
        if before.ends_with('$') {
            continue;
        }
        let Some(end) = tail.find('}') else {
            continue;
        };
        let name = &tail[..end];
        let Some(suffix) = name.strip_prefix("FRACTAL_") else {
            continue;
        };
        if let Some(registered) = VARIABLES
            .iter()
            .find(|variable| variable.strip_prefix("ST_") == Some(suffix))
        {
            output.push_str(registered);
            output.push('}');
            rest = &tail[end + 1..];
            lowered = true;
        }
    }
    output.push_str(rest);
    lowered.then_some(output)
}
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-09-kdl-fractal-context-names

pub fn is_reserved_context_name(name: &str) -> bool {
    (VARIABLES.contains(&name) && name != "PATH") || name == "ST3_SUBJECT"
}

pub fn child_integer(document: &KdlDocument, name: &str) -> Result<Option<i128>, FractalError> {
    let Some(node) = unique_child(document, name)? else {
        return Ok(None);
    };
    ensure_no_properties(node)?;
    ensure_no_children(node)?;
    let [entry] = node.entries() else {
        return Err(FractalError::new(
            "wrong-argument-count",
            format!("child `{name}` needs one integer"),
        ));
    };
    match entry.value() {
        KdlValue::Integer(value) => Ok(Some(*value)),
        _ => Err(FractalError::new(
            "expected-integer",
            format!("child `{name}` needs one integer"),
        )),
    }
}

pub fn child_string(document: &KdlDocument, name: &str) -> Result<Option<String>, FractalError> {
    unique_child(document, name)?.map(one_string).transpose()
}

pub fn child_strings(document: &KdlDocument, name: &str) -> Result<Option<Vec<String>>, FractalError> {
    unique_child(document, name)?
        .map(positional_strings)
        .transpose()
}

pub fn ensure_bare(node: &KdlNode) -> Result<(), FractalError> {
    reject_type(node)?;
    if !node.entries().is_empty() {
        return Err(FractalError::new(
            "unexpected-value",
            format!("node `{}` cannot have values", node.name().value()),
        ));
    }
    Ok(())
}

pub fn ensure_no_children(node: &KdlNode) -> Result<(), FractalError> {
    if node.children().is_some() {
        Err(FractalError::new(
            "unexpected-children",
            format!("node `{}` cannot have children", node.name().value()),
        ))
    } else {
        Ok(())
    }
}

pub fn ensure_no_properties(node: &KdlNode) -> Result<(), FractalError> {
    ensure_only_properties(node, &[])
}

pub fn ensure_only_properties(node: &KdlNode, allowed: &[&str]) -> Result<(), FractalError> {
    reject_type(node)?;
    let mut seen = HashSet::new();
    for entry in node.entries().iter().filter(|entry| entry.name().is_some()) {
        let name = entry.name().expect("filtered").value();
        if !allowed.contains(&name) {
            return Err(FractalError::new(
                "unknown-property",
                format!(
                    "node `{}` does not accept property `{name}`",
                    node.name().value()
                ),
            ));
        }
        if !seen.insert(name) {
            return Err(FractalError::new(
                "duplicate-property",
                format!("node `{}` repeats property `{name}`", node.name().value()),
            ));
        }
    }
    Ok(())
}

pub fn first_string(node: &KdlNode) -> Result<String, FractalError> {
    positional_values(node)
        .first()
        .ok_or_else(|| {
            FractalError::new(
                "missing-argument",
                format!("node `{}` needs a name", node.name().value()),
            )
        })
        .and_then(|value| value_string(value))
}

pub use crate::declaration_kdl::{json_value, parse_duration};

pub fn one_string(node: &KdlNode) -> Result<String, FractalError> {
    ensure_no_children(node)?;
    one_string_with_children(node)
}

pub fn one_string_with_children(node: &KdlNode) -> Result<String, FractalError> {
    first_string(node).and_then(|first| {
        if positional_values(node).len() == 1 {
            Ok(first)
        } else {
            Err(FractalError::new(
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

pub fn parse_map_child(
    document: &KdlDocument,
    name: &str,
) -> Result<BTreeMap<String, String>, FractalError> {
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
            return Err(FractalError::new(
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

pub fn positional_strings(node: &KdlNode) -> Result<Vec<String>, FractalError> {
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

pub fn property_string(node: &KdlNode, property: &str) -> Result<Option<String>, FractalError> {
    node.entries()
        .iter()
        .find(|entry| entry.name().is_some_and(|name| name.value() == property))
        .map(|entry| value_string(entry.value()))
        .transpose()
}

pub fn reject_type(node: &KdlNode) -> Result<(), FractalError> {
    if node.ty().is_some() {
        Err(FractalError::new(
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
) -> Result<(), FractalError> {
    for child in document.nodes() {
        if !allowed.contains(&child.name().value()) {
            return Err(FractalError::new(
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
) -> Result<String, FractalError> {
    child_string(document, child)?
        .ok_or_else(|| FractalError::new("missing-child", format!("`{owner}` needs child `{child}`")))
}

pub fn unique_child<'a>(
    document: &'a KdlDocument,
    name: &str,
) -> Result<Option<&'a KdlNode>, FractalError> {
    let matches = document
        .nodes()
        .iter()
        .filter(|node| node.name().value() == name)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(*node)),
        _ => Err(FractalError::new(
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

pub fn validate_document_ref(value: &str) -> Result<(), FractalError> {
    let (name, hash) = value.rsplit_once('@').unwrap_or((value, ""));
    if name.len() <= 4
        || name.contains("..")
        || name.ends_with('/')
        || (!hash.is_empty()
            && (hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())))
    {
        return Err(FractalError::new(
            "invalid-document-reference",
            format!("invalid document reference `{value}`"),
        ));
    }
    Ok(())
}

pub fn validate_environment_name(name: &str) -> Result<(), FractalError> {
    let valid = name
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !valid {
        return Err(FractalError::new(
            "invalid-environment-name",
            format!("invalid environment name `{name}`"),
        ));
    }
    Ok(())
}

pub fn validate_full_subject(value: &str) -> Result<(), FractalError> {
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
                return Err(FractalError::new(
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
            return Err(FractalError::new(
                "invalid-file-subject",
                "a file subject needs `file/HOST:/ABSOLUTE-PATH`",
            ));
        };
        validate_name(host, false)?;
        if !path.starts_with('/') || path.contains("/../") || path.ends_with("/..") {
            return Err(FractalError::new(
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
pub fn validate_lane(node: &KdlNode) -> Result<(), FractalError> {
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
            return Err(FractalError::new(
                "invalid-lane-entries",
                "a lane's `entries` prefix must end with `/`",
            ));
        }
        validate_full_subject(&format!("{entries}entry"))?;
    }
    if let Some(approver) = child_string(body, "approver")? {
        validate_full_subject(&approver)?;
        if !approver.starts_with("person/") {
            return Err(FractalError::new(
                "invalid-lane-approver",
                "a lane's approver must be a `person/` subject",
            ));
        }
    }
    Ok(())
}

pub fn validate_name(value: &str, full: bool) -> Result<(), FractalError> {
    if value.is_empty() || value.len() > 512 || !value.is_ascii() {
        return Err(FractalError::new(
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
        return Err(FractalError::new(
            "invalid-subject-name",
            format!("invalid subject name `{value}`"),
        ));
    }
    Ok(())
}

pub fn validate_string_map(node: &KdlNode, environment: bool) -> Result<(), FractalError> {
    ensure_bare(node)?;
    let Some(body) = node.children() else {
        return Ok(());
    };
    let mut names = HashSet::new();
    for child in body.nodes() {
        let name = child.name().value();
        if !names.insert(name) {
            return Err(FractalError::new(
                "duplicate-map-key",
                format!("map key `{name}` repeats"),
            ));
        }
        if environment {
            validate_environment_name(name)?;
            if is_reserved_context_name(name) {
                return Err(FractalError::new(
                    "reserved-context-variable",
                    format!("environment cannot override `{name}`"),
                ));
            }
        }
        one_string(child)?;
    }
    Ok(())
}

pub fn value_string(value: &KdlValue) -> Result<String, FractalError> {
    match value {
        KdlValue::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(FractalError::new(
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
) -> Result<crate::model::DeclaredResourceReference, FractalError> {
    reject_type(node)?;
    ensure_only_properties(node, &["uri", "subject", "reason"])?;
    ensure_no_children(node)?;
    let name = one_string(node)?;
    validate_name(&name, false)?;
    let uri = property_string(node, "uri")?;
    let subject = property_string(node, "subject")?;
    if uri.is_some() == subject.is_some() {
        return Err(FractalError::new(
            "invalid-resource-reference",
            format!("resource `{name}` needs exactly one of uri=\"...\" or subject=\"resource/...\""),
        ));
    }
    let reason = property_string(node, "reason")?;
    if reason.as_deref().is_some_and(|reason| reason.trim().is_empty()) {
        return Err(FractalError::new(
            "invalid-resource-reason",
            format!("resource `{name}` has an empty reason"),
        ));
    }
    if let Some(subject) = subject {
        let local = subject.strip_prefix("resource/").ok_or_else(|| {
            FractalError::new("invalid-resource-subject", "a resource reference subject must start with resource/")
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
pub fn validate_resource_uri(uri: &str) -> Result<(), FractalError> {
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
        Err(FractalError::new(
            "invalid-resource-uri",
            format!("`{uri}` is not an absolute URI; any scheme is accepted"),
        ))
    }
}

/// `capacity cores=N memory="SIZE"`: the seat's lane (docs/st3/seat-lanes.md). What it leaves
/// out is the base lane's.
pub fn parse_capacity(node: &KdlNode) -> Result<fractal_runtime::Capacity, FractalError> {
    let (cores, memory) = capacity_properties(node)?;
    fractal_runtime::Capacity::declared(cores, memory.as_deref())
        .map_err(|error| FractalError::new("invalid-capacity", format!("{error:#}")))
}

/// A step's `capacity cores=N memory="SIZE"`: what it raises its seat's lane to while the seat holds
/// the step. What it leaves out stays the seat's own.
pub fn parse_step_capacity(node: &KdlNode) -> Result<fractal_runtime::Raise, FractalError> {
    let (cores, memory) = capacity_properties(node)?;
    fractal_runtime::Raise::declared(cores, memory.as_deref())
        .map_err(|error| FractalError::new("invalid-capacity", format!("{error:#}")))
}

fn capacity_properties(node: &KdlNode) -> Result<(Option<u32>, Option<String>), FractalError> {
    let invalid = |message: String| FractalError::new("invalid-capacity", message);
    ensure_only_properties(node, &["cores", "memory"])?;
    ensure_no_children(node)?;
    if node.entries().iter().any(|entry| entry.name().is_none()) {
        return Err(invalid("capacity takes only cores= and memory=".into()));
    }
    let cores = match node.get("cores") {
        None => None,
        Some(KdlValue::Integer(cores)) => Some(
            u32::try_from(*cores).map_err(|_| invalid(format!("capacity cores {cores} is out of range")))?,
        ),
        Some(_) => return Err(invalid("capacity cores must be a whole number".into())),
    };
    Ok((cores, property_string(node, "memory")?))
}
