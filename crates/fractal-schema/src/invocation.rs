//! Durable native launch evidence. Null means unobserved, never a guessed value.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{ValidationError, error};

pub const KIND: &str = "harness.invocation";
pub const ENV: &str = "FRACTAL_INVOCATION_ID";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvocationPhase {
    Admitted,
    LaunchInputs,
    LaunchAccepted,
    Started,
    StartupFailed,
    Revoked,
    Cancelled,
    ProducerClosed,
    Ended,
    Observed,
}

/// Each phase is an immutable receipt; observations use their source claim as event_key.
/// Optional metadata is additive, and readers ignore fields added by newer producers.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InvocationReceipt {
    pub invocation_id: String,
    pub phase: InvocationPhase,
    pub agent: String,
    pub operation_id: String,
    #[serde(default)]
    pub event_key: Option<String>,
    #[serde(default)]
    pub owner_run: Option<String>,
    #[serde(default)]
    pub owner_step: Option<String>,
    #[serde(default)]
    pub owner_attempt: Option<u32>,
    #[serde(default)]
    pub parent_invocation_id: Option<String>,
    /// Only populated after exact launch-operation provenance verification.
    #[serde(default)]
    pub incarnation: Option<String>,
    #[serde(default)]
    pub requested_model: Option<String>,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub harness_version: Option<String>,
    /// Declaration/input fingerprint, NOT the exact provider request prompt.
    #[serde(default)]
    pub launch_input_digest: Option<String>,
    #[serde(default)]
    pub provider_binary_digest: Option<String>,
    #[serde(default)]
    pub prompt_digest: Option<String>,
    #[serde(default)]
    pub transcript_ref: Option<String>,
    #[serde(default)]
    pub trace_context: Option<Value>,
    /// Launch selection is evidence of intent, not proof of a fresh provider context.
    #[serde(default)]
    pub freshness: FreshnessEvidence,
    #[serde(default)]
    pub fallback: Option<FallbackEvidence>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FreshnessEvidence {
    pub requested_fresh: Option<bool>,
    pub resume_selected: Option<bool>,
    pub observed_fresh: Option<bool>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FallbackEvidence {
    pub event: String,
    pub reason: String,
    pub origin_ref: String,
}

pub fn validate(subject: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    let receipt: InvocationReceipt = serde_json::from_value(serde_json::to_value(fields)
        .map_err(|e| error("invalid-invocation", e.to_string()))?)
        .map_err(|e| error("invalid-invocation", e.to_string()))?;
    if receipt.agent != subject || receipt.operation_id.is_empty()
        || receipt.invocation_id != format!("invocation/{}", receipt.operation_id)
        || receipt.incarnation.as_ref().is_some_and(String::is_empty)
        || receipt.incarnation.is_some() && matches!(receipt.phase,
            InvocationPhase::Admitted | InvocationPhase::LaunchInputs | InvocationPhase::StartupFailed | InvocationPhase::Cancelled)
        || receipt.fallback.as_ref().is_some_and(|fallback|
            fallback.event.is_empty() || fallback.reason.is_empty() || fallback.origin_ref.is_empty())
    {
        return Err(error("invalid-invocation", "invocation must bind its exact agent/operation and observed provenance"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn additive_metadata_and_unknown_evidence_remain_readable() {
        let receipt: InvocationReceipt = serde_json::from_value(json!({
            "invocation_id":"invocation/op", "operation_id":"op", "phase":"admitted",
            "agent":"agent/example/worker", "future_metadata":{"value":true}
        })).unwrap();
        assert_eq!(receipt.prompt_digest, None);
        assert_eq!(receipt.harness_version, None);
        assert_eq!(receipt.freshness.observed_fresh, None);
        assert_eq!(receipt.fallback, None);
    }

    #[test]
    fn receipt_contract_is_system_only_and_durable() {
        let spec = crate::registry().claim(KIND).unwrap();
        assert_eq!(spec.write_policy, crate::WritePolicy::SystemOnly);
        assert_eq!(spec.retention, crate::Retention::Durable);
        assert_eq!(crate::retention::policy().kind(KIND).class, crate::retention::Class::Durable);
        let fields = serde_json::from_value(json!({"invocation_id":"invocation/op", "operation_id":"op",
            "phase":"startup-failed", "agent":"agent/example/worker", "incarnation":"guessed"})).unwrap();
        assert!(validate("agent/example/worker", &fields).is_err());
    }
}
