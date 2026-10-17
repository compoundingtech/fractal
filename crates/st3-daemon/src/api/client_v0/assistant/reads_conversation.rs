//! Bounded conversation discovery and native timeline/search projections. Only chat text,
//! opaque content references and conversation identities leave this boundary. Native tool,
//! status, attachment, source-record and diagnostic bodies are deliberately not copied.
use super::*;

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;
const SNIPPET_BYTES: usize = 512;
const REFERENCE_BYTES: usize = 4096;

pub(super) async fn project(
    state: &AppState,
    snapshot: &ClientSnapshot,
    projection: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
) -> Result<(ClientSnapshot, Value), ApiError> {
    // Even cached discovery pages require a configured owner.
    let person = owner(state)?;
    match projection {
        "conversations" => {
            allow(parameters, &["cursor", "limit", "max_response_bytes"], max_bytes)?;
            let mut query = parameters.clone();
            let limit = limit(parameters)?;
            if !parameters.contains_key("cursor") || parameters.contains_key("limit") {
                query.insert("limit".into(), limit.to_string());
            }
            page(state, snapshot, "assistant.conversations", &query, max_bytes, conversations).await
        }
        "conversation-detail" => {
            allow(parameters, &["agent_id", "max_response_bytes"], max_bytes)?;
            timeline(state, snapshot, &person, parameters, (DEFAULT_LIMIT, max_bytes), true).await
        }
        "conversation-timeline" => {
            allow(parameters, &["agent_id", "session_id", "cursor", "limit", "max_response_bytes"], max_bytes)?;
            timeline(state, snapshot, &person, parameters, (limit(parameters)?, max_bytes), false).await
        }
        "conversation-search" => {
            allow(parameters, &["text", "agent_id", "cursor", "limit", "max_response_bytes"], max_bytes)?;
            search(state, snapshot, &person, parameters, max_bytes).await
        }
        _ => Err(ApiError::not_found("unknown assistant conversation projection")),
    }
}

fn limit(parameters: &BTreeMap<String, String>) -> Result<usize, ApiError> {
    parameters.get("limit").map_or(Ok(DEFAULT_LIMIT), |value| {
        value.parse::<usize>().ok().filter(|limit| (1..=MAX_LIMIT).contains(limit))
            .ok_or_else(|| validation(format!("limit must be between 1 and {MAX_LIMIT}")))
    })
}

fn agent(parameters: &BTreeMap<String, String>) -> Result<&str, ApiError> {
    let id = required(parameters, "agent_id")?;
    if !id.starts_with("agent/") || id.len() == "agent/".len() || ident(Some(id)).is_none() {
        return Err(validation("agent_id must name an agent"));
    }
    Ok(id)
}

fn conversations(state: &AppState, _person: &str, index: u64, _now: u128) -> anyhow::Result<Vec<Value>> {
    let owners = state.store.conversation_owners_at(index)?;
    Ok(owners.iter().filter_map(|(agent, owner)| {
        let id = ident(Some(agent.as_str()))?;
        if !id.starts_with("agent/") { return None; }
        let incarnation = owner.incarnation.as_deref().or(owner.runtime.as_deref())?;
        Some(json!({"id":id, "agent_id":id, "current_session_id":client_session_id(agent, incarnation)}))
    }).collect())
}

async fn timeline(
    state: &AppState,
    snapshot: &ClientSnapshot,
    person: &str,
    parameters: &BTreeMap<String, String>,
    capacity: (usize, usize),
    detail: bool,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let (limit, max_bytes) = capacity;
    let agent_id = agent(parameters)?.to_owned();
    let session_id = conversation_session_id(state, &agent_id)?;
    if parameters.get("session_id").is_some_and(|requested| requested != &session_id) {
        return Err(stale("the session is not the agent's current conversation"));
    }
    let collection = if detail { "assistant.conversation-detail:" } else { "assistant.conversation-timeline:" };
    let cursor = parameters.get("cursor").map(|cursor| {
        cursor.strip_prefix(collection).map(str::to_owned)
            .ok_or_else(|| stale("the cursor belongs to another assistant projection"))
    }).transpose()?;
    if cursor.as_ref().is_some_and(|cursor| cursor.is_empty() || cursor.len() > 8192) {
        return Err(validation("conversation cursor must contain 1 through 8192 bytes"));
    }
    let mut session = ClientSession::local(Some(person))?;
    session.conversation_blocks = true;
    let reader = state.clone();
    let pinned = snapshot.clone();
    let id = session_id.clone();
    let native = crate::api::read_deadline::spawn_blocking(move || {
        timeline_value(&reader, &pinned, &session, &id, &ClientListQuery {
            cursor, limit: Some(limit), ..Default::default()
        }).map(|Json(value)| value)
    }).await.map_err(ApiError::internal)??;
    let snippet_bytes = SNIPPET_BYTES.min(max_bytes / limit);
    let items = native["items"].as_array().into_iter().flatten().take(limit)
        .map(|item| timeline_item(item, snippet_bytes)).collect::<Vec<_>>();
    let mut projected_page = safe_page(&native["page"]);
    if let Some(cursor) = projected_page["next_cursor"].as_str() {
        projected_page["next_cursor"] = json!(format!("{collection}{cursor}"));
    }
    Ok((snapshot.clone(), json!({
        "kind":if detail { "conversation-detail" } else { "conversation-timeline" },
        "agent_id":agent_id, "session_id":session_id, "current_session_id":session_id,
        "items":items, "page":projected_page,
    })))
}

