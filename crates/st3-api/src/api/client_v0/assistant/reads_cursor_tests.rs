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
async fn assistant_mission_cursor_expires_when_admitted_replication_projects_after_first_page() {
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let source = crate::store::Store::open_memory("cursor-source").unwrap();
    source.bind_fleet(FLEET).unwrap();
    state.store.bind_fleet(FLEET).unwrap();
    let intent = crate::graph::parse_internal_intent(
        r#"version 2
mission "cursor-first" state="ready" { goal "first"; }
mission "cursor-second" state="ready" { goal "second"; }
"#,
        "cursor-source",
    ).unwrap();
    source.apply_internal(&intent, "replicated-cursor-initial").unwrap();
    let admit = || {
        let exchange = source.export_replication_exchange(
            FLEET, &state.store.replication_inventory().unwrap(),
        ).unwrap();
        state.store.receive_replication_exchange("cursor-source", FLEET, &exchange).unwrap();
        state.store.validate_replication_backlog().unwrap();
    };
    admit();
    state.store.project_replication_backlog().unwrap();
    let (_, before) = project(
        &state, &new_client_snapshot(&state), "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    let returned = before["items"][0]["id"].as_str().unwrap();
    let unread = if returned == "mission/cursor-first" { "cursor-second" } else { "cursor-first" };
    source.create_mission_run(&crate::model::MissionRunRequest {
        mission: unread.to_owned(), revision: None,
        workspace: root.path().display().to_string(), requester: Some(OWNER.into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: "replicated-cursor-run".into(),
    }).unwrap();
    admit();
    let admitted = state.store.index().unwrap();
    let frontier = state.store.attention_projection_frontier().unwrap();
    let generation = smallclaims::store::graph_generation(&state.store.readers.get()).unwrap();
    let (snapshot, first) = project(
        &state, &new_client_snapshot(&state), "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    assert_eq!(snapshot.store_index, admitted, "first page includes the admitted run in its claim cut");
    assert_eq!(first["items"][0]["id"], returned, "admission has not projected the run");
    let cursor = first["page"]["next_cursor"].as_str().unwrap();
    project(
        &state, &snapshot, "missions", &params(&[("cursor", cursor)]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.expect("admission alone leaves the captured projection stable");
    state.store.project_replication_backlog().unwrap();
    assert_eq!(state.store.index().unwrap(), admitted, "projection admits no new claim");
    assert_ne!(state.store.attention_projection_frontier().unwrap(), frontier);
    assert_ne!(smallclaims::store::graph_generation(&state.store.readers.get()).unwrap(), generation);
    let (_, reordered) = project(
        &state, &new_client_snapshot(&state), "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    assert_eq!(reordered["items"][0]["id"], format!("mission/{unread}"),
        "projection moves the unread mission before the saved key");
    assert_eq!(project(
        &state, &snapshot, "missions", &params(&[("cursor", cursor)]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap_err().code, "page-cursor-expired");
}

#[tokio::test]
async fn assistant_mission_cursor_expires_on_claimless_full_replay_at_the_same_frontier() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    fleet(&state, root.path(), false);
    state.store.replay_replication_graph().unwrap();
    let (snapshot, first) = project(
        &state, &new_client_snapshot(&state), "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    let frontier = state.store.attention_projection_frontier().unwrap();
    let generation = smallclaims::store::graph_generation(&state.store.readers.get()).unwrap();
    state.store.replay_replication_graph().unwrap();
    assert_eq!(state.store.index().unwrap(), snapshot.store_index);
    assert_eq!(state.store.attention_projection_frontier().unwrap(), frontier);
    assert_ne!(smallclaims::store::graph_generation(&state.store.readers.get()).unwrap(), generation);
    assert_eq!(project(
        &state, &snapshot, "missions",
        &params(&[("cursor", first["page"]["next_cursor"].as_str().unwrap())]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap_err().code, "page-cursor-expired");
}

#[tokio::test]
async fn assistant_mission_cursor_conservatively_expires_on_unrelated_graph_projection() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    fleet(&state, root.path(), false);
    let (snapshot, first) = project(
        &state, &new_client_snapshot(&state), "missions", &params(&[("limit", "1")]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap();
    let intent = crate::graph::parse_internal_intent(
        r#"version 2
agent "reads-fixture.unrelated" { workspace "/tmp"; command "true"; }
"#,
        "reads-fixture",
    ).unwrap();
    state.store.apply_internal(&intent, "cursor-unrelated-agent").unwrap();
    assert_eq!(project(
        &state, &snapshot, "missions",
        &params(&[("cursor", first["page"]["next_cursor"].as_str().unwrap())]),
        MAX_RESPONSE_BYTES, "test-grant:1",
    ).await.unwrap_err().code, "page-cursor-expired");
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
    let generation = smallclaims::store::graph_generation(&state.store.readers.get()).unwrap();
    for n in 0..256 {
        // Use the production receipt writer and both real read-authorization claim kinds.
        let subject = format!("custom/client/assistant-v1-read-cursor-{n}");
        let consumed = append(
            &state, subject.clone(), "custom.client.assistant.v1.read-used", PRINCIPAL,
            json!({"version":1,"projection":"missions","outcome":"consumed"}), None,
        ).unwrap();
        append(
            &state, subject, "custom.client.assistant.v1.read-outcome", PRINCIPAL,
            json!({"version":1,"outcome":"delivered","returned_bytes":0}), Some(&consumed),
        ).unwrap();
    }
    assert_eq!(smallclaims::store::graph_generation(&state.store.readers.get()).unwrap(), generation,
        "read-authorization receipts do not mutate graph projections");
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
