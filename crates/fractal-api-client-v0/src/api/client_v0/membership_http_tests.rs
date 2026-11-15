const MEMBERSHIP_PATH: &str = "/v1/client/arrangements/ada/019a0000-0000-7000-8000-000000000001/memberships?person=person%2Fada&limit=1";

async fn membership_json(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let response = crate::api::router(state.clone()).oneshot(Request::builder()
        .uri(uri).header(LOCAL_PERSON_HEADER, "person/ada")
        .body(Body::empty()).unwrap()).await.unwrap();
    (response.status(), serde_json::from_slice(
        &to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap()).unwrap())
}

async fn two_members(state: &AppState) {
    let session = ClientSession::local(Some("person/ada")).unwrap();
    create_v2(state, &session).await;
    for member in ["resource/a", "resource/b"] { declare_resource_member(state, member); }
    edit_memberships(state, &session, &membership_request(state, "http-rows", json!([
        {"op":"place","member":"resource/a","bucket":null,"key":"a0"},
        {"op":"place","member":"resource/b","bucket":null,"key":"a0"}
    ]))).await.unwrap();
}

#[tokio::test]
async fn collection_http_empty_terminal_and_one_unrelated_commit() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "http-collection-progress");
    let session = ClientSession::local(Some("person/ada")).unwrap();
    create_v2(&state, &session).await;
    let path = format!("{MEMBERSHIP_PATH}&paging_contract=collection-v1");
    let (status, empty) = membership_json(&state, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["value"]["membership"]["live_count"], 0);
    assert_eq!(empty["value"]["items"], json!([]));
    assert_eq!(empty["value"]["page"]["has_more"], false);
    for member in ["resource/a", "resource/b"] { declare_resource_member(&state, member); }
    edit_memberships(&state, &session, &membership_request(&state, "progress-rows", json!([
        {"op":"place","member":"resource/a","bucket":null,"key":"a0"},
        {"op":"place","member":"resource/b","bucket":null,"key":"a0"}
    ]))).await.unwrap();
    let (status, first) = membership_json(&state, &path).await;
    assert_eq!(status, StatusCode::OK);
    let before = state.store.index().unwrap();
    state.store.append_claim(&ClaimInput {
        subject:"custom/http-unrelated".into(), kind:"custom.http.unrelated".into(),
        actor:None, fields:BTreeMap::new(), evidence:vec![],
        expected_subject:None, idempotency_key:None,
    }).unwrap();
    assert_eq!(state.store.index().unwrap(), before + 1);
    let cursor = first["value"]["page"]["next_cursor"].as_str().unwrap();
    let (status, last) = membership_json(&state, &format!("{path}&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(first["snapshot"]["store_index"], last["snapshot"]["store_index"]);
    assert_eq!(first["value"]["membership"], last["value"]["membership"]);
    assert_eq!(last["value"]["paging_contract"], "collection-v1");
    assert_eq!(last["value"]["membership"]["live_count"], 2);
    assert_eq!(first["value"]["items"][0]["member"], "resource/a");
    assert_eq!(last["value"]["items"][0]["member"], "resource/b");
    assert_eq!(last["value"]["page"]["has_more"], false);
    assert!(last["value"]["page"]["next_cursor"].is_null());
    assert!(first["value"]["page"]["cursor_expires_at"].is_null());
}

#[tokio::test]
async fn collection_http_errors_and_legacy_wire_shape() {
    use fractal_api_core::api::client_v0::membership_paging as paging;
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "http-collection-errors");
    two_members(&state).await;
    let path = format!("{MEMBERSHIP_PATH}&paging_contract=collection-v1");
    let (_, first) = membership_json(&state, &path).await;
    let token = first["value"]["page"]["next_cursor"].as_str().unwrap();
    let (_, legacy) = membership_json(&state, MEMBERSHIP_PATH).await;
    let legacy_token = legacy["value"]["page"]["next_cursor"].as_str().unwrap();
    let legacy_value: ClientResourcePage = serde_json::from_value(legacy["value"].clone()).unwrap();
    assert_eq!(serde_json::to_vec(&legacy["value"]).unwrap(),
        serde_json::to_vec(&serde_json::to_value(legacy_value).unwrap()).unwrap());
    assert!(legacy["value"].get("membership").is_none());
    assert!(legacy["value"].get("paging_contract").is_none());
    assert!(legacy_token.starts_with("membership/"));
    assert!(legacy["value"]["page"]["cursor_expires_at"].is_string());
    let mut cases = vec![
        (format!("{path}&cursor=!"), StatusCode::UNPROCESSABLE_ENTITY, paging::INVALID),
        (format!("{path}&cursor={legacy_token}"), StatusCode::UNPROCESSABLE_ENTITY, paging::INVALID),
        (format!("{MEMBERSHIP_PATH}&cursor={token}"), StatusCode::UNPROCESSABLE_ENTITY, paging::INVALID),
        (format!("{}&cursor={token}", path.replace("limit=1", "limit=2")), StatusCode::UNPROCESSABLE_ENTITY, paging::INVALID),
        (format!("{path}&cursor={token}&status=active"), StatusCode::UNPROCESSABLE_ENTITY, paging::INVALID),
        (path.replace("collection-v1", "unknown"), StatusCode::UNPROCESSABLE_ENTITY, paging::UNSUPPORTED),
        ("/v1/client/agents?paging_contract=collection-v1".into(), StatusCode::UNPROCESSABLE_ENTITY, paging::UNSUPPORTED),
        (format!("{path}&cursor={}", "a".repeat(16_385)), StatusCode::UNPROCESSABLE_ENTITY, paging::INVALID),
    ];
    let cursor = paging::Cursor::decode(token, "person/ada", SUBJECT, 1).unwrap();
    for (field, code, status) in [
        ("host_id", paging::RESET, StatusCode::CONFLICT),
        ("projection_version", paging::RESET, StatusCode::CONFLICT),
        ("epoch", paging::RESET, StatusCode::CONFLICT),
        ("live_count", paging::INCONSISTENT, StatusCode::INTERNAL_SERVER_ERROR),
        ("changed_index", paging::INCONSISTENT, StatusCode::INTERNAL_SERVER_ERROR),
        ("version", paging::INVALID, StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let mut changed = serde_json::to_value(&cursor).unwrap();
        changed[field] = match field {
            "live_count" | "changed_index" => json!(u64::MAX),
            "version" => json!(2),
            _ => json!("different-scope"),
        };
        let token = format!("membership-v1/{}", base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&changed).unwrap()));
        cases.push((format!("{path}&cursor={token}"), status, code));
    }
    for (uri, status, code) in cases {
        let (actual, body) = membership_json(&state, &uri).await;
        assert_eq!(actual, status, "{uri}");
        assert_eq!(body["code"], code);
        assert!(body.get("value").is_none());
        assert!(body.get("membership").is_none());
    }
    let session = ClientSession::local(Some("person/ada")).unwrap();
    edit_memberships(&state, &session, &membership_request(&state, "http-reorder", json!([
        {"op":"place","member":"resource/b","bucket":null,"key":"a1"}
    ]))).await.unwrap();
    let (status, body) = membership_json(&state, &format!("{path}&cursor={token}")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], paging::CHANGED);
}

