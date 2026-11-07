//! Explicit notice ownership binds an admitted fleet incarnation, not an agent runtime.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ValidationError, error};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub member: String,
    pub incarnation: String,
    #[serde(rename = "always-on")]
    pub always_on: bool,
}

fn invalid(message: impl Into<String>) -> ValidationError {
    error("invalid-notice-owner", message)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 512
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-@/".contains(&b))
        && name.split('/').all(|part| !part.is_empty() && part != "..")
}

impl Declaration {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let host = self.member.strip_prefix("host/");
        if crate::registry().validate_subject(&self.member).is_err()
            || !host.is_some_and(valid_name)
        {
            return Err(invalid("notice-owner member must be a concrete host/NAME subject"));
        }
        if self.incarnation.len() != 64
            || !self.incarnation.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("notice-owner incarnation must be a lowercase admission claim hash"));
        }
        if !self.always_on {
            return Err(invalid("notice-owner requires explicit always-on=true"));
        }
        Ok(())
    }
}

impl Declaration {
    /// Decode the canonical desired KDL node; no placement or lifecycle implies ownership.
    pub fn from_desired(desired: &Value) -> Result<Self, ValidationError> {
        let node = desired.as_object().ok_or_else(|| invalid("notice-owner needs a canonical node"))?;
        if node.keys().any(|key| !matches!(key.as_str(), "name" | "arguments" | "properties" | "children"))
            || desired["name"] != "notice-owner"
            || desired["arguments"].as_array().is_none_or(|args| {
                args.len() != 1 || args[0].as_str().is_none_or(|name| {
                    !valid_name(name)
                        || crate::registry().validate_subject(&format!("notice-owner/{name}")).is_err()
                })
            })
            || node.get("children").is_some_and(|children| children.as_array().is_none_or(|items| !items.is_empty()))
        {
            return Err(invalid("notice-owner needs one name argument and no children or extra fields"));
        }
        let declaration: Self = serde_json::from_value(desired["properties"].clone())
            .map_err(|e| invalid(format!("invalid notice-owner properties: {e}")))?;
        declaration.validate()?;
        Ok(declaration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn notice_contract_requires_exact_explicit_properties() {
        let valid = json!({"name": "notice-owner", "arguments": ["default"],
            "properties": {"member": "host/example", "incarnation": "a".repeat(64), "always-on": true}});
        let declaration = Declaration::from_desired(&valid).unwrap();
        assert_eq!(serde_json::to_value(&declaration).unwrap(), valid["properties"]);
        for (key, value) in [
            ("always-on", json!(false)), ("always-on", Value::Null),
            ("member", json!("agent/worker")), ("member", json!("host/")),
            ("member", json!("host/../example")), ("incarnation", json!("A".repeat(64))),
            ("incarnation", json!("short")), ("unknown", json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid["properties"][key] = value;
            assert!(Declaration::from_desired(&invalid).is_err(), "{invalid}");
        }
        let mut missing = valid.clone();
        missing["properties"].as_object_mut().unwrap().remove("always-on");
        assert!(Declaration::from_desired(&missing).is_err());
        for (key, value) in [("children", json!([{"name": "agent"}])),
            ("arguments", json!(["default", "extra"])), ("extra", json!(true))] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(Declaration::from_desired(&invalid).is_err());
        }
    }
}
