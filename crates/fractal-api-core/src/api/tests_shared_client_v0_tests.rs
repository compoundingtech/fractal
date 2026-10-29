#![allow(unused_imports)]
use crate::RolloutsStore;
use crate::api::*;
use crate::model::SESSION_CLAIMS;
use axum::body::to_bytes;
use axum::http::Request;
use std::path::PathBuf;
pub use crate::api::tests_shared::state;

pub async fn json_request(app: Router, path: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    if path.starts_with("/v1/client/") {
        assert_eq!(envelope["api_version"], "st3.client.v0");
        assert_eq!(envelope["snapshot"]["host_id"], "host/node");
        assert!(envelope["request_id"].as_str().unwrap().contains('-'));
        assert!(envelope["snapshot"]["store_index"].is_u64());
    } else {
        assert_eq!(envelope["api_version"], "st3.v1");
        assert_eq!(envelope["snapshot_host"], "node");
        assert!(envelope["request_id"].as_str().unwrap().contains('-'));
        assert!(envelope["store_index"].is_u64());
    }
    let value = if status.is_success() {
        envelope["value"].clone()
    } else {
        envelope
    };
    (status, value)
}

pub async fn get_request(app: Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    if path.starts_with("/v1/client/") {
        assert_eq!(envelope["api_version"], "st3.client.v0");
        assert!(envelope["request_id"].as_str().unwrap().contains('-'));
        if status.is_success() {
            assert_eq!(envelope["snapshot"]["host_id"], "host/node");
            assert!(envelope["snapshot"]["store_index"].is_u64());
        } else {
            assert_eq!(envelope["error_version"], "st3.client.error.v0");
            assert!(envelope["code"].as_str().is_some());
        }
    } else {
        assert_eq!(envelope["api_version"], "st3.v1");
        assert_eq!(envelope["snapshot_host"], "node");
        assert!(envelope["request_id"].as_str().unwrap().contains('-'));
        assert!(envelope["store_index"].is_u64());
    }
    let value = if status.is_success() {
        envelope["value"].clone()
    } else {
        envelope
    };
    (status, value)
}

pub fn materialize_run_agents(state: &AppState, run: &MissionRunView) {
    let mission = state
        .store
        .mission_spec(
            run.mission.strip_prefix("mission/").unwrap_or(&run.mission),
            Some(&run.revision),
        )
        .unwrap()
        .unwrap();
    let Some(source) = mission.declarations_kdl.as_deref() else {
        return;
    };
    let intent = crate::graph::parse_execution_intent(source, &state.node, &run.id).unwrap();
    state
        .store
        .apply_internal(&intent, &format!("test-materialize:{}", run.id))
        .unwrap();
}

pub fn checked_agent_cache(store: &Store, history: bool, index: u64) -> Vec<Value> {
    // Both sides build as the roster refresher does, on a pinned snapshot. Unpinned and outside a
    // request, a card-fault read has a 25 ms wall bound that a loaded host can spend waiting.
    store.read_snapshot(|_| {
        let cached = client_agent_resources_cached(store, history, index).unwrap();
        let mut direct = client_agent_resources_uncached(store, history, index).unwrap();
        add_agent_todos(store, &mut direct, index).unwrap();
        assert_eq!(cached, direct);
        Ok(cached)
    }).unwrap()
}
