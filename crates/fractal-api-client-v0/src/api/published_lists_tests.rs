#![allow(unused_imports)]
use super::*;
use crate::model::{MissionRunRequest, WorkRequest};
use smallclaims::store::set_thread_clock;
use fractal_api_core::api::published_lists::tests_shared_client_v0_tests::{app_state, fold, mission, oracle, presented, publish, rows};


#[tokio::test]
async fn mission_http_not_ready_and_ended_keep_public_codes_and_history_and_detail_stay_direct() {
    let root = tempfile::tempdir().unwrap();
    let state = app_state(root.path());
    mission(&state.store, "garden/alpha");
    // The collection must be registered: holding an unregistered view panics in debug builds.
    state.store.hold_collection_view("missions");
    assert!(state.store.collection_view_published("missions"));
    let (status, error) = missions_http(&state, "/v1/client/missions").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{error}");
    assert_eq!(error["code"], "mission-list-not-ready", "{error}");
    assert_eq!(error["retryable"], true, "{error}");
    state.store.published_missions_list().start();
    let (status, error) = missions_http(&state, "/v1/client/missions").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{error}");
    assert_eq!(error["code"], "mission-list-not-ready", "{error}");
    for path in ["/v1/client/missions?history=true", "/v1/client/missions/garden/alpha"] {
        let (status, page) = missions_http(&state, path).await;
        assert_eq!(status, StatusCode::OK, "{page}");
    }
    let (publication, _) = fold(&state.store);
    let (status, first) = missions_http(&state, "/v1/client/missions?limit=1").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["value"]["items"][0], *publication.rows.cards[&publication.rows.order[0]]);
    state.store.published_missions_list().withdraw();
    let (_, error) = missions_http(&state, "/v1/client/missions").await;
    assert_eq!(error["code"], "mission-list-not-ready", "{error}");
    state.store.published_missions_list().end();
    let (status, error) = missions_http(&state, "/v1/client/missions").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{error}");
    assert_eq!(error["code"], "mission-list-ended", "{error}");
    assert_eq!(error["retryable"], false, "{error}");
}

pub async fn missions_http(state: &AppState, path: &str) -> (StatusCode, Value) {
    use tower::ServiceExt as _;
    let response = router(state.clone()).oneshot(
        axum::http::Request::builder().uri(path).body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
