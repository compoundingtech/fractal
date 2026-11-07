//! Immutable owner acknowledgements; authority requires an authenticated member envelope.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ValidationError, error};

pub const KIND: &str = "notice.owner-acknowledged";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Active,
    Ended,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Acknowledgement {
    pub owner_binding: String,
    pub previous_ack: Option<String>,
    pub status: Status,
    pub runtime_binding: Option<String>,
}

impl Acknowledgement {
    /// Structural validation only; causal chain and owner authority are checked separately.
    pub fn validate(&self) -> Result<(), ValidationError> {
        for hash in std::iter::once(&self.owner_binding)
            .chain(self.previous_ack.iter())
            .chain(self.runtime_binding.iter())
        {
            if hash.len() != 64
                || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid("claim hashes must be 64 lowercase hex digits"));
            }
        }
        match self.status {
            Status::Active if self.runtime_binding.is_none() => {
                Err(invalid("active acknowledgement requires runtime_binding"))
            }
            Status::Ended if self.runtime_binding.is_some() => {
                Err(invalid("ended acknowledgement requires runtime_binding=null"))
            }
            _ => Ok(()),
        }
    }

    /// Nullable fields must be explicit: omission is not a root or an ended interval.
    pub fn from_fields(fields: &BTreeMap<String, Value>) -> Result<Self, ValidationError> {
        for name in ["owner_binding", "previous_ack", "status", "runtime_binding"] {
            if !fields.contains_key(name) {
                return Err(invalid(format!("acknowledgement requires explicit field `{name}`")));
            }
        }
        let acknowledgement: Self = serde_json::from_value(Value::Object(
            fields.iter().map(|(name, value)| (name.clone(), value.clone())).collect(),
        ))
        .map_err(|error| invalid(format!("invalid acknowledgement fields: {error}")))?;
        acknowledgement.validate()?;
        Ok(acknowledgement)
    }
}

fn invalid(message: impl Into<String>) -> ValidationError {
    error("invalid-notice-owner-ack", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields() -> BTreeMap<String, Value> {
        serde_json::from_value(json!({
            "owner_binding": "a".repeat(64), "previous_ack": null,
            "status": "active", "runtime_binding": "b".repeat(64),
        })).unwrap()
    }

    #[test]
    fn notice_owner_ack_roundtrips_active_and_ended() {
        for status in [Status::Active, Status::Ended] {
            for previous_ack in [None, Some("c".repeat(64))] {
                let acknowledgement = Acknowledgement {
                    owner_binding: "a".repeat(64), previous_ack, status,
                    runtime_binding: (status == Status::Active).then(|| "b".repeat(64)),
                };
                acknowledgement.validate().unwrap();
                let encoded = serde_json::to_value(&acknowledgement).unwrap();
                assert_eq!(encoded["status"], if status == Status::Active { "active" } else { "ended" });
                assert!(encoded.get("previous_ack").is_some());
                assert!(encoded.get("runtime_binding").is_some());
                let fields = serde_json::from_value(encoded).unwrap();
                assert_eq!(Acknowledgement::from_fields(&fields).unwrap(), acknowledgement);
                crate::registry().validate_claim("notice-owner/default", KIND, &fields).unwrap();
            }
        }
    }

    #[test]
    fn notice_owner_ack_requires_every_explicit_field() {
        for name in ["owner_binding", "previous_ack", "status", "runtime_binding"] {
            let mut omitted = fields();
            omitted.remove(name);
            assert!(Acknowledgement::from_fields(&omitted).is_err(), "accepted omitted {name}");
            assert!(crate::registry().validate_claim("notice-owner/default", KIND, &omitted).is_err());
        }
    }

    #[test]
    fn notice_owner_ack_rejects_malformed_hashes_states_and_unknown_fields() {
        for (name, value) in [
            ("owner_binding", Value::Null),
            ("owner_binding", json!("A".repeat(64))),
            ("owner_binding", json!("a".repeat(63))),
            ("previous_ack", json!("g".repeat(64))),
            ("previous_ack", json!("")),
            ("runtime_binding", json!("B".repeat(64))),
            ("runtime_binding", json!("b".repeat(65))),
            ("runtime_binding", Value::Null),
            ("status", json!("Active")),
            ("status", json!("unknown")),
            ("status", Value::Null),
            ("status", json!("ended")),
            ("unknown", json!(true)),
        ] {
            let mut malformed = fields();
            malformed.insert(name.into(), value);
            assert!(Acknowledgement::from_fields(&malformed).is_err(), "accepted {name}: {malformed:?}");
            assert!(crate::registry().validate_claim("notice-owner/default", KIND, &malformed).is_err());
        }
        let spec = crate::registry().claim(KIND).unwrap();
        assert_eq!(spec.write_policy, crate::WritePolicy::SystemOnly);
        assert_eq!(spec.cardinality, crate::Cardinality::Append);
        assert!(!spec.additional_fields);
        assert!(crate::registry().validate_claim("host/example", KIND, &fields()).is_err());
    }
}
