//! A person's own controls over their attention rows, shared by every client: when they first
//! saw a row, and until when they put it off. Both are claims on `person/NAME` that only that
//! person writes, through the `attention.seen` and `attention.snooze` actions only. A control
//! changes only its person's view of the row it names: a row's lifecycle, deadline and asker
//! never read it, so a control naming another person's row changes nothing.
use std::collections::BTreeMap;

use serde_json::Value;

use crate::{ValidationError, error};

/// The person saw `attention_id`; the earliest accepted claim is when.
pub const SEEN_KIND: &str = "person.attention-seen";
/// The person put `attention_id` off until `until_unix_ms`, or null to bring it back.
pub const SNOOZED_KIND: &str = "person.attention-snoozed";
pub const KINDS: [&str; 2] = [SEEN_KIND, SNOOZED_KIND];
/// The client the person acted from: its pairing grant, or the local member.
pub const MAX_SOURCE_BYTES: usize = 256;

/// An attention row id: `attention/` and 32 lowercase hex digits. Mirrors `AttentionControlId`.
/// Notice ids join when notice rows are served as attention rows.
pub fn valid_attention_id(id: &str) -> bool {
    id.strip_prefix("attention/").is_some_and(|hash| {
        hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Only the subject person writes their controls, through every admission path.
pub fn validate_actor(subject: &str, kind: &str, actor: Option<&str>) -> Result<(), ValidationError> {
    if KINDS.contains(&kind)
        && (!subject.strip_prefix("person/").is_some_and(|name| !name.is_empty() && !name.contains('/'))
            || actor != Some(subject))
    {
        return Err(error(
            "attention-control-forbidden",
            "only the subject person may mark their attention rows seen or snoozed",
        ));
    }
    Ok(())
}

pub fn validate_fields(kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    if !KINDS.contains(&kind) {
        return Ok(());
    }
    if !fields.get("attention_id").and_then(Value::as_str).is_some_and(valid_attention_id) {
        return Err(error(
            "invalid-attention-control",
            "attention_id must be an attention row id: attention/HASH",
        ));
    }
    if !fields
        .get("source")
        .and_then(Value::as_str)
        .is_some_and(|source| !source.trim().is_empty() && source.len() <= MAX_SOURCE_BYTES)
    {
        return Err(error("invalid-attention-control", "source is nonblank and at most 256 bytes"));
    }
    if kind == SNOOZED_KIND
        && !fields
            .get("until_unix_ms")
            .is_some_and(|until| until.is_null() || until.as_u64().is_some_and(|until| until > 0 && i64::try_from(until).is_ok()))
    {
        return Err(error(
            "invalid-attention-control",
            "until_unix_ms is positive Unix milliseconds, or null to end the snooze",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;
    use serde_json::json;

    fn fields(value: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    const ROW: &str = "attention/0123456789abcdef0123456789abcdef";

    #[test]
    fn attention_controls_bound_ids_sources_and_times() {
        let person = "person/avery";
        let notice = "custom/attention/snapshot/01234567-89ab-cdef-0123-456789abcdef";
        for (kind, body) in [
            (SEEN_KIND, json!({"attention_id": ROW, "source": "host/alder"})),
            (SEEN_KIND, json!({"attention_id": ROW, "source": "custom/client/pairing-1"})),
            (SNOOZED_KIND, json!({"attention_id": ROW, "source": "host/alder", "until_unix_ms": 5})),
            (SNOOZED_KIND, json!({"attention_id": ROW, "source": "host/alder", "until_unix_ms": null})),
        ] {
            assert!(registry().validate_claim(person, kind, &fields(body.clone())).is_ok(), "{kind} {body}");
        }
        for (kind, body) in [
            (SEEN_KIND, json!({"attention_id": ROW})),
            (SEEN_KIND, json!({"attention_id": ROW, "source": " "})),
            (SEEN_KIND, json!({"attention_id": ROW, "source": "s".repeat(MAX_SOURCE_BYTES + 1)})),
            (SEEN_KIND, json!({"attention_id": "attention/closed/abc", "source": "s"})),
            (SEEN_KIND, json!({"attention_id": "attention/0123456789ABCDEF0123456789abcdef", "source": "s"})),
            (SEEN_KIND, json!({"attention_id": "custom/attention/source/01234567-89ab-cdef-0123-456789abcdef", "source": "s"})),
            (SEEN_KIND, json!({"attention_id": notice, "source": "s"})),
            (SNOOZED_KIND, json!({"attention_id": ROW, "source": "s"})),
            (SNOOZED_KIND, json!({"attention_id": ROW, "source": "s", "until_unix_ms": 0})),
            (SNOOZED_KIND, json!({"attention_id": ROW, "source": "s", "until_unix_ms": -1})),
            (SNOOZED_KIND, json!({"attention_id": ROW, "source": "s", "until_unix_ms": "soon"})),
        ] {
            assert!(registry().validate_claim(person, kind, &fields(body.clone())).is_err(), "{kind} {body}");
        }
    }

    #[test]
    fn only_the_subject_person_writes_attention_controls() {
        for kind in KINDS {
            for actor in [None, Some("person/robin"), Some("agent/worker")] {
                assert!(validate_actor("person/avery", kind, actor).is_err());
            }
            assert!(validate_actor("person/avery", kind, Some("person/avery")).is_ok());
            assert!(validate_actor("agent/worker", kind, Some("agent/worker")).is_err());
            // Dedicated actions only: the ordinary claim surface refuses even the person.
            let body = json!({"attention_id": ROW, "source": "s", "until_unix_ms": 5});
            assert!(registry()
                .validate_public_claim("person/avery", kind, &fields(body), Some("person/avery"))
                .is_err());
        }
    }
}
