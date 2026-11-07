//! The part of `api::client_v0::assistant::reads` that moved up from fractal-api-core.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use crate::SeatsStore;
use crate::EvidenceStore;
use super::*;
use crate::api::published_attention::{
    published_assistant_attention_rows, published_view_not_ready,
};
use fractal_api_core::api::client_v0::assistant::reads::paging::page;
pub use fractal_api_core::api::client_v0::assistant::reads::*;
pub use fractal_api_core::api::client_v0::assistant::reads::{DEFAULT_RESPONSE_BYTES, MAX_RESPONSE_BYTES, terminal};
use fractal_api_core::api::client_v0::assistant::reads::project_with_context as project_lower;

#[path = "reads_conversation.rs"]
pub mod conversation_reads;
#[path = "reads_discovery.rs"]
mod discovery;

/// Assistant projections. The conversation discovery, timeline and search projections need the
/// client-v0 search and conversation-block readers, which sit above fractal-api-core, so they are served
/// here; every other projection is fractal-api-core's.
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
        "threads" | "thread-detail" | "thread-timeline" =>
            thread_read(state, projection, parameters, max_bytes, binding).await,
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
/// The assistant gateway revalidates the grant before serving each response. All
/// thread data and the exact conversation identity use one store snapshot.
async fn thread_read(
    state: &AppState,
    projection: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
    binding: &str,
) -> Result<(ClientSnapshot, Value), ApiError> {
    use crate::store::thread_metadata;

    let list = projection == "threads";
    let timeline = projection == "thread-timeline";
    let mut fields = vec!["agent_id", "session_id", "max_response_bytes"];
    if !list {
        fields.push("thread_id");
    }
    if list || timeline {
        fields.extend(["cursor", "limit"]);
    }
    allow(parameters, &fields, max_bytes)?;
    let agent = required(parameters, "agent_id")?.to_owned();
    if !agent.starts_with("agent/") || agent == "agent/" || ident(Some(&agent)).is_none() {
        return Err(validation("agent_id must name an agent"));
    }
    let session = required(parameters, "session_id")?.to_owned();
    let unavailable = || ApiError::not_found("the thread is unavailable");
    let id = (!list).then(|| required(parameters, "thread_id")).transpose()?;
    if let Some(id) = id {
        let Some(suffix) = id.strip_prefix("thread/") else {
            return Err(unavailable());
        };
        if suffix.is_empty()
            || suffix.len() > 128
            || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(unavailable());
        }
    }
    let id = id.map(str::to_owned);
    let person = owner(state)?;
    let collection = format!("assistant.{projection}:{}:", credential_digest(
        &serde_json::to_string(&(binding, &person, &agent, &session, &id))
            .map_err(ApiError::internal)?,
    ));
    let cursor = parameters.get("cursor")
        .map(|cursor| conversation_reads::timeline_cursor(cursor, &collection)).transpose()?;
    let limit = parameters.get("limit").map_or(Ok(10), |limit| {
        limit.parse::<usize>().ok().filter(|limit| (1..=50).contains(limit))
            .ok_or_else(|| validation("limit must be between 1 and 50"))
    })?;
    let state = state.clone();
    blocking_store(move || {
        state.store.read_snapshot(|index| {
            let result = (|| {
                let conversation = state.store.conversation_owner_at(&agent, index)
                    .map_err(ApiError::internal)?;
                let incarnation = conversation.as_ref().and_then(|owner| {
                    owner.incarnation.as_deref().or(owner.runtime.as_deref())
                });
                if !incarnation.is_some_and(|incarnation| client_session_id(&agent, incarnation) == session) {
                    return Err(unavailable());
                }
                let store_error = |error: FractalError| match error.code {
                    "thread-unavailable" => unavailable(),
                    "stale-thread-cursor" => stale(&error.message),
                    _ => ApiError::bad(error),
                };
                let bind = |cursor: Option<&str>| {
                    cursor.map(|cursor| conversation_reads::bind_cursor(cursor, &collection)).transpose()
                };
                let value = if let Some(id) = id.as_deref() {
                    let summary = thread_metadata::summary_for_conversation(&state.store, &person, &agent, id)
                        .map_err(store_error)?;
                    if timeline {
                        let page = thread_metadata::history(&state.store, &person, id, cursor.as_deref(), limit)
                            .map_err(store_error)?;
                        json!({"items":page.items,"next_cursor":bind(page.next_cursor.as_deref())?})
                    } else {
                        json!(summary)
                    }
                } else {
                    let page = thread_metadata::list_conversation(&state.store, &person, &agent, cursor.as_deref(), limit)
                        .map_err(store_error)?;
                    json!({"items":page.items,"next_cursor":bind(page.next_cursor.as_deref())?})
                };
                Ok((client_snapshot_at(&state, index), value))
            })();
            Ok(result)
        })
    })
    .await?
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
        state: state.clone(), client_request: true, local_only_response: false,
        request_id: "request/00000000-0000-0000-0000-000000000000".into(),
        client_snapshot: Some(snapshot.clone()), server: None, roster: false,
    };
    project_with_context(state, snapshot, projection, parameters, max_bytes, binding, &context).await
}

pub fn projector<'a>(state: &'a AppState, snapshot: &'a ClientSnapshot, projection: &'a str,
                     parameters: &'a BTreeMap<String, String>, max_bytes: usize, binding: &'a str,
                     context: &'a response_body::Context) -> fractal_api_core::api::client_v0::assistant::authorization::ProjectFuture<'a> {
    Box::pin(project_with_context(state, snapshot, projection, parameters, max_bytes, binding, context))
}
