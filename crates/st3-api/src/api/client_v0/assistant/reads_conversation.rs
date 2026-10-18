//! Bounded conversation discovery and native timeline/search projections. Only chat text,
//! opaque content references and conversation identities leave this boundary. Native tool,
//! status, attachment, source-record and diagnostic bodies are deliberately not copied.
use super::*;

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;
const SNIPPET_BYTES: usize = 512;
const REFERENCE_BYTES: usize = 4096;

#[cfg(test)]
pub(super) async fn project(
    state: &AppState,
    snapshot: &ClientSnapshot,
    projection: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
) -> Result<(ClientSnapshot, Value), ApiError> {
    project_bound(state, snapshot, projection, parameters, max_bytes, "conversation-test-binding").await
}

#[cfg(test)]
pub(super) async fn project_bound(
    state: &AppState, snapshot: &ClientSnapshot, projection: &str,
    parameters: &BTreeMap<String, String>, max_bytes: usize, binding: &str,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let context = response_body::Context {
        state: state.clone(), client_request: true, observation_report: false,
        request_id: "request/00000000-0000-0000-0000-000000000000".into(),
        client_snapshot: Some(snapshot.clone()), server: None, roster: false,
    };
    project_bound_with_context(state, snapshot, projection, parameters, max_bytes, binding, &context).await
}

