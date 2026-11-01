use serde::{Deserialize, Serialize};

use super::locator::TaskLocatorV1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskClickEffectV1 {
    Delete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskHumanKindV1 {
    Phone,
    SecurityKey,
    Passkey,
    MacosPermission,
    LegalConsent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskRevealSourceKindV1 {
    Input,
    MaskedText,
    TextNode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRevealTriggerV1 {
    pub locator: TaskLocatorV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskTextNodeV1 {
    pub pattern: String,
}

/// Closed operation shapes; validation checks paired optional fields and bounds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TaskBrowserStepV1 {
    Navigate {
        index: u64,
        literal: String,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        locator: Option<TaskLocatorV1>,
    },
    Click {
        index: u64,
        locator: TaskLocatorV1,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        effect: Option<TaskClickEffectV1>,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    Fill {
        index: u64,
        locator: TaskLocatorV1,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        literal: Option<String>,
    },
    FillSecret {
        index: u64,
        locator: TaskLocatorV1,
        inventory_id: String,
    },
    RevealOnce {
        index: u64,
        locator: TaskLocatorV1,
        trigger: TaskRevealTriggerV1,
        issues: String,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        intent: Option<String>,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        safe_url: Option<String>,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        source_kind: Option<TaskRevealSourceKindV1>,
        #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
        text_node: Option<TaskTextNodeV1>,
    },
    Human {
        index: u64,
        kind: TaskHumanKindV1,
        note: String,
    },
}

impl TaskBrowserStepV1 {
    pub fn index(&self) -> u64 {
        match self {
            Self::Navigate { index, .. }
            | Self::Click { index, .. }
            | Self::Fill { index, .. }
            | Self::FillSecret { index, .. }
            | Self::RevealOnce { index, .. }
            | Self::Human { index, .. } => *index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TaskBrowserStepV1;
    use serde_json::{Value, json};

    fn examples() -> Vec<Value> {
        let locator = json!({"kind":"role","role":"button","name":"Save"});
        let mut cases = vec![
            json!({"op":"navigate","index":7,"literal":"https://example.com"}),
            json!({"op":"navigate","index":7,"literal":"https://example.com","locator":locator}),
            json!({"op":"click","index":7,"locator":locator}),
            json!({"op":"click","index":7,"locator":locator,"effect":"delete","target":"old key"}),
            json!({"op":"fill","index":7,"locator":locator}),
            json!({"op":"fill","index":7,"locator":locator,"literal":"name"}),
            json!({"op":"fillSecret","index":7,"locator":locator,"inventoryId":"password"}),
            json!({"op":"revealOnce","index":7,"locator":locator,"trigger":{"locator":locator},"issues":"new-key"}),
        ];
        for source_kind in ["input", "maskedText", "textNode"] {
            let mut value = json!({"op":"revealOnce","index":7,"locator":locator,"trigger":{"locator":locator},"issues":"new-key","sourceKind":source_kind,"intent":"Create key","safeUrl":"https://example.com"});
            if source_kind == "textNode" {
                value["textNode"] = json!({"pattern":"key_[A-Z]+"});
            }
            cases.push(value);
        }
        for kind in ["phone", "securityKey", "passkey", "macosPermission", "legalConsent"] {
            cases.push(json!({"op":"human","index":7,"kind":kind,"note":"Continue manually"}));
        }
        cases
    }

    #[test]
    fn every_operation_round_trips_with_absent_optionals_preserved() {
        for value in examples() {
            let step: TaskBrowserStepV1 = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(step.index(), 7);
            assert_eq!(serde_json::to_value(step).unwrap(), value);
        }
    }

    #[test]
    fn every_operation_rejects_unknown_keys() {
        for mut value in examples() {
            value.as_object_mut().unwrap().insert("extra".into(), json!(true));
            assert!(serde_json::from_value::<TaskBrowserStepV1>(value).is_err());
        }
    }

    #[test]
    fn operation_shapes_reject_foreign_fields_and_unknown_tags() {
        let locator = json!({"kind":"text","value":"Save"});
        let mut invalid = vec![
            json!({"op":"script","index":7}),
            json!({"op":"click","index":7,"locator":locator,"effect":"archive","target":"key"}),
            json!({"op":"human","index":7,"kind":"captcha","note":"Continue"}),
            json!({"op":"revealOnce","index":7,"locator":locator,"trigger":{"locator":locator},"issues":"key","sourceKind":"text"}),
            json!({"op":"revealOnce","index":7,"locator":locator,"trigger":{"locator":locator,"extra":true},"issues":"key"}),
            json!({"op":"revealOnce","index":7,"locator":locator,"trigger":{"locator":locator},"issues":"key","sourceKind":"textNode","textNode":{"pattern":"key","extra":true}}),
            json!({"op":"fillSecret","index":7,"locator":locator,"inventory_id":"key"}),
            json!({"op":"fill","index":-1,"locator":locator}),
            json!({"op":"fill","index":1.5,"locator":locator}),
        ];
        for value in examples() {
            let mut without_index = value;
            without_index.as_object_mut().unwrap().remove("index");
            invalid.push(without_index);
        }
        for (op, field, payload) in [
            ("navigate", "effect", json!("delete")),
            ("click", "literal", json!("secret")),
            ("fill", "inventoryId", json!("key")),
            ("fillSecret", "literal", json!("secret")),
            ("human", "locator", locator.clone()),
        ] {
            let mut value = examples().into_iter().find(|value| value["op"] == op).unwrap();
            value[field] = payload;
            invalid.push(value);
        }
        for value in invalid {
            assert!(serde_json::from_value::<TaskBrowserStepV1>(value).is_err());
        }
    }
}
