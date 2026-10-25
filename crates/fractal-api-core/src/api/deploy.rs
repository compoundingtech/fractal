use super::*;
use fractal_schema::deploy::{VerifyGoReceipt, VerifyGoRequest};
use fractal_schema::deploy_policy::Policy;

pub async fn verify_go(
    State(state): State<AppState>,
    Extension(transport): Extension<ClientTransportBoundary>,
    Json(request): Json<VerifyGoRequest>,
) -> Result<Json<VerifyGoReceipt>, ApiError> {
    if !matches!(transport, ClientTransportBoundary::Unix) {
        return Err(ApiError::bad(St3Error::new(
            "deploy-local-only",
            "deploy verification requires the local Unix endpoint",
        )));
    }
    let store = state.store.clone();
    blocking_action(move || {
        let policy = Policy::load_packaged().map_err(|error| {
            St3Error::new("deploy-policy-unavailable", error.to_string())
        })?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| St3Error::new("deploy-clock-invalid", error.to_string()))?
            .as_millis();
        store.verify_deploy_go(&policy, &request, now)
    })
    .await
    .map(Json)
}
