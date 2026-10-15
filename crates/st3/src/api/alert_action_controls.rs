use super::tests::{json_request, state};
use super::*;

#[tokio::test]
async fn alerts_resolve_dismisses_only_the_exact_granted_denial_source() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let (denial, proof) =
        crate::store::alert_action_controls::seed_denial(&state.store, &["close-item"]);
    let app = router(state.clone());
    let card = client_attention_id(&denial.subject, &proof.person, &denial.id).unwrap();
    let path = format!("/v1/attention/resolve/{}", urlencoding::encode(&card));
    let body = json!({"actor":"agent/alder.helper","outcome":"dismissed","delegation":proof,"idempotency_key":"api-dismiss-source"});
    let mut wrong = body.clone();
    wrong["outcome"] = json!("resolved");
    assert_ne!(
        json_request(app.clone(), &path, wrong).await.0,
        StatusCode::OK
    );
    assert_eq!(state.store.prompt_answer(&denial.id).unwrap().answer, None);
    let (status, result) = json_request(app.clone(), &path, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "dismissed");
    assert_eq!(result["actor"], "agent/alder.helper");
    let (status, repeated) = json_request(app, &path, body).await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(
        state
            .store
            .messages(Some("agent/alder.worker"), true)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn alerts_withdraw_resolves_its_own_update_source_and_refuses_other_sources() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let source = "version 2\nagent \"alder.helper\" { workspace \"/tmp\"; command \"true\" }\n";
    let intent = crate::graph::parse_internal_intent(source, "node").unwrap();
    state
        .store
        .apply_internal(&intent, "declared-helper")
        .unwrap();
    state.store.append_claim(&ClaimInput {subject:"message/update-instruction".into(),kind:"message.sent".into(),actor:Some("person/avery".into()),
        fields:serde_json::from_value(json!({"from":"person/avery","to":"agent/alder.helper","content":"Report the result","status":"sent"})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:Some("update-instruction".into())}).unwrap();
    let app = router(state.clone());
    let (status, update) = json_request(
        app.clone(),
        "/v1/work/ask",
        json!({"person":"person/avery","actor":"agent/alder.helper",
        "title":"Invented status","reason":"The report is obsolete","idempotency_key":"own-update",
        "request":{"version":1,"type":"update","about":"message/update-instruction"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{update}");
    let subject = update["subject"].as_str().unwrap();
    let episode = state
        .store
        .claims_for(subject, Some("work.person-asked"))
        .unwrap()[0]
        .id
        .clone();
    let card = client_attention_id(subject, "person/avery", &episode).unwrap();
    let path = format!("/v1/attention/withdraw/{}", urlencoding::encode(&card));
    let mut body = json!({"actor":"agent/alder.stranger","episode":episode,"reason":"Withdrawn","idempotency_key":"withdraw-own"});
    assert_ne!(
        json_request(app.clone(), &path, body.clone()).await.0,
        StatusCode::OK
    );
    body["actor"] = json!("agent/alder.helper");
    let (status, result) = json_request(app.clone(), &path, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["status"], "cancelled");
    assert_eq!(json_request(app, &path, body).await.0, StatusCode::OK);
    assert!(
        state
            .store
            .attention_items(Some("person/avery"))
            .unwrap()
            .iter()
            .all(|item| item.subject != subject)
    );
}
