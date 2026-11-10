//! Shared admission validation for read positions: per person and conversation, the time of the
//! newest prose the person has seen there. One subject per person holds every conversation's
//! position; projections keep the largest `at_ms` (ties by canonical claim key), so positions only
//! move forward whatever order hosts receive the claims in.
use super::{ValidationError, error};
use serde_json::Value;
use std::collections::BTreeMap;

/// The only claim kind a read-position subject takes.
pub const KIND: &str = "read-position.advanced";
/// The subject family: `read-position/person/NAME`.
pub const FAMILY: &str = "read-position";
/// The client-v0 collection and stream name.
pub const COLLECTION: &str = "read-positions";
/// Conversation references are bounded like other client identifiers.
pub const MAX_CONVERSATION_BYTES: usize = 512;

/// `read-position/person/NAME` names its person, `person/NAME`.
pub fn owner(subject: &str) -> Result<&str, ValidationError> {
    let tail = subject.strip_prefix("read-position/").unwrap_or_default();
    let name = tail.strip_prefix("person/").unwrap_or_default();
    if name.is_empty() || name.contains('/') || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(error(
            "invalid-read-position-subject",
            "a read-position subject must be read-position/person/NAME",
        ));
    }
    Ok(tail)
}

/// The subject holding `person`'s read positions.
pub fn subject(person: &str) -> String {
    format!("read-position/{person}")
}

/// A conversation reference: `family/rest`, a lowercase family (`agent/…`, `session/…`), no
/// whitespace, at most [`MAX_CONVERSATION_BYTES`] bytes. Mirrors `ReadPositionConversation`.
pub fn valid_conversation(conversation: &str) -> bool {
    let Some((family, rest)) = conversation.split_once('/') else { return false };
    conversation.len() <= MAX_CONVERSATION_BYTES
        && family.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase())
        && family.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !rest.is_empty()
        && !rest.chars().any(char::is_whitespace)
}

/// A position in Unix milliseconds that SQLite stores as a signed integer.
pub fn at_ms(value: &Value) -> Option<u64> {
    value.as_u64().filter(|at| i64::try_from(*at).is_ok())
}

/// The fields of one advance: its owner matches the subject, a valid conversation and time.
pub fn validate_fields(subject: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    let owner = owner(subject)?;
    if fields.get("owner").and_then(Value::as_str) != Some(owner) {
        return Err(error(
            "read-position-owner-forbidden",
            "owner must match the person in the read-position subject",
        ));
    }
    if !fields.get("conversation").and_then(Value::as_str).is_some_and(valid_conversation) {
        return Err(error(
            "invalid-read-position",
            "conversation must be FAMILY/ID without whitespace, at most 512 bytes",
        ));
    }
    if fields.get("at_ms").and_then(at_ms).is_none() {
        return Err(error(
            "invalid-read-position",
            "at_ms must be a non-negative Unix millisecond time",
        ));
    }
    Ok(())
}

/// Only the person, or a trusted fleet agent acting in free mode, may advance a person's
/// positions as its real actor, as for arrangements.
pub fn validate_actor(subject: &str, actor: Option<&str>) -> Result<(), ValidationError> {
    if !subject.starts_with("read-position/") {
        return Ok(());
    }
    if actor == Some(owner(subject)?)
        || actor.is_some_and(|a| a.starts_with("agent/") && super::registry().validate_subject(a).is_ok())
    {
        return Ok(());
    }
    Err(error(
        "read-position-owner-forbidden",
        "only the person or a trusted fleet agent may advance a person's read positions",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(value: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn subjects_name_exactly_one_person() {
        assert_eq!(owner("read-position/person/ada").unwrap(), "person/ada");
        assert_eq!(subject("person/ada"), "read-position/person/ada");
        for invalid in [
            "read-position/person/",
            "read-position/person/ada/extra",
            "read-position/agent/ada",
            "read-position/person/a da",
            "arrangement/person/ada",
        ] {
            assert!(owner(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn conversations_follow_the_wire_pattern_and_bound() {
        for valid in ["agent/fleet/fixture-seat", "session/019a-abc", "a/b", "agent-2/x"] {
            assert!(valid_conversation(valid), "{valid}");
        }
        for invalid in ["", "agent", "agent/", "Agent/x", "1agent/x", "agent/a b", "/x", "agent_x/y"] {
            assert!(!valid_conversation(invalid), "{invalid}");
        }
        let longest = format!("agent/{}", "x".repeat(MAX_CONVERSATION_BYTES - 6));
        assert!(valid_conversation(&longest));
        assert!(!valid_conversation(&format!("{longest}x")));
    }

    #[test]
    fn fields_require_matching_owner_and_bounded_time() {
        let subject = "read-position/person/ada";
        validate_fields(subject, &fields(json!({"owner":"person/ada","conversation":"agent/a/b","at_ms":0}))).unwrap();
        for invalid in [
            json!({"owner":"person/robin","conversation":"agent/a/b","at_ms":1}),
            json!({"owner":"person/ada","conversation":"agent","at_ms":1}),
            json!({"owner":"person/ada","conversation":"agent/a/b","at_ms":-1}),
            json!({"owner":"person/ada","conversation":"agent/a/b","at_ms":1.5}),
            json!({"owner":"person/ada","conversation":"agent/a/b","at_ms":u64::MAX}),
            json!({"owner":"person/ada","conversation":"agent/a/b"}),
        ] {
            assert!(validate_fields(subject, &fields(invalid.clone())).is_err(), "{invalid}");
        }
    }

    #[test]
    fn only_the_person_or_a_fleet_agent_advances() {
        let subject = "read-position/person/ada";
        validate_actor(subject, Some("person/ada")).unwrap();
        validate_actor(subject, Some("agent/fleet/fixture-seat")).unwrap();
        for actor in [None, Some("person/robin"), Some("requester")] {
            assert_eq!(validate_actor(subject, actor).unwrap_err().code, "read-position-owner-forbidden");
        }
        validate_actor("glass/person/ada/x", None).unwrap();
    }
}
