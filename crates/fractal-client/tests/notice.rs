// Models remain internal: no library export or transport registration.
#[allow(dead_code)]
#[path = "../src/notice_models.generated.rs"]
mod models;
use models::*;
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
}