/// Clip on UTF-8 boundaries, never clone an unbounded source string first.
fn snippet(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) { end -= 1; }
    &text[..end]
}

fn timeline_item(item: &Value, snippet_bytes: usize) -> Value {
    let kind = closed(item["type"].as_str(), &["content", "message", "tool_call", "tool_result", "status", "usage", "error", "truncation", "redaction"]);
    let mut result = json!({
        "id":ident(item["id"].as_str()),
        "sequence":item["sequence"].as_u64(),
        "revision":revision(&item["revision"]),
        "timestamp":timestamp(&item["timestamp"]),
        "type":kind,
        "role":closed(item["role"].as_str(), &["assistant", "user", "system", "tool"]),
        "final":item["final"].as_bool(),
    });
    // Only actual chat content has a text disclosure. Tool and fallback diagnostic
    // bodies can contain arbitrary environments, paths or raw native records.
    if matches!(kind, "content" | "message")
        && matches!(item["role"].as_str(), Some("assistant" | "user"))
        && item["body"]["media_type"] == "text/plain"
    {
        if let Some(text) = item["body"]["text"].as_str() {
            result["text"] = json!(snippet(text, snippet_bytes));
            result["text_truncated"] = json!(text.len() > snippet_bytes);
        }
        let content = item["body"]["blocks"].as_array().into_iter().flatten()
            .filter(|block| block["kind"] == "raw_text" && block["source_type"] == "content")
            .filter_map(|block| {
                let reference = block["continuation"]["ref"].as_str()?;
                if reference.len() > REFERENCE_BYTES { return None; }
                Some(json!({"reference":reference, "revision":revision(&item["revision"]), "media_type":"text/plain"}))
            }).take(1).collect::<Vec<_>>();
        if !content.is_empty() { result["content"] = json!(content); }
    }
    result
}

fn revision(value: &Value) -> Value {
    if let Some(number) = value.as_u64() { return json!(number.to_string()); }
    ident(value.as_str()).map_or(Value::Null, Value::String)
}

fn safe_page(page: &Value) -> Value {
    json!({
        "limit":page["limit"].as_u64(),
        "has_more":page["has_more"].as_bool().unwrap_or(false),
        "next_cursor":page["next_cursor"].as_str().filter(|cursor| cursor.len() <= 8192),
        "cursor_expires_at":timestamp(&page["cursor_expires_at"]),
    })
}

