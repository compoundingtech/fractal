//! Reader for the shared `fixtures/scenarios` client-v0 wire fixtures.
//!
//! Only the declared JSON pointers are rebased; narrative text and replay offsets stay opaque.

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fs::File, path::Path, sync::LazyLock};

pub const ANCHOR: &str = "2030-01-01T12:00:00.000Z";
const MIN_MS: i64 = -62_135_596_800_000;
const MAX_MS: i64 = 253_402_300_799_999;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Index {
    pub format: String,
    pub anchor: String,
    pub worlds: Vec<World>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct World {
    pub id: String,
    pub title: String,
    pub narrative: String,
    pub seed: u64,
    pub slices: BTreeMap<String, String>,
    pub cast: Cast,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Cast {
    pub agents: Vec<CastMember>,
    pub people: Vec<CastMember>,
    pub hosts: Vec<CastMember>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CastMember {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DecodeMode {
    Strict,
    Tolerant,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimeCodec {
    Timestamp,
    EpochMs,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TimePointer {
    pub pointer: String,
    pub codec: TimeCodec,
}

/// Raw slice metadata and replay data. Unknown declarations are diagnostic metadata only.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Slice {
    pub format: String,
    pub world: String,
    pub slice: String,
    pub variant: String,
    pub anchor: String,
    pub source: Value,
    pub decode: DecodeMode,
    pub loading: bool,
    pub times: Vec<TimePointer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown: Vec<Value>,
    pub state: Value,
    pub timeline: Vec<Value>,
}

/// One wire value and its location in the slice, decoded into the existing client models.
#[derive(Debug)]
pub struct DecodedValue {
    pub pointer: String,
    pub value: WireValue,
}

#[derive(Debug)]
pub enum WireValue {
    Resource(crate::Resource),
    TimelineEntry(crate::TimelineEntry),
    TerminalScreen(crate::TerminalScreen),
    Capabilities(crate::Capabilities),
    ErrorEnvelope(crate::ErrorEnvelope),
    SyncPeer(crate::SyncPeer),
}

pub fn load_index(path: impl AsRef<Path>) -> Result<Index> {
    let index: Index = read_json(path.as_ref())?;
    ensure!(index.format == "st3.scenario.index.v1", "unsupported scenario index format: {}", index.format);
    Ok(index)
}

pub fn load_slice(path: impl AsRef<Path>) -> Result<Slice> {
    let slice: Slice = read_json(path.as_ref())?;
    ensure!(slice.format == "st3.scenario.slice.v1", "unsupported scenario slice format: {}", slice.format);
    ensure!(["roster", "conversation", "sync", "terminal", "details", "attention"].contains(&slice.slice.as_str()), "unknown slice: {}", slice.slice);
    Ok(slice)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_reader(File::open(path).with_context(|| format!("open {}", path.display()))?)
        .with_context(|| format!("decode {}", path.display()))
}

/// Shift exactly `times`, truncating RFC3339 inputs to milliseconds before arithmetic.
/// Errors identify the offending pointer. `now` is an integer epoch-millisecond clock.
pub fn rebase(value: &mut Value, times: &[TimePointer], anchor: &str, now: i64) -> Result<()> {
    let anchor = DateTime::parse_from_rfc3339(anchor).context("invalid scenario anchor")?.timestamp_millis();
    let delta = now.checked_sub(anchor).context("scenario clock delta overflows")?;
    for time in times {
        let pointer = &time.pointer;
        let target = value.pointer_mut(pointer).with_context(|| format!("missing instant at {pointer}"))?;
        let instant = match time.codec {
            TimeCodec::Timestamp => DateTime::parse_from_rfc3339(target.as_str().with_context(|| format!("expected timestamp at {pointer}"))?)
                .with_context(|| format!("invalid timestamp at {pointer}"))?.timestamp_millis(),
            TimeCodec::EpochMs => target.as_i64().with_context(|| format!("expected integer epoch-ms at {pointer}"))?,
        };
        let shifted = instant.checked_add(delta).filter(|ms| (MIN_MS..=MAX_MS).contains(ms))
            .with_context(|| format!("instant out of range at {pointer}"))?;
        *target = match time.codec {
            TimeCodec::Timestamp => Value::String(DateTime::<Utc>::from_timestamp_millis(shifted)
                .context("validated instant is not representable")?.to_rfc3339_opts(SecondsFormat::Millis, true)),
            TimeCodec::EpochMs => Value::from(shifted),
        };
    }
    Ok(())
}

impl Slice {
    /// Rebase an owned slice, leaving the loaded fixture available for other clocks.
    pub fn rebased(mut self, now: i64) -> Result<Self> {
        let mut wire = serde_json::json!({"state": self.state, "timeline": self.timeline});
        rebase(&mut wire, &self.times, &self.anchor, now)?;
        self.state = wire["state"].take();
        self.timeline = serde_json::from_value(wire["timeline"].take())?;
        Ok(self)
    }

    /// Decode every wire value in state and timeline, using this slice's declared mode.
    /// Replay-only fields (offsets, casts, expectations) are not client-v0 values.
    pub fn decode(&self) -> Result<Vec<DecodedValue>> {
        let mut out = Vec::new();
        let mut add = |pointer: String, definition: &str, value: &Value| -> Result<()> {
            validator(definition, self.decode)?.validate(value)
                .map_err(|error| anyhow::anyhow!("{:?} {definition} at {pointer}: {error}", self.decode))?;
            let decoded = match definition {
                "TimelineEntry" => WireValue::TimelineEntry(crate::TimelineEntry::deserialize(value)?),
                "TerminalScreen" => WireValue::TerminalScreen(crate::TerminalScreen::deserialize(value)?),
                "Capabilities" => WireValue::Capabilities(crate::Capabilities::deserialize(value)?),
                "ErrorEnvelope" => WireValue::ErrorEnvelope(crate::ErrorEnvelope::deserialize(value)?),
                "SyncPeer" => WireValue::SyncPeer(crate::SyncPeer::deserialize(value)?),
                _ => WireValue::Resource(crate::Resource::deserialize(value)?),
            };
            out.push(DecodedValue { pointer, value: decoded });
            Ok(())
        };
        let state = &self.state;
        let fields: &[(&str, &str)] = match self.slice.as_str() {
            "roster" => &[("agents", "Agent"), ("arrangements", "Arrangement"), ("resources", "Resource"), ("runtimes", "Runtime"), ("machines", "Machine")],
            "details" => &[("missions", "Mission"), ("resources", "Resource"), ("work", "Work")],
            "attention" => &[("attention", "Attention"), ("resources", "Resource"), ("messages", "Message")],
            "conversation" => {
                for (index, thread) in array(state, "threads")?.iter().enumerate() {
                    add_list(&mut add, &format!("/state/threads/{index}/items"), array(thread, "items")?, "TimelineEntry")?;
                }
                &[]
            }
            "terminal" => {
                for (index, record) in array(state, "terminals")?.iter().enumerate() {
                    add_terminal(&mut add, &format!("/state/terminals/{index}"), record)?;
                }
                &[]
            }
            "sync" => {
                if let Some(value) = state.get("capabilities") { add("/state/capabilities".into(), "Capabilities", value)?; }
                &[]
            }
            other => bail!("unknown slice: {other}"),
        };
        for &(field, definition) in fields {
            if let Some(values) = state.get(field) {
                add_list(&mut add, &format!("/state/{field}"), values.as_array().context("expected state array")?, definition)?;
            } else if !["resources", "arrangements"].contains(&field) {
                bail!("missing state field: {field}");
            }
        }
        if let Some(history) = state.get("recorded_history") {
            for (index, event) in history.as_array().context("expected recorded_history array")?.iter().enumerate() {
                let base = format!("/state/recorded_history/{index}");
                match self.slice.as_str() {
                    "roster" => add_list(&mut add, &format!("{base}/upserts"), array(event, "upserts")?, "Resource")?,
                    "conversation" => add_list(&mut add, &format!("{base}/items"), array(event, "items")?, "TimelineEntry")?,
                    "terminal" => add(format!("{base}/screen"), "TerminalScreen", &event["screen"])?,
                    _ => {}
                }
            }
        }
        for (index, event) in self.timeline.iter().enumerate() {
            let base = format!("/timeline/{index}");
            match event["_tag"].as_str().context("missing timeline event tag")? {
                "thread-create" => add_list(&mut add, &format!("{base}/thread/items"), array(&event["thread"], "items")?, "TimelineEntry")?,
                "terminal-create" => add_terminal(&mut add, &format!("{base}/record"), &event["record"])?,
                "changes" => add_list(&mut add, &format!("{base}/upserts"), array(event, "upserts")?, "Resource")?,
                "entries" | "replace" => add_list(&mut add, &format!("{base}/items"), array(event, "items")?, "TimelineEntry")?,
                "screen" => add(format!("{base}/screen"), "TerminalScreen", &event["screen"])?,
                "http-error" => add(format!("{base}/envelope"), "ErrorEnvelope", &event["envelope"])?,
                "notice" => add_list(&mut add, &format!("{base}/peers"), array(event, "peers")?, "SyncPeer")?,
                _ => {}
            }
        }
        Ok(out)
    }
}

fn array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    value.get(field).and_then(Value::as_array).with_context(|| format!("expected array: {field}"))
}

type AddWire<'a> = dyn FnMut(String, &str, &Value) -> Result<()> + 'a;
fn add_list(add: &mut AddWire<'_>, base: &str, values: &[Value], definition: &str) -> Result<()> {
    for (index, value) in values.iter().enumerate() { add(format!("{base}/{index}"), definition, value)?; }
    Ok(())
}
fn add_terminal(add: &mut AddWire<'_>, base: &str, record: &Value) -> Result<()> {
    add(format!("{base}/runtime"), "Runtime", &record["runtime"])?;
    for (index, screen) in array(record, "screens")?.iter().enumerate() {
        add(format!("{base}/screens/{index}/screen"), "TerminalScreen", &screen["screen"])?;
    }
    Ok(())
}

// Serde's production models deliberately accept future fields/enums. Strict fixtures additionally
// use the authoritative schema with open enums closed and composed resource objects closed.
fn validator(definition: &str, mode: DecodeMode) -> Result<&'static jsonschema::Validator> {
    static STRICT: LazyLock<Result<BTreeMap<String, jsonschema::Validator>, String>> =
        LazyLock::new(|| build_validators(DecodeMode::Strict).map_err(|error| error.to_string()));
    static TOLERANT: LazyLock<Result<BTreeMap<String, jsonschema::Validator>, String>> =
        LazyLock::new(|| build_validators(DecodeMode::Tolerant).map_err(|error| error.to_string()));
    let validators = match mode { DecodeMode::Strict => &*STRICT, DecodeMode::Tolerant => &*TOLERANT }
        .as_ref().map_err(|error| anyhow::anyhow!(error.clone()))?;
    validators.get(definition).with_context(|| format!("unknown wire definition: {definition}"))
}

fn build_validators(mode: DecodeMode) -> Result<BTreeMap<String, jsonschema::Validator>> {
    let mut schema: Value = serde_json::from_str(include_str!("../../../docs/st3/client-v0/schemas/client-v0.schema.json"))?;
    if mode == DecodeMode::Strict {
    let header = schema["$defs"]["ResourceHeader"].clone();
    schema["$defs"]["UnknownResource"] = Value::Bool(false);
    for definition in schema["$defs"].as_object_mut().context("missing schema definitions")?.values_mut() {
        // Merge the resource header before closing its object, rather than closing each allOf arm.
        if let Some(parts) = definition.get_mut("allOf").and_then(Value::as_array_mut) {
            if parts.first().and_then(|part| part["$ref"].as_str()) == Some("#/$defs/ResourceHeader") && parts.len() == 2 {
                let mut object = parts[1].clone();
                let properties = object["properties"].as_object_mut().context("missing resource properties")?;
                for (key, value) in header["properties"].as_object().context("missing header properties")? {
                    properties.entry(key.clone()).or_insert_with(|| value.clone());
                }
                let required = object["required"].as_array_mut().context("missing resource required fields")?;
                for field in header["required"].as_array().context("missing header required fields")? {
                    if !required.contains(field) { required.push(field.clone()); }
                }
                *definition = object;
            }
        }
    }
    close_schema(&mut schema);
    } else {
        allow_excess_fields(&mut schema);
    }
    schema.as_object_mut().context("schema is not an object")?.remove("oneOf");
    let mut out = BTreeMap::new();
    for name in ["Agent", "Arrangement", "Runtime", "Machine", "Mission", "Work", "Attention", "Message", "Resource", "TimelineEntry", "TerminalScreen", "Capabilities", "ErrorEnvelope", "SyncPeer"] {
        schema["$ref"] = Value::String(format!("#/$defs/{name}"));
        out.insert(name.into(), jsonschema::options().should_validate_formats(true).build(&schema)?);
    }
    Ok(out)
}

fn close_schema(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            // Open enums are encoded as anyOf(enum, string). Strict mode accepts only known cases.
            if let Some(arms) = object.get("anyOf").and_then(Value::as_array) {
                if arms.len() == 2 && arms[0].get("enum").is_some() && arms[1] == serde_json::json!({"type": "string"}) {
                    let known = arms[0]["enum"].clone();
                    object.remove("anyOf");
                    object.insert("enum".into(), known);
                }
            }
            if object.get("type") == Some(&Value::String("object".into())) && object.contains_key("properties") {
                object.insert("additionalProperties".into(), Value::Bool(false));
            }
            for value in object.values_mut() { close_schema(value); }
        }
        Value::Array(values) => { for value in values { close_schema(value); } }
        _ => {}
    }
}

fn allow_excess_fields(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            if object.get("additionalProperties") == Some(&Value::Bool(false)) {
                object.remove("additionalProperties");
            }
            for value in object.values_mut() { allow_excess_fields(value); }
        }
        Value::Array(values) => { for value in values { allow_excess_fields(value); } }
        _ => {}
    }
}
