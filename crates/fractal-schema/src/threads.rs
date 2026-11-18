//! Metadata reserved for participant-authorized thread reads.

/// Version 1 reserves thread routing fields and protects generic graph reads.
/// Version 2 also protects canonical thread messages on conversation read paths.
/// Version 3 additionally projects metadata by causal ancestry and fences its full frontier.
pub const READ_PRIVACY_VERSION: u32 = 3;

// A causal reader's same-store floor prevents a timestamp-only binary reopening that store.
// Fleet readiness remains a separate prerequisite: this floor cannot stop a fresh old peer.
const _: () =
    assert!(!read_privacy_supports_writes(READ_PRIVACY_VERSION) || super::STORAGE_VERSION >= 23);

pub const fn read_privacy_supports_writes(version: u32) -> bool {
    version >= 3
}

/// Every thread message carries this signed marker. Older closed schemas hold the
/// envelope instead of applying a message their conversation readers cannot protect.
pub const MESSAGE_READER_VERSION_FIELD: &str = "thread_reader_version";
pub const MESSAGE_READER_VERSION: u64 = 2;

pub const MESSAGE_RELATION_FIELDS: &[&str] = &[
    "thread_id",
    "thread_quote_id",
    "thread_origin_ask",
    "thread_origin_source",
    "thread_origin_episode",
    MESSAGE_READER_VERSION_FIELD,
];

/// Whether a message body contains a private thread relation, including legacy flat claims.
pub fn message_has_relation(body: &serde_json::Value) -> bool {
    MESSAGE_RELATION_FIELDS.iter().any(|field| {
        body.get(*field).is_some()
            || body
                .get("fields")
                .and_then(|fields| fields.get(*field))
                .is_some()
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

/// Signed, owner-scoped observed-remove reminder operations. Receipts are independent.
pub const UNREAD_MARKED: &str = "thread.unread-marked";
pub const UNREAD_CLEARED: &str = "thread.unread-cleared";
pub const UNREAD_KINDS: [&str; 2] = [UNREAD_MARKED, UNREAD_CLEARED];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreadUnreadMark {
    /// Stable signed operation ID, independent of equivalent claim receipt selection.
    pub mark_id: String,
    pub anchor_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreadUnreadReminder {
    /// Each captured generation retains its own visibility anchor.
    pub marks: Vec<ThreadUnreadMark>,
    pub anchor_message_id: String,
}

pub fn validate_reminder_fields(
    kind: &str,
    fields: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<(), super::ValidationError> {
    if !UNREAD_KINDS.contains(&kind) {
        return Ok(());
    }
    let invalid = || {
        super::error(
            "invalid-thread-reminder",
            "expected distinct exact observed mark operation IDs",
        )
    };
    let marks = fields
        .get("observed_marks")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(invalid)?;
    let mut unique = std::collections::BTreeSet::new();
    for id in marks {
        let id = id
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?;
        if !unique.insert(id) {
            return Err(invalid());
        }
    }
    Ok(())
}

pub fn validate_reminder_actor(
    kind: &str,
    person: Option<&str>,
    actor: Option<&str>,
) -> Result<(), super::ValidationError> {
    if UNREAD_KINDS.contains(&kind)
        && (actor != person || !actor.is_some_and(|id| id.starts_with("person/")))
    {
        return Err(super::error(
            "thread-reminder-forbidden",
            "only the named person may change their reminder",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod reminder_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reminder_schema_reserves_dedicated_owner_operations() {
        for kind in UNREAD_KINDS {
            assert_eq!(
                crate::retention::policy().kind(kind).class,
                crate::retention::Class::Durable
            );
            let mut fields: std::collections::BTreeMap<String, serde_json::Value> =
                serde_json::from_value(
                    json!({"person":"person/example","observed_marks":["op/exact"]}),
                )
                .unwrap();
            if kind == UNREAD_MARKED {
                fields.insert("anchor_message".into(), json!("message/exact"));
            }
            assert!(
                crate::registry()
                    .validate_claim("thread/example", kind, &fields)
                    .is_ok()
            );
            assert!(
                crate::registry()
                    .validate_public_claim("thread/example", kind, &fields, Some("person/example"))
                    .is_err()
            );
            assert!(
                validate_reminder_actor(
                    kind,
                    fields.get("person").and_then(serde_json::Value::as_str),
                    Some("person/example")
                )
                .is_ok()
            );
            assert!(
                validate_reminder_actor(
                    kind,
                    fields.get("person").and_then(serde_json::Value::as_str),
                    Some("person/foreign")
                )
                .is_err()
            );
            assert!(is_reserved_record("thread/example", kind));
            for invalid in [json!(["duplicate", "duplicate"]), json!([4]), json!([""])] {
                fields.insert("observed_marks".into(), invalid);
                assert!(
                    crate::registry()
                        .validate_claim("thread/example", kind, &fields)
                        .is_err()
                );
            }
        }
    }
}
