//! Person availability: three person-authored axes and device presence reports on `person/NAME`.
//! Availability informs agents and pauses ask deadline clocks; it never grants authority.
use std::collections::BTreeMap;

use serde_json::Value;

use crate::{ValidationError, error};

/// `status`: `available` or `away`, or null to clear the override and derive status from presence.
pub const STATUS_KIND: &str = "person.status-set";
/// `interruptions`: `all`, `urgent` or `none`.
pub const INTERRUPTIONS_KIND: &str = "person.interruptions-set";
/// `deadline_clock`: `run`, `hold-routine` or `hold-all`.
pub const DEADLINE_CLOCK_KIND: &str = "person.deadline-clock-set";
/// One device's presence report: `present`, `idle` or `locked`, from one `source`.
pub const PRESENCE_KIND: &str = "person.presence-observed";
pub const KINDS: [&str; 4] = [STATUS_KIND, INTERRUPTIONS_KIND, DEADLINE_CLOCK_KIND, PRESENCE_KIND];

pub const STATUSES: &[&str] = &["available", "away"];
pub const INTERRUPTIONS: &[&str] = &["all", "urgent", "none"];
pub const DEADLINE_CLOCKS: &[&str] = &["run", "hold-routine", "hold-all"];
pub const PRESENCES: &[&str] = &["present", "idle", "locked"];
/// A presence source names the reporting client: its pairing grant, or the local member.
pub const MAX_SOURCE_BYTES: usize = 256;

/// Only the subject person writes their availability, through every admission path.
pub fn validate_actor(subject: &str, kind: &str, actor: Option<&str>) -> Result<(), ValidationError> {
    if KINDS.contains(&kind)
        && (!subject.strip_prefix("person/").is_some_and(|name| !name.is_empty() && !name.contains('/'))
            || actor != Some(subject))
    {
        return Err(error(
            "availability-forbidden",
            "only the subject person may set their availability or report their presence",
        ));
    }
    Ok(())
}

pub fn validate_fields(kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    if !KINDS.contains(&kind) {
        return Ok(());
    }
    let positive = |name: &str| match fields.get(name) {
        None => true,
        Some(value) => value.as_u64().is_some_and(|value| value > 0),
    };
    if !positive("until_unix_ms") || !positive("observed_at_unix_ms") {
        return Err(error("invalid-availability", "times must be positive Unix milliseconds"));
    }
    if kind == PRESENCE_KIND
        && !fields
            .get("source")
            .and_then(Value::as_str)
            .is_some_and(|source| !source.trim().is_empty() && source.len() <= MAX_SOURCE_BYTES)
    {
        return Err(error("invalid-availability", "a presence source is nonblank and at most 256 bytes"));
    }
    if kind == STATUS_KIND && fields.get("status").is_some_and(Value::is_null) && fields.contains_key("until_unix_ms") {
        return Err(error("invalid-availability", "clearing the status override takes no expiry"));
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

    #[test]
    fn availability_kinds_bound_values_and_times() {
        let person = "person/avery";
        for (kind, body) in [
            (STATUS_KIND, json!({"status": "away", "until_unix_ms": 1})),
            (STATUS_KIND, json!({"status": null})),
            (INTERRUPTIONS_KIND, json!({"interruptions": "urgent"})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "hold-all", "until_unix_ms": 5})),
            (PRESENCE_KIND, json!({"presence": "idle", "source": "custom/client/pairing-1", "observed_at_unix_ms": 7})),
        ] {
            assert!(registry().validate_claim(person, kind, &fields(body.clone())).is_ok(), "{kind} {body}");
        }
        for (kind, body) in [
            (STATUS_KIND, json!({})),
            (STATUS_KIND, json!({"status": "focus"})),
            (STATUS_KIND, json!({"status": null, "until_unix_ms": 5})),
            (STATUS_KIND, json!({"status": "away", "until_unix_ms": 0})),
            (INTERRUPTIONS_KIND, json!({"interruptions": null})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "pause"})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "run", "until_unix_ms": -1})),
            (PRESENCE_KIND, json!({"presence": "present", "source": " ", "observed_at_unix_ms": 7})),
            (PRESENCE_KIND, json!({"presence": "present", "source": "s"})),
            (PRESENCE_KIND, json!({"presence": "away", "source": "s", "observed_at_unix_ms": 7})),
        ] {
            assert!(registry().validate_claim(person, kind, &fields(body.clone())).is_err(), "{kind} {body}");
        }
    }

    #[test]
    fn only_the_subject_person_writes_availability() {
        for kind in KINDS {
            for actor in [None, Some("person/robin"), Some("agent/worker")] {
                assert!(validate_actor("person/avery", kind, actor).is_err());
            }
            assert!(validate_actor("person/avery", kind, Some("person/avery")).is_ok());
            assert!(validate_actor("agent/worker", kind, Some("agent/worker")).is_err());
            // Dedicated operations only: the ordinary claim surface refuses even the person.
            let body = match kind {
                STATUS_KIND => json!({"status": "away"}),
                INTERRUPTIONS_KIND => json!({"interruptions": "none"}),
                DEADLINE_CLOCK_KIND => json!({"deadline_clock": "hold-all"}),
                _ => json!({"presence": "present", "source": "s", "observed_at_unix_ms": 1}),
            };
            assert!(registry()
                .validate_public_claim("person/avery", kind, &fields(body), Some("person/avery"))
                .is_err());
        }
    }
}
