//! Immutable native effect receipts, bound to one operation and its admitting work lease.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ValidationError, error};

pub const KIND_INTENDED: &str = "effect.intended";
pub const KIND_IN_FLIGHT: &str = "effect.in-flight";
pub const KIND_CONFIRMED: &str = "effect.confirmed";
pub const KIND_UNRESOLVED: &str = "effect.unresolved";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectClass {
    NativeCoordinated,
    ProviderIdempotent,
    FailClosed,
}

/// Every phase repeats the immutable identity and admission bindings.
/// Provider observations are additive; absent metadata is never guessed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectRecord {
    pub effect_id: String,
    pub operation_id: String,
    pub provider: String,
    pub target: String,
    pub payload_digest: String,
    pub class: EffectClass,
    pub bound_step_run: String,
    pub bound_attempt: u32,
    pub lease_owner: String,
    pub lease_incarnation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovering_actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

pub fn validate(
    kind: &str,
    subject: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<(), ValidationError> {
    if !matches!(kind, KIND_INTENDED | KIND_IN_FLIGHT | KIND_CONFIRMED | KIND_UNRESOLVED) {
        return Ok(());
    }
    let record: EffectRecord = serde_json::from_value(
        serde_json::to_value(fields).map_err(|e| error("invalid-effect", e.to_string()))?,
    )
    .map_err(|e| error("invalid-effect", e.to_string()))?;
    if record.effect_id != subject
        || record.effect_id != format!("effect/{}", record.operation_id)
        || [
            &record.operation_id,
            &record.provider,
            &record.target,
            &record.payload_digest,
            &record.bound_step_run,
            &record.lease_owner,
            &record.lease_incarnation,
        ]
        .iter()
        .any(|binding| binding.trim().is_empty())
    {
        return Err(error(
            "invalid-effect",
            "an effect must bind its dedicated operation subject and nonempty admission bindings",
        ));
    }
    if record.recovering_actor.as_ref().is_some_and(|actor| actor.trim().is_empty()) {
        return Err(error(
            "invalid-effect",
            "a recovering actor must be nonempty when present",
        ));
    }
    if kind == KIND_CONFIRMED
        && (record.provider_ref.as_ref().is_none_or(|value| value.trim().is_empty())
            || record.evidence.is_empty()
            || record.evidence.iter().any(|value| value.trim().is_empty()))
    {
        return Err(error(
            "invalid-effect",
            "a confirmed effect requires a provider reference and nonempty evidence",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields() -> BTreeMap<String, Value> {
        serde_json::from_value(json!({
            "effect_id": "effect/op", "operation_id": "op", "provider": "native.message",
            "target": "agent/example/receiver", "payload_digest": "digest",
            "class": "native-coordinated", "bound_step_run": "step-run/example/send",
            "bound_attempt": 0, "lease_owner": "agent/example/sender", "lease_incarnation": "incarnation"
        })).unwrap()
    }

    #[test]
    fn all_phases_are_system_only_durable_appends_on_effect_subjects() {
        for kind in [KIND_INTENDED, KIND_IN_FLIGHT, KIND_CONFIRMED, KIND_UNRESOLVED] {
            let spec = crate::registry().claim(kind).unwrap();
            assert_eq!(spec.write_policy, crate::WritePolicy::SystemOnly);
            assert_eq!(spec.cardinality, crate::Cardinality::Append);
            assert_eq!(spec.retention, crate::Retention::Durable);
            assert_eq!(crate::retention::policy().kind(kind).class, crate::retention::Class::Durable);
            assert_eq!(spec.subjects, vec!["effect"]);
        }
        assert_eq!(crate::registry().subjects["effect"].pattern, "effect/OPERATION_ID");
    }

    #[test]
    fn registry_dispatches_effect_validation_and_typed_fields() {
        let registry = crate::registry();
        let mut input = fields();
        assert!(registry.validate_claim("effect/op", KIND_INTENDED, &input).is_ok());
        assert!(registry.validate_claim("effect/op", KIND_IN_FLIGHT, &input).is_ok());
        assert!(registry.validate_claim("effect/op", KIND_UNRESOLVED, &input).is_ok());
        assert!(registry.validate_claim("effect/op", KIND_CONFIRMED, &input).is_err());
        input.insert("provider_ref".into(), json!("message/example"));
        input.insert("evidence".into(), json!(["message/example"]));
        assert!(registry.validate_claim("effect/op", KIND_CONFIRMED, &input).is_ok());
        input.insert("class".into(), json!("unknown-class"));
        assert!(registry.validate_claim("effect/op", KIND_INTENDED, &input).is_err());
        input.insert("class".into(), json!("fail-closed"));
        input.insert("bound_step_run".into(), json!("agent/example/not-a-step"));
        assert!(registry.validate_claim("effect/op", KIND_INTENDED, &input).is_err());
    }

    #[test]
    fn binds_the_exact_subject_operation_and_required_metadata() {
        let original = fields();
        assert!(validate(KIND_INTENDED, "effect/op", &original).is_ok());
        assert!(validate(KIND_INTENDED, "step-run/example/send", &original).is_err());
        for field in ["effect_id", "operation_id", "provider", "target", "payload_digest",
            "bound_step_run", "lease_owner", "lease_incarnation"] {
            let mut invalid = original.clone();
            invalid.insert(field.into(), json!(""));
            assert!(validate(KIND_INTENDED, "effect/op", &invalid).is_err(), "{field}");
            invalid.remove(field);
            assert!(validate(KIND_INTENDED, "effect/op", &invalid).is_err(), "missing {field}");
        }
        for attempt in [json!(-1), json!(4_294_967_296_u64), json!("0"), Value::Null] {
            let mut invalid = original.clone();
            invalid.insert("bound_attempt".into(), attempt);
            assert!(validate(KIND_INTENDED, "effect/op", &invalid).is_err());
        }
        let mut invalid = original;
        invalid.remove("bound_attempt");
        assert!(validate(KIND_INTENDED, "effect/op", &invalid).is_err());
    }

    #[test]
    fn confirmation_requires_observed_reference_and_evidence() {
        let mut confirmed = fields();
        assert!(validate(KIND_CONFIRMED, "effect/op", &confirmed).is_err());
        confirmed.insert("provider_ref".into(), json!("message/example"));
        for evidence in [json!([]), json!([""]), json!(["receipt", " "])] {
            confirmed.insert("evidence".into(), evidence);
            assert!(validate(KIND_CONFIRMED, "effect/op", &confirmed).is_err());
        }
        confirmed.insert("evidence".into(), json!(["message/example"]));
        assert!(validate(KIND_CONFIRMED, "effect/op", &confirmed).is_ok());
        confirmed.insert("provider_ref".into(), json!(""));
        assert!(validate(KIND_CONFIRMED, "effect/op", &confirmed).is_err());
    }

    #[test]
    fn recovering_actor_is_optional_nonempty_provenance_for_every_phase() {
        let registry = crate::registry();
        for kind in [KIND_INTENDED, KIND_IN_FLIGHT, KIND_CONFIRMED, KIND_UNRESOLVED] {
            let mut input = fields();
            input.insert("provider_ref".into(), json!("message/example"));
            input.insert("evidence".into(), json!(["message/example"]));
            assert!(registry.validate_claim("effect/op", kind, &input).is_ok());
            input.insert("recovering_actor".into(), json!("agent/example/recoverer"));
            assert!(registry.validate_claim("effect/op", kind, &input).is_ok());
            for invalid in ["", " \t\n"] {
                input.insert("recovering_actor".into(), json!(invalid));
                assert!(registry.validate_claim("effect/op", kind, &input).is_err());
            }
        }
    }

    #[test]
    fn observed_recovering_actor_is_serialized_separately_from_lease_owner() {
        let record = EffectRecord {
            effect_id: "effect/op".into(),
            operation_id: "op".into(),
            provider: "native.message".into(),
            target: "agent/example/receiver".into(),
            payload_digest: "digest".into(),
            class: EffectClass::NativeCoordinated,
            bound_step_run: "step-run/example/send".into(),
            bound_attempt: 0,
            lease_owner: "agent/example/sender".into(),
            lease_incarnation: "incarnation".into(),
            recovering_actor: Some("agent/example/recoverer".into()),
            provider_ref: None,
            result_digest: None,
            error: None,
            evidence: vec![],
        };
        let output = serde_json::to_value(&record).unwrap();
        assert_eq!(output["recovering_actor"], json!("agent/example/recoverer"));
        assert_eq!(output["lease_owner"], json!("agent/example/sender"));
        assert_eq!(serde_json::from_value::<EffectRecord>(output).unwrap(), record);
    }

    #[test]
    fn optional_metadata_is_omitted_and_future_metadata_is_readable() {
        let mut input = fields();
        input.insert("future_metadata".into(), json!({"observed": true}));
        let record: EffectRecord = serde_json::from_value(serde_json::to_value(input).unwrap()).unwrap();
        assert!(record.evidence.is_empty());
        let output = serde_json::to_value(&record).unwrap();
        for field in ["recovering_actor", "provider_ref", "result_digest", "error"] {
            assert!(output.get(field).is_none());
        }
        for (class, encoded) in [(EffectClass::NativeCoordinated, "native-coordinated"),
            (EffectClass::ProviderIdempotent, "provider-idempotent"), (EffectClass::FailClosed, "fail-closed")] {
            assert_eq!(serde_json::to_value(class).unwrap(), json!(encoded));
        }
    }
}
