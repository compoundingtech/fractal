//! The Unix-only authority bridge. The parent supplies the selected journal/host-key/session
//! service; this API never opens a second authority store or exports the bearer credential.
use super::{ApiError, BoundRun, RunCredential};
use crate::model::{ClaimInput, FractalError};
use crate::store::run_credentials::{ExecutionBinding, assertions::{RequestAssertion, RequestIdentity}};
use axum::{Extension, Json, http::Method};
use serde::Deserialize;
use serde_json::Value;
use std::{pin::Pin, sync::Arc};

pub const CLAIM_PATH: &str = "/v1/internal/native/run-request/claim";

pub trait LocalRunAuthority: Send + Sync {
    fn resolve(&self, secret: &str) -> Result<ExecutionBinding, FractalError>;
    fn sign(&self, secret: &str, request: RequestIdentity, lifetime_ms: u64) -> Result<RequestAssertion, FractalError>;
    fn execute_claim<'a>(
        &'a self, secret: &'a str, request_id: &'a str, claim: ClaimInput,
    ) -> Pin<Box<dyn Future<Output = Result<Value, FractalError>> + Send + 'a>>;
}

pub(super) fn allows_bound_request(method: &Method, path: &str) -> bool {
    *method == Method::POST && path == CLAIM_PATH
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimRequest {
    pub request_id: String,
    pub claim: ClaimInput,
}

pub(super) async fn claim(
    service: Option<Extension<Arc<dyn LocalRunAuthority>>>,
    credential: Option<Extension<RunCredential>>,
    bound: Option<Extension<BoundRun>>,
    Json(request): Json<ClaimRequest>,
) -> Result<Json<Value>, ApiError> {
    let refused = || ApiError::bad(FractalError::new(
        "run-request-refused", "the native run route requires a locally admitted program credential",
    ));
    let Some(Extension(bound)) = bound else { return Err(refused()); };
    let Some(Extension(credential)) = credential else { return Err(refused()); };
    let Some(Extension(service)) = service else { return Err(refused()); };
    if request.request_id.is_empty() || request.request_id.len() > 128
        || request.claim.actor.as_deref().is_some_and(|actor| actor != bound.0.run)
    { return Err(refused()); }
    service.execute_claim(credential.secret(), &request.request_id, request.claim)
        .await.map(Json).map_err(ApiError::bad)
}
