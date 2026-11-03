//! Canonical identity of the private notes subject for one host-qualified seat.
//! Filesystem realization and ordinary subject admission belong to the owning node.
use super::{ValidationError, error};
use std::fmt;

pub const SCHEME: &str = "dev.schickling.agent-private-notes";
const PREFIX: &str = "dev.schickling.agent-private-notes://";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateNotesUri {
    raw: String,
    host: String,
    identity: String,
}

impl PrivateNotesUri {
    pub fn for_subject(host: &str, identity: &str) -> Result<Self, ValidationError> {
        validate_component(host)?;
        validate_component(identity)?;
        let mut raw = String::from(PREFIX);
        encode_into(host, &mut raw);
        raw.push('/');
        encode_into(identity, &mut raw);
        Ok(Self { raw, host: host.into(), identity: identity.into() })
    }

    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let rest = value.strip_prefix(PREFIX).ok_or_else(shape)?;
        let (host, identity) = rest.split_once('/').ok_or_else(shape)?;
        let host = decode_component(host)?;
        let identity = decode_component(identity)?;
        let parsed = Self::for_subject(&host, &identity)?;
        if parsed.raw != value {
            return Err(error("private-notes-uri-component", "notes URI components must use canonical percent encoding"));
        }
        Ok(parsed)
    }

    pub fn for_expected_subject(value: &str, host: &str, identity: &str) -> Result<Self, ValidationError> {
        let parsed = Self::parse(value)?;
        if parsed.host != host || parsed.identity != identity {
            return Err(error("private-notes-subject-mismatch", "notes URI does not identify the exact subject"));
        }
        Ok(parsed)
    }

    pub fn as_str(&self) -> &str { &self.raw }
    pub fn host(&self) -> &str { &self.host }
    pub fn identity(&self) -> &str { &self.identity }
}

impl fmt::Display for PrivateNotesUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.raw)
    }
}

pub fn claims(value: &str) -> bool {
    value.split_once(':').is_some_and(|(scheme, _)| scheme == SCHEME)
}

fn shape() -> ValidationError {
    error("private-notes-uri-shape", "notes URI must identify one host and identity")
}

fn validate_component(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() || value.contains(['/', '\0']) || matches!(value, "." | "..") {
        return Err(shape());
    }
    Ok(())
}

fn encode_into(value: &str, encoded: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
}