#[tokio::test]
async fn collection_http_cursor_survives_more_than_five_minutes() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "http-no-cursor-expiry");
    two_members(&state).await;
    let path = format!("{MEMBERSHIP_PATH}&paging_contract=collection-v1");
    let (status, first) = membership_json(&state, &path).await;
    assert_eq!(status, StatusCode::OK);
    let token = first["value"]["page"]["next_cursor"].as_str().unwrap();
    assert!(first["value"]["page"]["cursor_expires_at"].is_null());
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.strip_prefix("membership-v1/").unwrap()).unwrap();
    let payload: Value = serde_json::from_slice(&payload).unwrap();
    assert!(payload.get("expires").is_none());
    let (status, legacy) = membership_json(&state, MEMBERSHIP_PATH).await;
    assert_eq!(status, StatusCode::OK);
    let legacy_token = legacy["value"]["page"]["next_cursor"].as_str().unwrap();
    // Process-wide clock: run with nextest's per-test process isolation, not cargo test.
    // The guard also restores the clock if a request panics before normal cleanup.
    struct RestoreClientClock;
    impl Drop for RestoreClientClock {
        fn drop(&mut self) {
            fractal_api_core::test_support::skew_client_clock(0);
        }
    }
    let restore_clock = RestoreClientClock;
    let before = client_now_ms();
    fractal_api_core::test_support::skew_client_clock(CLIENT_PAGE_TTL_MS as u64 + 1_000);
    let elapsed = client_now_ms().saturating_sub(before);
    let (status, last) = membership_json(&state, &format!("{path}&cursor={token}")).await;
    let (legacy_status, legacy_error) = membership_json(&state,
        &format!("{MEMBERSHIP_PATH}&cursor={legacy_token}")).await;
    drop(restore_clock);
    assert!(elapsed > CLIENT_PAGE_TTL_MS);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["value"]["membership"], last["value"]["membership"]);
    assert_eq!(last["value"]["items"][0]["member"], "resource/b");
    assert_eq!(legacy_status, StatusCode::GONE);
    assert_eq!(legacy_error["code"], "page-cursor-expired");
}

