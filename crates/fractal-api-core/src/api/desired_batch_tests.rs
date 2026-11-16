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
    let (status, batch) = get_request(app.clone(), "/v1/desired").await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    for name in ["agent/batch/running", "agent/batch/stopped", retired] {
        let item = batch["items"].as_array().unwrap().iter().find(|item| item["subject"] == name).unwrap();
        let (status, card) = get_request(app.clone(), &format!("/v1/status?subject={name}")).await;
        assert_eq!(status, StatusCode::OK, "{card}");
        assert_eq!(item["desired_token"], card["subjects"][0]["desired_token"], "{name}: {card}");
        assert!(item["desired_token"].is_string(), "{item}");
        assert_eq!(item["actual"], json!({"revision": card["subjects"][0]["desired_revision"]}), "{name}: {card}");
        assert_eq!(item["desired"], serde_json::to_value(state.store.desired_subjects().unwrap()
            .iter().find(|item| item.subject == name).unwrap()).unwrap()["desired"]);
    }
    assert!(batch["store_index"].is_u64());
    assert!(batch["next_cursor"].is_null());
    let (status, selected) = get_request(
        app.clone(),
        "/v1/desired?subject=agent/batch/stopped&subject=agent/batch/retired&subject=agent/batch/stopped",
    ).await;
    assert_eq!(status, StatusCode::OK, "{selected}");
    assert_eq!(selected["items"].as_array().unwrap().len(), 2);
    let (status, filtered) = get_request(app.clone(), "/v1/desired?kind=stop").await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    assert_eq!(filtered["items"].as_array().unwrap().len(), 2);
    assert!(filtered["items"].as_array().unwrap().iter().all(|item| item["kind"] == "stop"));
    assert!(filtered["items"].as_array().unwrap().iter().any(|item| item["subject"] == "agent/batch/stopped"));
    assert!(filtered["items"].as_array().unwrap().iter().any(|item| item["subject"] == retired));
    let (status, filtered) = get_request(
        app, "/v1/desired?subject=agent/batch/running&subject=agent/batch/stopped&kind=stop",
    ).await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["items"][0]["subject"], "agent/batch/stopped");
}

#[tokio::test]
async fn desired_batch_rejects_more_than_one_thousand_subject_names() {
    let root = tempfile::tempdir().unwrap();
    let app = router(state(root.path()));
    let query = std::iter::repeat_n("subject=agent/batch/example", 1001).collect::<Vec<_>>().join("&");
    let (status, error) = get_request(app, &format!("/v1/desired?{query}")).await;
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
    let (status, batch) = get_request(
        app.clone(), "/v1/desired?subject=mission/batch&kind=mission",
    ).await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch["items"].as_array().unwrap().len(), 1);
    let (status, card) = get_request(app, "/v1/status?subject=mission/batch").await;
    assert_eq!(status, StatusCode::OK, "{card}");
    let item = &batch["items"][0];
    assert_eq!(item["subject"], "mission/batch");
    assert_eq!(item["desired_token"], card["subjects"][0]["desired_token"]);
    assert!(item["desired_token"].is_null());
    for field in ["revision", "state"] {
        assert_eq!(item["actual"][field], card["subjects"][0]["actual"][field], "{field}: {card}");
    }
    assert!(item["actual"]["digest"].as_str().unwrap().starts_with("sha256:"));
    assert!(item["actual"].get("body").is_none());
}

fn declare_page_agents(state: &AppState, count: usize) {
    let mut source = String::from("version 2\n");
    for index in 0..count {
        source.push_str(&format!(
            "agent \"page/{index:04}\" {{ command \"true\"; workspace \".\"; restart \"never\"; }}\n",
        ));
    }
    let intent = parse_intent(&source, state.store.origin()).unwrap();
    state.store.apply_internal(&intent, "desired-page-agents").unwrap();
}

