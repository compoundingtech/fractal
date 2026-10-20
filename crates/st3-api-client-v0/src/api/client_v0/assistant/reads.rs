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
pub use st3_api_core::api::client_v0::assistant::reads::{DEFAULT_RESPONSE_BYTES, MAX_RESPONSE_BYTES, terminal};
use st3_api_core::api::client_v0::assistant::reads::project_with_context as project_lower;

#[path = "reads_conversation.rs"]
pub mod conversation_reads;
#[path = "reads_discovery.rs"]
mod discovery;

/// Assistant projections. The conversation discovery, timeline and search projections need the
/// client-v0 search and conversation-block readers, which sit above st3-api, so they are served
/// here; every other projection is st3-api's.
#[allow(clippy::too_many_arguments)]
pub async fn project_with_context(
    state: &AppState,
    snapshot: &ClientSnapshot,
    projection: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
    binding: &str,
    context: &response_body::Context,
) -> Result<(ClientSnapshot, Value), ApiError> {
    match projection {
        "conversations" | "conversation-detail" | "conversation-timeline" | "conversation-search" =>
            conversation_reads::project_bound_with_context(state, snapshot, projection, parameters, max_bytes, binding, context).await,
        "conversation-content" => {
            let mut parameters = parameters.clone();
            let reference = required(&parameters, "reference")?;
            let unbound = conversation_reads::unbind_reference(reference, binding)?;
            parameters.insert("reference".into(), unbound);
            let (snapshot, mut value) = project_lower(state, snapshot, projection, &parameters, max_bytes, binding, context).await?;
            value["reference"] = json!(conversation_reads::bind_reference(parameters["reference"].as_str(), binding)?);
            Ok((snapshot, value))
        }
        _ => project_lower(state, snapshot, projection, parameters, max_bytes, binding, context).await,
    }
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
    let context = response_body::Context {
        state: state.clone(), client_request: true, observation_report: false,
        request_id: "request/00000000-0000-0000-0000-000000000000".into(),
        client_snapshot: Some(snapshot.clone()), server: None, roster: false,
    };
    project_with_context(state, snapshot, projection, parameters, max_bytes, binding, &context).await
}

pub fn projector<'a>(state: &'a AppState, snapshot: &'a ClientSnapshot, projection: &'a str,
                     parameters: &'a BTreeMap<String, String>, max_bytes: usize, binding: &'a str,
                     context: &'a response_body::Context) -> st3_api_core::api::client_v0::assistant::authorization::ProjectFuture<'a> {
    Box::pin(project_with_context(state, snapshot, projection, parameters, max_bytes, binding, context))
}
