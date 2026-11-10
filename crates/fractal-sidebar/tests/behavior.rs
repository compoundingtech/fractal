//! Shared v1 golden cases exercise the public, transport-free core.
use std::collections::BTreeSet;

use fractal_sidebar::model::ArrangementInput;
use fractal_sidebar::plan::{MoveIntent, can_place, plan_move};
use fractal_sidebar::projection::{build, rollup};
use serde_json::{Value, json};

    #[test]
    fn shared_vectors() {
        let vectors = behavior_vectors();
        for case in vectors["cases"].as_array().unwrap() {
            let input = &case["initial"]["arrangement"];
            let arrangement = ArrangementInput {
                subject: input["id"].as_str().unwrap().into(),
                owner: input["owner"].as_str().unwrap().into(),
                revision: input["revision"].as_str().unwrap().into(),
                deleted: input["deleted"].as_bool().unwrap(),
                body: serde_json::from_value(input["body"].clone()).unwrap(),
                resolved: serde_json::from_value(input["resolved"].clone()).unwrap(),
            };
            let inventory: Vec<String> = serde_json::from_value(case["catalog"].clone()).unwrap();
            let event = &case["events"][0];
            let checkpoint = &case["checkpoints"][0];
            assert_eq!(event["type"], "ProjectAndPlanV1");
            // This event invokes pure APIs only: no reducer effects or wire dispatch.
            assert_eq!(checkpoint["effects"], json!([]), "{}", case["name"]);
            assert_eq!(checkpoint["wireActions"], json!([]), "{}", case["name"]);
            let tree = build(&arrangement, &inventory).unwrap();
            assert_eq!(
                serde_json::to_value(&tree).unwrap(),
                checkpoint["tree"],
                "{}",
                case["name"]
            );
            assert_eq!(event["moves"].as_array().unwrap().len(), checkpoint["moves"].as_array().unwrap().len());
            for (intent, expected) in event["moves"].as_array().unwrap().iter().zip(checkpoint["moves"].as_array().unwrap()) {
                let intent: MoveIntent = serde_json::from_value(intent.clone()).unwrap();
                let result = plan_move(&tree, &intent);
                assert_eq!(can_place(&tree, &intent), result.clone().map(|_| ()));
                let serialized = match result {
                    Ok(intent) => json!({"_tag": "Planned", "intent": intent}),
                    Err(refusal) => json!({"_tag": "Refused", "refusal": refusal}),
                };
                assert_eq!(serialized, *expected, "{}: {intent:?}", case["name"]);
            }
            let needs_you: BTreeSet<String> =
                serde_json::from_value(event["needsYou"].clone()).unwrap();
            assert_eq!(
                serde_json::to_value(rollup(&tree, &needs_you)).unwrap(),
                checkpoint["rollup"],
                "{}",
                case["name"]
            );
        }
    }

    fn behavior_vectors() -> Value {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../fixtures/clients/sidebar/behavior.schema.json"
        )).unwrap();
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../fixtures/clients/sidebar/behavior-v1.json"
        )).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors: Vec<_> = validator.iter_errors(&vectors).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{errors:#?}");
        vectors
    }

    #[test]
    fn behavior_schema_round_trip_and_rejects_unknown_fields() {
        let vectors = behavior_vectors();
        let encoded = serde_json::to_string(&vectors).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), vectors);
        assert_eq!(vectors["cases"].as_array().unwrap().len(), 5);
        assert_eq!(vectors["cases"].as_array().unwrap().iter()
            .map(|case| case["events"][0]["moves"].as_array().unwrap().len()).sum::<usize>(), 10);
        let schema: Value = serde_json::from_str(include_str!(
            "../../../fixtures/clients/sidebar/behavior.schema.json"
        )).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for pointer in ["", "/cases/0", "/cases/0/initial", "/cases/0/initial/arrangement",
            "/cases/0/initial/arrangement/body", "/cases/0/events/0",
            "/cases/0/identities", "/cases/0/checkpoints/0",
            "/cases/1/checkpoints/0/tree/nodes/folder:00000000-0000-7000-8000-00000000000a",
            "/cases/1/checkpoints/0/moves/1/refusal/path/0"] {
            let mut invalid = vectors.clone();
            invalid.pointer_mut(pointer).unwrap()["unknown"] = json!(true);
            assert!(!validator.is_valid(&invalid), "{pointer}");
        }
        for (pointer, value) in [("/schemaVersion", json!(2)),
            ("/cases/0/initial/arrangement/body/version", json!(2)),
            ("/cases/0/events/0/type", json!("Unknown")),
            ("/cases/0/checkpoints/0/effects", json!([{}])),
            ("/cases/0/checkpoints/0/wireActions", json!([{}]))] {
            let mut invalid = vectors.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(!validator.is_valid(&invalid), "{pointer}");
        }
    }