#[tokio::test]
async fn desired_pages_reject_zero_and_over_maximum_limits() {
    let root = tempfile::tempdir().unwrap();
    let app = router(state(root.path()));
    for limit in [0, 1001] {
        let (status, error) = get_request(app.clone(), &format!("/v1/desired?limit={limit}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "desired-page-limit", "{error}");
    }
}

#[tokio::test]
async fn desired_default_and_maximum_pages_cover_more_than_one_thousand_subjects() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare_page_agents(&state, 1003);
    let index = state.store.index().unwrap();
    let app = router(state);
    for base in ["/v1/desired?kind=agent", "/v1/desired?kind=agent&limit=1000"] {
        let (status, first) = get_request(app.clone(), base).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["store_index"], index);
        let items = first["items"].as_array().unwrap();
        assert_eq!(items.len(), 1000);
        let cursor = first["next_cursor"].as_str().unwrap();
        let (status, last) = get_request(
            app.clone(), &format!("{base}&cursor={}", urlencoding::encode(cursor)),
        ).await;
        assert_eq!(status, StatusCode::OK, "{last}");
        assert_eq!(last["store_index"], index);
        assert!(last["next_cursor"].is_null());
        assert_eq!(last["items"].as_array().unwrap().len(), 3);
        let subjects = items.iter().chain(last["items"].as_array().unwrap())
            .map(|item| item["subject"].as_str().unwrap()).collect::<Vec<_>>();
        let expected = (0..1003).map(|index| format!("agent/page/{index:04}")).collect::<Vec<_>>();
        assert_eq!(subjects, expected);
    }
}

#[tokio::test]
async fn desired_named_pages_keep_the_same_selection_and_limit_until_complete() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare_page_agents(&state, 7);
    let index = state.store.index().unwrap();
    let app = router(state);
    let base = "/v1/desired?subject=agent/page/0006&subject=agent/page/0000&subject=agent/page/0004&subject=agent/page/0002&kind=agent&limit=2";
    let mut path = base.to_owned();
    let mut subjects = Vec::new();
    loop {
        let (status, page) = get_request(app.clone(), &path).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["store_index"], index);
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        subjects.extend(items.iter().map(|item| item["subject"].as_str().unwrap().to_owned()));
        let Some(cursor) = page["next_cursor"].as_str() else { break };
        assert!(subjects.len() < 4, "pagination must advance: {page}");
        path = format!("{base}&cursor={}", urlencoding::encode(cursor));
    }
    assert_eq!(subjects, [
        "agent/page/0000", "agent/page/0002", "agent/page/0004", "agent/page/0006",
    ]);
}

#[tokio::test]
async fn desired_cursor_rejects_changed_cut_instead_of_restarting() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare_page_agents(&state, 3);
    let app = router(state.clone());
    let (status, first) = get_request(app.clone(), "/v1/desired?kind=agent&limit=1").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let cursor = first["next_cursor"].as_str().unwrap();
    let intent = parse_intent(
        "version 2\nagent \"page/new\" { command \"true\"; workspace \".\"; restart \"never\"; }\n",
        state.store.origin(),
    ).unwrap();
    state.store.apply_internal(&intent, "desired-page-change").unwrap();
    assert!(state.store.index().unwrap() > first["store_index"].as_u64().unwrap());
    let (status, error) = get_request(
        app, &format!("/v1/desired?kind=agent&limit=1&cursor={}", urlencoding::encode(cursor)),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "desired-cursor-expired", "{error}");
    assert!(error.get("value").is_none(), "{error}");
}

