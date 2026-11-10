//! Canonical incoming replies are independent of the sender's current runtime incarnation.
use super::*;
use base64::Engine as _;

pub(super) const SESSION: &str = "session/assistant-replies";
const RANGE_BYTES: usize = 512;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    snapshot: ClientSnapshot,
    stream_through: u64,
    after: (u128, String),
    limit: usize,
    expires: u128,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    message_id: String,
    claim_id: String,
}

fn domain(binding: &str, principal: &str, kind: &str) -> Result<String, ApiError> {
    if !matches!(principal, "assistant/chatgpt" | "assistant/mcp") {
        return Err(forbidden("reply reads require an authenticated assistant principal"));
    }
    Ok(format!("assistant.replies-{kind}:{binding}:{principal}:"))
}

fn encode<T: Serialize>(value: &T, domain: &str) -> Result<String, ApiError> {
    let bytes = serde_json::to_vec(value).map_err(ApiError::internal)?;
    conversation_reads::bind_cursor(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes), domain)
}

fn decode<T: serde::de::DeserializeOwned>(value: &str, domain: &str) -> Result<T, ApiError> {
    let value = conversation_reads::timeline_cursor(value, domain)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value)
        .map_err(|_| stale("invalid assistant reply handle"))?;
    serde_json::from_slice(&bytes).map_err(|_| stale("invalid assistant reply handle"))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn page(
    state: &AppState, snapshot: &ClientSnapshot, parameters: &BTreeMap<String, String>,
    max_bytes: usize, binding: &str, principal: &str, context: &response_body::Context,
) -> Result<(ClientSnapshot, Value), ApiError> {
    allow(parameters, &["replies", "cursor", "limit", "max_response_bytes"], max_bytes)?;
    owner(state)?;
    let cursor_domain = domain(binding, principal, "page")?;
    let reference_domain = domain(binding, principal, "content")?;
    let requested = parameters.get("limit").map(|value| value.parse::<usize>().ok()
        .filter(|value| (1..=50).contains(value)).ok_or_else(|| validation("limit must be between 1 and 50"))).transpose()?;
    let cursor: Option<Cursor> = parameters.get("cursor").map(|value| decode(value, &cursor_domain)).transpose()?;
    if let Some(cursor) = &cursor
        && (cursor.snapshot.host_id != client_host_id(&state.node) || cursor.expires <= client_now_ms()
            || !(1..=50).contains(&cursor.limit) || requested.is_some_and(|limit| limit != cursor.limit)) {
        return Err(stale("the reply page does not match its host, limit or expiry"));
    }
    let limit = cursor.as_ref().map_or(requested.unwrap_or(10), |cursor| cursor.limit);
    let expires = cursor.as_ref().map_or_else(|| client_now_ms().saturating_add(CLIENT_PAGE_TTL_MS), |cursor| cursor.expires);
    let reader = state.clone();
    let pinned = cursor.as_ref().map_or(snapshot, |cursor| &cursor.snapshot).clone();
    let after = cursor.as_ref().map(|cursor| cursor.after.clone());
    let stream = cursor.as_ref().map(|cursor| cursor.stream_through);
    let continuation = cursor.is_some();
    let recipient = principal.to_owned();
    let (snapshot, headers, stream_through) = blocking_store(move || reader.store.clone().read_snapshot(|index| {
        let snapshot = if continuation { pinned } else { client_snapshot_at(&reader, index) };
        let (headers, stream) = reader.store.assistant_reply_headers_page(&recipient,
            (snapshot.store_index, stream), after.as_ref(), limit)?;
        Ok((snapshot, headers, stream))
    })).await?;
    let has_more = headers.len() > limit;
    let source_count = headers.len().min(limit);
    let mut items = Vec::with_capacity(limit);
    let mut last = None;
    for (position, header) in headers.into_iter().take(limit).enumerate() {
        if header.to != principal { return Err(forbidden("reply recipient does not match the authenticated principal")); }
        let reader = state.clone();
        let id = header.message_id.clone();
        let claim_id = header.claim_id.clone();
        let end = header.text_bytes.min(RANGE_BYTES);
        let text = blocking_store(move || reader.store.assistant_reply_text_range(&id, &claim_id, 0, end)).await?
            .ok_or_else(|| stale("the reply text is unavailable"))?;
        let reference = encode(&Reference { message_id:header.message_id.clone(), claim_id:header.claim_id.clone() }, &reference_domain)?;
        let read = json!({"agent_id":header.from,"session_id":SESSION,"reference":reference,
            "revision":header.claim_id,"start":0,"end":text.len()});
        items.push(json!({"agent_id":header.from,"message_id":header.message_id,"from":header.from,
            "to":header.to,"reply_to":header.in_reply_to,"sent_at":client_timestamp(header.accepted_at_unix_ms),
            "source":{"kind":"message.sent","claim_id":header.claim_id},"read":read}));
        let key = (header.accepted_at_unix_ms, header.message_id);
        let more = has_more || position + 1 < source_count;
        let candidate = page_value(state, &items, &snapshot, stream_through, &key, limit, expires, more, &cursor_domain)?;
        if context.value_envelope_size(&snapshot, &candidate)? > max_bytes {
            items.pop();
            let Some(last) = last else { return Err(budget_exceeded("one reply discovery item exceeds the response byte budget")); };
            return Ok((snapshot.clone(), page_value(state, &items, &snapshot, stream_through, &last, limit, expires, true, &cursor_domain)?));
        }
        last = Some(key);
    }
    let value = match last {
        Some(last) => page_value(state, &items, &snapshot, stream_through, &last, limit, expires, has_more, &cursor_domain)?,
        None => json!({"kind":"page","items":[],"page":{"limit":limit,"has_more":false,"next_cursor":null}}),
    };
    Ok((snapshot, value))
}