/// The envelope identity comes from the gateway; continuation authority remains the grant binding.
#[allow(clippy::too_many_arguments)]
pub(super) async fn project_bound_with_context(
    state: &AppState,
    snapshot: &ClientSnapshot,
    projection: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
    binding: &str,
    context: &response_body::Context,
) -> Result<(ClientSnapshot, Value), ApiError> {
    // Even cached discovery pages require a configured owner.
    let person = owner(state)?;
    match projection {
        "conversations" => discovery::page(state, snapshot, "conversations", parameters, max_bytes, binding, context).await,
        "conversation-detail" => {
            allow(parameters, &["agent_id", "max_response_bytes"], max_bytes)?;
            timeline(state, snapshot, &person, parameters, (DEFAULT_LIMIT, max_bytes), true, binding).await
        }
        "conversation-timeline" => {
            allow(parameters, &["agent_id", "session_id", "cursor", "limit", "max_response_bytes"], max_bytes)?;
            timeline(state, snapshot, &person, parameters, (limit(parameters)?, max_bytes), false, binding).await
        }
        "conversation-search" => {
            allow(parameters, &["text", "agent_id", "cursor", "limit", "max_response_bytes"], max_bytes)?;
            search(state, snapshot, &person, parameters, max_bytes, binding).await
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


/// Authenticate the wrapper as well as the native handle: a caller must not relabel
/// a native cursor with another grant's otherwise-public collection identity.
fn timeline_cursor_mac(collection: &str, cursor: &str) -> Result<hmac::Hmac<sha2::Sha256>, ApiError> {
    use hmac::Mac as _;
    static KEY: std::sync::LazyLock<[u8; 32]> = std::sync::LazyLock::new(|| {
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).expect("creating the assistant timeline cursor authentication key");
        key
    });
    let mut mac = <hmac::Hmac<sha2::Sha256> as hmac::Mac>::new_from_slice(&*KEY)
        .expect("HMAC accepts a 32-byte key");
    mac.update(&serde_json::to_vec(&(collection, cursor)).map_err(ApiError::internal)?);
    Ok(mac)
}

pub(in super::super) fn bind_reference(reference: &str, binding: &str) -> Result<String, ApiError> {
    use hmac::Mac as _;
    let collection = format!("assistant.conversation-content:{binding}:");
    let signature = hex::encode(timeline_cursor_mac(&collection, reference)?.finalize().into_bytes());
    Ok(format!("{collection}{signature}:{reference}"))
}

pub(in super::super) fn unbind_reference(reference: &str, binding: &str) -> Result<String, ApiError> {
    timeline_cursor(reference, &format!("assistant.conversation-content:{binding}:"))
}

fn timeline_cursor(encoded: &str, collection: &str) -> Result<String, ApiError> {
    use hmac::Mac as _;
    let invalid = || stale("the cursor does not belong to this assistant projection and grant policy");
    let (signature, cursor) = encoded.strip_prefix(collection).and_then(|value| value.split_once(':'))
        .ok_or_else(invalid)?;
    if cursor.is_empty() || cursor.len() > 8192 || signature.len() != 64 {
        return Err(invalid());
    }
    let signature = hex::decode(signature).map_err(|_| invalid())?;
    timeline_cursor_mac(collection, cursor)?.verify_slice(&signature).map_err(|_| invalid())?;
    Ok(cursor.to_owned())
}

async fn timeline(
    state: &AppState,
    snapshot: &ClientSnapshot,
    person: &str,
    parameters: &BTreeMap<String, String>,
    capacity: (usize, usize),
    detail: bool,
    binding: &str,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let (limit, max_bytes) = capacity;
    let agent_id = agent(parameters)?.to_owned();
    let current = state.store.index().map_err(ApiError::internal)?;
    let owner = state.store.conversation_owner_at(&agent_id, current).map_err(ApiError::internal)?;
    let incarnation = owner.incarnation.as_deref().or(owner.runtime.as_deref())
        .ok_or_else(|| ApiError::not_found("the agent has no current conversation"))?;
    let session_id = client_session_id(&agent_id, incarnation);
    if parameters.get("session_id").is_some_and(|requested| requested != &session_id) {
        return Err(stale("the session is not the agent's current conversation"));
    }
    let projection = if detail { "assistant.conversation-detail" } else { "assistant.conversation-timeline" };
    let collection = format!("{projection}:{binding}:");
    let cursor = parameters.get("cursor")
        .map(|cursor| timeline_cursor(cursor, &collection)).transpose()?;
    // The gateway cannot inspect the authenticated assistant wrapper. Recover the
    // native page's pinned snapshot only after verifying its grant-policy MAC.
    let snapshot = cursor.as_deref().map(decode_client_cursor_snapshot).transpose()?
        .unwrap_or_else(|| snapshot.clone());
    let mut session = ClientSession::local(Some(person))?;
    session.conversation_blocks = true;
    session.assistant_content_handles = true;
    let reader = state.clone();
    let pinned = snapshot.clone();
    let id = session_id.clone();
    let selected_owner = agent_id.clone();
    let native = crate::api::read_deadline::spawn_blocking(move || {
        timeline_value(&reader, &pinned, &session, &id, &ClientListQuery {
            cursor, limit: Some(limit), owner: Some(selected_owner), ..Default::default()
        }).map(|Json(value)| value)
    }).await.map_err(ApiError::internal)??;
    let snippet_bytes = SNIPPET_BYTES.min(max_bytes / limit);
    let items = native["items"].as_array().into_iter().flatten().take(limit)
        .map(|item| {
            let mut value = timeline_item(item, snippet_bytes);
            if let Some(reference) = value["content"][0]["reference"].as_str() {
                value["content"][0]["reference"] = json!(bind_reference(reference, binding)?);
            }
            value["read"] = read_handle(item, &value, &agent_id, &session_id);
            Ok(value)
        }).collect::<Result<Vec<_>, ApiError>>()?;
    let mut projected_page = safe_page(&native["page"]);
    if let Some(cursor) = projected_page["next_cursor"].as_str() {
        use hmac::Mac as _;
        let signature = hex::encode(timeline_cursor_mac(&collection, cursor)?.finalize().into_bytes());
        projected_page["next_cursor"] = json!(format!("{collection}{signature}:{cursor}"));
    }
    Ok((snapshot, json!({
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
            .filter(|block| matches!(block["kind"].as_str(), Some("text" | "raw_text"))
                && block["source_type"] == "content" && block["payload"]["body_ref"] == true)
            .filter_map(|block| {
                let reference = block["continuation"]["ref"].as_str()?;
                if reference.len() > REFERENCE_BYTES { return None; }
                Some(json!({"reference":reference, "revision":revision(&item["revision"]), "media_type":"text/plain"}))
            }).take(1).collect::<Vec<_>>();
        if !content.is_empty() { result["content"] = json!(content); }
    }
    result
}

fn read_handle(native: &Value, projected: &Value, agent: &str, session: &str) -> Value {
    let Some(reference) = projected["content"][0]["reference"].as_str() else { return Value::Null; };
    let Some(text) = native["body"]["text"].as_str() else { return Value::Null; };
    let end = snippet(text, SNIPPET_BYTES).len();
    if end == 0 { return Value::Null; }
    json!({"agent_id":agent,"session_id":session,"reference":reference,
        "revision":projected["content"][0]["revision"],"start":0,"end":end})
}

pub(super) async fn discovery_read(
    state: &AppState, snapshot: &ClientSnapshot, item: &Value, binding: &str,
) -> Result<Value, ApiError> {
    let parameters = BTreeMap::from([
        ("agent_id".into(), item["agent_id"].as_str().unwrap_or_default().into()),
        ("session_id".into(), item["session_id"].as_str().unwrap_or_default().into()),
    ]);
    let (_, page) = timeline(state, snapshot, &owner(state)?, &parameters,
        (DEFAULT_LIMIT, MAX_RESPONSE_BYTES), false, binding).await?;
    Ok(page["items"].as_array().into_iter().flatten()
        .find(|item| item["read"].is_object()).map_or(Value::Null, |item| item["read"].clone()))
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
    binding: &str,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let limit = limit(parameters)?;
    let text = required(parameters, "text")?;
    if parameters.contains_key("agent_id") { agent(parameters)?; }
    if parameters.get("cursor").is_some_and(|cursor| cursor.is_empty() || cursor.len() > 2048) {
        return Err(validation("search cursor must contain 1 through 2048 bytes"));
    }
    let Json(native) = super::super::super::search::assistant_search(
        state.clone(), ClientSession::local(Some(person))?, text.to_owned(),
        parameters.get("agent_id").cloned(), parameters.get("cursor").cloned(), limit, binding,
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
    fn content_references_reject_foreign_grants_policies_and_relabeling() {
        let reference = bind_reference("native-reference", "grant-a-policy-1").unwrap();
        assert_eq!(unbind_reference(&reference, "grant-a-policy-1").unwrap(), "native-reference");
        for binding in ["grant-b-policy-1", "grant-a-policy-2"] {
            assert_eq!(unbind_reference(&reference, binding).unwrap_err().code, "stale-fence");
            let relabeled = reference.replace("grant-a-policy-1", binding);
            assert_eq!(unbind_reference(&relabeled, binding).unwrap_err().code, "stale-fence");
        }
        assert_eq!(unbind_reference("native-reference", "grant-a-policy-1").unwrap_err().code, "stale-fence");
    }

    #[test]
    fn timeline_allowlist_omits_native_payloads_and_bounds_unicode() {
        let projected = timeline_item(&json!({
            "id":"entry/one", "revision":3, "type":"content", "role":"assistant",
            "body":{"media_type":"text/plain","text":"ééé", "environment":"PRIVATE",
                "blocks":[{"kind":"source_record","payload":{"cwd":"/PRIVATE"}},
                    {"kind":"raw_text","source_type":"content","payload":{"body_ref":true},"continuation":{"ref":"opaque"}}]},
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
    async fn canonical_message_sent_replies_supply_readable_short_and_long_handles() {
        for text in ["ACK canonical short".to_owned(), format!("ACK canonical long {}", "reply body ".repeat(900))] {
            let root = tempfile::tempdir().unwrap();
            let (state, session_id, _, _) = super::super::tests::managed_conversation(root.path(), "unrelated native text");
            state.store.append_claim(&ClaimInput {
                subject:"message/canonical-reply".into(), kind:"message.sent".into(), actor:Some("agent/reads-codex".into()),
                fields:serde_json::from_value(json!({"from":"agent/reads-codex", "to":"person/operator",
                    "session_id":session_id, "status":"sent", "content":text})).unwrap(),
                evidence:Vec::new(), expected_subject:None, idempotency_key:None,
            }).unwrap();
            let snapshot = new_client_snapshot(&state);
            let (_, timeline) = project(&state, &snapshot, "conversation-timeline",
                &parameters(&[("agent_id","agent/reads-codex")]), MAX_RESPONSE_BYTES).await.unwrap();
            let item = timeline["items"].as_array().unwrap().iter()
                .find(|item| item["text"].as_str().is_some_and(|text| text.starts_with("ACK canonical")))
                .expect("the canonical message.sent content is projected");
            let handle = item["read"].as_object().expect("canonical replies have content handles");
            let query = handle.iter().map(|(key, value)| (key.clone(),
                value.as_str().map_or_else(|| value.to_string(), str::to_owned))).collect();
            let (_, content) = super::super::project(&state, &snapshot, "conversation-content", &query,
                MAX_RESPONSE_BYTES, "conversation-test-binding").await.unwrap();
            assert!(text.starts_with(content["text"].as_str().unwrap()));
            assert_eq!(content["reference"], handle["reference"]);
            let reference = unbind_reference(handle["reference"].as_str().unwrap(), "conversation-test-binding").unwrap();
            let (full, revision) = super::super::super::super::conversation_blocks::assistant_text(
                &state, &session_id, &reference, MAX_RESPONSE_BYTES).unwrap();
            assert_eq!(full, text);
            assert_eq!(revision, 1);
            assert_eq!(super::super::super::super::conversation_blocks::assistant_text(
                &state, "session/foreign", &reference, MAX_RESPONSE_BYTES).unwrap_err().code, "conversation-content-invalidated");
            if text.len() > DEFAULT_RESPONSE_BYTES {
                assert_eq!(super::super::super::super::conversation_blocks::assistant_text(
                    &state, &session_id, &reference, DEFAULT_RESPONSE_BYTES).unwrap_err().code, "response-budget-exceeded");
            }
            let (_, listing) = project(&state, &snapshot, "conversations", &BTreeMap::new(), MAX_RESPONSE_BYTES).await.unwrap();
            assert!(listing["items"].as_array().unwrap().iter().any(|item| item["read"].is_object()));
        }
    }

    #[tokio::test]
    async fn conversation_discovery_output_alone_feeds_content_read() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, _, _) = super::super::tests::managed_conversation(root.path(), "a short agent reply");
        let (_, listing) = project(&state, &new_client_snapshot(&state), "conversations",
            &BTreeMap::new(), MAX_RESPONSE_BYTES).await.unwrap();
        let handles = listing["items"].as_array().unwrap().iter()
            .find_map(|item| item["read"].as_object()).expect("discovery supplies a complete read handle");
        let query = handles.iter().map(|(key, value)| (key.clone(),
            value.as_str().map_or_else(|| value.to_string(), str::to_owned))).collect();
        let (_, content) = super::super::project(&state, &new_client_snapshot(&state),
            "conversation-content", &query, MAX_RESPONSE_BYTES, "conversation-test-binding").await.unwrap();
        assert_eq!(content["text"], "a short agent reply");
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
            ("reference",&unbind_reference(content["reference"].as_str().unwrap(), "conversation-test-binding").unwrap()),
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
    async fn timeline_continuations_are_bound_to_grant_policy_and_projection() {
        use std::io::Write;

        let root = tempfile::tempdir().unwrap();
        let (state, _, _, _) = super::super::tests::managed_conversation(
            root.path(), &"word ".repeat(1900));
        let mut transcript = std::fs::OpenOptions::new().append(true).open(
            root.path().join("home/.codex/sessions/2026/10/11/assistant.jsonl")).unwrap();
        for number in 2..=3 {
            writeln!(transcript, "{}", json!({
                "type":"response_item", "timestamp":format!("2026-10-11T12:00:0{number}Z"),
                "payload":{"type":"message", "role":"assistant", "id":format!("answer-{number}"),
                    "content":[{"type":"output_text", "text":format!("answer {number}")}]},
            })).unwrap();
        }
        let snapshot = new_client_snapshot(&state);
        let query = parameters(&[("agent_id","agent/reads-codex"), ("limit","1")]);
        let (_, first) = project_bound(&state, &snapshot, "conversation-timeline", &query,
            MAX_RESPONSE_BYTES, "grant-a-policy-1").await.unwrap();
        let cursor = first["page"]["next_cursor"].as_str().unwrap();
        let mut next = query.clone();
        next.insert("cursor".into(), cursor.into());
        let (_, second) = project_bound(&state, &snapshot, "conversation-timeline", &next,
            MAX_RESPONSE_BYTES, "grant-a-policy-1").await.unwrap();
        assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
        for binding in ["grant-b-policy-1", "grant-a-policy-2"] {
            let error = project_bound(&state, &snapshot, "conversation-timeline", &next,
                MAX_RESPONSE_BYTES, binding).await.unwrap_err();
            assert_eq!(error.code, "stale-fence");
            let mut relabeled = next.clone();
            relabeled.insert("cursor".into(), cursor.replace("grant-a-policy-1", binding));
            let error = project_bound(&state, &snapshot, "conversation-timeline", &relabeled,
                MAX_RESPONSE_BYTES, binding).await.unwrap_err();
            assert_eq!(error.code, "stale-fence");
        }
        let (_, native) = cursor.strip_prefix("assistant.conversation-timeline:grant-a-policy-1:")
            .unwrap().split_once(':').unwrap();
        for foreign in [native.to_owned(), format!("assistant.conversation-timeline:{native}"),
            format!("assistant.conversation-detail:grant-a-policy-1:{native}")] {
            next.insert("cursor".into(), foreign);
            let error = project_bound(&state, &snapshot, "conversation-timeline", &next,
                MAX_RESPONSE_BYTES, "grant-a-policy-1").await.unwrap_err();
            assert_eq!(error.code, "stale-fence");
        }
    }

    #[tokio::test]
    async fn native_search_returns_bounded_chat_snippets() {
        let root = tempfile::tempdir().unwrap();
        let text = format!("boundedneedle {}", "word ".repeat(1900));
        let (state, session_id, _, _) = super::super::tests::managed_conversation(root.path(), &text);
        let session = ClientSession::local(Some("person/operator")).unwrap();
        crate::api::client_v0::search::prepare_assistant_test_index(
            &state, &session, "conversation-test-binding").await.unwrap();
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