async fn assert_desired_cursor_expires_after_backlog_projection(kind: &str) {
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let source = Store::open_memory("desired-cursor-source").unwrap();
    source.bind_fleet(FLEET).unwrap();
    state.store.bind_fleet(FLEET).unwrap();
    source.set_write_clock_at(1_700_000_000_000).unwrap();
    let declaration = |name: &str| match kind {
        "agent" => format!(
            "agent \"projection/{name}\" {{ command \"true\"; workspace \".\"; restart \"never\"; }}\n",
        ),
        "mission" => format!(
            "mission \"projection/{name}\" state=\"ready\" {{\n goal \"Cursor projection fence.\"\n step \"work\" {{ agentless }}\n}}\n",
        ),
        _ => unreachable!("only desired and mission projection families are exercised"),
    };
    let mut kdl = String::from("version 2\n");
    for name in ["0000", "0001", "0002"] {
        kdl.push_str(&declaration(name));
    }
    let intent = parse_intent(&kdl, source.origin()).unwrap();
    source.apply_internal(&intent, "desired-cursor-initial").unwrap();
    let exchange = source.export_replication_exchange(
        FLEET, &state.store.replication_inventory().unwrap(),
    ).unwrap();
    state.store.receive_replication_exchange(source.origin(), FLEET, &exchange).unwrap();
    let admission = state.store.validate_replication_backlog().unwrap();
    assert_eq!((admission.invalid, admission.unknown), (0, 0));
    assert!(state.store.project_replication_backlog().unwrap());
    let published = state.store.index().unwrap();

    source.set_write_clock_at(1_700_000_001_000).unwrap();
    let kdl = format!("version 2\n{}", declaration("0003"));
    let intent = parse_intent(&kdl, source.origin()).unwrap();
    source.apply_internal(&intent, "desired-cursor-backlog").unwrap();
    let exchange = source.export_replication_exchange(
        FLEET, &state.store.replication_inventory().unwrap(),
    ).unwrap();
    state.store.receive_replication_exchange(source.origin(), FLEET, &exchange).unwrap();
    let admission = state.store.validate_replication_backlog().unwrap();
    assert_eq!((admission.invalid, admission.unknown), (0, 0));
    let admitted = state.store.index().unwrap();
    assert!(admitted > published);
    let app = router(state.clone());
    let base = format!("/v1/desired?kind={kind}");
    let (status, before) = get_request(app.clone(), &base).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(before["items"].as_array().unwrap().len(), 3);
    let (status, first) = get_request(app.clone(), &format!("{base}&limit=1")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["store_index"], admitted);
    let cursor = first["next_cursor"].as_str().unwrap();

    // Exercise the real projection writer, not a synthetic health-row mutation.
    smallclaims::store::FULL_REPLAYS.with(|count| count.set(0));
    assert!(state.store.project_replication_backlog().unwrap());
    assert_eq!(smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get), 0,
        "the regression must exercise incremental projection, not replay");
    assert_eq!(state.store.index().unwrap(), admitted, "projection appends no claims");
    let (status, after) = get_request(app.clone(), &base).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["store_index"], admitted);
    assert_eq!(after["items"].as_array().unwrap().len(), 4,
        "projection must actually change the selected {kind} declarations");
    let (status, error) = get_request(
        app, &format!("{base}&limit=1&cursor={}", urlencoding::encode(cursor)),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "desired-cursor-expired", "{error}");
}

#[tokio::test]
async fn desired_cursor_expires_when_desired_backlog_projects_without_appending_claims() {
    assert_desired_cursor_expires_after_backlog_projection("agent").await;
}

#[tokio::test]
async fn desired_cursor_expires_when_mission_backlog_projects_without_appending_claims() {
    assert_desired_cursor_expires_after_backlog_projection("mission").await;
}

