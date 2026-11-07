//! Metadata reserved for participant-authorized thread reads.

pub const MESSAGE_RELATION_FIELDS: &[&str] = &[
    "thread_id",
    "thread_quote_id",
    "thread_origin_ask",
    "thread_origin_episode",
];

/// Whether a message body contains a private thread relation, including legacy flat claims.
pub fn message_has_relation(body: &serde_json::Value) -> bool {
    MESSAGE_RELATION_FIELDS.iter().any(|field| {
        body.get(*field).is_some()
            || body.get("fields").and_then(|fields| fields.get(*field)).is_some()
    })
}

/// Remove thread relations from a serving projection of a message.
pub fn redact_message_relations(body: &mut serde_json::Value) {
    if let Some(fields) = body.get_mut("fields").and_then(serde_json::Value::as_object_mut) {
        for field in MESSAGE_RELATION_FIELDS {
            fields.remove(*field);
        }
    }
    if let Some(fields) = body.as_object_mut() {
        for field in MESSAGE_RELATION_FIELDS {
            fields.remove(*field);
        }
    }
}

pub fn is_reserved_subject(subject: &str) -> bool {
    subject.starts_with("thread/")
}

pub fn is_reserved_kind(kind: &str) -> bool {
    kind.starts_with("thread.")
}

pub fn is_reserved_record(subject: &str, kind: &str) -> bool {
    is_reserved_subject(subject) || is_reserved_kind(kind)
}
