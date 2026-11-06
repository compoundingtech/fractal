// Models remain internal: no library export or transport registration.
#[allow(dead_code)]
#[path = "../src/notice_models.generated.rs"]
mod notice_models;
#[path = "../src/notice_descriptors.generated.rs"]
mod descriptors;
use notice_models::*;
use serde::Serialize;
use serde_json::Value;

fn round_trip<T: NoticeModel + Serialize>(value: &Value) {
    let typed: T = decode_notice(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), *value);
    let direct: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(direct).unwrap(), *value);
}

#[test]
fn notice_fixture_models() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/fixtures/notice-conformance.json"
    ))
    .unwrap();
    round_trip::<NoticeSnapshot>(&fixture["snapshot"]);
    let wire = serde_json::to_vec(&fixture["snapshot"]).unwrap();
    let direct = descriptors::decode_attention_notice(&wire).unwrap();
    assert_eq!(serde_json::to_value(direct).unwrap(), fixture["snapshot"]);
    let metadata: Value = serde_json::from_str(descriptors::DESCRIPTORS_JSON).unwrap();
    assert_eq!(metadata[0], serde_json::json!({
        "model": "NoticeSnapshot", "kind": "attention-notice", "collection": "notices",
        "id_schema": "NoticeId", "internal": true
    }));
    round_trip::<NoticeCreated>(&fixture["snapshot"]["input"]);
    for value in fixture["valid_delivery"].as_array().unwrap() {
        round_trip::<NoticeDelivery>(value);
    }
    for value in fixture["valid_integrity"].as_array().unwrap() {
        round_trip::<NoticeIntegrity>(value);
    }
    for mutation in fixture["invalid_snapshot"].as_array().unwrap() {
        let mut value = fixture["snapshot"].clone();
        let pointer = mutation["pointer"].as_str().unwrap();
        if pointer.ends_with("/extra") {
            value
                .pointer_mut(pointer.strip_suffix("/extra").unwrap())
                .unwrap()["extra"] = mutation["value"].clone();
        } else {
            *value.pointer_mut(pointer).unwrap() = mutation["value"].clone();
        }
        assert!(descriptors::decode_attention_notice(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(
            decode_notice::<NoticeSnapshot>(value.clone()).is_err(),
            "{pointer}"
        );
        assert!(
            serde_json::from_value::<NoticeSnapshot>(value).is_err(),
            "{pointer}"
        );
    }
    let mut value = fixture["snapshot"].clone();
    value.as_object_mut().unwrap().remove("seen_at");
    assert!(serde_json::from_value::<NoticeSnapshot>(value).is_err());
    let mut input = fixture["snapshot"]["input"].clone();
    input.as_object_mut().unwrap().remove("speech");
    round_trip::<NoticeCreated>(&input);
    input["summary"] = Value::String("😀".repeat(200));
    round_trip::<NoticeCreated>(&input);
    input["summary"] = Value::String("😀".repeat(201));
    assert!(serde_json::from_value::<NoticeCreated>(input).is_err());
    let mut value = fixture["snapshot"].clone();
    value["seen_at"] = fixture["snapshot"]["updated_at"].clone();
    value["control_revision"] = Value::String("control-v1".into());
    round_trip::<NoticeSnapshot>(&value);
    value["creation_claims"] = serde_json::json!([value["basis_claim"], value["basis_claim"]]);
    assert!(serde_json::from_value::<NoticeSnapshot>(value).is_err());
    let id = fixture["snapshot"]["id"].clone();
    round_trip::<NoticeSeenParameters>(&serde_json::json!({"notice_id": id}));
    round_trip::<NoticeDismissParameters>(&serde_json::json!({"notice_id": id}));
    round_trip::<NoticeSnoozeParameters>(&serde_json::json!({"notice_id": id, "until": null}));
    round_trip::<NoticeFence>(
        &serde_json::json!({"notice_id": id, "source_revision": "v1", "control_revision": null}),
    );
    for (key, accepted) in [("valid_timestamps", true), ("invalid_timestamps", false)] {
        for scalar in fixture[key].as_array().unwrap() {
            assert_eq!(serde_json::from_value::<NoticeTimestamp>(scalar.clone()).is_ok(), accepted);
            assert_eq!(decode_notice::<NoticeTimestamp>(scalar.clone()).is_ok(), accepted);
            if accepted { round_trip::<NoticeTimestamp>(scalar); }
            let mut nested = fixture["snapshot"].clone();
            nested["input"]["created_at"] = scalar.clone();
            assert_eq!(serde_json::from_value::<NoticeSnapshot>(nested.clone()).is_ok(), accepted);
            assert_eq!(decode_notice::<NoticeSnapshot>(nested.clone()).is_ok(), accepted);
            if accepted { round_trip::<NoticeSnapshot>(&nested); }
        }
    }
}
