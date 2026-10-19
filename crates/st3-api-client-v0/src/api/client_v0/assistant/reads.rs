//! The part of `api::client_v0::assistant::reads` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use crate::SeatsStore;
use crate::EvidenceStore;
use super::*;
use crate::api::published_attention::{
    published_assistant_attention_rows, published_view_not_ready,
};
use st3_api_core::api::client_v0::assistant::reads::paging::page;
pub use st3_api_core::api::client_v0::assistant::reads::*;
pub use st3_api_core::api::client_v0::assistant::reads::{DEFAULT_RESPONSE_BYTES, MAX_RESPONSE_BYTES, project_with_context, terminal};
#[cfg(test)]
#[path = "reads_tests.rs"]
pub mod tests;

/// Project one assistant read with the snapshot its rows were read in. `parameters` are the
/// exact decoded query parameters the read authorization names; the person is the
/// configured owner, never a parameter. A cursor or content read keeps `snapshot`.
#[cfg(test)]
pub async fn project(
    state: &AppState,
    snapshot: &ClientSnapshot,
    projection: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
    binding: &str,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let context = response_body::Context {
        state: state.clone(), client_request: true, local_only_response: false,
        request_id: "request/00000000-0000-0000-0000-000000000000".into(),
        client_snapshot: Some(snapshot.clone()), server: None, roster: false,
    };
    project_with_context(state, snapshot, projection, parameters, max_bytes, binding, &context).await
}
