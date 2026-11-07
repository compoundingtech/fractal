//! Immutable delivery evidence. Structure alone never establishes owner authority.
use crate::{ValidationError, error};
use serde::{Deserialize, Serialize};

pub const KIND: &str = "custom.attention.receipt-created";
pub const MAX_TEXT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface { Tray, Speech, Push, Log }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase", deny_unknown_fields)]
pub enum Outcome {
    Delivered { completed_at: String, evidence: String },
    Failed { reason: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub source_id: String,
    pub creation_claim: String,
    pub producer: String,
    pub recipient: String,
    /// Opaque reviewed owner-incarnation declaration claim hash, not a resolved binding.
    pub owner_binding: String,
    pub surface: Surface,
    pub outcome: Outcome,
}

impl Receipt {
    /// Structural validation only; a receipt still requires independent owner authorization.
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::notice::id_for_source(&self.source_id)?;
        for hash in [&self.creation_claim, &self.owner_binding] {
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err(invalid("claim hashes must be 64 lowercase hex digits"));
            }
        }
        for (subject, families) in [
            (&self.producer, &["agent", "person"][..]),
            (&self.recipient, &["person"][..]),
        ] {
            let spec = crate::registry().validate_subject(subject)?;
            if !families.contains(&spec.family.as_str()) {
                return Err(invalid("producer must be agent/person and recipient must be person"));
            }
        }
        match &self.outcome {
            Outcome::Delivered { completed_at, evidence } => {
                if completed_at.len() > 128 || crate::directive_notes::expiry(completed_at).is_err() {
                    return Err(invalid("completed_at must be a bounded UTC timestamp"));
                }
                bounded_text(evidence)?;
            }
            Outcome::Failed { reason } => bounded_text(reason)?,
        }
        Ok(())
    }
}

fn invalid(message: &str) -> ValidationError { error("invalid-notice-receipt", message) }
fn bounded_text(value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.len() > MAX_TEXT_BYTES {
        return Err(invalid("outcome text must be nonblank and at most 4096 UTF-8 bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn receipt() -> Receipt {
        Receipt {
            source_id: "custom/attention/source/01900000-0000-7000-8000-000000000002".into(),
            creation_claim: "a".repeat(64), producer: "agent/worker".into(),
            recipient: "person/reader".into(), owner_binding: "b".repeat(64),
            surface: Surface::Tray,
            outcome: Outcome::Delivered {
                completed_at: "2026-01-01T00:00:00Z".into(), evidence: "display completed".into(),
            },
        }
    }

    #[test]
    fn notice_receipt_roundtrip_preserves_each_surface_and_outcome() {
        for surface in [Surface::Tray, Surface::Speech, Surface::Push, Surface::Log] {
            for outcome in [receipt().outcome, Outcome::Failed { reason: "surface unavailable".into() }] {
                let value = Receipt { surface, outcome, ..receipt() };
                value.validate().unwrap();
                let encoded = serde_json::to_value(&value).unwrap();
                assert!(encoded.get("runtime").is_none());
                assert_eq!(serde_json::from_value::<Receipt>(encoded).unwrap(), value);
            }
        }
    }

    #[test]
    fn notice_receipt_rejects_invalid_structure() {
        for (field, value) in [
            ("source_id", json!("custom/attention/snapshot/01900000-0000-7000-8000-000000000002")),
            ("source_id", json!("custom/attention/source/not-a-uuid")),
            ("creation_claim", json!("A".repeat(64))), ("owner_binding", json!("b".repeat(63))),
            ("owner_binding", json!("g".repeat(64))), ("producer", json!("resource/task")),
            ("producer", json!("agent/")), ("producer", json!(format!("agent/{}", "a".repeat(512)))),
            ("recipient", json!("agent/worker")), ("recipient", json!("person/has space")),
            ("outcome", json!({"state":"delivered", "completed_at":"invalid", "evidence":"shown"})),
            ("outcome", json!({"state":"delivered", "completed_at":"2026-01-01T00:00:00Z", "evidence":" "})),
            ("outcome", json!({"state":"failed", "reason":""})),
            ("outcome", json!({"state":"failed", "reason":"x".repeat(MAX_TEXT_BYTES + 1)})),
            ("surface", json!("unknown")), ("runtime", json!("agent/runtime")),
            ("outcome", json!({"state":"failed", "reason":"failure", "runtime":"agent/runtime"})),
        ] {
            let mut encoded = serde_json::to_value(receipt()).unwrap();
            encoded.as_object_mut().unwrap().insert(field.into(), value);
            assert!(serde_json::from_value::<Receipt>(encoded)
                .map_or(true, |value| value.validate().is_err()), "accepted {field}");
        }
        let mut encoded = serde_json::to_value(receipt()).unwrap();
        encoded["outcome"] = Value::Null;
        assert!(serde_json::from_value::<Receipt>(encoded).is_err());
    }
}
