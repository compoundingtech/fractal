use serde::Deserialize;
use serde_json::{Value, json};

use super::{append, canonicalize_task_manifest_v1, digest_task_manifest_v1};
use crate::task_grants::parse_task_manifest_v1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    format: String,
    version: u64,
    valid: Vec<Valid>,
    invalid: Vec<Invalid>,
    equivalent: Vec<[String; 2]>,
    different: Vec<[String; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Valid {
    name: String,
    input: String,
    canonical: String,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invalid {
    name: String,
    input: String,
}

fn fixture() -> Fixture {
    // The fixture was generated independently with Python, sorting keys by
    // UTF-16BE bytes and hashing compact UTF-8 JSON, not by this encoder.
    serde_json::from_str(include_str!("../../fixtures/task-manifest-v1.golden.json")).unwrap()
}

#[test]
fn shared_golden_vectors_validate_canonicalize_and_hash() {
    let fixture = fixture();
    assert_eq!(fixture.format, "task-manifest-v1-golden");
    assert_eq!(fixture.version, 1);
    for case in fixture.valid {
        let input = case.input.as_bytes();
        parse_task_manifest_v1(input)
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        assert_eq!(
            canonicalize_task_manifest_v1(input).unwrap(),
            case.canonical,
            "{} canonical bytes",
            case.name
        );
        assert_eq!(
            digest_task_manifest_v1(input).unwrap(),
            case.digest,
            "{} digest",
            case.name
        );
        // Preserve the document except the permitted integer -0 to 0 spelling.
        assert_eq!(
            crate::task_grants::strict::parse_json(case.canonical.as_bytes()).unwrap(),
            crate::task_grants::strict::parse_json(case.input.as_bytes()).unwrap(),
            "{} document preservation",
            case.name
        );
        assert_eq!(
            canonicalize_task_manifest_v1(case.canonical.as_bytes()).unwrap(),
            case.canonical,
            "{} idempotence",
            case.name
        );
    }
}

#[test]
fn invalid_shared_vectors_fail_every_public_boundary() {
    for case in fixture().invalid {
        let input = case.input.as_bytes();
        assert!(parse_task_manifest_v1(input).is_err(), "{} parse", case.name);
        assert!(canonicalize_task_manifest_v1(input).is_err(), "{} canonical", case.name);
        assert!(digest_task_manifest_v1(input).is_err(), "{} digest", case.name);
    }
}

#[test]
fn object_order_is_ignored_but_arrays_defaults_and_spelling_are_not() {
    let fixture = fixture();
    let digest = |name: &str| {
        let case = fixture.valid.iter().find(|case| case.name == name).unwrap();
        digest_task_manifest_v1(case.input.as_bytes()).unwrap()
    };
    for [left, right] in &fixture.equivalent {
        assert_eq!(digest(left), digest(right), "{left} vs {right}");
    }
    for [left, right] in &fixture.different {
        assert_ne!(digest(left), digest(right), "{left} vs {right}");
    }
}

#[test]
fn historical_reveal_vector_remains_pinned() {
    let fixture = fixture();
    let case = fixture.valid.iter().find(|case| case.name == "revealOnceTextNode").unwrap();
    assert_eq!(case.digest, "80fb6ef6e2f9ae31f8fea172bde4cb3247f9db641758bcbf41790f68f1df82a1");
}

#[test]
fn pilot_is_zero_reader_rotation_into_exactly_one_secret_write() {
    let fixture = fixture();
    let case = fixture.valid.iter().find(|case| case.name == "tokenRotationPilot").unwrap();
    let document: Value = serde_json::from_str(&case.input).unwrap();
    assert_eq!(document["readers"], json!([]));
    let writes = document["writes"].as_array().unwrap();
    assert_eq!(writes.len(), 1);
    let fields = writes[0]["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["kind"], "secret");
    let reveals: Vec<_> = document["browserSteps"].as_array().unwrap().iter()
        .filter(|step| step["op"] == "revealOnce").collect();
    assert_eq!(reveals.len(), 1);
    assert_eq!(reveals[0]["issues"], fields[0]["inventoryId"]);
}

#[test]
fn private_encoder_orders_utf16_units_not_utf8_or_scalar_values() {
    // U+1F600 sorts before U+E000 by UTF-16, but after it by scalar/UTF-8.
    // Closed manifest keys are ASCII, so test the private encoder directly.
    let value = json!({"\u{e000}": 3, "😀": 2, "a": 1, "\r": 0});
    let mut output = Vec::new();
    append(&value, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "{\"\\r\":0,\"a\":1,\"😀\":2,\"\u{e000}\":3}");
}

#[test]
fn serde_json_strings_match_jcs_control_and_unicode_escaping() {
    // RFC 8785 §3.2.2.2: short escapes for these five controls, lowercase
    // hex for all other controls, no slash/HTML/non-ASCII escaping.
    let edge = "\0\u{1}\u{8}\t\n\u{c}\r\u{1f}\"\\/<>é😀\u{2028}\u{2029}";
    let expected = "\"\\u0000\\u0001\\b\\t\\n\\f\\r\\u001f\\\"\\\\/<>é😀\u{2028}\u{2029}\"";
    let mut output = Vec::new();
    append(&json!(edge), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let mut object = serde_json::Map::new();
    object.insert(edge.into(), json!(edge));
    let mut output = Vec::new();
    append(&Value::Object(object), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), format!("{{{expected}:{expected}}}"));
}

#[test]
fn private_encoder_preserves_arrays_and_plain_safe_integer_decimal() {
    let mut output = Vec::new();
    append(&json!([9_007_199_254_740_991_u64, 0, -9_007_199_254_740_991_i64, [3, 1, 2]]), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "[9007199254740991,0,-9007199254740991,[3,1,2]]");
}

#[test]
fn invalid_utf8_is_refused_before_hashing() {
    let input = b"{\"title\":\"\xff\"}";
    assert!(parse_task_manifest_v1(input).is_err());
    assert!(canonicalize_task_manifest_v1(input).is_err());
    assert!(digest_task_manifest_v1(input).is_err());
}

#[test]
fn shared_boundary_vectors_pin_exact_units_and_odd_node_counts() {
    fn bounds(locator: &Value) -> (usize, usize) {
        let children = match locator["kind"].as_str().unwrap() {
            "nth" => vec![bounds(&locator["within"]), (1, 1)],
            "within" => vec![bounds(&locator["scope"]), bounds(&locator["target"])],
            "withinFrame" => vec![bounds(&locator["frame"]), bounds(&locator["target"])],
            _ => vec![],
        };
        (1 + children.iter().map(|(nodes, _)| nodes).sum::<usize>(),
            1 + children.iter().map(|(_, depth)| *depth).max().unwrap_or(0))
    }
    let fixture = fixture();
    let document = |name: &str| {
        let input = fixture.valid.iter().find(|case| case.name == name).map(|case| &case.input)
            .or_else(|| fixture.invalid.iter().find(|case| case.name == name).map(|case| &case.input))
            .unwrap();
        serde_json::from_str::<Value>(input).unwrap()
    };
    for (name, expected) in [
        ("locatorDepthAt8WithNth", (15, 8)),
        ("locatorDepthOver9WithNth", (17, 9)),
        ("locatorOddNodeCountAt31WithNth", (31, 5)),
        ("locatorOddNodeCountOver33WithNth", (33, 6)),
    ] {
        assert_eq!(bounds(&document(name)["browserSteps"][0]["locator"]), expected, "{name}");
    }
    // Binary branches (including nth's synthetic leaf) make 32 unreachable.
    for (name, units) in [("cssAt2048Utf16Units", 2048), ("cssOver2049Utf16Units", 2049)] {
        assert_eq!(document(name)["browserSteps"][0]["locator"]["selector"]
            .as_str().unwrap().encode_utf16().count(), units, "{name}");
    }
    for (name, length, multibyte) in [
        ("safeUrlAt4096Utf16Units", 4096, false),
        ("safeUrlOver4097Utf16Units", 4097, false),
        ("safeUrlMultibyteAt4096Utf8Bytes", 4096, true),
        ("safeUrlMultibyteOver4097Utf8BytesBelowUtf16Limit", 4097, true),
    ] {
        let document = document(name);
        let value = document["browserSteps"][0]["safeUrl"].as_str().unwrap();
        assert_eq!(value.len(), length, "{name}");
        if multibyte {
            assert!(value.encode_utf16().count() < 4096, "{name}");
        } else {
            assert_eq!(value.encode_utf16().count(), length, "{name}");
        }
    }
}
