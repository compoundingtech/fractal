use super::*;

#[tokio::test]
async fn assistant_mission_cursor_expires_when_unread_mission_moves_before_its_key() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let intent = crate::graph::parse_internal_intent(
        r#"version 2
mission "cursor-first" state="ready" { goal "first"; }
mission "cursor-second" state="ready" { goal "second"; }
"#,
        "reads-fixture",
    ).unwrap();
    state.store.apply_internal(&intent, "cursor-initial").unwrap();
    let admitted = new_client_snapshot(&state);
    let (snapshot, first) = project(
        &state, &admitted, "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    let cursor = first["page"]["next_cursor"].as_str().expect("second mission remains");
    let returned = first["items"][0]["id"].as_str().unwrap();
    let unread = if returned == "mission/cursor-first" { "cursor-second" } else { "cursor-first" };
    // A normal new run moves the unread mission above the saved mutable sort key.
    state.store.create_mission_run(&crate::model::MissionRunRequest {
        mission: unread.to_owned(), revision: None,
        workspace: root.path().display().to_string(), requester: Some(OWNER.into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: "cursor-updated-run".into(),
    }).unwrap();
    let error = project(
        &state, &snapshot, "missions", &params(&[("cursor", cursor)]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap_err();
    assert_eq!(error.code, "page-cursor-expired");
}

#[tokio::test]
async fn assistant_mission_cursor_bounds_unrelated_receipts_and_rejects_relevant_changes() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    fleet(&state, root.path(), false);
    let (snapshot, first) = project(
        &state, &new_client_snapshot(&state), "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    let cursor = first["page"]["next_cursor"].as_str().unwrap();
    let query = params(&[("cursor", cursor)]);
    for n in 0..512 {
        state.store.append_claim(&ClaimInput {
            subject: format!("custom/assistant-receipt/{n}"),
            kind: "custom.test.receipt".into(), actor: None, fields: BTreeMap::new(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    let (_, continued) = project(
        &state, &snapshot, "missions", &query, MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.expect("exactly 512 unrelated receipt headers remain within the bound");
    assert_ne!(continued["items"][0]["id"], first["items"][0]["id"]);
    assert_eq!(project(
        &state, &snapshot, "missions", &query, MAX_RESPONSE_BYTES, "another-grant:1",
    ).await.unwrap_err().code, "page-cursor-expired");
    state.store.append_claim(&ClaimInput {
        subject: "custom/assistant-receipt/overflow".into(),
        kind: "custom.test.receipt".into(), actor: None, fields: BTreeMap::new(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert_eq!(project(
        &state, &snapshot, "missions", &query, MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap_err().code, "page-cursor-expired");

    let (fresh_snapshot, fresh) = project(
        &state, &new_client_snapshot(&state), "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    let fresh_query = params(&[("cursor", fresh["page"]["next_cursor"].as_str().unwrap())]);
    state.store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "reads-fixture-second".into(), revision: None,
        workspace: root.path().display().to_string(), requester: Some(OWNER.into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: "cursor-relevant-change".into(),
    }).unwrap();
    assert_eq!(project(
        &state, &fresh_snapshot, "missions", &fresh_query, MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap_err().code, "page-cursor-expired");
}