#[tokio::test]
async fn collection_http_revoked_authorization_precedes_cursor_metadata() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "http-collection-revoked");
    two_members(&state).await;
    let append = |kind: &str, fields: Value| state.store.append_client_operation_claim(&ClaimInput {
        subject:"custom/client/pairing-http-membership".into(), kind:kind.into(),
        actor:Some("person/ada".into()), fields:serde_json::from_value(fields).unwrap(),
        evidence:vec![], expected_subject:None, idempotency_key:None,
    }).unwrap();
    append("custom.client.pairing-completed", json!({
        "session_actor":"client/http-membership","person_id":"person/ada",
        "credential_hash":credential_digest("http-membership-test-credential"),
        "expires_at_unix_ms":u64::MAX,"scopes":["read.projections","read.arrangements"]
    }));
    let get = |uri: String, credential: &str| {
        let app = crate::api::router(state.clone());
        let request = Request::builder().uri(uri)
            .header("authorization", format!("Bearer {credential}")).body(Body::empty()).unwrap();
        async move {
            let response = app.oneshot(request).await.unwrap();
            (response.status(), serde_json::from_slice::<Value>(
                &to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap()).unwrap())
        }
    };
    let path = format!("{MEMBERSHIP_PATH}&paging_contract=collection-v1");
    let (status, first) = get(path.clone(), "http-membership-test-credential").await;
    assert_eq!(status, StatusCode::OK);
    let token = first["value"]["page"]["next_cursor"].as_str().unwrap();
    append("custom.client.pairing-revoked", json!({"reason":"revoked between pages"}));
    for uri in [format!("{path}&cursor={token}"), format!("{path}&cursor=!")] {
        let (status, body) = get(uri, "http-membership-test-credential").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "forbidden");
        assert!(body.get("value").is_none());
        assert!(body.get("membership").is_none());
    }
    let (status, body) = membership_json(&state, &format!(
        "{}&cursor=!", path.replace("person%2Fada", "person%2Fother"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "forbidden");
}

#[tokio::test]
async fn collection_http_maximal_rows_and_byte_budget_packer() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "http-collection-byte-budget");
    let session = ClientSession::local(Some("person/ada")).unwrap();
    create_v2(&state, &session).await;
    let key = format!("a0{}", "1".repeat(fractal_schema::arrangements::MAX_KEY_BYTES - 2));
    let members: Vec<_> = (0..201).map(|index| format!("resource/{}-{index:03}", "a".repeat(499))).collect();
    let mut operations = Vec::new();
    for member in &members {
        assert_eq!(member.len(), 512);
        declare_resource_member(&state, member);
        operations.push(json!({"op":"place","member":member,"bucket":null,"key":key}));
    }
    edit_memberships(&state, &session, &membership_request(&state, "http-maximal", json!(operations))).await.unwrap();
    let path = format!("{}&paging_contract=collection-v1", MEMBERSHIP_PATH.replace("limit=1", "limit=200"));
    let (status, first) = membership_json(&state, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["value"]["items"].as_array().unwrap().len(), 200);
    let token = first["value"]["page"]["next_cursor"].as_str().unwrap();
    let (status, last) = membership_json(&state, &format!("{path}&cursor={token}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["value"]["membership"], last["value"]["membership"]);
    assert_eq!(last["value"]["membership"]["live_count"], 201);
    assert_eq!(last["value"]["items"][0]["member"], members[200]);
    assert_eq!(last["value"]["page"]["has_more"], false);
    // Legal rows at the current count/key/member caps cannot fill the byte budget.
    // Exercise the same assembler with synthetic oversized rows, not invalid store claims.
    use fractal_api_core::api::client_v0::membership_paging as paging;
    let snapshot = new_client_snapshot(&state);
    let mut row = first["value"]["items"][0].clone();
    row["padding"] = json!("x".repeat((CLIENT_MAX_RESPONSE_BYTES - 128_000) / 2));
    let captured = fractal_store::store::ordered_membership::MembershipPage {
        store_index:snapshot.store_index, generation:0,
        state:fractal_schema::ordered_membership::State { changed_index:0, live_count:2 },
        items:vec![row.clone(), row],
    };
    let page = paging::assemble(captured, &snapshot, state.store.ordered_membership_lifetime(),
        "person/ada", SUBJECT, 2, None).unwrap();
    assert_eq!(page.value.items.len(), 1);
    assert!(page.value.page.has_more);
    let cursor = paging::Cursor::decode(page.value.page.next_cursor.as_deref().unwrap(),
        "person/ada", SUBJECT, 2).unwrap();
    assert_eq!(cursor.after.member, members[0]);
    assert_eq!(page.membership.unwrap().live_count, 2);
}

#[tokio::test]
async fn collection_http_cursor_rejects_changed_container_and_invalid_tuple() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "http-collection-cursor-scope");
    two_members(&state).await;
    let path = format!("{MEMBERSHIP_PATH}&paging_contract=collection-v1");
    let (_, first) = membership_json(&state, &path).await;
    let token = first["value"]["page"]["next_cursor"].as_str().unwrap();
    let changed_path = path.replace("8000-000000000001", "8000-000000000002");
    let (status, body) = membership_json(&state, &format!("{changed_path}&cursor={token}")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "invalid-membership-cursor");
    use fractal_api_core::api::client_v0::membership_paging as paging;
    let cursor = paging::Cursor::decode(token, "person/ada", SUBJECT, 1).unwrap();
    let mut cursor = serde_json::to_value(cursor).unwrap();
    cursor["after"]["key"] = json!("");
    let token = format!("membership-v1/{}", base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&cursor).unwrap()));
    let (status, body) = membership_json(&state, &format!("{path}&cursor={token}")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "invalid-membership-cursor");
}
