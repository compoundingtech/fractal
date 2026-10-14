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
pub const TARGET_FAMILIES: &[&str] = &["agent", "mission", "mission-run", "step-run"];

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
        return Err(error("invalid-subject-note", "target must be an agent, mission, mission-run or step-run subject"));
    }
    match fields.get("text") {
        Some(Value::Null) => {}
        Some(Value::String(text)) if !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES => {}
        _ => return Err(error("invalid-subject-note", "text must be null or nonblank text of at most 1024 UTF-8 bytes")),
    }
    if !fields.get("audience").and_then(Value::as_str).is_some_and(|value| !value.trim().is_empty()) {
        return Err(error("invalid-subject-note", "audience must be a nonblank open-enum string"));
    }
    if let Some(session) = fields.get("session") {
        if target_spec.family != "agent" || !session.as_str().is_some_and(|value| !value.trim().is_empty()) {
            return Err(error("invalid-subject-note", "session must be a nonblank incarnation id on an agent target"));
        }
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
            assert!(registry().validate_public_claim(ID, KIND, &fields(), actor).is_err());
        }
        assert!(registry().validate_public_claim(ID, KIND, &fields(), Some("person/avery")).is_ok());
        assert!(registry().validate_public_claim(&agent_id, KIND, &fields(), Some("agent/org/worker")).is_ok());
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
        for target in ["agent/org/worker", "mission/task", "mission-run/run", "step-run/gen/step"] {
            let mut values = fields();
            values.insert("target".into(), json!(target));
            assert!(registry().validate_claim(ID, KIND, &values).is_ok());
            values.insert("session".into(), json!("incarnation:2026-10-14T16:20:00Z"));
            assert_eq!(registry().validate_claim(ID, KIND, &values).is_ok(), target.starts_with("agent/"));
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
