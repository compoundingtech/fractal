//! Authored subject context. Notes inform; they never assign work or grant authority.
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use crate::{ValidationError, error};

pub const KIND: &str = "note.set";
pub const MAX_TEXT_BYTES: usize = 1024;
/// Publication requires this version on every active member's own-origin daemon.started.
pub const FEATURE: &str = "subject_notes";
pub const FEATURE_VERSION: u64 = 1;
/// Known audience labels; the wire vocabulary remains open to future labels.
pub const AUDIENCES: &[&str] = &["self", "agent"];
pub const TARGET_FAMILIES: &[&str] = &["agent", "session", "mission", "mission-run", "step-run"];

/// Canonical note-target identity for an existing runtime incarnation.
pub fn session_subject(parent: &str, incarnation: &str) -> Result<String, ValidationError> {
    if !parent.starts_with("agent/") || incarnation.trim().is_empty() {
        return Err(error("invalid-session-subject", "a session requires an agent parent and nonblank incarnation"));
    }
    crate::registry().validate_subject(parent)?;
    let mut subject = format!("session/{parent}/");
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in incarnation.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            subject.push(char::from(byte));
        } else {
            subject.push('%');
            subject.push(char::from(HEX[usize::from(byte >> 4)]));
            subject.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    if subject.len() > 512 {
        return Err(error("invalid-session-subject", "session subject exceeds 512 bytes"));
    }
    Ok(subject)
}

/// Split parentage and decode the canonical final component exactly once.
pub fn session_parts(subject: &str) -> Result<(&str, String), ValidationError> {
    let invalid = || error("invalid-session-subject", "expected session/agent/ID/ENCODED_INCARNATION");
    let (parent, component) = subject.strip_prefix("session/").and_then(|tail| tail.rsplit_once('/'))
        .ok_or_else(invalid)?;
    let mut decoded = Vec::with_capacity(component.len());
    let mut bytes = component.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|b| char::from(b).to_digit(16)).ok_or_else(invalid)?;
            let low = bytes.next().and_then(|b| char::from(b).to_digit(16)).ok_or_else(invalid)?;
            decoded.push(u8::try_from(high * 16 + low).map_err(|_| invalid())?);
        } else {
            decoded.push(byte);
        }
    }
    let incarnation = String::from_utf8(decoded).map_err(|_| invalid())?;
    if session_subject(parent, &incarnation)? != subject { return Err(invalid()); }
    Ok((parent, incarnation))
}

/// The immutable person or agent author encoded before the lowercase UUIDv7.
pub fn author(subject: &str) -> Result<&str, ValidationError> {
    let tail = subject.strip_prefix("note/").unwrap_or_default();
    let Some((author, id)) = tail.rsplit_once('/') else {
        return Err(error("invalid-note-subject", "a note subject must encode a person or agent author followed by a lowercase UUIDv7"));
    };
    let valid_author = if let Some(name) = author.strip_prefix("person/") {
        !name.is_empty() && !name.contains('/')
    } else if let Some(path) = author.strip_prefix("agent/") {
        !path.is_empty() && path.split('/').all(|part| !part.is_empty())
    } else {
        false
    };
    if subject.len() > 512 || !valid_author || !crate::arrangements::valid_uuid(id)
        || subject.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(error("invalid-note-subject", "a note subject must encode a person or agent author followed by a lowercase UUIDv7"));
    }
    Ok(author)
}

pub fn validate_actor(subject: &str, kind: &str, actor: Option<&str>) -> Result<(), ValidationError> {
    if kind == KIND && actor != Some(author(subject)?) {
        return Err(error("note-author-forbidden", "only the author encoded in the note subject may set or archive it"));
    }
    Ok(())
}

/// Accept RFC 3339 UTC timestamps, including the equivalent explicit +00:00 offset.
pub fn expiry(value: &str) -> Result<DateTime<FixedOffset>, ValidationError> {
    crate::directive_notes::expiry(value)
        .map_err(|_| error("invalid-subject-note", "expires_at must be a valid UTC timestamp"))
}

