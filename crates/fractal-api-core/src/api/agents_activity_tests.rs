use super::*;
use crate::api::tests_shared_client_v0_tests::{get_request, state};

const ROUTE: &str = "/v1/internal/agents/activity";

fn declare(state: &AppState, name: &str) {
    let intent = parse_intent(&format!(
        "version 2\nagent \"{name}\" {{ command \"true\"; workspace \"/declared/workspace\"; restart \"never\"; }}\n"
    ), state.store.origin()).unwrap();
    state.store.apply_internal(&intent, &format!("declare-{name}")).unwrap();
}

fn observe(store: &Store, name: &str, fields: Value) -> String {
    store.append_claim(&ClaimInput {
        subject: format!("agent/{name}"), kind: "runtime.observed".into(), actor: None,
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap().id
}

fn running() -> Value {
    json!({"status":"running", "workspace":"/actual/../workspace", "incarnation_id":"one"})
}

async fn activity(state: &AppState, suffix: &str) -> Value {
    let (status, body) = get_request(router(state.clone()), &format!("{ROUTE}{suffix}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn actual_workspace_and_selected_origin_override_declaration() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "actual");
    state.store.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let worker = Store::open_memory("worker").unwrap();
    worker.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let claim = observe(&worker, "actual", running());
    crate::test_support::replicate(&worker, &state.store);
    let body = activity(&state, "").await;
    assert_eq!(body["agents"], json!([{
        "id":"agent/actual", "status":"running", "host":"host/worker",
        "workspace":"/actual/../workspace", "incarnation_id":"one",
        "actual_claim":claim, "actual_origin":"worker"
    }]));
    assert_eq!(body["snapshot"]["store_index"], state.store.index().unwrap());
    assert_eq!(body["snapshot"]["host_id"], "host/node");
    assert_eq!(body["snapshot"]["local_host_id"], "host/node");
    assert_eq!(body["incomplete"], json!([]));
    assert_eq!(body["complete"], true);
    assert!(body["next_cursor"].is_null());
    println!("ACTIVITY_SAMPLE={body}");
}

#[tokio::test]
async fn declaration_without_actual_is_incomplete() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "missing");
    let body = activity(&state, "").await;
    assert_eq!(body["agents"], json!([]));
    assert_eq!(body["incomplete"], json!([{"id":"agent/missing", "reason":"no-actual"}]));
    assert_eq!(body["complete"], false);
}

#[tokio::test]
async fn min_cut_above_head_is_retryable_cut_not_ready() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let wanted = state.store.index().unwrap() + 1;
    let (status, body) = get_request(router(state), &format!("{ROUTE}?min_cut={wanted}")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "agent-roster-cut-not-ready");
    assert_eq!(body["details"]["min_cut"], wanted);
}

#[tokio::test]
async fn host_filter_uses_actual_origin_and_scopes_completeness() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "local");
    declare(&state, "unobserved");
    // A host field inside the observation is not its selected origin authority.
    let mut fields = running();
    fields["host"] = json!("other");
    observe(&state.store, "local", fields);
    let body = activity(&state, "?host=host%2Fnode").await;
    assert_eq!(body["agents"].as_array().unwrap().len(), 1);
    assert_eq!(body["agents"][0]["id"], "agent/local");
    assert_eq!(body["agents"][0]["host"], "host/node");
    assert_eq!(body["incomplete"], json!([]));
    assert_eq!(body["complete"], true);
    let empty = activity(&state, "?host=host%2Fother").await;
    assert_eq!(empty["agents"], json!([]));
    assert_eq!(empty["complete"], true);
}

#[tokio::test]
async fn pagination_keeps_one_cut_and_completes_only_on_last_page() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    for name in ["a", "b", "c"] {
        declare(&state, name);
        observe(&state.store, name, running());
    }
    let first = activity(&state, "?limit=1").await;
    assert_eq!(first["complete"], false);
    assert_eq!(first["agents"][0]["id"], "agent/a");
    let mut moved = running();
    moved["workspace"] = json!("/new/workspace");
    observe(&state.store, "b", moved);
    declare(&state, "later");
    let mut page = first.clone();
    for (name, complete) in [("b", false), ("c", true)] {
        let cursor = page["next_cursor"].as_str().unwrap();
        page = activity(&state, &format!("?cursor={}", urlencoding::encode(cursor))).await;
        assert_eq!(page["snapshot"], first["snapshot"]);
        assert_eq!(page["agents"][0]["id"], format!("agent/{name}"));
        assert_eq!(page["agents"][0]["workspace"], "/actual/../workspace");
        assert_eq!(page["complete"], complete);
    }
    assert!(page["next_cursor"].is_null());
}

#[tokio::test]
async fn pagination_remembers_incomplete_rows_on_earlier_pages() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "a");
    declare(&state, "b");
    observe(&state.store, "b", running());
    let first = activity(&state, "?limit=1").await;
    assert_eq!(first["incomplete"][0]["reason"], "no-actual");
    let cursor = first["next_cursor"].as_str().unwrap();
    let last = activity(&state, &format!("?cursor={}", urlencoding::encode(cursor))).await;
    assert_eq!(last["incomplete"], json!([]));
    assert_eq!(last["agents"].as_array().unwrap().len(), 1);
    assert!(last["next_cursor"].is_null());
    assert_eq!(last["complete"], false);
    let (status, _) = get_request(router(state), &format!(
        "{ROUTE}?host=host%2Fnode&cursor={}", urlencoding::encode(cursor)
    )).await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test]
async fn unknown_status_is_explicit_and_retains_raw_value() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "future");
    let mut fields = running();
    fields["status"] = json!("future-status");
    observe(&state.store, "future", fields);
    let body = activity(&state, "").await;
    assert_eq!(body["agents"][0]["status"], "unknown");
    assert_eq!(body["agents"][0]["status_raw"], "future-status");
    assert_eq!(body["complete"], true);
}

#[tokio::test]
async fn missing_workspace_never_uses_declared_workspace() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "workspace");
    observe(&state.store, "workspace", json!({"status":"running", "incarnation_id":"one"}));
    let body = activity(&state, "").await;
    assert_eq!(body["agents"], json!([]));
    assert_eq!(body["incomplete"][0]["reason"], "missing-workspace");
    assert_eq!(body["complete"], false);
}

#[tokio::test]
async fn expired_cursor_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    for name in ["a", "b"] {
        declare(&state, name);
        observe(&state.store, name, running());
    }
    let first = activity(&state, "?limit=1").await;
    let mut cursor = decode_client_cursor(first["next_cursor"].as_str().unwrap()).unwrap();
    cursor.expires_at_unix_ms = 0;
    let cursor = encode_client_cursor(&cursor).unwrap();
    let (status, body) = get_request(router(state), &format!(
        "{ROUTE}?cursor={}", urlencoding::encode(&cursor)
    )).await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    assert_eq!(body["code"], "page-cursor-expired");
}

#[tokio::test]
async fn concurrent_actual_authorities_remain_incomplete() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "concurrent");
    state.store.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    for source in [&left, &right] {
        source.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
        observe(source, "concurrent", running());
        crate::test_support::replicate(source, &state.store);
    }
    let body = activity(&state, "").await;
    assert_eq!(body["agents"], json!([]));
    assert_eq!(body["incomplete"], json!([{
        "id":"agent/concurrent", "reason":"indeterminate-authority"
    }]));
    assert_eq!(body["complete"], false);
}

#[tokio::test]
async fn invalid_page_limits_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    for limit in [0, 201] {
        let (status, _) = get_request(router(state.clone()), &format!("{ROUTE}?limit={limit}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
