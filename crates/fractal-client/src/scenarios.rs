//! Reader for the shared `fixtures/scenarios` client-v0 wire fixtures.
//!
//! Only the declared JSON pointers are rebased; narrative text and replay offsets stay opaque.

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{borrow::Cow, collections::BTreeMap, fs::File, path::Path, sync::LazyLock};

pub const ANCHOR: &str = "2030-01-01T12:00:00.000Z";
/// Client-v0 roots carried by the kit's wire-value traversal.
pub const WIRE_DEFINITIONS: &[&str] = &[
    "Agent", "Arrangement", "Runtime", "Machine", "Session", "Device", "Launch",
    "LaunchVariant", "Mission", "Work", "Attention",
    "Message", "Resource", "TimelineEntry", "TerminalScreen", "Capabilities",
    "ErrorEnvelope", "SyncPeer",
];
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
    Resource(Box<crate::Resource>),
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
    parse_slice(&std::fs::read_to_string(path.as_ref())
        .with_context(|| format!("read {}", path.as_ref().display()))?)
}

/// Read an embedded committed fixture with the same checks as [`load_slice`].
pub fn parse_slice(json: &str) -> Result<Slice> {
    let slice: Slice = serde_json::from_str(json).context("decode scenario slice")?;
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
    let delta = i128::from(now) - i128::from(anchor);
    for time in times {
        let pointer = &time.pointer;
        let target = value.pointer_mut(pointer).with_context(|| format!("missing instant at {pointer}"))?;
        let instant = match time.codec {
            TimeCodec::Timestamp => DateTime::parse_from_rfc3339(target.as_str().with_context(|| format!("expected timestamp at {pointer}"))?)
                .with_context(|| format!("invalid timestamp at {pointer}"))?.timestamp_millis(),
            TimeCodec::EpochMs => target.as_i64().with_context(|| format!("expected integer epoch-ms at {pointer}"))?,
        };
        let shifted = i64::try_from(i128::from(instant) + delta).ok()
            .filter(|ms| (MIN_MS..=MAX_MS).contains(ms))
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
            // JavaScript JSON numbers have no distinct integer spelling. Preserve ordinary
            // inputs by borrowing; normalize only mathematically integral floating spellings.
            let model_input = model_input(value);
            let value = model_input.as_ref();
            let decoded = match definition {
                "TimelineEntry" => WireValue::TimelineEntry(crate::TimelineEntry::deserialize(value)?),
                "TerminalScreen" => WireValue::TerminalScreen(crate::TerminalScreen::deserialize(value)?),
                "Capabilities" => WireValue::Capabilities(crate::Capabilities::deserialize(value)?),
                "ErrorEnvelope" => WireValue::ErrorEnvelope(crate::ErrorEnvelope::deserialize(value)?),
                "SyncPeer" => WireValue::SyncPeer(crate::SyncPeer::deserialize(value)?),
                _ => WireValue::Resource(Box::new(crate::Resource::deserialize(value)?)),
            };
            out.push(DecodedValue { pointer, value: decoded });
            Ok(())
        };
        let state = &self.state;
        let fields: &[(&str, &str)] = match self.slice.as_str() {
            "roster" => &[("agents", "Agent"), ("arrangements", "Arrangement"), ("resources", "Resource"), ("runtimes", "Runtime"), ("machines", "Machine"), ("sessions", "Session"), ("devices", "Device")],
            "details" => &[("missions", "Mission"), ("resources", "Resource"), ("work", "Work"), ("launches", "Launch"), ("launch_variants", "LaunchVariant")],
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
        schema["$defs"]["UnknownResource"] = Value::Bool(false);
        let definitions = schema["$defs"].clone();
        merge_object_intersections(&mut schema, &definitions)?;
        close_schema(&mut schema);
    } else {
        allow_excess_fields(&mut schema);
    }
    let mut lengths = BTreeMap::new();
    apply_wire_codecs(&mut schema, &mut lengths)?;
    schema.as_object_mut().context("schema is not an object")?.remove("oneOf");
    let mut out = BTreeMap::new();
    for &name in WIRE_DEFINITIONS {
        schema["$ref"] = Value::String(format!("#/$defs/{name}"));
        let mut options = jsonschema::options()
            .with_format("date-time", date_time_grammar)
            .with_format("st-timestamp", timestamp_codec)
            // The kit deliberately treats uri-reference as an unconstrained string.
            .with_format("uri-reference", |_| true)
            .should_validate_formats(true);
        for (format, &(minimum, maximum)) in &lengths {
            options = options.with_format(format.clone(), move |text| {
                let length = text.encode_utf16().count() as u64;
                (minimum..=maximum).contains(&length)
            });
        }
        out.insert(name.into(), options.build(&schema)?);
    }
    Ok(out)
}

