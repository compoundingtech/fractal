//! Metadata reserved for participant-authorized thread reads.

/// Every thread message carries this signed marker. Older closed schemas hold the
/// envelope instead of applying a message their conversation readers cannot protect.
pub const MESSAGE_READER_VERSION_FIELD: &str = "thread_reader_version";
pub const MESSAGE_READER_VERSION: u64 = 2;

pub const MESSAGE_RELATION_FIELDS: &[&str] = &[
    "thread_id",
    "thread_quote_id",
    "thread_origin_ask",
    "thread_origin_episode",
    MESSAGE_READER_VERSION_FIELD,
];

/// Whether a message body contains a private thread relation, including legacy flat claims.
pub fn message_has_relation(body: &serde_json::Value) -> bool {
    MESSAGE_RELATION_FIELDS.iter().any(|field| {
        body.get(*field).is_some()
            || body.get("fields").and_then(|fields| fields.get(*field)).is_some()
    })
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
