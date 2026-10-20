//! Task grant v1 wire contract and deterministic, secret-free manifest identity.
//!
//! Validation is explicit at admission. Canonicalization validates first and implements
//! RFC 8785 for this contract's integer-only JSON subset (no floating-point values).
use std::collections::BTreeSet;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const OP_PROXY_PRINCIPAL: &str = "service/op-proxy";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskGrantError(pub String);

impl fmt::Display for TaskGrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TaskGrantError {}

fn invalid(message: impl Into<String>) -> TaskGrantError {
    TaskGrantError(message.into())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskManifest {
    pub version: u64,
    pub title: String,
    pub requester: TaskRequester,
    pub window: TaskGrantWindow,
    pub provider: TaskProvider,
    pub browser_steps: Vec<TaskBrowserStep>,
    pub writes: Vec<TaskWrite>,
    pub readers: Vec<TaskReader>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRequester {
    pub principal: String,
    pub run: String,
    pub step: String,
    pub incarnation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGrantWindow {
    pub not_before: String,
    pub not_after: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskProvider {
    pub name: String,
    pub browser_profile: String,
    pub origins: Vec<String>,
}

// An omitted optional member stays omitted. Explicit null is not a second spelling
// of omission: accepting it would silently change the document before hashing.
fn present<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskBrowserStep {
    pub index: u64,
    pub op: TaskBrowserOp,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub literal: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub issues: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub effect: Option<TaskBrowserEffect>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub inventory_id: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub kind: Option<TaskHumanKind>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskBrowserOp {
    Navigate,
    Click,
    Fill,
    RevealOnce,
    FillSecret,
    Human,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskBrowserEffect {
    Delete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskHumanKind {
    Phone,
    SecurityKey,
    Passkey,
    MacosPermission,
    LegalConsent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskWriteOp {
    Set,
    Remove,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskItemOp {
    Archive,
    Destroy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskWrite {
    pub vault_id: String,
    pub item_id: String,
    pub fields: Vec<TaskWriteField>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub item_op: Option<TaskItemOp>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskWriteField {
    pub inventory_id: String,
    pub kind: TaskFieldKind,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub op: Option<TaskWriteOp>,
}

impl TaskWriteField {
    /// Interpret omission as `set` without changing the hashed wire document.
    pub fn effective_op(&self) -> TaskWriteOp {
        self.op.unwrap_or(TaskWriteOp::Set)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskFieldKind {
    Secret,
    Metadata,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskReader {
    pub principal: String,
    pub peer_host: String,
    pub inventory_ids: Vec<String>,
    pub max_uses: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTaskGrantRequest {
    pub manifest: TaskManifest,
    pub op_proxy_case_id: String,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevokeTaskGrant {
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskGrantState {
    Active,
    Expired,
    Revoked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGrantReceipt {
    pub grant_id: String,
    pub digest: String,
    pub window: TaskGrantWindow,
    pub state: TaskGrantState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGrantRecord {
    pub grant_id: String,
    pub manifest: TaskManifest,
    pub digest: String,
    pub op_proxy_case_id: String,
    pub window: TaskGrantWindow,
    pub state: TaskGrantState,
}

fn text(name: &str, value: &str) -> Result<(), TaskGrantError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(invalid(format!("{name} must be nonempty and contain no control characters")));
    }
    Ok(())
}

fn exact_principal(value: &str) -> bool {
    let mut parts = value.split('/');
    matches!(parts.next(), Some("person" | "agent" | "task" | "service"))
        && parts.clone().next().is_some()
        && parts.all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')) && !matches!(part, "." | ".."))
}

fn https_origin(value: &str) -> bool {
    let Some(authority) = value.strip_prefix("https://") else { return false; };
    if authority.is_empty() || authority.bytes().any(|byte| !byte.is_ascii() || byte.is_ascii_whitespace() || matches!(byte, b'/' | b'\\' | b'?' | b'#' | b'@' | b'*' | b'%')) {
        return false;
    }
    let (host, port) = if let Some(ipv6) = authority.strip_prefix('[') {
        let Some((address, suffix)) = ipv6.split_once(']') else { return false; };
        if address.parse::<std::net::Ipv6Addr>().is_err() { return false; }
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        };
        (address, port)
    } else {
        let (host, port) = authority.split_once(':').map_or((authority, None), |(host, port)| (host, Some(port)));
        if host.len() > 253 || host.split('.').any(|label| label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') || !label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')) {
            return false;
        }
        (host, port)
    };
    !host.is_empty() && port.is_none_or(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) && port.parse::<u16>().is_ok_and(|port| port != 0))
}

impl TaskGrantWindow {
    pub fn bounds(&self) -> Result<(DateTime<Utc>, DateTime<Utc>), TaskGrantError> {
        fn parse(value: &str) -> Result<DateTime<Utc>, TaskGrantError> {
            let time = DateTime::parse_from_rfc3339(value).map_err(|_| invalid("window timestamps must be RFC3339 UTC strings"))?;
            if time.offset().local_minus_utc() != 0 || value.ends_with("-00:00") {
                return Err(invalid("window timestamps must be UTC"));
            }
            Ok(time.with_timezone(&Utc))
        }
        let start = parse(&self.not_before)?;
        let end = parse(&self.not_after)?;
        if end <= start || end.signed_duration_since(start) > chrono::Duration::hours(24) {
            return Err(invalid("window must increase and span at most 24 hours"));
        }
        Ok((start, end))
    }

    pub fn validate(&self) -> Result<(), TaskGrantError> {
        self.bounds().map(|_| ())
    }
}

impl TaskManifest {
    pub fn validate(&self) -> Result<(), TaskGrantError> {
        if self.version != 1 { return Err(invalid("manifest version must be 1")); }
        text("title", &self.title)?;
        if !exact_principal(&self.requester.principal) { return Err(invalid("requester principal must be exact")); }
        for (name, value) in [("requester run", &self.requester.run), ("requester step", &self.requester.step), ("requester incarnation", &self.requester.incarnation), ("provider name", &self.provider.name), ("browser profile", &self.provider.browser_profile)] {
            text(name, value)?;
        }
        self.window.validate()?;
        if self.provider.origins.is_empty() || self.provider.origins.iter().any(|origin| !https_origin(origin)) {
            return Err(invalid("provider origins must be exact HTTPS origins without paths or wildcards"));
        }
        let mut items = BTreeSet::new();
        let mut inventory = BTreeSet::new();
        for write in &self.writes {
            text("vaultId", &write.vault_id)?;
            text("itemId", &write.item_id)?;
            if write.item_op.is_some() && !write.fields.is_empty() {
                return Err(invalid("itemOp requires empty fields"));
            }
            if write.item_op == Some(TaskItemOp::Destroy) && write.item_id.contains('*') {
                return Err(invalid("destroy requires an exact itemId"));
            }
            if !items.insert((&write.vault_id, &write.item_id)) { return Err(invalid("duplicate vaultId/itemId write")); }
            for field in &write.fields {
                text("inventoryId", &field.inventory_id)?;
                if field.op == Some(TaskWriteOp::Remove) && field.inventory_id.contains('*') {
                    return Err(invalid("remove requires an exact inventoryId"));
                }
                if !inventory.insert(field.inventory_id.as_str()) { return Err(invalid("duplicate inventoryId")); }
            }
        }
        let mut last_index = None;
        for step in &self.browser_steps {
            if step.index > MAX_SAFE_INTEGER || last_index.is_some_and(|last| step.index <= last) { return Err(invalid("browser step indexes must be increasing safe integers")); }
            last_index = Some(step.index);
            if let Some(locator) = &step.locator {
                text("locator", locator)?;
            }
            if matches!(step.op, TaskBrowserOp::Click | TaskBrowserOp::Fill | TaskBrowserOp::FillSecret | TaskBrowserOp::RevealOnce) {
                text("locator", step.locator.as_deref().ok_or_else(|| invalid("operation requires locator"))?)?;
            }
            if step.op == TaskBrowserOp::Navigate {
                let literal = step.literal.as_deref().ok_or_else(|| invalid("navigate requires literal HTTPS URL"))?;
                let url = url::Url::parse(literal).map_err(|_| invalid("navigate requires an exact HTTPS URL"))?;
                let authority = literal.strip_prefix("https://").unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
                if !literal.starts_with("https://") || !https_origin(&literal[..8 + authority.len()]) {
                    return Err(invalid("navigate requires an exact HTTPS URL"));
                }
                if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
                    || literal.chars().any(|character| character.is_control() || character.is_whitespace())
                    || literal.contains('\\') || !literal.starts_with("https://")
                    || !self.provider.origins.iter().any(|origin| {
                        url::Url::parse(origin).is_ok_and(|allowed| allowed.origin() == url.origin())
                    })
                {
                    return Err(invalid("navigate URL origin must be declared by provider"));
                }
            }
            if step.effect == Some(TaskBrowserEffect::Delete) {
                let target = step.target.as_deref().ok_or_else(|| invalid("delete requires target"))?;
                text("delete target", target)?;
                if target.contains('*') { return Err(invalid("delete requires an exact target")); }
            }
            if step.op == TaskBrowserOp::FillSecret {
                let id = step.inventory_id.as_deref().ok_or_else(|| invalid("fillSecret requires inventoryId"))?;
                if !self.readers.iter().any(|reader| reader.principal == self.requester.principal && reader.inventory_ids.iter().any(|allowed| allowed == id)) {
                    return Err(invalid("fillSecret requires inventoryId granted to requester"));
                }
            }
            if step.op == TaskBrowserOp::Human {
                if step.kind.is_none() { return Err(invalid("human requires kind")); }
                text("human note", step.note.as_deref().ok_or_else(|| invalid("human requires note"))?)?;
                if step.locator.is_some() || step.literal.is_some() || step.inventory_id.is_some() || step.issues.is_some() {
                    return Err(invalid("human must not carry locator, literal, inventoryId or issues"));
                }
            }
            if step.literal.is_some() && !matches!(step.op, TaskBrowserOp::Fill | TaskBrowserOp::Navigate) { return Err(invalid("literal is allowed only on fill or navigate")); }
            if let Some(issue) = &step.issues {
                if step.op != TaskBrowserOp::RevealOnce { return Err(invalid("issues is allowed only on revealOnce")); }
                if !inventory.contains(issue.as_str()) { return Err(invalid("issued inventoryId must be declared in writes")); }
            }
        }
        for reader in &self.readers {
            if !exact_principal(&reader.principal) || !(reader.principal.starts_with("agent/") || reader.principal.starts_with("task/")) { return Err(invalid("reader principal must be an exact agent or task principal")); }
            text("peerHost", &reader.peer_host)?;
            if reader.peer_host.contains('*') { return Err(invalid("peerHost must be exact")); }
            if reader.max_uses == 0 || reader.max_uses > MAX_SAFE_INTEGER { return Err(invalid("maxUses must be a positive safe integer")); }
            if reader.inventory_ids.iter().any(|id| !inventory.contains(id.as_str())) { return Err(invalid("reader inventoryIds must be declared in writes")); }
        }
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, TaskGrantError> {
        self.validate()?;
        canonical_json(&serde_json::to_value(self).map_err(|error| invalid(error.to_string()))?)
    }

    pub fn digest(&self) -> Result<String, TaskGrantError> {
        Ok(hex::encode(Sha256::digest(self.canonical_json()?.as_bytes())))
    }
}

impl CreateTaskGrantRequest {
    pub fn validate(&self) -> Result<(), TaskGrantError> {
        text("opProxyCaseId", &self.op_proxy_case_id)?;
        if self.digest != self.manifest.digest()? { return Err(invalid("digest does not match canonical manifest")); }
        Ok(())
    }
}

/// RFC 8785 serialization for exact safe integers, strings, arrays and objects.
/// Object keys use UTF-16 ordering, not Rust's Unicode scalar ordering.
pub fn canonical_json(value: &Value) -> Result<String, TaskGrantError> {
    fn append(value: &Value, out: &mut String) -> Result<(), TaskGrantError> {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Value::Number(number) => {
                let safe = number.as_u64().is_some_and(|number| number <= MAX_SAFE_INTEGER)
                    || number.as_i64().is_some_and(|number| number.unsigned_abs() <= MAX_SAFE_INTEGER);
                if !safe { return Err(invalid("canonical JSON accepts only exact safe integers, never floats")); }
                out.push_str(&number.to_string());
            }
            Value::String(value) => out.push_str(&serde_json::to_string(value).map_err(|error| invalid(error.to_string()))?),
            Value::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 { out.push(','); }
                    append(value, out)?;
                }
                out.push(']');
            }
            Value::Object(values) => {
                let mut entries: Vec<_> = values.iter().collect();
                entries.sort_unstable_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()));
                out.push('{');
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index != 0 { out.push(','); }
                    out.push_str(&serde_json::to_string(key).map_err(|error| invalid(error.to_string()))?);
                    out.push(':');
                    append(value, out)?;
                }
                out.push('}');
            }
        }
        Ok(())
    }
    let mut result = String::new();
    append(value, &mut result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GOLDEN_INPUT: &str = r#"{
        "version":1,"title":"Example grant",
        "requester":{"principal":"person/robin","run":"example-run","step":"example-step","incarnation":"example-1"},
        "window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},
        "provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},
        "browserSteps":[],"writes":[],"readers":[]
    }"#;
    const GOLDEN_CANONICAL: &str = r#"{"browserSteps":[],"provider":{"browserProfile":"example","name":"example","origins":["https://example.com"]},"readers":[],"requester":{"incarnation":"example-1","principal":"person/robin","run":"example-run","step":"example-step"},"title":"Example grant","version":1,"window":{"notAfter":"2026-10-20T13:00:00Z","notBefore":"2026-10-20T12:00:00Z"},"writes":[]}"#;
    const GOLDEN_DIGEST: &str =
        "70c88a0ad7bb265cf380bfa29f231eb2881ae6ff04cbf68692799946a7ec5e89";

    fn fixture() -> Value {
        let mut value: Value = serde_json::from_str(GOLDEN_INPUT).unwrap();
        value["browserSteps"] = json!([
            {"index":0,"op":"navigate","locator":"page","literal":"https://example.com"},
            {"index":1,"op":"fill","locator":"label","literal":"public label"},
            {"index":2,"op":"click","locator":"submit"},
            {"index":3,"op":"revealOnce","locator":"token","issues":"example-token"}
        ]);
        value["writes"] = json!([{
            "vaultId":"example-vault","itemId":"example-item",
            "fields":[{"inventoryId":"example-token","kind":"secret"},{"inventoryId":"example-label","kind":"metadata"}]
        }]);
        value["readers"] = json!([{
            "principal":"agent/example-reader","peerHost":"example-peer",
            "inventoryIds":["example-token","example-label"],"maxUses":1
        }]);
        value
    }

    fn accepted(value: Value) -> TaskManifest {
        let manifest: TaskManifest = serde_json::from_value(value).unwrap();
        manifest.validate().unwrap();
        manifest
    }

    fn refused(value: Value) {
        if let Ok(manifest) = serde_json::from_value::<TaskManifest>(value) {
            assert!(manifest.validate().is_err(), "unexpectedly admitted {manifest:?}");
            assert!(manifest.canonical_json().is_err());
            assert!(manifest.digest().is_err());
        }
    }

    #[test]
    fn manifest_golden_vector() {
        let manifest: TaskManifest = serde_json::from_str(GOLDEN_INPUT).unwrap();
        assert_eq!(manifest.canonical_json().unwrap(), GOLDEN_CANONICAL);
        assert_eq!(manifest.digest().unwrap(), GOLDEN_DIGEST);
        let reordered: TaskManifest = serde_json::from_str(GOLDEN_CANONICAL).unwrap();
        assert_eq!(reordered.digest().unwrap(), GOLDEN_DIGEST);
    }

    #[test]
    fn full_manifest_and_wire_round_trip() {
        let manifest = accepted(fixture());
        let request = CreateTaskGrantRequest {
            digest: manifest.digest().unwrap(),
            manifest,
            op_proxy_case_id: "example-case".into(),
        };
        request.validate().unwrap();
        let serialized = serde_json::to_value(&request).unwrap();
        assert!(serialized.get("opProxyCaseId").is_some());
        assert_eq!(serde_json::from_value::<CreateTaskGrantRequest>(serialized).unwrap(), request);
        let receipt = TaskGrantReceipt {
            grant_id: "example-grant".into(),
            digest: request.digest.clone(),
            window: request.manifest.window.clone(),
            state: TaskGrantState::Active,
        };
        assert_eq!(serde_json::to_value(&receipt).unwrap()["state"], "active");
        assert_eq!(serde_json::from_value::<TaskGrantReceipt>(serde_json::to_value(&receipt).unwrap()).unwrap(), receipt);
        for state in [TaskGrantState::Active, TaskGrantState::Expired, TaskGrantState::Revoked] {
            let record = TaskGrantRecord {
                grant_id: receipt.grant_id.clone(),
                manifest: request.manifest.clone(),
                digest: request.digest.clone(),
                op_proxy_case_id: request.op_proxy_case_id.clone(),
                window: receipt.window.clone(),
                state,
            };
            assert_eq!(serde_json::from_value::<TaskGrantRecord>(serde_json::to_value(&record).unwrap()).unwrap(), record);
        }
        let revoke: RevokeTaskGrant = serde_json::from_value(json!({"reason":"example ended"})).unwrap();
        assert_eq!(revoke.reason, "example ended");
    }

    #[test]
    fn unknown_fields_refused_at_every_manifest_level() {
        for pointer in ["", "/requester", "/window", "/provider", "/browserSteps/0", "/writes/0", "/writes/0/fields/0", "/readers/0"] {
            let mut value = fixture();
            value.pointer_mut(pointer).unwrap().as_object_mut().unwrap().insert("unexpected".into(), json!("refused"));
            assert!(serde_json::from_value::<TaskManifest>(value).is_err(), "{pointer}");
        }
        let mut value = fixture();
        value["writes"][0]["fields"][0]["value"] = json!("must not accept secret values");
        assert!(serde_json::from_value::<TaskManifest>(value).is_err());
    }

    #[test]
    fn unknown_wire_fields_and_nested_manifest_refused() {
        let manifest = accepted(fixture());
        let request = json!({"manifest":manifest,"opProxyCaseId":"example-case","digest":"example-digest","unexpected":true});
        assert!(serde_json::from_value::<CreateTaskGrantRequest>(request.clone()).is_err());
        let mut nested = request;
        nested.as_object_mut().unwrap().remove("unexpected");
        nested["manifest"]["requester"]["unexpected"] = json!(true);
        assert!(serde_json::from_value::<CreateTaskGrantRequest>(nested).is_err());
        assert!(serde_json::from_value::<RevokeTaskGrant>(json!({"reason":"example","unexpected":true})).is_err());
        let receipt = json!({"grantId":"example-grant","digest":"example-digest","window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},"state":"active","unexpected":true});
        assert!(serde_json::from_value::<TaskGrantReceipt>(receipt.clone()).is_err());
        let mut record = receipt;
        record["manifest"] = fixture();
        record["opProxyCaseId"] = json!("example-case");
        assert!(serde_json::from_value::<TaskGrantRecord>(record).is_err());
        assert!(serde_json::from_value::<TaskGrantState>(json!("pending")).is_err());
    }

    #[test]
    fn required_fields_version_and_integer_types() {
        let base = fixture();
        for field in ["version", "title", "requester", "window", "provider", "browserSteps", "writes", "readers"] {
            let mut value = base.clone();
            value.as_object_mut().unwrap().remove(field);
            refused(value);
        }
        for pointer in ["/requester/principal", "/requester/run", "/requester/step", "/requester/incarnation", "/window/notBefore", "/window/notAfter"] {
            let mut value = base.clone();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            value.pointer_mut(parent).unwrap().as_object_mut().unwrap().remove(key);
            refused(value);
        }
        for version in [json!(0), json!(2), json!(1.0), json!(-1), json!(null)] {
            let mut value = base.clone();
            value["version"] = version;
            refused(value);
        }
        for pointer in ["/browserSteps/0/index", "/readers/0/maxUses"] {
            for number in [json!(1.5), json!(1.0), json!(-1), json!(MAX_SAFE_INTEGER + 1)] {
                let mut value = base.clone();
                *value.pointer_mut(pointer).unwrap() = number;
                refused(value);
            }
        }
    }

    #[test]
    fn windows_require_utc_increasing_and_at_most_one_day() {
        for (start, end) in [
            ("", "2026-10-20T13:00:00Z"),
            ("2026-10-20", "2026-10-20T13:00:00Z"),
            ("2026-10-20T12:00:00Z", "invalid"),
            ("2026-10-20T12:00:00+01:00", "2026-10-20T13:00:00Z"),
            ("2026-10-20T12:00:00Z", "2026-10-20T13:00:00-01:00"),
            ("2026-10-20T12:00:00-00:00", "2026-10-20T13:00:00Z"),
            ("2026-10-20T12:00:00Z", "2026-10-20T12:00:00Z"),
            ("2026-10-20T12:00:00Z", "2026-10-20T11:59:59Z"),
            ("2026-10-20T12:00:00Z", "2026-10-21T12:00:00.001Z"),
        ] {
            let mut value = fixture();
            value["window"] = json!({"notBefore":start,"notAfter":end});
            refused(value);
        }
        let mut value = fixture();
        value["window"] = json!({"notBefore":"2026-10-20T12:00:00+00:00","notAfter":"2026-10-21T12:00:00Z"});
        accepted(value);
    }

    #[test]
    fn exact_https_origins_only() {
        for origin in ["http://example.com", "https://*.example.com", "https://example.com/", "https://example.com/path", "https://example.com?query", "https://example.com#fragment", "https://user@example.com", "https://", "https://example.com:0", "https://example.com:65536", "https://example.com:", "https://example.com:abc", "https://example.com\\path", "https://example.com\n", "https://-example.com", "https://example..com", "https://example.com%2f"] {
            let mut value = fixture();
            value["provider"]["origins"] = json!([origin]);
            refused(value);
        }
        let mut value = fixture();
        value["provider"]["origins"] = json!([]);
        refused(value);
        for origin in ["https://example.com", "https://example.com:443", "https://example.com:8443"] {
            let mut value = fixture();
            value["provider"]["origins"] = json!([origin]);
            accepted(value);
        }
    }

    #[test]
    fn browser_step_operation_and_order_refusals() {
        for op in ["click", "revealOnce"] {
            let mut value = fixture();
            value["browserSteps"][0]["op"] = json!(op);
            refused(value);
        }
        for op in ["navigate", "fill", "click"] {
            let mut value = fixture();
            value["browserSteps"][3]["op"] = json!(op);
            refused(value);
        }
        for index in [0, 1] {
            let mut value = fixture();
            value["browserSteps"][2]["index"] = json!(index);
            refused(value);
        }
        let mut value = fixture();
        value["browserSteps"][0]["op"] = json!("execute");
        refused(value);
        let mut value = fixture();
        value["browserSteps"][3]["issues"] = json!("example-undeclared");
        refused(value);
        let mut value = fixture();
        value["browserSteps"][3]["literal"] = json!("not allowed");
        refused(value);
        let mut value = fixture();
        value["browserSteps"][3]["issues"] = json!({"inventoryId":"example-token"});
        refused(value);
    }

    #[test]
    fn write_and_reader_scope_refusals() {
        let mut value = fixture();
        let duplicate = value["writes"][0].clone();
        value["writes"].as_array_mut().unwrap().push(duplicate);
        refused(value);
        let mut value = fixture();
        value["writes"][0]["fields"][1]["inventoryId"] = json!("example-token");
        refused(value);
        let mut value = fixture();
        value["writes"][0]["fields"][0]["kind"] = json!("plaintext");
        refused(value);
        for principal in ["person/robin", "service/op-proxy", "host/example", "agent/*", "agent/", "agent//example", "agent/example@peer", "task/../example", "agent/example reader", "agent/example?scope"] {
            let mut value = fixture();
            value["readers"][0]["principal"] = json!(principal);
            refused(value);
        }
        for principal in ["agent/example-reader", "task/example-task"] {
            let mut value = fixture();
            value["readers"][0]["principal"] = json!(principal);
            accepted(value);
        }
        let mut value = fixture();
        value["readers"][0]["maxUses"] = json!(0);
        refused(value);
        let mut value = fixture();
        value["readers"][0]["inventoryIds"] = json!(["example-undeclared"]);
        refused(value);
        let mut value = fixture();
        value["readers"][0]["peerHost"] = json!("*");
        refused(value);
    }

    #[test]
    fn identifiers_are_nonempty_and_no_controls() {
        for pointer in ["/title", "/requester/run", "/requester/step", "/requester/incarnation", "/provider/name", "/provider/browserProfile", "/browserSteps/0/locator", "/writes/0/vaultId", "/writes/0/itemId", "/writes/0/fields/0/inventoryId", "/readers/0/peerHost"] {
            for text in ["", "  ", "example\n"] {
                let mut value = fixture();
                *value.pointer_mut(pointer).unwrap() = json!(text);
                refused(value);
            }
        }
        for principal in ["person/", "person/*", "host/example", "person/../robin", "person/robin@example"] {
            let mut value = fixture();
            value["requester"]["principal"] = json!(principal);
            refused(value);
        }
    }

    #[test]
    fn request_digest_binds_entire_manifest() {
        let manifest = accepted(fixture());
        let mut request = CreateTaskGrantRequest {
            digest: manifest.digest().unwrap(),
            manifest,
            op_proxy_case_id: "example-case".into(),
        };
        request.validate().unwrap();
        request.manifest.title.push('!');
        assert!(request.validate().is_err());
        request.digest = request.manifest.digest().unwrap();
        request.op_proxy_case_id.clear();
        assert!(request.validate().is_err());
    }

    #[test]
    fn jcs_sorts_utf16_and_preserves_array_order_and_string_escaping() {
        let value = json!({"\u{e000}":1,"\u{10000}":2,"a":["\u{000f}\n\"\\",true,false,null]});
        assert_eq!(canonical_json(&value).unwrap(), "{\"a\":[\"\\u000f\\n\\\"\\\\\",true,false,null],\"\u{10000}\":2,\"\u{e000}\":1}");
        assert_ne!(canonical_json(&json!([1,2])).unwrap(), canonical_json(&json!([2,1])).unwrap());
    }

    #[test]
    fn jcs_refuses_floats_and_unsafe_integers_recursively() {
        for value in [json!(1.0), json!(1.5), json!(-0.0), json!(MAX_SAFE_INTEGER + 1), json!(-9_007_199_254_740_992_i64)] {
            assert!(canonical_json(&value).is_err());
            assert!(canonical_json(&json!({"nested":[value]})).is_err());
        }
        for number in [json!(MAX_SAFE_INTEGER), json!(-9_007_199_254_740_991_i64), json!(0)] {
            assert_eq!(canonical_json(&number).unwrap(), number.to_string());
        }
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;
    use serde_json::json;

    const INPUT: &str = r#"{"version":1,"title":"Example extended grant","requester":{"principal":"agent/example-requester","run":"example-run","step":"example-step","incarnation":"example-1"},"window":{"notBefore":"2026-10-20T12:00:00Z","notAfter":"2026-10-20T13:00:00Z"},"provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},"browserSteps":[{"index":0,"op":"fillSecret","locator":"token","inventoryId":"example-token"},{"index":1,"op":"human","kind":"passkey","note":"Approve example action"},{"index":2,"op":"click","locator":"delete","effect":"delete","target":"example-record"}],"writes":[{"vaultId":"example-vault","itemId":"example-item","fields":[{"inventoryId":"example-token","kind":"secret","op":"remove"}]},{"vaultId":"example-vault","itemId":"example-old-item","fields":[],"itemOp":"destroy"}],"readers":[{"principal":"agent/example-requester","peerHost":"example-peer","inventoryIds":["example-token"],"maxUses":1}]}"#;

    fn fixture() -> Value {
        serde_json::from_str(INPUT).unwrap()
    }

    fn refused(value: Value) {
        if let Ok(manifest) = serde_json::from_value::<TaskManifest>(value) {
            assert!(manifest.validate().is_err(), "unexpectedly admitted {manifest:?}");
        }
    }

    #[test]
    fn extended_golden_and_absent_operations_preserved() {
        let manifest: TaskManifest = serde_json::from_str(INPUT).unwrap();
        assert_eq!(manifest.digest().unwrap(), "02ce1423cadba9cdae9086100d3c865dba0661c6cdafab057a8b6659015220fc");
        let mut value = fixture();
        value["writes"][0]["fields"][0].as_object_mut().unwrap().remove("op");
        let absent: TaskManifest = serde_json::from_value(value.clone()).unwrap();
        assert!(!serde_json::to_value(&absent).unwrap()["writes"][0]["fields"][0].as_object().unwrap().contains_key("op"));
        value["writes"][0]["fields"][0]["op"] = json!("set");
        let explicit: TaskManifest = serde_json::from_value(value).unwrap();
        assert_ne!(absent.digest().unwrap(), explicit.digest().unwrap());
        for kind in ["phone", "securityKey", "passkey", "macosPermission", "legalConsent"] {
            let mut value = fixture();
            value["browserSteps"][1]["kind"] = json!(kind);
            serde_json::from_value::<TaskManifest>(value).unwrap().validate().unwrap();
        }
    }

    #[test]
    fn human_requires_kind_note_and_forbids_machine_payload() {
        for key in ["kind", "note"] {
            let mut value = fixture();
            value["browserSteps"][1].as_object_mut().unwrap().remove(key);
            refused(value);
        }
        for (key, payload) in [("kind", json!("unknown")), ("note", json!(" ")), ("locator", json!("example")), ("literal", json!("example")), ("inventoryId", json!("example-token")), ("issues", json!("example-token"))] {
            let mut value = fixture();
            value["browserSteps"][1][key] = payload;
            refused(value);
        }
    }

    #[test]
    fn secret_fill_requires_requester_reader_scope_and_locator() {
        for key in ["inventoryId", "locator"] {
            let mut value = fixture();
            value["browserSteps"][0].as_object_mut().unwrap().remove(key);
            refused(value);
        }
        let mut value = fixture();
        value["browserSteps"][0]["inventoryId"] = json!("example-other");
        refused(value);
        let mut value = fixture();
        value["requester"]["principal"] = json!("agent/example-other");
        refused(value);
        for (key, payload) in [("literal", json!("forbidden")), ("issues", json!("example-token"))] {
            let mut value = fixture();
            value["browserSteps"][0][key] = payload;
            refused(value);
        }
    }

    #[test]
    fn destructive_operations_require_exact_targets_and_empty_item_fields() {
        for target in ["", " ", "*", "example-*"] {
            let mut value = fixture();
            value["browserSteps"][2]["target"] = json!(target);
            refused(value);
            let mut value = fixture();
            value["writes"][1]["itemId"] = json!(target);
            refused(value);
            let mut value = fixture();
            value["writes"][0]["fields"][0]["inventoryId"] = json!(target);
            refused(value);
        }
        let mut value = fixture();
        value["browserSteps"][2].as_object_mut().unwrap().remove("target");
        refused(value);
        for item_op in ["archive", "destroy"] {
            let mut value = fixture();
            value["writes"][0]["itemOp"] = json!(item_op);
            refused(value);
            let mut value = fixture();
            value["writes"][1]["itemOp"] = json!(item_op);
            serde_json::from_value::<TaskManifest>(value).unwrap().validate().unwrap();
        }
        for (pointer, operation) in [("/browserSteps/2/effect", "erase"), ("/writes/0/fields/0/op", "append"), ("/writes/1/itemOp", "delete")] {
            let mut value = fixture();
            *value.pointer_mut(pointer).unwrap() = json!(operation);
            refused(value);
        }
    }

    #[test]
    fn navigation_requires_allowed_exact_https_url_but_no_locator() {
        for literal in ["https://example.com", "https://example.com/path?example=1#section"] {
            let mut value = fixture();
            value["browserSteps"] = json!([{"index":0,"op":"navigate","literal":literal}]);
            serde_json::from_value::<TaskManifest>(value).unwrap().validate().unwrap();
        }
        for literal in ["http://example.com", "https://example.com:8443", "https://example.com.evil", "https://*.example.com", "https://user@example.com", "https:///example.com", "https://example.com\\path", " https://example.com", "https://example.com\n"] {
            let mut value = fixture();
            value["browserSteps"] = json!([{"index":0,"op":"navigate","literal":literal}]);
            refused(value);
        }
        let mut value = fixture();
        value["browserSteps"] = json!([{"index":0,"op":"navigate"}]);
        refused(value);
        for op in ["click", "fill", "fillSecret", "revealOnce"] {
            let mut value = fixture();
            value["browserSteps"] = json!([{"index":0,"op":op,"inventoryId":"example-token"}]);
            refused(value);
        }
    }
}

#[cfg(test)]
mod optional_member_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_null_is_not_silently_dropped_before_hashing() {
        for member in ["locator", "literal", "issues", "effect", "target", "inventoryId", "kind", "note"] {
            let mut step = json!({"index":0,"op":"click","locator":"example"});
            step[member] = Value::Null;
            assert!(serde_json::from_value::<TaskBrowserStep>(step).is_err(), "{member}");
        }
        assert!(serde_json::from_value::<TaskWrite>(json!({"vaultId":"example-vault","itemId":"example-item","fields":[],"itemOp":null})).is_err());
        assert!(serde_json::from_value::<TaskWriteField>(json!({"inventoryId":"example-token","kind":"secret","op":null})).is_err());
    }
}

#[cfg(test)]
mod write_default_tests {
    use super::*;

    #[test]
    fn absent_write_operation_means_set_without_inserting_it() {
        let field: TaskWriteField = serde_json::from_str(
            r#"{"inventoryId":"example-token","kind":"secret"}"#,
        ).unwrap();
        assert_eq!(field.effective_op(), TaskWriteOp::Set);
        assert!(field.op.is_none());
        assert!(serde_json::to_value(field).unwrap().get("op").is_none());
    }
}