/// Compose object fields before closing structs. Keep assertion/combinator siblings intact.
fn merge_object_intersections(schema: &mut Value, definitions: &Value) -> Result<()> {
    match schema {
        Value::Object(object) => {
            if let Some(Value::Array(mut parts)) = object.remove("allOf") {
                for part in &mut parts {
                    if let Some(reference) = part.get("$ref").and_then(Value::as_str) {
                        let name = reference.strip_prefix("#/$defs/").context("non-local object reference")?;
                        *part = definitions.get(name).with_context(|| format!("missing definition {name}"))?.clone();
                    }
                    merge_object_intersections(part, definitions)?;
                }
                let composed_object = object.get("type").and_then(Value::as_str) == Some("object")
                    || parts.iter().any(|part| part.get("type").and_then(Value::as_str) == Some("object"));
                if composed_object {
                    object.insert("type".into(), Value::String("object".into()));
                    for part in &mut parts {
                        let Some(fragment) = part.as_object_mut() else { continue };
                        if fragment.contains_key("if") { continue; }
                        if fragment.get("type").is_some_and(|kind| kind != "object") { continue; }
                        fragment.remove("type");
                        if let Some(Value::Object(fields)) = fragment.remove("properties") {
                            let properties = object.entry("properties").or_insert_with(|| serde_json::json!({}))
                                .as_object_mut().context("object properties")?;
                            for (name, field) in fields {
                                match properties.get_mut(&name) {
                                    Some(existing) if *existing != field => {
                                        *existing = serde_json::json!({"allOf": [existing.clone(), field]});
                                    }
                                    Some(_) => {}
                                    None => { properties.insert(name, field); }
                                }
                            }
                        }
                        if let Some(Value::Array(fields)) = fragment.remove("required") {
                            let required = object.entry("required").or_insert_with(|| serde_json::json!([]))
                                .as_array_mut().context("object required fields")?;
                            for field in fields {
                                if !required.contains(&field) { required.push(field); }
                            }
                        }
                        if let Some(excess) = fragment.remove("additionalProperties") {
                            ensure!(excess.is_boolean(), "composed record schemas need explicit normalization");
                            if excess == Value::Bool(false) {
                                object.insert("additionalProperties".into(), excess);
                            }
                        }
                    }
                }
                // Do not drop siblings (notably Device.oneOf), conditionals, or other assertions.
                object.insert("allOf".into(), Value::Array(parts));
            }
            for child in object.values_mut() { merge_object_intersections(child, definitions)?; }
        }
        Value::Array(children) => {
            for child in children { merge_object_intersections(child, definitions)?; }
        }
        _ => {}
    }
    Ok(())
}

fn apply_wire_codecs(schema: &mut Value, lengths: &mut BTreeMap<String, (u64, u64)>) -> Result<()> {
    match schema {
        Value::Object(object) => {
            if object.get("type").is_some_and(|kind| kind == "integer"
                || kind.as_array().is_some_and(|kinds| kinds.iter().any(|kind| kind == "integer")))
            {
                let minimum = object.get("minimum").and_then(Value::as_i64)
                    .map_or(-9_007_199_254_740_991, |old| old.max(-9_007_199_254_740_991));
                let maximum = object.get("maximum").and_then(Value::as_i64)
                    .map_or(9_007_199_254_740_991, |old| old.min(9_007_199_254_740_991));
                object.insert("minimum".into(), minimum.into());
                object.insert("maximum".into(), maximum.into());
            }
            match object.get("x-st-codec").and_then(Value::as_str) {
                Some("timestamp") => { object.insert("format".into(), Value::String("st-timestamp".into())); }
                Some("duration-ms" | "duration-s" | "epoch-ms") => {
                    let maximum = if object["x-st-codec"] == "epoch-ms" { MAX_MS } else { 9_007_199_254_740_991 };
                    let maximum = object.get("maximum").and_then(Value::as_i64).map_or(maximum, |old| old.min(maximum));
                    object.insert("maximum".into(), maximum.into());
                    let minimum = object.get("minimum").and_then(Value::as_i64).unwrap_or(0).max(0);
                    object.insert("minimum".into(), minimum.into());
                }
                _ => {}
            }
            if let Some(Value::String(pattern)) = object.get_mut("pattern") {
                // Expand ECMAScript whitespace instead of jsonschema's approximate table.
                *pattern = pattern.replace(r"\s", "\u{0009}-\u{000D}\u{0020}\u{00A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}");
            }
            if object.contains_key("minLength") || object.contains_key("maxLength") {
                let minimum = object.remove("minLength").and_then(|bound| bound.as_u64()).unwrap_or(0);
                let maximum = object.remove("maxLength").and_then(|bound| bound.as_u64()).unwrap_or(u64::MAX);
                let format = format!("st-utf16-length-{minimum}-{maximum}");
                lengths.insert(format.clone(), (minimum, maximum));
                // A separate conjunction preserves any existing format or pattern checks.
                object.entry("allOf").or_insert_with(|| serde_json::json!([]))
                    .as_array_mut().context("string length conjunction")?
                    .push(serde_json::json!({"format": format}));
            }
            for child in object.values_mut() { apply_wire_codecs(child, lengths)?; }
        }
        Value::Array(children) => { for child in children { apply_wire_codecs(child, lengths)?; } }
        _ => {}
    }
    Ok(())
}