pub fn validate_fields(fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    let target = fields.get("target").and_then(Value::as_str)
        .ok_or_else(|| error("invalid-subject-note", "target must be a subject reference"))?;
    let target_spec = crate::registry().validate_subject(target)?;
    if !TARGET_FAMILIES.contains(&target_spec.family.as_str()) {
        return Err(error("invalid-subject-note", "target must be an agent, session, mission, mission-run or step-run subject"));
    }
    match fields.get("text") {
        Some(Value::Null) => {}
        Some(Value::String(text)) if !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES => {}
        _ => return Err(error("invalid-subject-note", "text must be null or nonblank text of at most 1024 UTF-8 bytes")),
    }
    if !fields.get("audience").and_then(Value::as_str).is_some_and(|value| !value.trim().is_empty()) {
        return Err(error("invalid-subject-note", "audience must be a nonblank open-enum string"));
    }
    if let Some(value) = fields.get("expires_at") {
        expiry(value.as_str().ok_or_else(|| error("invalid-subject-note", "expires_at must be a UTC timestamp string"))?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cardinality, Retention, ValueType, WritePolicy, registry};
    use serde_json::json;

    const ID: &str = "note/person/avery/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d";

    fn fields() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("target".into(), json!("agent/org/worker")),
            ("text".into(), json!("Context, not authority")),
            ("audience".into(), json!("self")),
        ])
    }

    #[test]
    fn session_identity_is_canonical_and_decodes_once() {
        let parent = "agent/example/worker";
        for incarnation in ["1234:2026-10-14T15:57:40.016Z", "%2F/é", "AbC-._~"] {
            let subject = session_subject(parent, incarnation).unwrap();
            assert_eq!(session_parts(&subject).unwrap(), (parent, incarnation.to_owned()));
            assert!(!registry().validate_subject(&subject).unwrap().client_writable);
        }
        assert_eq!(session_subject(parent, "%2F/é").unwrap(), "session/agent/example/worker/%252F%2F%C3%A9");
        for subject in ["session/agent/example/worker/%2f", "session/agent/example/worker/%41",
            "session/person/example/first", "session/agent/example/worker/%FF",
            "session/agent/example/worker/%", "session/agent/example/worker/"] {
            assert!(registry().validate_subject(subject).is_err(), "{subject}");
        }
    }

    #[test]
    fn registration_and_feature_policy() {
        let spec = registry().claim(KIND).unwrap();
        assert_eq!(spec.write_policy, WritePolicy::SubjectEncodedAuthor);
        assert_eq!(spec.cardinality, Cardinality::StateTransition);
        assert_eq!(spec.retention, Retention::Durable);
        assert_eq!(spec.projection.as_deref(), Some("subject-notes"));
        assert!(!spec.wakes_reconciler && !spec.additional_fields);
        assert!(registry().subject(ID).unwrap().client_writable);
        assert!(!registry().subject("arrangement/person/avery/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d").unwrap().client_writable);
        assert!(spec.fields["text"].nullable && spec.fields["text"].required);
        assert_eq!(spec.fields["target"].value_type, ValueType::SubjectReference);
        assert!(spec.fields["audience"].values.is_empty(), "audiences must remain open");
        assert_eq!((FEATURE, FEATURE_VERSION), ("subject_notes", 1));
    }

    #[test]
    fn identity_and_exact_author_admission() {
        assert_eq!(author(ID).unwrap(), "person/avery");
        let agent_id = ID.replace("person/avery", "agent/org/worker");
        assert_eq!(author(&agent_id).unwrap(), "agent/org/worker");
        for id in [ID.to_uppercase(), ID.replace("7c3a", "4c3a"), ID.replace("9b2d", "7b2d"), ID.replace("person/avery", "person/avery/other"), ID.replace("person/avery", "agent//worker"), ID.replace("person/avery", "mission/run"), ID.replace("person/avery", "person/a b")] {
            assert!(author(&id).is_err(), "{id}");
            assert!(registry().validate_subject(&id).is_err(), "{id}");
        }
        for actor in [None, Some("person/other"), Some("agent/org/worker")] {
            assert!(validate_actor(ID, KIND, actor).is_err());
        }
        assert!(validate_actor(ID, KIND, Some("person/avery")).is_ok());
        assert!(validate_actor(&agent_id, KIND, Some("agent/org/worker")).is_ok());
    }

    #[test]
    fn subject_note_claims_require_dedicated_operations() {
        for (subject, actor) in [(ID.to_owned(), "person/avery"),
            (ID.replace("person/avery", "agent/org/worker"), "agent/org/worker")] {
            for text in [json!("Context, not authority"), Value::Null] {
                let mut values = fields();
                values.insert("text".into(), text);
                let error = registry().validate_public_claim(&subject, KIND, &values, Some(actor)).unwrap_err();
                assert_eq!(error.code, "claim-write-forbidden");
                assert_eq!(error.message, "claim kind `note.set` requires a dedicated authorized operation");
            }
        }
    }

    #[test]
    fn bounded_nullable_text_and_open_audiences() {
        for text in [json!(null), json!("é".repeat(512))] {
            let mut values = fields();
            values.insert("text".into(), text);
            assert!(registry().validate_claim(ID, KIND, &values).is_ok());
        }
        for text in [json!(""), json!(" \n\t"), json!("é".repeat(513)), json!(42), json!([])] {
            let mut values = fields();
            values.insert("text".into(), text);
            assert!(registry().validate_claim(ID, KIND, &values).is_err());
        }
        for audience in ["self", "agent", "future-crew"] {
            let mut values = fields();
            values.insert("audience".into(), json!(audience));
            assert!(registry().validate_claim(ID, KIND, &values).is_ok());
        }
        for field in ["target", "text", "audience"] {
            let mut values = fields();
            values.remove(field);
            assert!(registry().validate_claim(ID, KIND, &values).is_err());
        }
        for field in ["author", "extra"] {
            let mut values = fields();
            values.insert(field.into(), json!("person/avery"));
            assert!(registry().validate_claim(ID, KIND, &values).is_err());
        }
    }

    #[test]
    fn target_session_and_utc_expiry_constraints() {
        for target in ["agent/org/worker", "session/agent/org/worker/incarnation%3A1", "mission/task", "mission-run/run", "step-run/gen/step"] {
            let mut values = fields();
            values.insert("target".into(), json!(target));
            assert!(registry().validate_claim(ID, KIND, &values).is_ok());
            values.insert("session".into(), json!("incarnation:2026-10-14T16:20:00Z"));
            assert!(registry().validate_claim(ID, KIND, &values).is_err());
        }
        for (field, value) in [("target", json!("person/avery")), ("target", json!("agent/")), ("session", json!(null)), ("session", json!(" ")), ("audience", json!(null)), ("audience", json!(" ")), ("expires_at", json!(null)), ("expires_at", json!("2099-01-01T00:00:00+01:00")), ("expires_at", json!("2099-01-01T00:00:00-00:00")), ("expires_at", json!("2099-02-30T00:00:00Z"))] {
            let mut values = fields();
            values.insert(field.into(), value);
            assert!(registry().validate_claim(ID, KIND, &values).is_err());
        }
        for at in ["2099-01-01T00:00:00Z", "2099-01-01T00:00:00.123+00:00"] {
            let mut values = fields();
            values.insert("expires_at".into(), json!(at));
            assert!(registry().validate_claim(ID, KIND, &values).is_ok());
        }
    }
}
