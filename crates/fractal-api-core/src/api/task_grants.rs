//! Q336 P1/P2 routes. Mutations stay dark until P3 binds op-proxy to kernel identity.
use super::*;
use fractal_schema::task_grants::{CreateTaskGrantRequest, RevokeTaskGrant, TaskGrantReceipt, TaskGrantRecord};

pub struct TaskGrantCaller {
    pub principal: String,
}

/// Trusted transport integration, never a request header or manifest body field.
/// P3 supplies the kernel process/service-principal binding. Production denies all for now.
pub trait TaskGrantCallerAuthority: Send + Sync {
    fn caller(&self) -> Result<TaskGrantCaller, ApiError>;
}

pub struct UnavailableCallerAuthority;
impl TaskGrantCallerAuthority for UnavailableCallerAuthority {
    fn caller(&self) -> Result<TaskGrantCaller, ApiError> {
        Err(forbidden("caller-binding-unavailable"))
    }
}

fn forbidden(reason: &str) -> ApiError {
    ApiError {
        status: StatusCode::FORBIDDEN,
        code: "forbidden".into(),
        message: reason.into(),
        details: Box::new(serde_json::Map::from_iter([("reason".into(), json!(reason))])),
    }
}

pub fn authorize_task_grant_caller(caller: &TaskGrantCaller) -> Result<(), ApiError> {
    if caller.principal == "service/op-proxy" {
        Ok(())
    } else {
        Err(forbidden("caller-not-op-proxy"))
    }
}

pub async fn create(
    State(state): State<AppState>,
    Extension(authority): Extension<Arc<dyn TaskGrantCallerAuthority>>,
    Json(input): Json<CreateTaskGrantRequest>,
) -> Result<Json<TaskGrantReceipt>, ApiError> {
    authorize_task_grant_caller(&authority.caller()?)?;
    let store = state.store.clone();
    let receipt = blocking_action(move || store.create_task_grant(&input)).await?;
    signal_visible_change(&state);
    Ok(Json(receipt))
}

pub async fn get(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<TaskGrantRecord>, ApiError> {
    let record = blocking_action(move || state.store.task_grant(&id)).await?
        .ok_or_else(|| ApiError::not_found("task grant not found"))?;
    Ok(Json(record))
}

pub async fn revoke(
    State(state): State<AppState>,
    Extension(authority): Extension<Arc<dyn TaskGrantCallerAuthority>>,
    AxumPath(id): AxumPath<String>,
    Json(input): Json<RevokeTaskGrant>,
) -> Result<Json<TaskGrantRecord>, ApiError> {
    authorize_task_grant_caller(&authority.caller()?)?;
    let store = state.store.clone();
    let record = blocking_action(move || store.revoke_task_grant(&id, &input.reason)).await?
        .ok_or_else(|| ApiError::not_found("task grant not found"))?;
    signal_visible_change(&state);
    Ok(Json(record))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    struct Caller(&'static str);
    impl TaskGrantCallerAuthority for Caller {
        fn caller(&self) -> Result<TaskGrantCaller, ApiError> {
            Ok(TaskGrantCaller { principal: self.0.into() })
        }
    }

    fn app(state: AppState, principal: &'static str) -> Router {
        super::super::router_for_transport_with_task_grant_authority(
            state, ClientTransportBoundary::Unix, &[], Arc::new(Caller(principal)),
        )
    }

    fn input() -> CreateTaskGrantRequest {
        let manifest = serde_json::from_value(json!({
            "version":1,"title":"Example approval",
            "requester":{"principal":"agent/example-requester","run":"run/example","step":"approve","incarnation":"1"},
            "window":{"notBefore":"2026-10-20T00:00:00Z","notAfter":"2026-10-20T01:00:00Z"},
            "provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},
            "browserSteps":[],
            "writes":[{"vaultId":"example","itemId":"example","fields":[{"inventoryId":"example-secret","kind":"secret"}]}],
            "readers":[{"principal":"agent/example-reader","peerHost":"example","inventoryIds":["example-secret"],"maxUses":1}]
        })).unwrap();
        let mut input = CreateTaskGrantRequest { manifest, op_proxy_case_id: "example-case".into(), digest: String::new() };
        input.digest = input.manifest.digest().unwrap();
        input
    }

    async fn request(app: Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = app.oneshot(Request::builder().method(method).uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        (status, value.get("value").cloned().unwrap_or(value))
    }

    #[tokio::test]
    async fn task_grant_routes_round_trip_and_idempotent_revoke() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let (status, receipt) = request(app(state.clone(), "service/op-proxy"), "POST", "/v1/task-grants", json!(input())).await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        let path = format!("/v1/task-grants/{}", receipt["grantId"].as_str().unwrap().replace('/', "%2F"));
        let (status, record) = request(router(state.clone()), "GET", &path, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{record}");
        assert_eq!(record["digest"], receipt["digest"]);
        let path = format!("{path}/revoke");
        let (status, first) = request(app(state.clone(), "service/op-proxy"), "POST", &path, json!({"reason":"example completed"})).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["state"], "revoked");
        let (status, second) = request(app(state, "service/op-proxy"), "POST", &path, json!({"reason":"again"})).await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn task_grant_routes_reject_invalid_manifest_and_digest() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let mut mismatch = input();
        mismatch.digest = "0".repeat(64);
        let mut long_window = input();
        long_window.manifest.window.not_after = "2026-10-22T00:00:00Z".into();
        let mut undeclared = input();
        undeclared.manifest.readers[0].inventory_ids = vec!["undeclared".into()];
        for input in [mismatch, long_window, undeclared] {
            let (status, body) = request(app(state.clone(), "service/op-proxy"), "POST", "/v1/task-grants", json!(input)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        }
    }

    #[tokio::test]
    async fn task_grant_routes_deny_unbound_and_other_callers() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        for app in [router(state.clone()), app(state.clone(), "agent/example-caller")] {
            for (path, body) in [("/v1/task-grants", json!(input())), ("/v1/task-grants/example/revoke", json!({"reason":"example"}))] {
                let (status, body) = request(app.clone(), "POST", path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            }
        }
    }
}
