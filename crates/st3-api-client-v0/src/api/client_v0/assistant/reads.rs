//! The part of `api::client_v0::assistant::reads` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use crate::SeatsStore;
use crate::EvidenceStore;
use super::*;
use crate::api::published_attention::{
    published_assistant_attention_rows, published_view_not_ready,
};
use st3_api::api::client_v0::assistant::reads::paging::page;
pub use st3_api::api::client_v0::assistant::reads::*;
pub use st3_api::api::client_v0::assistant::reads::{DEFAULT_RESPONSE_BYTES, MAX_RESPONSE_BYTES, project_with_context, terminal};
#[path = "reads_conversation.rs"]
mod conversation_reads;
#[path = "reads_operational.rs"]
mod operational_reads;
#[path = "reads_discovery.rs"]
mod discovery;

fn extension<'a>(
    state: &'a AppState, snapshot: &'a ClientSnapshot, projection: &'a str,
    parameters: &'a BTreeMap<String, String>, max_bytes: usize, binding: &'a str,
    context: &'a response_body::Context,
) -> ProjectionFuture<'a> {
    Box::pin(async move {
        match projection {
            "conversations" | "conversation-detail" | "conversation-timeline" | "conversation-search" =>
                conversation_reads::project_bound(state, snapshot, projection, parameters, max_bytes, binding, context).await,
            _ => operational_reads::project_bound(state, snapshot, projection, parameters, max_bytes, binding, context).await,
        }
    })
}

/// Serve the conversation and operational projections from the base reader.
pub fn install() {
    install_projection_extension(extension);
}

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
    install();
    let context = test_context(state, snapshot);
    project_with_context(state, snapshot, projection, parameters, max_bytes, binding, &context).await
}

#[cfg(test)]
pub async fn test_project(
    state: &AppState, snapshot: &ClientSnapshot, projection: &str,
    parameters: &BTreeMap<String, String>, max_bytes: usize,
) -> Result<(ClientSnapshot, Value), ApiError> {
    project(state, snapshot, projection, parameters, max_bytes, "test-grant-policy").await
}

#[cfg(test)]
pub fn test_context(state: &AppState, snapshot: &ClientSnapshot) -> response_body::Context {
    response_body::Context {
        state: state.clone(), client_request: true, observation_report: false,
        request_id: "request/00000000-0000-0000-0000-000000000000".into(),
        client_snapshot: Some(snapshot.clone()), server: None, roster: false,
    }
}