async fn search(
    state: &AppState,
    snapshot: &ClientSnapshot,
    person: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let limit = limit(parameters)?;
    let text = required(parameters, "text")?;
    if parameters.contains_key("agent_id") { agent(parameters)?; }
    if parameters.get("cursor").is_some_and(|cursor| cursor.is_empty() || cursor.len() > 2048) {
        return Err(validation("search cursor must contain 1 through 2048 bytes"));
    }
    let Json(native) = super::super::super::search::assistant_search(
        state.clone(), ClientSession::local(Some(person))?, text.to_owned(),
        parameters.get("agent_id").cloned(), parameters.get("cursor").cloned(), limit,
    ).await?;
    let snippet_bytes = SNIPPET_BYTES.min(max_bytes / limit);
    let items = native["items"].as_array().into_iter().flatten().take(limit).map(|hit| {
        let kind = closed(hit["entry_type"].as_str(), &["message", "content"]);
        let mut result = json!({
            "conversation_id":ident(hit["conversation_id"].as_str()),
            "entry_id":ident(hit["entry_id"].as_str()),
            "agent_id":ident(hit["agent_id"].as_str()),
            "timestamp":timestamp(&hit["timestamp"]), "entry_type":kind,
        });
        // This native index contains only plain chat text and canonical mailbox messages.
        if matches!(kind, "message" | "content") && let Some(excerpt) = hit["excerpt"].as_str() {
            result["excerpt"] = json!(snippet(excerpt, snippet_bytes));
            result["excerpt_truncated"] = json!(excerpt.len() > snippet_bytes);
        }
        result
    }).collect::<Vec<_>>();
    Ok((snapshot.clone(), json!({
        "kind":"conversation-search", "items":items,
        "indexed_at":timestamp(&native["indexed_at"]),
        "refreshing":native["refreshing"].as_bool().unwrap_or(false),
        // Diagnostic strings can contain native paths. A count conveys incompleteness
        // without exposing those strings or turning them into assistant instructions.
        "incomplete_source_count":native["incomplete_sources"].as_array().map_or(0, Vec::len),
        "page":safe_page(&native["page"]),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters(values: &[(&str, &str)]) -> BTreeMap<String, String> {
        values.iter().map(|(key, value)| ((*key).into(), (*value).into())).collect()
    }

    #[test]
    fn timeline_allowlist_omits_native_payloads_and_bounds_unicode() {
        let projected = timeline_item(&json!({
            "id":"entry/one", "revision":3, "type":"content", "role":"assistant",
            "body":{"media_type":"text/plain","text":"ééé", "environment":"PRIVATE",
                "blocks":[{"kind":"source_record","payload":{"cwd":"/PRIVATE"}},
                    {"kind":"raw_text","source_type":"content","continuation":{"ref":"opaque"}}]},
        }), 3);
        assert_eq!(projected["text"], "é");
        assert_eq!(projected["content"][0]["reference"], "opaque");
        assert_eq!(projected["content"][0]["revision"], "3");
        assert!(!projected.to_string().contains("PRIVATE"));
        let tool = timeline_item(&json!({
            "id":"entry/tool","type":"tool_result","role":"tool",
            "body":{"media_type":"text/plain","text":"PRIVATE","arguments":{"secret":"PRIVATE"}},
        }), 512);
        assert!(tool.get("text").is_none());
        assert!(tool.get("content").is_none());
        assert!(!tool.to_string().contains("PRIVATE"));
    }

    #[tokio::test]
    async fn native_detail_refs_feed_content_and_timeline_rejects_foreign_session() {
        let root = tempfile::tempdir().unwrap();
        let text = format!("boundedneedle {}", "é".repeat(4500));
        let (state, session_id, _, _) = super::super::tests::managed_conversation(root.path(), &text);
        let snapshot = new_client_snapshot(&state);
        let (_, detail) = project(&state, &snapshot, "conversation-detail",
            &parameters(&[("agent_id","agent/reads-codex")]), MAX_RESPONSE_BYTES).await.unwrap();
        assert_eq!(detail["session_id"], session_id);
        let content = detail["items"].as_array().unwrap().iter()
            .find_map(|item| item["content"].as_array().and_then(|refs| refs.first()))
            .expect("the native long text supplies its content reference");
        let read = super::super::conversation(&state, &parameters(&[
            ("agent_id","agent/reads-codex"), ("session_id",&session_id),
            ("reference",content["reference"].as_str().unwrap()),
            ("revision",content["revision"].as_str().unwrap()), ("start","0"), ("end","13"),
        ]), MAX_RESPONSE_BYTES).await.unwrap();
        assert_eq!(read["text"], "boundedneedle");
        let (_, timeline) = project(&state, &snapshot, "conversation-timeline", &parameters(&[
            ("agent_id","agent/reads-codex"), ("session_id",&session_id), ("limit","1"),
        ]), MAX_RESPONSE_BYTES).await.unwrap();
        assert!(timeline["items"].as_array().unwrap().len() <= 1);
        assert_eq!(timeline["page"]["limit"], 1);
        for item in timeline["items"].as_array().unwrap() {
            assert!(item["text"].as_str().is_none_or(|text| text.len() <= SNIPPET_BYTES));
        }
        let error = project(&state, &snapshot, "conversation-timeline", &parameters(&[
            ("agent_id","agent/reads-codex"), ("session_id","session/foreign"),
        ]), MAX_RESPONSE_BYTES).await.unwrap_err();
        assert_eq!(error.code, "stale-fence");
    }

    #[tokio::test]
    async fn native_search_returns_bounded_chat_snippets() {
        let root = tempfile::tempdir().unwrap();
        let text = format!("boundedneedle {}", "word ".repeat(1900));
        let (state, session_id, _, _) = super::super::tests::managed_conversation(root.path(), &text);
        let (_, result) = project(&state, &new_client_snapshot(&state), "conversation-search",
            &parameters(&[("text","boundedneedle"), ("agent_id","agent/reads-codex"), ("limit","1")]),
            MAX_RESPONSE_BYTES).await.unwrap();
        let hits = result["items"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["conversation_id"], session_id);
        assert_eq!(hits[0]["entry_type"], "content");
        let excerpt = hits[0]["excerpt"].as_str().unwrap();
        assert!(excerpt.contains("boundedneedle"));
        assert!(excerpt.len() <= SNIPPET_BYTES);
        assert!(!result.to_string().contains("source_record"));
    }

    #[tokio::test]
    async fn every_projection_requires_owner_and_allowlists_parameters() {
        let root = tempfile::tempdir().unwrap();
        let (mut state, _, _, _) = super::super::tests::managed_conversation(
            root.path(), &"word ".repeat(1900));
        for projection in ["conversations", "conversation-detail", "conversation-timeline", "conversation-search"] {
            let error = project(&state, &new_client_snapshot(&state), projection,
                &parameters(&[("unexpected","value")]), MAX_RESPONSE_BYTES).await.unwrap_err();
            assert_eq!(error.code, "validation-failed");
        }
        state.owning_person = None;
        for projection in ["conversations", "conversation-detail", "conversation-timeline", "conversation-search"] {
            let error = project(&state, &new_client_snapshot(&state), projection,
                &BTreeMap::new(), MAX_RESPONSE_BYTES).await.unwrap_err();
            assert_eq!(error.status, StatusCode::FORBIDDEN);
        }
    }
}
