//! RFC 8785 encoding restricted to validated TaskManifest v1 documents.
//!
//! The strict boundary admits only safe integer number tokens and I-JSON
//! strings. No floating-point/ECMAScript number formatter is needed: every
//! admitted number has an exact decimal representation below 1e21. This is
//! deliberately not a general-purpose public JCS API.

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{TaskManifestError, TaskManifestV1, invalid, strict};

/// Validate and canonicalize the exact document, without inserting defaults.
///
/// Object names sort by UTF-16 code units; arrays and string contents retain
/// their declared order and spelling (apart from JSON escape normalization).
pub fn canonicalize_task_manifest_v1(input: &[u8]) -> Result<String, TaskManifestError> {
    let value = strict::parse_json(input).map_err(|error| invalid(error.to_string()))?;
    let manifest = TaskManifestV1::deserialize(&value)
        .map_err(|error| invalid(error.to_string()))?;
    manifest.validate()?;
    let mut output = Vec::with_capacity(input.len());
    append(&value, &mut output).map_err(|error| invalid(error.to_string()))?;
    String::from_utf8(output).map_err(|error| invalid(error.to_string()))
}

/// Lowercase SHA-256 hex of the validated document's canonical UTF-8 bytes.
pub fn digest_task_manifest_v1(input: &[u8]) -> Result<String, TaskManifestError> {
    let canonical = canonicalize_task_manifest_v1(input)?;
    Ok(hex::encode(Sha256::digest(canonical.as_bytes())))
}

// Call only on the strict, validated integer-only domain. Tests exercise the
// encoder directly for supplementary object names, which the closed manifest
// schema cannot otherwise contain.
fn append(value: &Value, output: &mut Vec<u8>) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = object.iter().collect();
            entries.sort_unstable_by(|(left, _), (right, _)| {
                left.encode_utf16().cmp(right.encode_utf16())
            });
            output.push(b'{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                // serde_json uses RFC 8785's short control escapes, lowercase
                // \\u00xx for other controls, and literal Unicode otherwise.
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                append(value, output)?;
            }
            output.push(b'}');
        }
        Value::Array(array) => {
            output.push(b'[');
            for (index, value) in array.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                append(value, output)?;
            }
            output.push(b']');
        }
        // Safe integers emit plain decimal; strings use the same serializer
        // as object names. Null/floats cannot reach this private encoder via
        // either public function.
        _ => serde_json::to_writer(output, value)?,
    }
    Ok(())
}

#[cfg(test)]
#[path = "canonical_tests.rs"]
mod tests;
