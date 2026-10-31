//! TaskManifest v1 shared wire types. Authorization belongs to the admitting daemon.
//! Optional members remain absent; interpreting a default never rewrites the document.
use serde::{Deserialize, Serialize};

mod browser;
mod locator;
mod strict;
mod validation;
pub use browser::{
    TaskBrowserStepV1, TaskClickEffectV1, TaskHumanKindV1, TaskRevealSourceKindV1,
    TaskRevealTriggerV1, TaskTextNodeV1,
};
pub use locator::{AriaRole, TaskLocatorV1, TaskNonFrameLocatorV1};

/// Closed Manifest v1 document. Parsing types does not establish live authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskManifestV1 {
    pub version: u64,
    pub title: String,
    pub requester: TaskRequesterV1,
    pub window: TaskWindowV1,
    pub provider: TaskProviderV1,
    pub browser_steps: Vec<TaskBrowserStepV1>,
    pub writes: Vec<TaskWriteV1>,
    pub readers: Vec<TaskReaderV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskManifestError(pub String);

impl std::fmt::Display for TaskManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TaskManifestError {}

fn invalid(message: impl Into<String>) -> TaskManifestError {
    TaskManifestError(message.into())
}

/// The shared v1 blank predicate uses ECMAScript TrimString, never Rust trim.
/// WhiteSpace + LineTerminator: 0009–000D, 0020, 00A0, 1680, 2000–200A,
/// 2028, 2029, 202F, 205F, 3000, FEFF. In particular, 0085 is not whitespace.
fn ecma_blank(value: &str) -> bool {
    value.chars().all(|c| matches!(c,
        '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}' | '\u{feff}'
    ))
}

/// Strictly parse and cross-validate the closed v1 manifest scope.
/// Successful schema validation does not authenticate or confer live authority.
pub fn parse_task_manifest_v1(input: &[u8]) -> Result<TaskManifestV1, TaskManifestError> {
    let value = strict::parse_json(input).map_err(|error| invalid(error.to_string()))?;
    let manifest: TaskManifestV1 =
        serde_json::from_value(value).map_err(|error| invalid(error.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

/// Largest integer exactly representable by both manifest consumers.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

fn present<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRequesterV1 {
    pub principal: String,
    pub run: String,
    pub step: String,
    pub incarnation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskWindowV1 {
    pub not_before: String,
    pub not_after: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskProviderV1 {
    pub name: String,
    pub browser_profile: String,
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskWriteV1 {
    pub vault_id: String,
    pub item_id: String,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub item_op: Option<TaskItemOpV1>,
    pub fields: Vec<TaskWriteFieldV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskWriteFieldV1 {
    pub inventory_id: String,
    pub kind: TaskFieldKindV1,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub op: Option<TaskWriteOpV1>,
}

impl TaskWriteFieldV1 {
    /// Omission means set operationally, but stays omitted on the wire.
    pub fn effective_op(&self) -> TaskWriteOpV1 {
        self.op.unwrap_or(TaskWriteOpV1::Set)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskItemOpV1 { Archive, Destroy }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskFieldKindV1 { Secret, Metadata }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskWriteOpV1 { Set, Remove }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskReaderV1 {
    pub principal: String,
    pub peer_host: String,
    pub inventory_ids: Vec<String>,
    pub max_uses: u64,
}

#[cfg(test)]
mod core_tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn task_manifest_core_preserves_omitted_defaults() {
        for op in [None, Some("set"), Some("remove")] {
            let mut value = json!({"inventoryId":"token","kind":"secret"});
            if let Some(op) = op { value["op"] = json!(op); }
            let field: TaskWriteFieldV1 = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(&field).unwrap(), value);
            assert_eq!(field.effective_op(), if op == Some("remove") { TaskWriteOpV1::Remove } else { TaskWriteOpV1::Set });
        }
    }

    #[test]
    fn task_manifest_core_is_closed_and_absent_not_null() {
        let field = json!({"inventoryId":"token","kind":"secret"});
        for (key, value) in [("op", Value::Null), ("op", json!("replace")), ("kind", json!("password")), ("unknown", json!(true))] {
            let mut invalid = field.clone();
            invalid[key] = value;
            assert!(serde_json::from_value::<TaskWriteFieldV1>(invalid).is_err());
        }
        for operation in ["archive", "destroy"] {
            let value = json!({"vaultId":"vault","itemId":"item","itemOp":operation,"fields":[]});
            let write: TaskWriteV1 = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(write).unwrap(), value);
        }
        let mut requester = json!({"principal":"agent/example","run":"run","step":"step","incarnation":"incarnation"});
        requester["unknown"] = json!(true);
        assert!(serde_json::from_value::<TaskRequesterV1>(requester).is_err());
        assert!(serde_json::from_value::<TaskWindowV1>(json!({"notBefore":"a","notAfter":"b","extra":1})).is_err());
        assert!(serde_json::from_value::<TaskProviderV1>(json!({"name":"a","browserProfile":"b","origins":[],"extra":1})).is_err());
        assert!(serde_json::from_value::<TaskReaderV1>(json!({"principal":"agent/a","peerHost":"peer","inventoryIds":[],"maxUses":1,"extra":1})).is_err());
    }
}
