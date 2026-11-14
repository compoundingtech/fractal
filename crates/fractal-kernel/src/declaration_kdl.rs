//! The values of declaration KDL every declaration kind shares: a node as the canonical JSON
//! each declaration is stored, hashed and read as (`{name, arguments?, properties?, children?}`,
//! properties sorted by name), and durations (`300ms`, `5m`, `7d`).

use std::collections::BTreeMap;

use kdl::{KdlNode, KdlValue};
use serde_json::{Map, Value};

use crate::model::FractalError;

pub fn json_value(value: &KdlValue) -> Result<Value, FractalError> {
    Ok(match value {
        KdlValue::String(value) => Value::String(value.clone()),
        KdlValue::Integer(value) => {
            let integer = i64::try_from(*value).map_err(|_| {
                FractalError::new(
                    "integer-range",
                    "a KDL integer must fit a signed 64-bit value",
                )
            })?;
            Value::Number(integer.into())
        }
        KdlValue::Float(value) => Value::Number(
            serde_json::Number::from_f64(*value)
                .ok_or_else(|| FractalError::new("invalid-number", "a float must be finite"))?,
        ),
        KdlValue::Bool(value) => Value::Bool(*value),
        KdlValue::Null => Value::Null,
    })
}

pub fn canonical_node(node: &KdlNode) -> Result<Value, FractalError> {
    let mut properties = BTreeMap::<String, Value>::new();
    let mut arguments = Vec::new();
    for entry in node.entries() {
        let value = json_value(entry.value())?;
        if let Some(name) = entry.name() {
            if properties.insert(name.value().into(), value).is_some() {
                return Err(FractalError::new(
                    "duplicate-property",
                    format!(
                        "node `{}` repeats property `{}`",
                        node.name().value(),
                        name.value()
                    ),
                ));
            }
        } else {
            arguments.push(value);
        }
    }
    let children = node
        .children()
        .map(|document| {
            document
                .nodes()
                .iter()
                .map(canonical_node)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let mut output = Map::new();
    output.insert("name".into(), Value::String(node.name().value().into()));
    if !arguments.is_empty() {
        output.insert("arguments".into(), Value::Array(arguments));
    }
    if !properties.is_empty() {
        output.insert(
            "properties".into(),
            serde_json::to_value(properties).expect("BTreeMap serializes"),
        );
    }
    if !children.is_empty() {
        output.insert("children".into(), Value::Array(children));
    }
    Ok(Value::Object(output))
}

/// A duration in milliseconds: an integer with `ms`, `s`, `m`, `h` or `d`, seconds when bare.
pub fn parse_duration(value: &str, positive: bool) -> Result<u64, FractalError> {
    let (number, multiplier) = if let Some(value) = value.strip_suffix("ms") {
        (value, 1)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1_000)
    } else if let Some(value) = value.strip_suffix('m') {
        (value, 60_000)
    } else if let Some(value) = value.strip_suffix('h') {
        (value, 3_600_000)
    } else if let Some(value) = value.strip_suffix('d') {
        (value, 86_400_000)
    } else {
        (value, 1_000)
    };
    let number = number
        .parse::<u64>()
        .map_err(|_| FractalError::new("invalid-duration", format!("invalid duration `{value}`")))?;
    let duration = number
        .checked_mul(multiplier)
        .ok_or_else(|| FractalError::new("duration-range", "duration is too large"))?;
    if positive && duration == 0 {
        return Err(FractalError::new(
            "invalid-duration",
            "a required duration must be positive",
        ));
    }
    Ok(duration)
}
