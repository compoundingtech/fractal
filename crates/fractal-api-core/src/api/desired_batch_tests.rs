use super::*;
use crate::RetirementsStore;

#[tokio::test]
async fn desired_batch_keeps_stopped_and_retired_declarations_with_status_tokens() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let intent = parse_intent(
        "version 2\nagent \"batch/running\" { command \"true\"; workspace \".\"; restart \"never\"; }\nagent \"batch/stopped\" { command \"true\"; workspace \".\"; restart \"never\"; }\nagent \"batch/retired\" { command \"true\"; workspace \".\"; restart \"never\"; }\n",
        state.store.origin(),
    ).unwrap();
    state.store.apply_internal(&intent, "batch-declare").unwrap();
    let stop = parse_intent("version 2\nstop \"agent/batch/stopped\"\n", state.store.origin()).unwrap();
    state.store.apply_internal(&stop, "batch-stop").unwrap();
    let retired = "agent/batch/retired";
    let token = state.store.selected_desired_token(retired).unwrap().unwrap();
    state.store.retire_agent(retired, retired, "Finished", &token, None, "batch-retire", &[]).unwrap();
    let app = router(state.clone());
    let (status, batch) = json_request(app.clone(), "/v1/desired/batch", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    for name in ["agent/batch/running", "agent/batch/stopped", retired] {
        let item = batch.as_array().unwrap().iter().find(|item| item["subject"] == name).unwrap();
        let (status, card) = get_request(app.clone(), &format!("/v1/status?subject={name}")).await;
        assert_eq!(status, StatusCode::OK, "{card}");
        assert_eq!(item["desired_token"], card["subjects"][0]["desired_token"], "{name}: {card}");
        assert!(item["desired_token"].is_string(), "{item}");
        assert_eq!(item["actual"], json!({"revision": card["subjects"][0]["desired_revision"]}), "{name}: {card}");
        assert_eq!(item["desired"], serde_json::to_value(state.store.desired_subjects().unwrap()
            .iter().find(|item| item.subject == name).unwrap()).unwrap()["desired"]);
    }
    let (status, selected) = json_request(app.clone(), "/v1/desired/batch", json!({
        "subjects": ["agent/batch/stopped", "agent/batch/retired"],
    })).await;
    assert_eq!(status, StatusCode::OK, "{selected}");
    assert_eq!(selected.as_array().unwrap().len(), 2);
    let (status, filtered) = json_request(app, "/v1/desired/batch", json!({"kind":"stop"})).await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    assert_eq!(filtered.as_array().unwrap().len(), 2);
    assert!(filtered.as_array().unwrap().iter().all(|item| item["kind"] == "stop"));
    assert!(filtered.as_array().unwrap().iter().any(|item| item["subject"] == "agent/batch/stopped"));
    assert!(filtered.as_array().unwrap().iter().any(|item| item["subject"] == retired));
}

#[tokio::test]
async fn desired_batch_rejects_more_than_one_thousand_subject_names() {
    let root = tempfile::tempdir().unwrap();
    let app = router(state(root.path()));
    let subjects = vec!["agent/batch/example"; 1001];
    let (status, error) = json_request(app, "/v1/desired/batch", json!({"subjects":subjects})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "desired-batch-limit");
    assert_eq!(error["details"]["limit"], 1000);
    assert_eq!(error["details"]["supplied"], 1001);
}

#[tokio::test]
async fn desired_batch_includes_published_missions_without_inventing_desired_tokens() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let intent = parse_intent(
        "version 2\nmission \"batch\" state=\"ready\" {\n goal \"Bulk mission comparison.\"\n step \"work\" { agentless }\n}\n",
        state.store.origin(),
    ).unwrap();
    state.store.apply_internal(&intent, "batch-mission").unwrap();
    let app = router(state);
    let (status, batch) = json_request(app.clone(), "/v1/desired/batch", json!({
        "subjects": ["mission/batch"], "kind": "mission",
    })).await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch.as_array().unwrap().len(), 1);
    let (status, card) = get_request(app, "/v1/status?subject=mission/batch").await;
    assert_eq!(status, StatusCode::OK, "{card}");
    let item = &batch[0];
    assert_eq!(item["subject"], "mission/batch");
    assert_eq!(item["desired_token"], card["subjects"][0]["desired_token"]);
    assert!(item["desired_token"].is_null());
    for field in ["revision", "state"] {
        assert_eq!(item["actual"][field], card["subjects"][0]["actual"][field], "{field}: {card}");
    }
    assert!(item["actual"]["digest"].as_str().unwrap().starts_with("sha256:"));
    assert!(item["actual"].get("body").is_none());
}
