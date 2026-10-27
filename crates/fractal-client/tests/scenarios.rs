use fractal_client::scenarios::{self, DecodeMode, TimePointer, WireValue};
use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::PathBuf};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scenarios")
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
    let WireValue::Resource(fractal_client::Resource::Agent(agent)) = &agent.value else { panic!("expected typed agent") };
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
    assert!(decoded.iter().any(|value| matches!(&value.value, WireValue::Resource(fractal_client::Resource::Unknown(_)))));
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
