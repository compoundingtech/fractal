//! The part of `api::client_v0::assistant::authorization` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use axum::extract::OriginalUri;
use axum::http::HeaderMap;
use smallclaims::hash::canonical_json_text;
use super::reads::{DEFAULT_RESPONSE_BYTES, MAX_RESPONSE_BYTES};
pub use st3_api_core::api::client_v0::assistant::authorization::*;
pub use st3_api_core::api::client_v0::assistant::authorization::{ReadReceipt, ReadTrust, ReadTrustRequest, finish, pin, route};
#[cfg(test)]
#[path = "authorization_tests.rs"]
mod tests;

/// The assistant read handler, serving every projection (including the conversation ones that
/// need this layer's search and conversation readers).
pub async fn read(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
                  Extension(snapshot): Extension<ClientSnapshot>, OriginalUri(uri): OriginalUri,
                  headers: HeaderMap, Extension(context): Extension<response_body::Context>)
    -> Result<(Extension<ReadReceipt>, Extension<ClientSnapshot>, Json<Value>), ApiError> {
    st3_api_core::api::client_v0::assistant::authorization::read_with(State(state), Extension(session), Extension(snapshot), OriginalUri(uri), headers, Extension(context),
        super::reads::projector).await
}

pub async fn read_inner(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
                        Extension(snapshot): Extension<ClientSnapshot>, OriginalUri(uri): OriginalUri,
                        headers: HeaderMap, context: &response_body::Context)
    -> Result<(Extension<ReadReceipt>, Extension<ClientSnapshot>, Json<Value>), ApiError> {
    st3_api_core::api::client_v0::assistant::authorization::read_inner(State(state), Extension(session), Extension(snapshot), OriginalUri(uri), headers, context,
        super::reads::projector).await
}
