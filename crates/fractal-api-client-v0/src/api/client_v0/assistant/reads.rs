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
            thread_read(state, projection, parameters, max_bytes, binding, context).await,
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
    context: &response_body::Context,
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
    let content_collection = format!("{collection}content:");
    let (cursor, content_cursor) = match parameters.get("cursor") {
        Some(cursor) if timeline && cursor.starts_with(&content_collection) => {
            if parameters.contains_key("limit") {
                return Err(validation("a thread content cursor cannot be combined with limit"));
            }
            (None, Some(conversation_reads::timeline_cursor(cursor, &content_collection)?))
        }
        Some(cursor) => (Some(conversation_reads::timeline_cursor(cursor, &collection)?), None),
        None => (None, None),
    };
    let limit = parameters.get("limit").map_or(Ok(10), |limit| {
        limit.parse::<usize>().ok().filter(|limit| (1..=50).contains(limit))
            .ok_or_else(|| validation("limit must be between 1 and 50"))
    })?;
    let state = state.clone();
    let context = context.clone();
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
                let snapshot = client_snapshot_at(&state, index);
                let value = if let Some(id) = id.as_deref() {
                    let summary = thread_metadata::summary_for_conversation(&state.store, &person, &agent, id)
                        .map_err(store_error)?;
                    if timeline {
                        let budget = ThreadResponseBudget {
                            context: &context, snapshot: &snapshot, max_bytes, content_collection: &content_collection,
                        };
                        if let Some(handle) = content_cursor.as_deref() {
                            let (offset, claim_id) = handle.split_once(':')
                                .and_then(|(offset, claim_id)| Some((offset.parse::<usize>().ok()?, claim_id)))
                                .ok_or_else(|| stale("invalid thread content cursor"))?;
                            let content = thread_metadata::mixed_message_content(&state.store, &person, id, claim_id)
                                .map_err(store_error)?;
                            budget.chunk(&content, offset, claim_id, json!({"kind":"message-content"}), None)?
                        } else {
                            let mut page_limit = limit;
                            loop {
                                let mut page = thread_metadata::mixed_timeline(&state.store, &person, id, cursor.as_deref(), page_limit)
                                    .map_err(store_error)?;
                                let next = bind(page.next_cursor.as_deref())?;
                                let value = json!({"items":page.items,"next_cursor":next});
                                if budget.fits(&value)? {
                                    break value;
                                }
                                if page_limit > 1 {
                                    page_limit /= 2;
                                    continue;
                                }
                                let Some(thread_metadata::TimelineItem::Message(message)) = page.items.pop() else {
                                    return Err(budget_exceeded("the thread timeline item cannot fit the approved response byte budget"));
                                };
                                let item = json!(message);
                                break budget.chunk(&message.content, 0, &message.claim_id, item, next)?;
                            }
                        }
                    } else {
                        json!(summary)
                    }
                } else {
                    let page = thread_metadata::list_conversation(&state.store, &person, &agent, cursor.as_deref(), limit)
                        .map_err(store_error)?;
                    json!({"items":page.items,"next_cursor":bind(page.next_cursor.as_deref())?})
                };
                Ok((snapshot, value))
            })();
            Ok(result)
        })
    })
    .await?
}

struct ThreadResponseBudget<'a> {
    context: &'a response_body::Context,
    snapshot: &'a ClientSnapshot,
    max_bytes: usize,
    content_collection: &'a str,
}

impl ThreadResponseBudget<'_> {
    fn fits(&self, value: &Value) -> Result<bool, ApiError> {
        Ok(self.context.value_envelope_size(self.snapshot, value)? <= self.max_bytes)
    }

    fn chunk(
        &self,
        content: &str,
        offset: usize,
        claim_id: &str,
        mut item: Value,
        next: Option<String>,
    ) -> Result<Value, ApiError> {
        let tail = content.get(offset..).filter(|tail| !tail.is_empty())
            .ok_or_else(|| stale("invalid thread content cursor"))?;
        let mut bytes = tail.len();
        loop {
            while !tail.is_char_boundary(bytes) {
                bytes -= 1;
            }
            if bytes == 0 {
                return Err(budget_exceeded("the thread content chunk cannot fit the approved response byte budget"));
            }
            let end = offset + bytes;
            let continuation = (end < content.len())
                .then(|| conversation_reads::bind_cursor(&format!("{end}:{claim_id}"), self.content_collection))
                .transpose()?;
            item["content"] = json!(&tail[..bytes]);
            item["content_truncated"] = json!(continuation.is_some());
            item["next_content_cursor"] = json!(continuation);
            let value = json!({"items":[item],"next_cursor":next});
            if self.fits(&value)? {
                return Ok(value);
            }
            bytes /= 2;
        }
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