/// The kit's DATE_TIME grammar (ASCII digits, four-digit year, mandatory seconds and zone).
fn date_time_grammar(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 20 { return false; }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes[index].is_ascii_digit() { return false; }
    }
    if bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':' || bytes[16] != b':' { return false; }
    let mut zone = 19;
    if bytes[zone] == b'.' {
        zone += 1;
        let start = zone;
        while bytes.get(zone).is_some_and(u8::is_ascii_digit) { zone += 1; }
        if start == zone { return false; }
    }
    match &bytes[zone..] {
        [b'Z' | b'z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => [h1, h2, m1, m2].iter().all(|digit| digit.is_ascii_digit()),
        _ => false,
    }
}

fn timestamp_codec(text: &str) -> bool {
    date_time_grammar(text) && &text[17..19] != "60"
        && DateTime::parse_from_rfc3339(text).is_ok_and(|instant| {
            // The codec permits year zero; scenario rebasing separately requires 0001..9999.
            (-62_167_219_200_000..=MAX_MS).contains(&instant.timestamp_millis())
        })
}

fn model_input(value: &Value) -> Cow<'_, Value> {
    fn integral_float(number: &serde_json::Number) -> Option<i64> {
        if !number.is_f64() { return None; }
        number.as_f64()
            .filter(|number| number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0)
            .map(|number| number as i64)
    }
    fn has_integral_float(value: &Value) -> bool {
        match value {
            Value::Number(number) => integral_float(number).is_some(),
            Value::Array(values) => values.iter().any(has_integral_float),
            Value::Object(values) => values.values().any(has_integral_float),
            _ => false,
        }
    }
    fn normalize(value: &mut Value) {
        match value {
            Value::Number(number) => {
                if let Some(integer) = integral_float(number) { *number = integer.into(); }
            }
            Value::Array(values) => { for value in values { normalize(value); } }
            Value::Object(values) => { for value in values.values_mut() { normalize(value); } }
            _ => {}
        }
    }
    if has_integral_float(value) {
        let mut owned = value.clone();
        normalize(&mut owned);
        Cow::Owned(owned)
    } else {
        Cow::Borrowed(value)
    }
}


fn close_schema(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            // A negative enum marks the future-tag arm of an open discriminated union.
            // Reject these structurally, including inline arms and referenced definitions.
            if object.get("type").and_then(Value::as_str) == Some("string")
                && object.get("not").and_then(|excluded| excluded.get("enum")).is_some()
            {
                *schema = Value::Bool(false);
                return;
            }
            // Open enums are encoded as anyOf(enum, string). Strict mode accepts only known cases.
            if let Some(arms) = object.get("anyOf").and_then(Value::as_array)
                && arms.len() == 2 && arms[0].get("enum").is_some()
                && arms[1] == serde_json::json!({"type": "string"})
            {
                let known = arms[0]["enum"].clone();
                object.remove("anyOf");
                object.insert("enum".into(), known);
            }
            // An explicit schema-valued additionalProperties is a record/map, even when
            // it also declares known keys (for example ErrorEnvelope.details).
            if object.get("type") == Some(&Value::String("object".into()))
                && object.contains_key("properties")
                && !matches!(object.get("additionalProperties"), Some(Value::Object(_)))
            {
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
