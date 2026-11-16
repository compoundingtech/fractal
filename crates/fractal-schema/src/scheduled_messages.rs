//! Immutable timed native message requests and durable terminal receipts.
use std::collections::BTreeMap;

use serde_json::Value;

use crate::{ValidationError, error};

pub const REQUESTED: &str = "message-schedule.requested";
pub const CANCELLED: &str = "message-schedule.cancelled";
pub const DELIVERED: &str = "message-schedule.delivered";
pub const KINDS: [&str; 3] = [REQUESTED, CANCELLED, DELIVERED];

/// Shape validation shared by local and replicated admission. Lifecycle and receipt
/// references are resolved by the store, never inferred from runtime process identity.
pub fn validate(kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    let nonempty: &[&str] = match kind {
        REQUESTED => &["from", "to", "key", "recipient_declaration"],
        CANCELLED => &["reason", "request"],
        DELIVERED => &["request", "message", "owner_claim"],
        _ => return Ok(()),
    };
    if nonempty.iter().any(|name| fields.get(*name).and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty()))
    {
        return Err(error("invalid-message-schedule", "schedule bindings must be nonempty strings"));
    }
    if kind == CANCELLED && fields.get("owner_claim").is_some_and(|value|
        value.as_str().is_none_or(|id| id.trim().is_empty()))
    {
        return Err(error("invalid-message-schedule", "owner_claim must be a nonempty string when present"));
    }
    if kind == REQUESTED {
        if fields.get("at_unix_ms").and_then(Value::as_u64).is_none() {
            return Err(error("invalid-message-schedule", "at_unix_ms must be a nonnegative integer"));
        }
        if fields.get("message").and_then(Value::as_object).is_none() {
            return Err(error("invalid-message-schedule", "message must contain native message fields"));
        }
        if fields.get("evidence").and_then(Value::as_array).is_none_or(|values|
            values.iter().any(|value| value.as_str().is_none_or(|id| id.trim().is_empty())))
        {
            return Err(error("invalid-message-schedule", "evidence must be an array of nonempty strings"));
        }
    }
    Ok(())
}

/// Dedicated request/cancellation operations must bind their authenticated actor
/// to the original sender. Cancellation supplies the persisted request's sender,
/// not caller-provided receipt fields. Delegation, if any, is resolved by the store.
pub fn validate_request_actor(from: &str, actor: Option<&str>) -> Result<(), ValidationError> {
    if actor != Some(from) {
        return Err(error("claim-write-forbidden", "only the authenticated schedule sender may request or cancel delivery"));
    }
    Ok(())
}

/// Checkpoint dependencies include declarations used to fence delivery, original
/// request IDs, and the evidence that will be copied to the eventual message.
pub fn claim_references<'a>(kind: &str, body: &'a Value) -> impl Iterator<Item = &'a str> {
    let names: &[&str] = match kind {
        REQUESTED => &["recipient_declaration"],
        CANCELLED => &["request", "owner_claim"],
        DELIVERED => &["request", "owner_claim"],
        _ => &[],
    };
    let fields = body.get("fields");
    let bindings = names.iter().filter_map(move |name| fields?.get(*name)?.as_str());
    let evidence = (kind == REQUESTED)
        .then(|| fields?.get("evidence")?.as_array())
        .flatten().into_iter().flatten().filter_map(Value::as_str);
    bindings.chain(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cardinality, Retention, WritePolicy, registry};
    use serde_json::json;

    fn requested() -> BTreeMap<String, Value> {
        serde_json::from_value(json!({
            "from": "agent/host/sender", "to": "agent/host/recipient",
            "at_unix_ms": 1234, "key": "wake", "message": {"content": "hello"},
            "evidence": ["evidence-id"], "recipient_declaration": "declaration-id"
        })).unwrap()
    }

    #[test]
    fn schedule_claims_are_closed_immutable_durable_and_dedicated() {
        let registry = registry();
        assert!(registry.validate_subject("message-schedule/wake").is_ok());
        for kind in KINDS {
            let spec = registry.claim(kind).unwrap();
            assert_eq!(spec.cardinality, Cardinality::Once);
            assert!(spec.fields.values().all(|field| field.immutable));
            assert!(spec.fields.iter().all(|(name, field)| field.required
                || (kind == CANCELLED && name == "owner_claim")));
            assert_eq!(spec.retention, Retention::Durable);
            assert!(!spec.additional_fields);
            assert_eq!(crate::retention::policy().kind(kind).class, crate::retention::Class::Durable);
        }
        assert_eq!(registry.claim(DELIVERED).unwrap().write_policy, WritePolicy::SystemOnly);
        assert!(registry.validate_public_claim("message-schedule/wake", REQUESTED,
            &requested(), Some("agent/host/sender")).is_err());
    }

    #[test]
    fn requested_shape_rejects_invalid_time_evidence_and_bindings() {
        let registry = registry();
        assert!(registry.validate_claim("message-schedule/wake", REQUESTED, &requested()).is_ok());
        for (name, value) in [
            ("at_unix_ms", json!(-1)), ("at_unix_ms", json!(1.5)),
            ("evidence", json!([1])), ("evidence", json!([""])),
            ("recipient_declaration", json!("")), ("key", json!(" ")),
            ("message", json!("hello")),
        ] {
            let mut fields = requested();
            fields.insert(name.into(), value);
            assert!(registry.validate_claim("message-schedule/wake", REQUESTED, &fields).is_err(), "{name}");
        }
    }

    #[test]
    fn terminal_shapes_and_actor_authority_are_explicit() {
        let registry = registry();
        for (kind, value) in [
            (CANCELLED, json!({"reason": "sender-cancelled", "request": "request-id"})),
            (DELIVERED, json!({"request": "request-id", "message": "message/id", "owner_claim": "owner-id"})),
        ] {
            let mut fields = serde_json::from_value::<BTreeMap<String, Value>>(value).unwrap();
            assert!(registry.validate_claim("message-schedule/wake", kind, &fields).is_ok());
            fields.insert("request".into(), json!(""));
            assert!(registry.validate_claim("message-schedule/wake", kind, &fields).is_err());
        }
        assert!(validate_request_actor("agent/host/sender", Some("agent/host/sender")).is_ok());
        assert!(validate_request_actor("agent/host/sender", Some("agent/host/other")).is_err());
        assert!(validate_request_actor("agent/host/sender", None).is_err());
    }

    #[test]
    fn checkpoint_references_include_fencing_and_original_evidence() {
        let request = json!({"fields": requested()});
        assert_eq!(claim_references(REQUESTED, &request).collect::<Vec<_>>(),
            ["declaration-id", "evidence-id"]);
        let delivered = json!({"fields": {"request": "request-id", "owner_claim": "owner-id", "message": "message/id"}});
        assert_eq!(claim_references(DELIVERED, &delivered).collect::<Vec<_>>(),
            ["request-id", "owner-id"]);
    }
}
