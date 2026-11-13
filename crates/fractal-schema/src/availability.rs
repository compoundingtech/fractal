//! Person availability: three person-authored axes and device presence reports on `person/NAME`.
//! Availability informs agents and pauses ask deadline clocks; it never grants authority. Every
//! claim names the daemon-assigned `source` that wrote it: a pairing grant or the local member.
use std::collections::BTreeMap;

use serde_json::Value;

use crate::{ValidationError, error};

/// `status`: `available` or `away`, or null to clear the override and derive status from presence.
pub const STATUS_KIND: &str = "person.status-set";
/// `interruptions`: `all`, `urgent` or `none`.
pub const INTERRUPTIONS_KIND: &str = "person.interruptions-set";
/// `deadline_clock`: `run`, `hold-routine` or `hold-all`.
pub const DEADLINE_CLOCK_KIND: &str = "person.deadline-clock-set";
/// One device's raw state transition: `active`, `inactive`, `locked` or `sleeping`, with the
/// device's last input. The daemon derives presence from it; the device applies no thresholds.
pub const PRESENCE_KIND: &str = "person.presence-observed";
pub const KINDS: [&str; 4] = [STATUS_KIND, INTERRUPTIONS_KIND, DEADLINE_CLOCK_KIND, PRESENCE_KIND];

pub const STATUSES: &[&str] = &["available", "away"];
pub const INTERRUPTIONS: &[&str] = &["all", "urgent", "none"];
pub const DEADLINE_CLOCKS: &[&str] = &["run", "hold-routine", "hold-all"];
/// Raw device states a client reports. `active`: in use and unlocked (its last input decides idle);
/// `inactive`: open but not in front of the person; `locked`; `sleeping`.
pub const DEVICE_STATES: &[&str] = &["active", "inactive", "locked", "sleeping"];
/// A source names the writing client: its pairing grant, or the local member.
pub const MAX_SOURCE_BYTES: usize = 256;
/// Attention priorities, most pressing first. An ask may name one; it is `normal` without.
pub const PRIORITIES: &[&str] = &["critical", "high", "normal", "low"];

/// The one urgency rule: an attention row is urgent when its priority is `critical`. The deadline
/// clock's `hold-routine` leaves urgent asks running, and `interruptions: urgent` admits only them.
pub fn urgent_priority(priority: Option<&str>) -> bool {
    priority == Some("critical")
}

/// The same rule for a notice, which says `urgency` instead: only `confirm` is urgent.
pub fn urgent_notice(urgency: &str) -> bool {
    urgency == "confirm"
}

/// Whether `value` is a usable source: nonblank text of at most [`MAX_SOURCE_BYTES`].
pub fn valid_source(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|source| !source.trim().is_empty() && source.len() <= MAX_SOURCE_BYTES)
}

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
    if !positive("until_unix_ms") || !positive("observed_at_unix_ms") || !positive("last_input_at_unix_ms") {
        return Err(error("invalid-availability", "times must be positive Unix milliseconds"));
    }
    if !valid_source(fields.get("source")) {
        return Err(error("invalid-availability", "a source is nonblank and at most 256 bytes"));
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
    fn availability_kinds_bound_values_times_and_source() {
        let person = "person/avery";
        let source = "host/amber";
        for (kind, body) in [
            (STATUS_KIND, json!({"status": "away", "until_unix_ms": 1, "source": source})),
            (STATUS_KIND, json!({"status": null, "source": "custom/client/pairing-1"})),
            (INTERRUPTIONS_KIND, json!({"interruptions": "urgent", "source": source})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "hold-all", "until_unix_ms": 5, "source": source})),
            (PRESENCE_KIND, json!({"state": "active", "last_input_at_unix_ms": 6, "source": "custom/client/pairing-1", "observed_at_unix_ms": 7})),
            (PRESENCE_KIND, json!({"state": "sleeping", "source": "host/amber", "observed_at_unix_ms": 7})),
        ] {
            assert!(registry().validate_claim(person, kind, &fields(body.clone())).is_ok(), "{kind} {body}");
        }
        for (kind, body) in [
            (STATUS_KIND, json!({"source": source})),
            (STATUS_KIND, json!({"status": "focus", "source": source})),
            (STATUS_KIND, json!({"status": null, "until_unix_ms": 5, "source": source})),
            (STATUS_KIND, json!({"status": "away", "until_unix_ms": 0, "source": source})),
            // Every person-state claim names the source that wrote it.
            (STATUS_KIND, json!({"status": "away"})),
            (INTERRUPTIONS_KIND, json!({"interruptions": "none"})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "hold-all", "source": " "})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "run", "source": "s".repeat(MAX_SOURCE_BYTES + 1)})),
            (INTERRUPTIONS_KIND, json!({"interruptions": null, "source": source})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "pause", "source": source})),
            (DEADLINE_CLOCK_KIND, json!({"deadline_clock": "run", "until_unix_ms": -1, "source": source})),
            (PRESENCE_KIND, json!({"state": "active", "source": " ", "observed_at_unix_ms": 7})),
            (PRESENCE_KIND, json!({"state": "active", "source": "s"})),
            (PRESENCE_KIND, json!({"state": "present", "source": "s", "observed_at_unix_ms": 7})),
            (PRESENCE_KIND, json!({"state": "active", "last_input_at_unix_ms": 0, "source": "s", "observed_at_unix_ms": 7})),
        ] {
            assert!(registry().validate_claim(person, kind, &fields(body.clone())).is_err(), "{kind} {body}");
        }
    }

    #[test]
    fn only_critical_priority_and_confirm_urgency_are_urgent() {
        assert!(urgent_priority(Some("critical")));
        for priority in [None, Some("high"), Some("normal"), Some("low"), Some("urgent")] {
            assert!(!urgent_priority(priority), "{priority:?}");
        }
        assert!(urgent_notice("confirm"));
        for urgency in ["info", "attention", "critical"] {
            assert!(!urgent_notice(urgency), "{urgency}");
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
                STATUS_KIND => json!({"status": "away", "source": "s"}),
                INTERRUPTIONS_KIND => json!({"interruptions": "none", "source": "s"}),
                DEADLINE_CLOCK_KIND => json!({"deadline_clock": "hold-all", "source": "s"}),
                _ => json!({"state": "active", "source": "s", "observed_at_unix_ms": 1}),
            };
            assert!(registry()
                .validate_public_claim("person/avery", kind, &fields(body), Some("person/avery"))
                .is_err());
        }
    }
}
