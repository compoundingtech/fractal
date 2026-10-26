//! Additive message provenance from the canonical claim already being rendered.
//! No text grouping, native identity guess, sender substitution or extra Store read.
use super::*;

fn reference<'a>(value: &'a str, families: &[&str]) -> Option<&'a str> {
    let (family, name) = value.split_once('/')?;
    (!name.is_empty() && families.contains(&family) && !value.chars().any(char::is_whitespace))
        .then_some(value)
}

fn participant(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .and_then(|value| reference(value, &["agent", "person", "step"]))
}

pub fn apply(body: &mut Value, claim: &ClaimRecord, negotiated: bool) {
    let fields = claim.body.get("fields").unwrap_or(&claim.body);
    // These legacy properties remain optional ParticipantRef scalars. A daemon is
    // not a ParticipantRef: omit that invalid optional value, never relabel it.
    for name in ["from", "to"] {
        if let Some(value) = participant(fields.get(name)) {
            body[name] = json!(value);
        }
    }
    if !negotiated {
        return;
    }
    if let Some(author) = claim
        .actor
        .as_deref()
        .and_then(|actor| reference(actor, &["agent", "daemon", "person"]))
    {
        body["author"] = json!(author);
    }
    // The present claim proves one destination. It does not prove a legacy series
    // of separate sends was one broadcast, even if their content is identical.
    if let Some(recipient) = participant(fields.get("to")) {
        body["recipients"] = json!([recipient]);
    }
}

pub fn legacy_item(item: &mut Value, session: &ClientSession) {
    if !session.conversation_message_participants
        && item["type"] == "message"
        && let Some(body) = item["body"].as_object_mut()
    {
        body.remove("author");
        body.remove("recipients");
    }
}
