use super::*;
use crate::{OwnerListsStore, PersonWorkStore};

async fn control(state: &AppState, actor: &str, request: ActionRequest) -> Result<Json<Value>, ApiError> {
    let session = ClientSession::for_tests_scoped("client/attention-fixture", actor, &["control.attention"]);
    action_inner(State(state.clone()), Extension(new_client_snapshot(state)), Extension(session), Json(request), None).await
}

#[tokio::test]
async fn native_attention_controls_are_person_scoped_fenced_and_replay_the_original_revision() {
    let root = tempfile::tempdir().unwrap();
    let state = tests::test_state_named(root.path(), "attention-controls");
    let source = "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\"; }";
    let intent = crate::graph::parse_intent(source, "attention-controls").unwrap();
    let planned = state.store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    state.store.apply(&intent, &planned.subject_tokens, "declare").unwrap();
    state.store.ask_person(&serde_json::from_value(json!({
        "person":"person/example", "actor":"agent/example/worker", "title":"Choose",
        "reason":"A person decides", "new_run":"example/choice", "idempotency_key":"ask",
        "request":{"version":1,"type":"feedback","question":"Which?","why_person":"Only you can choose"}
    })).unwrap()).unwrap();
    let now = client_now_ms();
    let rows = |at| crate::api::client_attention_resources_at(&state.store, Some("person/example"), false, at).unwrap();
    let original = rows(now).remove(0);
    let request = |kind: &str, key: &str, revision: Value| ActionRequest {
        api_version: CLIENT_API_VERSION.into(), id: format!("action/{key}"),
        action_type: format!("attention.{kind}"), idempotency_key: format!("attention-controls-{key}"),
        fence: Fence { snapshot_id: new_client_snapshot(&state).id, ..Default::default() },
        parameters: json!({"target_id":original["id"], "source_id":original["source_id"],
            "attention_kind":original["attention_kind"], "episode":original["episode"],
            "source_revision":original["presentation_control"]["source_revision"], "expected_control_revision":revision}),
    };
    let dismiss = request("dismiss", "dismiss", Value::Null);
    let before = state.store.index().unwrap();
    for actor in ["person/other", "agent/example/worker"] {
        assert!(control(&state, actor, dismiss.clone()).await.unwrap_err().status.is_client_error());
        assert_eq!(state.store.index().unwrap(), before);
    }
    for (field, forged) in [("person", json!("person/example")), ("source_revision", json!("stale")),
        ("episode", json!("stale")), ("source_id", json!("agent/other")), ("attention_kind", json!("fault"))] {
        let mut stale = request("dismiss", field, Value::Null);
        stale.parameters[field] = forged;
        assert!(control(&state, "person/example", stale).await.is_err());
        assert_eq!(state.store.index().unwrap(), before);
    }
    for field in ["expected_control_revision", "source_revision", "episode"] {
        let mut missing = request("dismiss", field, Value::Null);
        missing.parameters.as_object_mut().unwrap().remove(field);
        assert!(control(&state, "person/example", missing).await.is_err());
        assert_eq!(state.store.index().unwrap(), before);
    }
    let dismissed = control(&state, "person/example", dismiss.clone()).await.unwrap().0;
    let revision = dismissed["control_revision"].clone();
    assert!(revision.is_string());
    assert!(rows(now).is_empty());
    assert_eq!(state.store.attention_snapshot(Some("person/example"), now).unwrap().len(), 1);
    state.store.start_attention_list_refresher().unwrap();
    crate::api::refresh_attention_list(&state.store).unwrap();
    assert!(state.store.published_attention_list().unwrap().rows.is_empty());
    let before = state.store.index().unwrap();
    assert!(control(&state, "person/example", request("restore", "stale-control", Value::Null)).await.is_err());
    assert_eq!(state.store.index().unwrap(), before);
    let restored = control(&state, "person/example", request("restore", "restore", revision.clone())).await.unwrap().0;
    assert_ne!(restored["control_revision"], revision);
    assert!(restored["control_revision"].is_string());
    crate::api::refresh_attention_list(&state.store).unwrap();
    assert_eq!(state.store.published_attention_list().unwrap().rows.len(), 1);
    let before = state.store.index().unwrap();
    let replay = control(&state, "person/example", dismiss).await.unwrap().0;
    assert_eq!(replay["control_revision"], dismissed["control_revision"]);
    assert_eq!(state.store.index().unwrap(), before);
    assert_eq!(rows(now).len(), 1, "replay must not dismiss again");
    let until = client_now_ms() + 60_000;
    let mut snooze = request("snooze", "snooze", restored["control_revision"].clone());
    assert!(control(&state, "person/example", snooze.clone()).await.is_err(), "until must be explicit, including when null");
    snooze.parameters["until"] = json!(crate::api::client_timestamp(until));
    let snoozed = control(&state, "person/example", snooze).await.unwrap().0;
    assert!(snoozed["control_revision"].is_string());
    assert!(rows(until - 1).is_empty());
    assert_eq!(rows(until).len(), 1, "expiry reveals the unresolved source");
    assert_eq!(state.store.attention_snapshot(Some("person/example"), now).unwrap().len(), 1);
    let mut clear = request("snooze", "clear", snoozed["control_revision"].clone());
    clear.parameters["until"] = Value::Null;
    let cleared = control(&state, "person/example", clear).await.unwrap().0;
    assert!(cleared["control_revision"].is_string());
    assert_ne!(cleared["control_revision"], snoozed["control_revision"]);
    assert_eq!(rows(client_now_ms()).len(), 1);
    state.store.stop_attention_list_refresher();
}
