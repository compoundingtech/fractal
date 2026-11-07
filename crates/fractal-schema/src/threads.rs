//! Metadata reserved for participant-authorized thread reads.

/// Version 1 reserves thread routing fields and protects generic graph reads.
/// Version 2 also protects canonical thread messages on conversation read paths.
pub const READ_PRIVACY_VERSION: u32 = 1;

pub const fn read_privacy_supports_writes(version: u32) -> bool {
    version >= 2
}

pub const MESSAGE_RELATION_FIELDS: &[&str] = &[
    "thread_id",
    "thread_quote_id",
    "thread_origin_ask",
    "thread_origin_episode",
];

/// Remove thread relations from a serving projection of a message.
pub fn redact_message_relations(body: &mut serde_json::Value) {
    if let Some(fields) = body
        .get_mut("fields")
        .and_then(serde_json::Value::as_object_mut)
    {
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