#[allow(clippy::too_many_arguments)]
fn page_value(state: &AppState, items: &[Value], snapshot: &ClientSnapshot, stream_through: u64,
    after: &(u128, String), limit: usize, expires: u128, more: bool, domain: &str) -> Result<Value, ApiError> {
    let next = more.then(|| encode(&Cursor { snapshot:snapshot.clone(), stream_through,
        after:after.clone(), limit, expires }, domain)).transpose()?;
    Ok(json!({"kind":"page","collection":"assistant.replies","items":items,
        "page":{"limit":limit,"has_more":more,"next_cursor":next,
            "cursor_expires_at":more.then(|| client_timestamp(expires))},"sync":client_sync_notice(state)}))
}

pub(super) async fn read(state: &AppState, snapshot: &ClientSnapshot, parameters: &BTreeMap<String, String>,
    max_bytes: usize, binding: &str, principal: &str) -> Result<(ClientSnapshot, Value), ApiError> {
    allow(parameters, &["agent_id", "session_id", "reference", "revision", "start", "end", "max_response_bytes"], max_bytes)?;
    owner(state)?;
    let reference: Reference = decode(required(parameters, "reference")?, &domain(binding, principal, "content")?)?;
    let start = required(parameters, "start")?.parse::<usize>().map_err(|_| validation("invalid reply range start"))?;
    let end = required(parameters, "end")?.parse::<usize>().map_err(|_| validation("invalid reply range end"))?;
    if end < start || end - start > RANGE_BYTES { return Err(validation("reply ranges must be at most 512 bytes")); }
    let reader = state.clone();
    let message_id = reference.message_id.clone();
    let header = blocking_store(move || reader.store.assistant_reply_header(&message_id)).await?
        .ok_or_else(|| stale("the canonical reply is unavailable"))?;
    if header.to != principal || header.from != required(parameters, "agent_id")?
        || header.claim_id != reference.claim_id || header.claim_id != required(parameters, "revision")? {
        return Err(forbidden("the reply handle does not match this principal, sender and immutable revision"));
    }
    if end > header.text_bytes { return Err(validation("reply range exceeds the pinned message")); }
    let reader = state.clone();
    let message_id = reference.message_id;
    let claim_id = reference.claim_id;
    let next_limit = end.saturating_add(RANGE_BYTES).min(header.text_bytes);
    let (text, next_end) = blocking_store(move || {
        let text = reader.store.assistant_reply_text_range(&message_id, &claim_id, start, end)?;
        let next = reader.store.assistant_reply_text_range(&message_id, &claim_id, end, next_limit)?;
        Ok((text, next.map(|text| end.saturating_add(text.len()))))
    }).await?;
    let text = text.filter(|text| text.len() == end - start)
        .ok_or_else(|| validation("reply range must use UTF-8 boundaries within the pinned message"))?;
    let next_range = next_end.filter(|next| *next > end).map(|next| json!({"start":end,"end":next}));
    Ok((snapshot.clone(), json!({"kind":"assistant-conversation-content","agent_id":header.from,
        "session_id":SESSION,"reference":required(parameters,"reference")?,"revision":header.claim_id,
        "message_id":header.message_id,"from":header.from,"to":header.to,"reply_to":header.in_reply_to,
        "source":{"kind":"message.sent","claim_id":header.claim_id},"start":start,"end":end,
        "returned_bytes":text.len(),"text":text,"truncated":end<header.text_bytes,"next_range":next_range})))
}
