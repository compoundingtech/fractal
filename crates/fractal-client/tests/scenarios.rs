#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

use fractal_client::scenarios::{self, DecodeMode, TimePointer, WireValue};
use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::PathBuf};

fn fixtures() -> PathBuf {
    PathBuf::from(test_env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scenarios")
}

#[derive(Deserialize)]
struct Vectors {
    vectors: Vec<Vector>,
}
#[derive(Deserialize)]
struct Vector {
    name: String,
    input: Value,
    times: Vec<TimePointer>,
    anchor: String,
    now: i64,
    expected: Value,
}

#[test]
fn scenarios_shared_rebase_vectors() {
    let vectors: Vectors = serde_json::from_slice(&fs::read(fixtures().join("_vectors/rebase.json")).unwrap()).unwrap();
    assert!(!vectors.vectors.is_empty());
    for vector in vectors.vectors {
        let mut value = vector.input;
        let result = scenarios::rebase(&mut value, &vector.times, &vector.anchor, vector.now);
        match vector.expected["_tag"].as_str().unwrap() {
            "ok" => {
                result.unwrap_or_else(|error| panic!("{}: {error:#}", vector.name));
                assert_eq!(value, vector.expected["value"], "{}", vector.name);
            }
            "range-error" => {
                let error = result.expect_err(&vector.name).to_string();
                assert!(error.contains("out of range"), "{}: {error}", vector.name);
                assert!(error.contains(vector.expected["pointer"].as_str().unwrap()), "{}: {error}", vector.name);
            }
            other => panic!("unknown vector expectation {other}"),
        }
    }
}

#[test]
fn scenarios_decode_every_committed_slice() {
    let root = fixtures();
    let index = scenarios::load_index(root.join("index.json")).unwrap();
    assert_eq!(index.anchor, scenarios::ANCHOR);
    let anchor = 1_893_499_200_000;
    let mut worlds = 0;
    let mut slices = 0;
    let mut tolerant = 0;
    // Enumerate the filesystem, not a pinned world list or only the index.
    for world in fs::read_dir(&root).unwrap() {
        let world = world.unwrap();
        if !world.file_type().unwrap().is_dir() || world.file_name().to_string_lossy().starts_with('_') { continue; }
        worlds += 1;
        let entry = index.worlds.iter().find(|entry| entry.id == world.file_name().to_string_lossy()).expect("world missing from index");
        let mut files = 0;
        for file in fs::read_dir(world.path()).unwrap() {
            let path = file.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") { continue; }
            files += 1;
            slices += 1;
            let slice = scenarios::load_slice(&path).unwrap();
            assert_eq!(slice.world, entry.id);
            let indexed = entry.slices.get(&slice.slice).expect("slice missing from index");
            assert_eq!(root.join(indexed), path);
            if slice.decode == DecodeMode::Tolerant { tolerant += 1; }
            let original = serde_json::to_value(&slice).unwrap();
            let identity = slice.clone().rebased(anchor).unwrap();
            assert_eq!(serde_json::to_value(&identity).unwrap(), original, "{}", path.display());
            for now in [anchor, anchor + 86_400_123] {
                slice.clone().rebased(now).unwrap().decode().unwrap_or_else(|error| panic!("{} at {now}: {error:#}", path.display()));
            }
        }
        assert_eq!(files, entry.slices.len(), "{}", world.path().display());
    }
    assert_eq!(worlds, index.worlds.len());
    assert!(slices > 0);
    assert!(tolerant > 0, "exercise declared tolerant slices too");
}

#[test]
fn scenarios_declared_mode_is_enforced_without_repairing_wire_values() {
    let mut slice = scenarios::load_slice(fixtures().join("fleet-mid-refactor/roster.json")).unwrap();
    slice.state["agents"][0]["state"] = Value::String("reconciling".into());
    assert!(slice.decode().is_err(), "strict must reject unknown enum strings");
    slice.decode = DecodeMode::Tolerant;
    let values = slice.decode().unwrap();
    let agent = values.iter().find(|value| value.pointer == "/state/agents/0").unwrap();
    let WireValue::Resource(resource) = &agent.value else { panic!("expected resource") };
    let fractal_client::Resource::Agent(agent) = resource.as_ref() else { panic!("expected typed agent") };
    assert_eq!(agent.state, "reconciling");

    let mut slice = scenarios::load_slice(fixtures().join("fleet-mid-refactor/roster.json")).unwrap();
    slice.state["agents"][0]["protocol_features"] = serde_json::json!(["future-protocol"]);
    assert!(slice.decode().is_err(), "strict must reject extra keys");
    slice.decode = DecodeMode::Tolerant;
    assert!(slice.decode().is_ok());

    // The fixture's repair witnesses must never replace its real transport values.
    let slice = scenarios::load_slice(fixtures().join("unknown-fields/roster.json")).unwrap();
    assert_eq!(slice.decode, DecodeMode::Tolerant);
    let decoded = slice.decode().unwrap();
    assert!(decoded.iter().any(|value| matches!(
        &value.value,
        WireValue::Resource(resource) if matches!(resource.as_ref(), fractal_client::Resource::Unknown(_))
    )));
}

#[test]
fn scenarios_invalid_instants_name_the_pointer() {
    for (input, codec) in [(serde_json::json!({"at": "not-a-time"}), "timestamp"), (serde_json::json!({"at": 1.5}), "epoch-ms"), (serde_json::json!({}), "timestamp")] {
        let times: Vec<TimePointer> = serde_json::from_value(serde_json::json!([{ "pointer": "/at", "codec": codec }])).unwrap();
        let error = scenarios::rebase(&mut input.clone(), &times, scenarios::ANCHOR, 1_893_499_200_000).unwrap_err();
        assert!(error.to_string().contains("/at"), "{error:#}");
    }
    for now in [i64::MIN, i64::MAX] {
        let times: Vec<TimePointer> = serde_json::from_value(serde_json::json!([{ "pointer": "/at", "codec": "timestamp" }])).unwrap();
        let mut input = serde_json::json!({ "at": scenarios::ANCHOR });
        let error = scenarios::rebase(&mut input, &times, scenarios::ANCHOR, now).unwrap_err();
        assert!(error.to_string().contains("/at"), "{error:#}");
    }
}

#[test]
fn scenarios_rebase_accepts_lowercase_rfc3339_separators() {
    let times: Vec<TimePointer> = serde_json::from_value(serde_json::json!([
        { "pointer": "/at", "codec": "timestamp" }
    ])).unwrap();
    let mut input = serde_json::json!({ "at": "2030-01-01t11:59:00.250999z" });
    scenarios::rebase(&mut input, &times, "2030-01-01t12:00:00.000z", 1_893_499_201_500).unwrap();
    assert_eq!(input["at"], "2030-01-01T11:59:01.750Z");
}

#[test]
fn scenarios_wire_definition_registry_matches_the_kit() {
    let source = fs::read_to_string(
        PathBuf::from(test_env!("CARGO_MANIFEST_DIR"))
            .join("../../clients/typescript/fractal-scenarios/src/kit/wire.ts"),
    ).unwrap();
    let declaration = source.split_once("export type WireDefinition =").unwrap().1
        .split_once("export interface WireValue").unwrap().0;
    let expected: std::collections::BTreeSet<_> = declaration.lines()
        .filter_map(|line| line.trim().strip_prefix("| '").and_then(|name| name.strip_suffix('\'')))
        .collect();
    let actual: std::collections::BTreeSet<_> = scenarios::WIRE_DEFINITIONS.iter().copied().collect();
    assert!(!expected.is_empty(), "kit wire declaration format changed");
    assert_eq!(actual, expected, "update Rust wire discovery when the kit adds a wire root");
}

#[test]
fn scenarios_strict_rejects_unknown_error_codes() {
    let mut slice = scenarios::load_slice(fixtures().join("failed-sync-rate-limited/sync.json")).unwrap();
    let envelope = slice.timeline.iter_mut()
        .find(|event| event["_tag"] == "http-error").unwrap();
    envelope["envelope"]["code"] = Value::String("future-scenario-error".into());
    assert!(slice.decode().is_err());
    slice.decode = DecodeMode::Tolerant;
    let decoded = slice.decode().unwrap();
    assert!(decoded.iter().any(|value| matches!(
        &value.value,
        WireValue::ErrorEnvelope(envelope) if envelope.code == fractal_client::ErrorCode::Unknown
    )));
}

fn client_fixture(name: &str) -> Value {
    serde_json::from_slice(&fs::read(
        PathBuf::from(test_env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/st3/client-v0/fixtures").join(name),
    ).unwrap()).unwrap()
}

#[test]
fn scenarios_strict_rejects_inline_future_union_views() {
    // Read the committed client-v0 timeline fixture, without pinning its payload bytes.
    let timeline = client_fixture("timeline-views.json");
    let entry = timeline["value"]["items"][1].clone();
    let mut slice = scenarios::load_slice(fixtures().join("empty/conversation.json")).unwrap();
    slice.state = serde_json::json!({ "threads": [{ "items": [entry] }] });
    slice.decode().expect("a known view must pass strict decoding");
    slice.state["threads"][0]["items"][0]["body"]["blocks"][0]["view"] =
        serde_json::json!({ "type": "future-view" });
    assert!(slice.decode().is_err(), "strict must reject inline unknown union arms");
    slice.decode = DecodeMode::Tolerant;
    slice.decode().expect("tolerant mode must retain future views");
}

#[test]
fn scenarios_strict_preserves_explicit_error_detail_maps() {
    let envelope = client_fixture("cursor-gap-error.json");
    let mut slice = scenarios::load_slice(fixtures().join("empty/sync.json")).unwrap();
    slice.timeline = vec![serde_json::json!({
        "_tag": "http-error", "envelope": envelope, "at_ms": 0
    })];
    let decoded = slice.decode().expect("strict must accept the committed cursor-gap details");
    assert!(decoded.iter().any(|value| matches!(
        &value.value,
        WireValue::ErrorEnvelope(error)
            if error.details.len() == envelope["details"].as_object().unwrap().len()
                && error.details.iter().all(|(key, value)| envelope["details"].get(key) == Some(value))
    )));
    // Opaque JSON bags must not be mistaken for unknown union/enum cases.
    slice.timeline[0]["envelope"]["details"]["opaque"] =
        serde_json::json!({ "_tag": "Unknown", "raw": "user-data" });
    slice.decode().expect("strict must leave arbitrary map values opaque");
    // Declared fields of an open map still retain their own constraints.
    slice.timeline[0]["envelope"]["details"]["applied"] = Value::String("future-applied".into());
    assert!(slice.decode().is_err(), "an explicit map does not disable known-field validation");
}

#[test]
fn scenarios_strict_accepts_shared_producer_conformance_entries() {
    // Same input as schema.test.cjs's producer conformance strict-codec test.
    let conformance = client_fixture("timeline-conformance.json");
    let items = conformance["items"].as_array().unwrap();
    assert!(!items.is_empty());
    let mut slice = scenarios::load_slice(fixtures().join("empty/conversation.json")).unwrap();
    slice.state = serde_json::json!({ "threads": [{ "items": items }] });
    slice.decode().expect("the shared kit producer vectors must pass strict decoding");
}

#[test]
fn scenarios_decode_all_committed_inventory_roots() {
    for (kind, field) in [
        ("roster", "sessions"), ("roster", "devices"),
        ("details", "launches"), ("details", "launch_variants"),
    ] {
        let mut slice = scenarios::load_slice(
            fixtures().join("fleet-mid-refactor").join(format!("{kind}.json")),
        ).unwrap();
        let inventory = slice.state[field].as_array().unwrap();
        assert!(!inventory.is_empty(), "exercise populated {field}");
        let decoded = slice.decode().unwrap();
        let prefix = format!("/state/{field}/");
        let values: Vec<_> = decoded.iter().filter(|value| value.pointer.starts_with(&prefix)).collect();
        assert_eq!(values.len(), inventory.len(), "decode every {field} record");
        for value in values {
            let WireValue::Resource(resource) = &value.value else { panic!("expected inventory resource") };
            assert!(matches!((field, resource.as_ref()),
                ("sessions", fractal_client::Resource::Session(_))
                | ("devices", fractal_client::Resource::Device(_))
                | ("launches", fractal_client::Resource::Launch(_))
                | ("launch_variants", fractal_client::Resource::LaunchVariant(_))
            ));
        }
        slice.state[field][0]["unexpected_inventory_field"] = Value::Bool(true);
        assert!(slice.decode().is_err(), "strict must validate {field}");
        slice.decode = DecodeMode::Tolerant;
        slice.decode().expect("tolerant inventories must accept future fields");
    }
}

#[test]
fn scenarios_strict_preserves_sibling_device_variant_constraints() {
    let roster = scenarios::load_slice(fixtures().join("fleet-mid-refactor/roster.json")).unwrap();
    let person = roster.state["devices"][0].clone();
    assert_eq!(person["grant_kind"], "person");
    let mut slice = roster;
    slice.state["devices"] = serde_json::json!([person]);
    slice.decode().expect("a complete person device must pass strict decoding");
    let mut missing_person = slice.clone();
    missing_person.state["devices"][0].as_object_mut().unwrap().remove("person_id");
    assert!(missing_person.decode().is_err(), "a person device requires person_id");

    let pairing = client_fixture("assistant-pairing.json");
    let grant = &pairing["value"]["grant"];
    let mut assistant = person.clone();
    assistant.as_object_mut().unwrap().remove("person_id");
    assistant["grant_kind"] = Value::String("assistant".into());
    for field in ["principal", "sender_class", "session_actor"] {
        assistant[field] = pairing["value"][field].clone();
    }
    assistant["grant_id"] = grant["id"].clone();
    assistant["policy_revision"] = grant["policy_revision"].clone();
    assistant["minimal_read_mode"] = grant["minimal_read_mode"].clone();
    slice.state["devices"] = serde_json::json!([assistant]);
    slice.decode().expect("a complete assistant device must pass strict decoding");
    for field in ["principal", "sender_class", "grant_kind", "grant_id", "policy_revision", "minimal_read_mode"] {
        let mut incomplete = slice.clone();
        incomplete.state["devices"][0].as_object_mut().unwrap().remove(field);
        assert!(incomplete.decode().is_err(), "an assistant device requires {field}");
    }
}

#[test]
fn scenarios_schema_engine_enforces_combinators() {
    // Use the same standards-based validator and options as the scenario reader, not
    // a hand-written union matcher: oneOf must reject two matching branches.
    let schema = serde_json::json!({
        "allOf": [
            {"type": "integer"},
            {"anyOf": [{"minimum": 0}, {"maximum": -10}]},
            {"not": {"const": 12}}
        ],
        "oneOf": [{"minimum": 0}, {"maximum": 10}]
    });
    let validator = jsonschema::options().should_validate_formats(true).build(&schema).unwrap();
    assert!(validator.is_valid(&serde_json::json!(11)));
    assert!(validator.is_valid(&serde_json::json!(-10)));
    assert!(!validator.is_valid(&serde_json::json!(5)), "two oneOf branches match");
    assert!(!validator.is_valid(&serde_json::json!(-5)), "neither anyOf branch matches");
    assert!(!validator.is_valid(&serde_json::json!(11.5)), "allOf requires an integer");
    assert!(!validator.is_valid(&serde_json::json!(12)), "not excludes its matching value");
}

#[test]
fn scenarios_strict_composes_presentation_now_before_closing() {
    let mut slice = scenarios::load_slice(fixtures().join("fleet-mid-refactor/roster.json")).unwrap();
    slice.state["agents"][0]["presentation"] = serde_json::json!({
        "state": "working", "group": "working", "because": null, "next": [], "queued": 0,
        "now": {
            "step": "step-run/x", "mission": "mission/x", "path": "a",
            "label": "x › a", "since": "2030-01-01T12:00:00.000Z"
        }
    });
    slice.decode().expect("the kit's non-null presentation.now counterexample must decode");
    for field in ["step", "mission", "path", "label", "since"] {
        let mut missing = slice.clone();
        missing.state["agents"][0]["presentation"]["now"].as_object_mut().unwrap().remove(field);
        assert!(missing.decode().is_err(), "composed presentation requires {field}");
    }
    slice.state["agents"][0]["presentation"]["now"]["extra"] = Value::Bool(true);
    assert!(slice.decode().is_err(), "the composed object remains closed");
}

fn sync_peer_slice() -> scenarios::Slice {
    let mut slice = scenarios::load_slice(fixtures().join("empty/sync.json")).unwrap();
    slice.timeline = vec![serde_json::json!({
        "_tag": "notice", "at_ms": 0,
        "peers": [{"host_id": "host/x", "peer_only_envelopes": 0, "local_only_envelopes": 0}]
    })];
    slice
}

#[test]
fn scenarios_enforces_duration_and_epoch_codec_bounds() {
    for mode in [DecodeMode::Strict, DecodeMode::Tolerant] {
        let mut sync = sync_peer_slice();
        sync.decode = mode;
        sync.timeline[0]["peers"][0]["estimated_catch_up_seconds"] = 9_007_199_254_740_991_i64.into();
        sync.decode().expect("the largest safe duration in seconds must pass");
        sync.timeline[0]["peers"][0]["estimated_catch_up_seconds"] = 9_007_199_254_740_992_i64.into();
        assert!(sync.decode().is_err(), "unsafe duration-s must fail in {mode:?}");

        let mut details = scenarios::load_slice(fixtures().join("fleet-mid-refactor/details.json")).unwrap();
        details.decode = mode;
        for field in ["execution_elapsed_ms", "timeout_ms"] {
            details.state["work"][0][field] = 9_007_199_254_740_991_i64.into();
            details.decode().expect("the largest safe duration in milliseconds must pass");
            details.state["work"][0][field] = 9_007_199_254_740_992_i64.into();
            assert!(details.decode().is_err(), "unsafe {field} must fail in {mode:?}");
            details.state["work"][0][field] = 0.into();
        }
        for field in ["claim_expires_at_unix_ms", "execution_started_at_unix_ms", "nudged_at_unix_ms"] {
            details.state["work"][0][field] = 253_402_300_799_999_i64.into();
            details.decode().expect("the final supported epoch millisecond must pass");
            details.state["work"][0][field] = 253_402_300_800_000_i64.into();
            assert!(details.decode().is_err(), "out-of-range {field} must fail in {mode:?}");
            details.state["work"][0][field] = 0.into();
        }
    }
}

#[test]
fn scenarios_timestamp_codec_matches_kit_grammar_and_calendar() {
    let mut slice = sync_peer_slice();
    for instant in [
        "2030-01-01T12:00:00.000Z", "2030-01-01t12:00:00z",
        "2000-02-29T23:59:59.123456789123+23:59",
        "0000-01-01T00:00:00Z", "9999-12-31T23:59:59.9999Z",
    ] {
        slice.timeline[0]["peers"][0]["last_exchange_at"] = instant.into();
        slice.decode().unwrap_or_else(|error| panic!("kit accepts {instant}: {error:#}"));
    }
    for instant in [
        "2016-12-31T23:59:60Z", "2030-02-30T12:00:00Z", "1900-02-29T12:00:00Z",
        "2030-01-01T24:00:00Z", "2030-01-01T12:60:00Z", "2030-01-01T12:00:00+24:00",
        "2030-01-01 12:00:00Z", "2030-01-01T12:00Z", "2030-01-01T12:00:00.Z",
        "0000-01-01T00:00:00+00:01", "9999-12-31T23:59:59-00:01",
    ] {
        slice.timeline[0]["peers"][0]["last_exchange_at"] = instant.into();
        assert!(slice.decode().is_err(), "kit rejects {instant}");
    }
}

#[test]
fn scenarios_uri_reference_remains_a_kit_string_annotation() {
    let mut slice = scenarios::load_slice(fixtures().join("empty/sync.json")).unwrap();
    slice.state["capabilities"]["schemas"] = serde_json::json!(["not a URI reference"]);
    slice.decode().expect("the kit does not validate the uri-reference annotation");
}

#[test]
fn scenarios_plain_integers_are_safe_and_accept_integral_spellings() {
    let mut slice = sync_peer_slice();
    for spelling in ["1.0", "1e0"] {
        slice.timeline[0]["peers"][0]["peer_only_envelopes"] = serde_json::from_str(spelling).unwrap();
        let decoded = slice.decode().expect("kit accepts integral JSON number spellings");
        assert!(decoded.iter().any(|item| matches!(
            &item.value, WireValue::SyncPeer(peer) if peer.peer_only_envelopes == 1
        )));
    }
    slice.timeline[0]["peers"][0]["peer_only_envelopes"] = 9_007_199_254_740_991_i64.into();
    slice.decode().expect("the largest safe counter must pass");
    slice.timeline[0]["peers"][0]["peer_only_envelopes"] = 9_007_199_254_740_992_i64.into();
    assert!(slice.decode().is_err(), "plain integer counters must also be JSON-safe");
}

#[test]
fn scenarios_strict_string_bounds_count_utf16_units() {
    let mut slice = scenarios::load_slice(fixtures().join("fleet-mid-refactor/roster.json")).unwrap();
    slice.state["agents"][0]["revision"] = "😀".repeat(128).into();
    slice.decode().expect("256 UTF-16 units must fit Revision");
    slice.state["agents"][0]["revision"] = "😀".repeat(129).into();
    assert!(slice.decode().is_err(), "129 code points exceed 256 UTF-16 units");
}

#[test]
fn scenarios_reference_patterns_use_ecmascript_whitespace() {
    let mut slice = sync_peer_slice();
    for whitespace in [
        '\t', '\n', '\u{000B}', '\u{000C}', '\r', ' ', '\u{00A0}', '\u{1680}',
        '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}',
        '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200A}', '\u{2028}',
        '\u{2029}', '\u{202F}', '\u{205F}', '\u{3000}', '\u{FEFF}',
    ] {
        slice.timeline[0]["peers"][0]["host_id"] = format!("host/a{whitespace}b").into();
        assert!(slice.decode().is_err(), "ECMAScript whitespace {whitespace:?} cannot occur in Id");
    }
    slice.timeline[0]["peers"][0]["host_id"] = "host/a\u{0085}b".into();
    slice.decode().expect("U+0085 is not ECMAScript whitespace");
    // Lookahead-bearing propertyNames bypasses jsonschema's normal ECMA translation.
    let mut roster = scenarios::load_slice(fixtures().join("fleet-mid-refactor/roster.json")).unwrap();
    let placement = roster.state["arrangements"][0]["body"]["placements"]
        .as_object().unwrap().values().next().unwrap().clone();
    roster.state["arrangements"][0]["body"]["placements"] =
        serde_json::json!({"agent/a\u{0085}b": placement});
    roster.decode().expect("propertyNames must also accept the non-JS whitespace U+0085");
    roster.state["arrangements"][0]["body"]["placements"] =
        serde_json::json!({"agent/a\u{FEFF}b": placement});
    assert!(roster.decode().is_err(), "propertyNames must reject JS whitespace U+FEFF");
}

const SAFE_INTEGER_MAX: i64 = 9_007_199_254_740_991;

fn integer_boundaries(minimum: i64) -> Vec<i64> {
    let mut values = vec![minimum, SAFE_INTEGER_MAX, 4_294_967_295, 4_294_967_296];
    if minimum < 0 {
        values.extend([-2_147_483_649, -2_147_483_648, -1, 0, 2_147_483_647, 2_147_483_648]);
    }
    values
}

fn assert_scenario_integer_round_trip(slice: &scenarios::Slice, root: &str, field: &str, minimum: i64) {
    let pointer = format!("{root}{field}");
    for mode in [DecodeMode::Strict, DecodeMode::Tolerant] {
        for number in integer_boundaries(minimum) {
            let mut input = serde_json::to_value(slice).unwrap();
            *input.pointer_mut(&pointer).expect("numeric fixture field") = number.into();
            let mut candidate: scenarios::Slice = serde_json::from_value(input).unwrap();
            candidate.decode = mode;
            let decoded = candidate.decode().unwrap_or_else(|error| panic!("{pointer}={number} in {mode:?}: {error:#}"));
            let value = &decoded.iter().find(|value| value.pointer == root).expect("decoded model root").value;
            let encoded = match value {
                WireValue::Resource(value) => serde_json::to_value(value).unwrap(),
                WireValue::TimelineEntry(value) => serde_json::to_value(value).unwrap(),
                WireValue::TerminalScreen(value) => serde_json::to_value(value).unwrap(),
                WireValue::Capabilities(value) => serde_json::to_value(value).unwrap(),
                other => panic!("unexpected numeric model: {other:?}"),
            };
            assert_eq!(encoded.pointer(field), Some(&Value::from(number)), "{pointer} in {mode:?}");
            assert_eq!(serde_json::to_value(&candidate).unwrap().pointer(&pointer), Some(&Value::from(number)));
        }
        for number in [minimum - 1, SAFE_INTEGER_MAX + 1] {
            let mut input = serde_json::to_value(slice).unwrap();
            *input.pointer_mut(&pointer).unwrap() = number.into();
            let mut candidate: scenarios::Slice = serde_json::from_value(input).unwrap();
            candidate.decode = mode;
            assert!(candidate.decode().is_err(), "{pointer}={number} must violate schema bounds in {mode:?}");
        }
    }
}

#[test]
fn scenarios_model_integer_boundaries_round_trip_without_truncating() {
    let mut sync = scenarios::load_slice(fixtures().join("empty/sync.json")).unwrap();
    sync.state["capabilities"] = client_fixture("capabilities.json")["value"].clone();
    assert_scenario_integer_round_trip(&sync, "/state/capabilities", "/capabilities/0/version", 0);

    let mut terminal = scenarios::load_slice(fixtures().join("empty/terminal.json")).unwrap();
    let mut screen = client_fixture("terminal-screen.json")["value"].clone();
    screen["facts"]["process"]["exit_code"] = 0.into();
    terminal.timeline = vec![serde_json::json!({"_tag": "screen", "at_ms": 0, "screen": screen})];
    for field in ["/cursor/row", "/cursor/column", "/facts/process/exit_code"] {
        assert_scenario_integer_round_trip(&terminal, "/timeline/0/screen", field, -SAFE_INTEGER_MAX);
    }
    for field in ["/modes/kitty_keyboard", "/relay/fence_conflicts"] {
        assert_scenario_integer_round_trip(&terminal, "/timeline/0/screen", field, 0);
    }

    let mut conversation = scenarios::load_slice(fixtures().join("fleet-mid-refactor/conversation.json")).unwrap();
    conversation.timeline.clear();
    assert_scenario_integer_round_trip(&conversation, "/state/threads/0/items/0", "/revision", 1);
}

#[test]
fn scenarios_resource_integer_boundaries_round_trip_without_truncating() {
    let resources = client_fixture("resources.json");
    let resource = |kind: &str| {
        resources.as_array().unwrap().iter().find(|value| value["kind"] == kind).unwrap().clone()
    };
    let mut details = scenarios::load_slice(fixtures().join("fleet-mid-refactor/details.json")).unwrap();
    details.timeline.clear();
    let mut mission = client_fixture("person-ask-mission.json");
    mission["run_details"][0]["steps"][0]["loop_round"] = 0.into();
    mission["run_details"][0]["steps"][0]["loop_max_rounds"] = 0.into();
    details.state["missions"] = serde_json::json!([mission]);
    details.state["work"] = serde_json::json!([resource("work")]);
    details.state["launches"] = serde_json::json!([resource("launch")]);
    for field in [
        "/run_details/0/steps/0/attempt",
        "/run_details/0/steps/0/loop_round",
        "/run_details/0/steps/0/loop_max_rounds",
        "/run_details/0/steps/0/wake/attempts",
    ] {
        assert_scenario_integer_round_trip(&details, "/state/missions/0", field, 0);
    }
    assert_scenario_integer_round_trip(&details, "/state/work/0", "/attempt", 0);
    assert_scenario_integer_round_trip(&details, "/state/launches/0", "/visualization/nodes/0/timeout_ms", -SAFE_INTEGER_MAX);

    // The committed launch supplies the surrounding resource; preview is optional
    // and absent in that fixture, so populate only its schema-required fields.
    details.state["launches"][0]["preview"] = serde_json::json!({
        "goal": details.state["launches"][0]["request"],
        "steps": [], "agents": [], "gates": {"human": 0, "automated": 0},
        "diagnostics_count": 0, "request_excerpt": ""
    });
    assert_scenario_integer_round_trip(&details, "/state/launches/0", "/preview/diagnostics_count", -SAFE_INTEGER_MAX);
    details.state["resources"] = serde_json::json!([resource("lane")]);
    assert_scenario_integer_round_trip(&details, "/state/resources/0", "/entries/0/position", 1);
}

fn assert_model_integer_round_trip<T>(input: &Value, field: &str, minimum: i64)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    for number in integer_boundaries(minimum) {
        let mut candidate = input.clone();
        // Optional fields need not be present in the committed fixture.
        candidate.as_object_mut().unwrap().insert(field.to_owned(), number.into());
        let decoded: T = serde_json::from_value(candidate).unwrap();
        let encoded = serde_json::to_value(decoded).unwrap();
        assert_eq!(encoded[field], number, "{field} must survive serde decode and encode");
    }
}

#[test]
fn action_fence_and_agent_queue_integer_boundaries_round_trip() {
    // These response/action models are not scenario wire roots.
    let action = client_fixture("terminal-create.json");
    assert_model_integer_round_trip::<fractal_client::Fence>(&action["fence"], "attempt", 0);

    let mission = client_fixture("person-ask-mission.json");
    let run = &mission["run_details"][0];
    let queue_run = serde_json::json!({
        "mission_run_id": run["id"], "position": 1,
        "state": "ready", "run_state": run["status"], "joined_at": run["state_since"],
        "claimed_work_ids": [], "ready_work_ids": [], "waiting_work_ids": []
    });
    assert_model_integer_round_trip::<fractal_client::AgentQueueRun>(&queue_run, "position", 1);
}