#[tokio::test]
async fn desired_cursor_rejects_changed_subjects_kind_limit_and_store_origin() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare_page_agents(&state, 3);
    let app = router(state);
    let base = "/v1/desired?subject=agent/page/0000&subject=agent/page/0001&kind=agent&limit=1";
    let (status, first) = get_request(app.clone(), base).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let cursor = urlencoding::encode(first["next_cursor"].as_str().unwrap());
    for base in [
        "/v1/desired?subject=agent/page/0000&subject=agent/page/0002&kind=agent&limit=1",
        "/v1/desired?subject=agent/page/0000&subject=agent/page/0001&kind=stop&limit=1",
        "/v1/desired?subject=agent/page/0000&subject=agent/page/0001&limit=1",
        "/v1/desired?subject=agent/page/0000&subject=agent/page/0001&kind=agent&limit=2",
    ] {
        let (status, error) = get_request(app.clone(), &format!("{base}&cursor={cursor}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "invalid-desired-cursor", "{error}");
    }
    let other_root = tempfile::tempdir().unwrap();
    let mut other = super::state(other_root.path());
    other.store = Arc::new(Store::open_memory("other-node").unwrap());
    declare_page_agents(&other, 3);
    let (status, error) = get_request(router(other), &format!("{base}&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "invalid-desired-cursor", "{error}");
}

async fn paired_desired_request(app: Router, path: &str, credential: &str) -> (StatusCode, Value) {
    let response = app.oneshot(Request::builder().uri(path)
        .header("authorization", format!("Bearer {credential}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn desired_pages_require_the_same_paired_device_scope_as_single_desired() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare_page_agents(&state, 1);
    for (credential, scopes) in [
        ("desired-no-projections", vec!["read.declarations"]),
        ("desired-projections", vec!["read.projections"]),
    ] {
        state.store.append_client_operation_claim(&ClaimInput {
            subject: format!("custom/client/{credential}"),
            kind: "custom.client.pairing-completed".into(),
            actor: Some("person/alex".into()),
            fields: BTreeMap::from([
                ("credential_hash".into(), json!(client_v0::credential_digest(credential))),
                ("session_actor".into(), json!(format!("client/{credential}"))),
                ("person_id".into(), json!("person/alex")),
                ("device_id".into(), json!("device/desired-test")),
                ("scopes".into(), json!(scopes)),
                ("expires_at_unix_ms".into(), json!(client_now_ms() as u64 + 60_000)),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    // Native routes authenticate paired Authorization headers too. The fabric gateway
    // separately forbids all raw routes, regardless of the credential's read scope.
    let app = router(state);
    let (single_status, single) = paired_desired_request(
        app.clone(), "/v1/desired/agent/page/0000", "desired-no-projections",
    ).await;
    let (page_status, page) = paired_desired_request(
        app.clone(), "/v1/desired?subject=agent/page/0000", "desired-no-projections",
    ).await;
    assert_eq!(single_status, StatusCode::FORBIDDEN, "{single}");
    assert_eq!(page_status, single_status, "{page}");
    for field in ["code", "message", "details"] {
        assert_eq!(page[field], single[field], "{field}: {page} vs {single}");
    }
    for path in ["/v1/desired/agent/page/0000", "/v1/desired?subject=agent/page/0000"] {
        let (status, response) = paired_desired_request(app.clone(), path, "desired-projections").await;
        assert_eq!(status, StatusCode::OK, "{response}");
    }
}

#[tokio::test]
async fn desired_pages_remain_readable_while_fleet_leave_refuses_writes() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare_page_agents(&state, 3);
    state.store.set_fleet_leaving(true).unwrap();
    let index = state.store.index().unwrap();
    let app = router(state.clone());
    let (status, error) = json_request(app.clone(), "/v1/intent/apply", json!({})).await;
    assert_eq!(error["code"], "fleet-leaving", "{status}: {error}");
    assert!(!status.is_success());
    let base = "/v1/desired?kind=agent&limit=2";
    let (status, first) = get_request(app.clone(), base).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    let cursor = first["next_cursor"].as_str().unwrap();
    let (status, last) = get_request(
        app, &format!("{base}&cursor={}", urlencoding::encode(cursor)),
    ).await;
    assert_eq!(status, StatusCode::OK, "{last}");
    assert_eq!(last["items"].as_array().unwrap().len(), 1);
    assert!(last["next_cursor"].is_null());
    assert_eq!(first["store_index"], index);
    assert_eq!(last["store_index"], index);
    assert_eq!(state.store.index().unwrap(), index);
}

#[tokio::test]
async fn desired_pages_merge_agent_and_mission_families_in_subject_order() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare_page_agents(&state, 2);
    let intent = parse_intent(
        "version 2\nmission \"page\" state=\"ready\" {\n goal \"Page mixed kinds.\"\n step \"work\" { agentless }\n}\n",
        state.store.origin(),
    ).unwrap();
    state.store.apply_internal(&intent, "desired-page-mission").unwrap();
    let app = router(state);
    let base = "/v1/desired?subject=agent/page/0001&subject=mission/page&subject=agent/page/0000&limit=1";
    let mut path = base.to_owned();
    let mut subjects = Vec::new();
    loop {
        let (status, page) = get_request(app.clone(), &path).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        subjects.push(page["items"][0]["subject"].as_str().unwrap().to_owned());
        let Some(cursor) = page["next_cursor"].as_str() else { break };
        assert!(subjects.len() < 3, "pagination must advance: {page}");
        path = format!("{base}&cursor={}", urlencoding::encode(cursor));
    }
    assert_eq!(subjects, ["agent/page/0000", "agent/page/0001", "mission/page"]);
}