fn decode_component(value: &str) -> Result<String, ValidationError> {
    let invalid = || error("private-notes-uri-component", "notes URI has an invalid percent-encoded component");
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            decoded.push(byte);
            index += 1;
        } else if byte == b'%' && index + 2 < bytes.len() {
            let high = hex(bytes[index + 1]).ok_or_else(invalid)?;
            let low = hex(bytes[index + 2]).ok_or_else(invalid)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            return Err(invalid());
        }
    }
    String::from_utf8(decoded).map_err(|_| invalid())
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Notes observations carry only identity and fixed audit attribution, never
/// private content, realization paths, fences, sizes, or content digests.
pub(crate) fn validate_observation(
    kind: &str, fields: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<(), ValidationError> {
    if kind != "resource.observed" {
        return Ok(());
    }
    let notes_uri = fields.get("facts").and_then(|facts| facts.get("uri"))
        .and_then(serde_json::Value::as_str).filter(|uri| claims(uri));
    if notes_uri.is_none() && fields.get("action").and_then(serde_json::Value::as_str) != Some("private-notes.write") {
        return Ok(());
    }
    let invalid = || error("private-notes-metadata", "private notes observations permit only identity and fixed audit metadata");
    let uri = notes_uri.ok_or_else(invalid)?;
    PrivateNotesUri::parse(uri)?;
    if fields.get("kind").and_then(serde_json::Value::as_str) != Some("uri.reference")
        || fields.get("facts").and_then(serde_json::Value::as_object).is_none_or(|facts| facts.len() != 1)
        || fields.keys().any(|key| !matches!(key.as_str(),
            "kind" | "facts" | "action" | "operation_id" | "stage" | "authority_actor" | "session_actor" | "transport"))
    {
        return Err(invalid());
    }
    if let Some(action) = fields.get("action") {
        if action.as_str() != Some("private-notes.write")
            || !matches!(fields.get("stage").and_then(serde_json::Value::as_str), Some("requested" | "completed"))
            || !matches!(fields.get("transport").and_then(serde_json::Value::as_str), Some("unix" | "fabric-loopback"))
        {
            return Err(invalid());
        }
        let operation = fields.get("operation_id").and_then(serde_json::Value::as_str)
            .and_then(|value| value.strip_prefix("operation/private-notes-")).ok_or_else(invalid)?;
        if operation.len() != 24 || !operation.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(invalid());
        }
        let authority = fields.get("authority_actor").and_then(serde_json::Value::as_str).ok_or_else(invalid)?;
        let subject = super::registry().validate_subject(authority)?;
        if !matches!(subject.family.as_str(), "person" | "agent") {
            return Err(invalid());
        }
        let actor = fields.get("session_actor").and_then(serde_json::Value::as_str).ok_or_else(invalid)?;
        // Paired clients retain their client/... session identity while acting
        // under a concrete person's authority.
        if !actor.strip_prefix("client/").is_some_and(|name| !name.is_empty()) {
            let subject = super::registry().validate_subject(actor)?;
            if !matches!(subject.family.as_str(), "person" | "agent") {
                return Err(invalid());
            }
        }
    } else if fields.len() != 2 {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_profile_and_observations_refuse_private_metadata() {
        use std::collections::BTreeMap;
        use serde_json::json;
        let uri = "dev.schickling.agent-private-notes://node/worker";
        super::super::registry().validate_resource_facts("uri.reference", &BTreeMap::from([("uri".into(), json!(uri))])).unwrap();
        assert!(super::super::registry().validate_resource_facts("uri.reference", &BTreeMap::from([
            ("uri".into(), json!("dev.schickling.agent-private-notes://node/%2F")),
        ])).is_err());
        let fields = BTreeMap::from([
            ("kind".into(), json!("uri.reference")),
            ("facts".into(), json!({"uri":uri})),
            ("action".into(), json!("private-notes.write")),
            ("operation_id".into(), json!("operation/private-notes-0123456789abcdef01234567")),
            ("stage".into(), json!("requested")),
            ("authority_actor".into(), json!("person/ada")),
            ("session_actor".into(), json!("person/ada")),
            ("transport".into(), json!("unix")),
        ]);
        validate_observation("resource.observed", &fields).unwrap();
        let mut paired = fields.clone();
        paired.insert("session_actor".into(), json!("client/paired-example"));
        validate_observation("resource.observed", &paired).unwrap();
        paired.insert("authority_actor".into(), json!("client/paired-example"));
        assert!(validate_observation("resource.observed", &paired).is_err());
        for name in ["markdown", "revision", "carrier_generation", "digest", "size", "path"] {
            let mut private = fields.clone();
            private.insert(name.into(), json!("private"));
            assert!(validate_observation("resource.observed", &private).is_err(), "{name}");
        }
        let mut facts = fields.clone();
        facts.insert("facts".into(), json!({"uri":uri, "digest":"private"}));
        assert!(validate_observation("resource.observed", &facts).is_err());
        let mut completed = fields;
        completed.insert("stage".into(), json!("completed"));
        validate_observation("resource.observed", &completed).unwrap();
    }

    use super::*;

    #[test]
    fn canonical_identity_is_exact_and_does_not_allow_path_selection() {
        let uri = PrivateNotesUri::for_subject("host a", "org.worker a").unwrap();
        assert_eq!(uri.as_str(), "dev.schickling.agent-private-notes://host%20a/org.worker%20a");
        assert_eq!(PrivateNotesUri::parse(uri.as_str()).unwrap(), uri);
        assert!(PrivateNotesUri::for_expected_subject(uri.as_str(), "host a", "org.worker b").is_err());
        for suffix in ["host/", "host/a/b", "host/%2F", "host/%00", "host/%61", "host/%FF", "host/..", "host/a?query", "host/a#fragment"] {
            assert!(PrivateNotesUri::parse(&format!("{PREFIX}{suffix}")).is_err(), "{suffix}");
        }
    }
}
