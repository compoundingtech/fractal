use super::*;
use crate::config::{NetworkOperation, NetworkPolicy};

impl AppState {
    pub fn local_only(&self) -> bool {
        NetworkPolicy::for_state_dir(&self.state_dir).is_local_only()
    }
}

pub(super) fn refusal(operation: NetworkOperation) -> ApiError {
    let refused = NetworkPolicy::LocalOnly.require(operation).unwrap_err();
    ApiError {
        status: StatusCode::FORBIDDEN,
        code: refused.code.into(),
        message: refused.to_string(),
        details: Box::new(serde_json::Map::from_iter([
            ("network_policy".into(), json!(NetworkPolicy::LocalOnly)),
            ("operation".into(), json!(operation)),
        ])),
    }
}

fn network_operation(path: &str) -> Option<NetworkOperation> {
    if path.starts_with("/v1/internal/replication/") || path == "/v1/internal/replication-wake" {
        Some(NetworkOperation::Replication)
    } else if path.starts_with("/v1/internal/fleet/") {
        Some(NetworkOperation::FleetJoin)
    } else if path == crate::peer::CLIENT_READ_FORWARD_PATH
        || path == crate::peer::SEAT_FRESHNESS_INTERNAL_PATH
        || path == crate::peer::SEAT_FRESHNESS_READS_INTERNAL_PATH
    {
        Some(NetworkOperation::PeerRelay)
    } else {
        None
    }
}

pub(super) async fn enforce(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if state.local_only()
        && let Some(operation) = network_operation(request.uri().path())
    {
        return refusal(operation).into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn fixture_refuses_join_replication_and_peer_relay_before_body_or_store_effects() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests_shared::state(root.path());
        state.state_dir = root.path().join(".fractal-instance/client-control-fixture/state/fractal");
        assert!(state.local_only());
        let before = state.store.index().unwrap();
        let app = super::super::router(state.clone());
        for (path, code) in [
            ("/v1/internal/fleet/redeem", "local-only-fleet-join-refused"),
            ("/v1/internal/fleet/pairing-admission", "local-only-fleet-join-refused"),
            ("/v1/internal/replication/export", "local-only-replication-refused"),
            ("/v1/internal/replication/receive", "local-only-replication-refused"),
            ("/v1/internal/replication/shard/messages/receive", "local-only-replication-refused"),
            (crate::peer::CLIENT_READ_FORWARD_PATH, "local-only-peer-relay-refused"),
            (crate::peer::SEAT_FRESHNESS_INTERNAL_PATH, "local-only-peer-relay-refused"),
        ] {
            let response = app.clone().oneshot(Request::builder().method("POST").uri(path)
                .body(Body::from("not even valid JSON")).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["code"], code, "{path}: {value}");
            assert_eq!(value["details"]["network_policy"], "local-only");
        }
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn health_proves_fixture_network_policy_and_nonfixture_stays_fleet_capable() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests_shared::state(root.path());
        let ordinary = super::super::health(State(state.clone())).await.unwrap().0;
        assert_eq!(ordinary["network_policy"]["mode"], "fleet-capable");
        state.state_dir = root.path().join(".st-instance/client-control-fixture/state/fractal");
        let health = super::super::health(State(state.clone())).await.unwrap().0;
        assert_eq!(health["network_policy"], json!({
            "mode":"local-only", "fleet_join":false, "replication":false, "peer_relay":false,
        }));
        let error = super::super::remote_unavailable_for_owner(&state, "host/personal");
        assert_eq!(error.code, "local-only-peer-relay-refused");
    }
}
