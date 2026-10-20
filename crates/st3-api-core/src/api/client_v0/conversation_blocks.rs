//! Owner-local wire normalization. References encrypt source metadata and hold no durable
//! state: a fetch reads one authenticated native record and refuses edited/replaced content.
use super::*;
use crate::external_sessions::{ExternalConversation, ExternalSession};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::aead;

use std::sync::LazyLock;

// Bound expensive reads without a queued backlog or a second content cache.
static READ_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// The read budget serving `transcript`: the process-wide one, or under test the budget of
/// the test's own scope, so parallel tests never take each other's slots.
pub fn read_slots(_transcript: &std::path::Path) -> &'static tokio::sync::Semaphore {
    &READ_SLOTS
}

static REF_KEY: LazyLock<[u8; 32]> = LazyLock::new(|| {
    let mut key = [0; 32];
    getrandom::fill(&mut key).expect("owner reference key entropy");
    key
});
pub fn read_slot(slots: &tokio::sync::Semaphore) -> Result<tokio::sync::SemaphorePermit<'_>, ApiError> {
    slots.try_acquire().map_err(|_| ApiError {
        status: StatusCode::TOO_MANY_REQUESTS,
        code: "rate-limited".into(),
        message: "owner conversation reads are busy; retry this read".into(),
        details: Box::default(),
    })
}

pub const VALUE_BYTES: usize = 8 * 1024;
pub fn invalidated() -> ApiError {
    ApiError { status: StatusCode::GONE, code: "conversation-content-invalidated".into(),
        message: "the owner content reference expired or its session/source revision changed; reload the conversation".into(),
        details: Box::new(serde_json::Map::from_iter([("full_resync".into(), json!(true))])) }
}

pub fn source(
    state: &AppState, session_id: &str, managed: Option<&super::super::ManagedSessionOwner>,
) -> Result<ExternalSession, ApiError> {
    if let Some((owner, incarnation, origin)) = managed {
        if origin
            .as_deref()
            .is_some_and(|origin| origin != state.store.origin())
        {
            return Err(availability(super::super::remote_unavailable(
                &client_host_id(origin.as_deref().unwrap()),
            )));
        }
        if let Some(incarnation) = incarnation
            && let Some(transcript) = managed_transcript(state, owner, incarnation)?
        {
            return transcript
                .transcript
                .map_err(|missing| transcript_unavailable(&missing.reason));
        }
        return Err(transcript_unavailable(
            "the managed native session has not been bound",
        ));
    }
    match crate::external_sessions::find_conversation(
        state.native_session_home.as_deref(),
        session_id,
    )
    .map_err(ApiError::internal)?
    {
        Some(ExternalConversation::Readable(source)) => Ok(source),
        Some(ExternalConversation::Unavailable(_)) => Err(transcript_unavailable(
            "the native session could not be identified",
        )),
        None => crate::subagent_sessions::resolve_child(
            state.native_session_home.as_deref(),
            session_id,
        )
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("the session does not exist")),
    }
}

/// Preserve transport/authorization codes while making conversation availability explicit.
pub fn availability(mut error: ApiError) -> ApiError {
    if error.code == "remote-unavailable" {
        error
            .details
            .insert("availability".into(), json!("owner-unavailable"));
        error.message = format!(
            "owner unavailable: {}",
            error.message.replace("; cached data remains usable", "")
        );
    }
    error
}

pub fn transcript_unavailable(reason: &str) -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "transcript-unavailable".into(),
        message: format!("transcript unavailable: {reason}"),
        details: Box::default(),
    }
}

pub fn basis(source: &ExternalSession) -> Result<String, ApiError> {
    let metadata = std::fs::metadata(&source.transcript)
        .map_err(|_| transcript_unavailable("the owner cannot inspect its transcript"))?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt as _;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", metadata.created().ok());
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&json!([
            source.driver.as_str(),
            source.native_id,
            source.transcript,
            identity
        ]))
        .map_err(ApiError::internal)?,
    )))
}

#[derive(Deserialize, Serialize)]
pub struct LocatedSource {
    driver: crate::external_sessions::ExternalDriver,
    native_id: String,
    pub transcript: std::path::PathBuf,
}

impl LocatedSource {
    fn from_session(source: &ExternalSession) -> Self {
        Self {
            driver: source.driver,
            native_id: source.native_id.clone(),
            transcript: source.transcript.clone(),
        }
    }

    pub fn session(&self, id: &str) -> ExternalSession {
        ExternalSession {
            id: id.into(),
            revision: String::new(),
            driver: self.driver,
            native_id: self.native_id.clone(),
            transcript: self.transcript.clone(),
            codex_home: None,
            cwd: None,
            title: None,
            started_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            process: None,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct ContentLocator {
    pub source: LocatedSource,
    pub basis: String,
    session: String,
    pub entry: Value,
    pub revision: Value,
    pub pointer: String,
    pub image: bool,
    pub native: Value,
    pub shape: String,
}

/// A digest of an entry's structure: its kind and its blocks. A timeline entry may be a bounded
/// preview of an oversized native record; a ref minted against the preview applies to the
/// owner's complete record only while both agree on it, so a pointer never lands elsewhere.
pub fn shape(item: &Value) -> String {
    let shape = json!({
        "type": item["type"],
        "role": item["role"],
        "blocks": item["body"]["blocks"].as_array().map(|blocks| {
            blocks
                .iter()
                .map(|block| json!([block["id"], block["kind"], block["source_type"]]))
                .collect::<Vec<_>>()
        }),
    });
    let encoded = serde_json::to_vec(&shape).expect("JSON value encodes");
    URL_SAFE_NO_PAD.encode(ring::digest::digest(&ring::digest::SHA256, &encoded))
}

pub fn reference(
    source: &ExternalSession,
    basis: &str,
    session: &str,
    item: &Value,
    pointer: &str,
) -> String {
    let locator = ContentLocator {
        source: LocatedSource::from_session(source),
        basis: basis.into(),
        session: session.into(),
        entry: item["id"].clone(),
        revision: item["revision"].clone(),
        pointer: pointer.into(),
        image: item
            .pointer(pointer)
            .is_some_and(|value| image(value) && !external_image(value)),
        native: item["_source"].clone(),
        shape: shape(item),
    };
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &*REF_KEY).expect("owner reference key"),
    );
    let mut nonce = [0; aead::NONCE_LEN];
    getrandom::fill(&mut nonce).expect("owner reference nonce entropy");
    let mut encrypted = serde_json::to_vec(&locator).expect("JSON value encodes");
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(b"st-conversation-ref-v2"),
        &mut encrypted,
    )
    .expect("owner reference encryption");
    format!(
        "v2.{}",
        URL_SAFE_NO_PAD.encode([nonce.as_slice(), &encrypted].concat())
    )
}

pub fn locator(wanted: &str, session: &str) -> Result<ContentLocator, ApiError> {
    if wanted.len() > 4096 {
        return Err(invalidated());
    }
    let encoded = wanted.strip_prefix("v2.").ok_or_else(invalidated)?;
    let mut encrypted = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalidated())?;
    if encrypted.len() < aead::NONCE_LEN + aead::CHACHA20_POLY1305.tag_len() {
        return Err(invalidated());
    }
    let (nonce, payload) = encrypted.split_at_mut(aead::NONCE_LEN);
    let nonce: [u8; aead::NONCE_LEN] = nonce.try_into().map_err(|_| invalidated())?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &*REF_KEY).expect("owner reference key"),
    );
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"st-conversation-ref-v2"),
            payload,
        )
        .map_err(|_| invalidated())?;
    let locator: ContentLocator = serde_json::from_slice(plaintext).map_err(|_| invalidated())?;
    if locator.session != session
        || locator.pointer.len() > 512
        || !(locator.pointer == "/body" || locator.pointer.starts_with("/body/"))
    {
        return Err(invalidated());
    }
    Ok(locator)
}

fn clipped(text: &str) -> String {
    if text.len() <= VALUE_BYTES {
        return text.to_owned();
    }
    let mut end = VALUE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[st truncated this native timeline value: size limit; {} bytes]",
        &text[..end],
        text.len()
    )
}

fn bound(value: &mut Value) {
    let encoded = serde_json::to_string(value).expect("JSON value encodes");
    if encoded.len() > VALUE_BYTES {
        *value = json!(clipped(&encoded));
    }
}

/// Bound an open object's values without replacing its keys or its outer object.
/// Callers attach a continuation to the original subtree, before this display-only edit.
pub fn bound_open_value(value: &mut Value) -> bool {
    match value {
        Value::String(text) if text.len() > VALUE_BYTES => {
            *text = clipped(text);
            true
        }
        Value::Object(values) => {
            let mut changed = false;
            for value in values.values_mut() {
                changed |= bound_open_value(value);
                // Leave room for the visible marker added to an 8 KiB string. Large
                // collections of small values still need a bounded JSON preview.
                if matches!(value, Value::Object(_) | Value::Array(_)) {
                    let encoded = serde_json::to_string(value).expect("JSON value encodes");
                    if encoded.len() > VALUE_BYTES * 2 {
                        *value = json!(clipped(&encoded));
                        changed = true;
                    }
                }
            }
            changed
        }
        Value::Array(values) => {
            let mut changed = false;
            for value in values.iter_mut() {
                changed |= bound_open_value(value);
            }
            let encoded = serde_json::to_string(values).expect("JSON value encodes");
            if encoded.len() > VALUE_BYTES * 2 {
                *value = json!(clipped(&encoded));
                changed = true;
            }
            changed
        }
        _ => false,
    }
}

/// Preview parsing already clips long strings before wire preparation sees them.
fn has_preview_marker(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            text.contains("[st truncated this native timeline value: size limit;")
        }
        Value::Array(values) => values.iter().any(has_preview_marker),
        Value::Object(values) => values.values().any(has_preview_marker),
        _ => false,
    }
}

fn image(value: &Value) -> bool {
    (value.get("type").and_then(Value::as_str) == Some("file")
        && value
            .get("mime")
            .and_then(Value::as_str)
            .is_some_and(|mime| mime.starts_with("image/")))
        || matches!(
            value.get("type").and_then(Value::as_str),
            Some("image" | "input_image" | "output_image" | "image_url")
        )
}

/// What a reader can say about an inline image before fetching it: the format its own first
/// bytes show and its decoded size. Native MIME labels are untrusted, so a source whose bytes are
/// not inline (a path, a URL) or not a known image keeps `application/octet-stream` and no size.
pub fn image_facts(value: &Value) -> (&'static str, Option<usize>) {
    let unknown = ("application/octet-stream", None);
    let Some(source) = image_source(value) else {
        return unknown;
    };
    // A preview keeps the head of a long value and names the whole value's length; the head
    // still shows the format, and the length gives the size to within the two padding bytes.
    let (source, clipped) = match source.split_once("\n[st truncated this native timeline value: size limit; ") {
        Some((head, marker)) => match marker.split_once(' ').and_then(|(bytes, _)| bytes.parse::<usize>().ok()) {
            Some(bytes) => (head, Some(bytes)),
            None => return unknown,
        },
        None => (source, None),
    };
    let data = match source.strip_prefix("data:") {
        Some(url) => match url.split_once(";base64,") {
            Some((_, data)) => data,
            None => return unknown,
        },
        None => source,
    };
    let prefix = source.len() - data.len();
    let data = data.trim_end();
    if data.len() < 16 || data.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return unknown;
    }
    let Ok(head) = STANDARD.decode(&data[..16]) else {
        return unknown;
    };
    let Some(media) = crate::blobs::sniff_media_type(&head) else {
        return unknown;
    };
    let size = match clipped {
        Some(bytes) => bytes.saturating_sub(prefix) / 4 * 3,
        None => {
            let padding = data.bytes().rev().take_while(|byte| *byte == b'=').count();
            data.len() / 4 * 3 - padding.min(2)
        }
    };
    (media, Some(size))
}

pub fn external_image(value: &Value) -> bool {
    image_source(value)
        .is_some_and(|source| source.starts_with("https://") || source.starts_with("http://"))
}

pub fn image_source(value: &Value) -> Option<&str> {
    value
        .get("data")
        .or_else(|| value.pointer("/source/data"))
        .or_else(|| value.get("image_url").filter(|value| value.is_string()))
        .or_else(|| value.pointer("/image_url/url"))
        .or_else(|| value.pointer("/source/url"))
        .or_else(|| value.get("url"))
        .and_then(Value::as_str)
}

fn continuation(reference: String, media: &str, size: Option<usize>, reason: &str) -> Value {
    let mut result = json!({"ref":reference,"media_type":media,"reason":reason});
    if let Some(size) = size {
        result["size"] = json!(size);
    }
    result
}

fn image_refs(
    value: &mut Value,
    pointer: &str,
    source: &ExternalSession,
    basis: &str,
    session: &str,
    item: &Value,
) -> bool {
    if image(value) {
        if external_image(value) {
            return false;
        }
        let (media, size) = image_facts(value);
        *value = json!({"type":"image","content":continuation(reference(source,basis,session,item,pointer),media,size,"on-demand")});
        return true;
    }
    let mut changed = false;
    match value {
        Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                changed |= image_refs(
                    value,
                    &format!("{pointer}/{index}"),
                    source,
                    basis,
                    session,
                    item,
                );
            }
        }
        Value::Object(values) => {
            for (key, value) in values.iter_mut() {
                changed |= image_refs(
                    value,
                    &format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
                    source,
                    basis,
                    session,
                    item,
                );
            }
        }
        _ => {}
    }
    changed
}

/// Called before pagination: its held vectors contain only bounded display values and refs.
pub fn read(
    source: &ExternalSession,
    session: &ClientSession,
    session_id: &str,
) -> Result<Vec<Value>, ApiError> {
    let timeline = crate::external_sessions::LineTimeline::acquire(source);
    let mut held = timeline.hold();
    // The fold is held before the slot is taken: waiting viewers of this conversation hold
    // no global read slot, so they never crowd an unrelated conversation out of the budget.
    let slots = read_slots(&source.transcript);
    let _slot = read_slot(slots)?;
    let before = basis(source)?;
    let items = held.read(source).map_err(ApiError::internal)?;
    if before != basis(source)? {
        return Err(invalidated());
    }
    let items = prepare(source, session, session_id, items)?;
    if before != basis(source)? {
        return Err(invalidated());
    }
    Ok(items)
}

pub struct PreparedSlice {
    pub items: Vec<Value>,
    pub has_more: bool,
    pub basis: String,
    /// Source content generation at this slice; moves on any non-append transcript change.
    pub generation: u64,
}

pub fn read_slice(
    source: &ExternalSession,
    session: &ClientSession,
    session_id: &str,
    order: crate::external_sessions::TimelineOrder,
    before: Option<&crate::external_sessions::TimelineKey>,
    limit: usize,
) -> Result<PreparedSlice, ApiError> {
    let queue = crate::profile::span("conversation/native-fold-queue");
    let timeline = crate::external_sessions::LineTimeline::acquire(source);
    let mut held = timeline.hold();
    // Same ordering as `read`: park on the shared fold first, then take the global slot.
    let slots = read_slots(&source.transcript);
    let _slot = read_slot(slots)?;
    drop(queue);
    let basis = basis(source)?;
    let fold = crate::profile::span("conversation/native-fold");
    let mut slice = held
        .slice(source, order, before, limit)
        .map_err(ApiError::internal)?;
    drop(fold);
    let _preparation = crate::profile::span("conversation/native-display-preparation");
    for item in &mut slice.items {
        prepare_one(source, session, session_id, &basis, item)?;
    }
    if basis != self::basis(source)? {
        return Err(invalidated());
    }
    Ok(PreparedSlice {
        items: slice.items,
        has_more: slice.has_more,
        basis,
        generation: slice.generation,
    })
}

/// Called before pagination: its held vectors contain only bounded display values and refs.
pub fn prepare(
    source: &ExternalSession,
    session: &ClientSession,
    session_id: &str,
    mut items: Vec<Value>,
) -> Result<Vec<Value>, ApiError> {
    let basis = basis(source)?;
    for item in &mut items {
        prepare_one(source, session, session_id, &basis, item)?;
    }
    if basis != self::basis(source)? {
        return Err(invalidated());
    }
    Ok(items)
}

pub fn prepare_one(
    source: &ExternalSession,
    session: &ClientSession,
    session_id: &str,
    basis: &str,
    item: &mut Value,
) -> Result<(), ApiError> {
    let original = item.clone();
    // The fold holds oversized native records as bounded previews; their long values reach
    // the client only through continuations, read from the owner's complete record.
    let preview = original["_preview_of_bytes"].is_number();
    let body = item["body"]
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("native body is not an object"))?;
    if let Some(blocks) = body.get_mut("blocks").and_then(Value::as_array_mut) {
        for (index, block) in blocks.iter_mut().enumerate() {
            // A fold may sit the full native record beside the marker (`raw`); the marker
            // key decides, so that sibling survives the expansion below.
            let body_ref = block["payload"].get("body_ref") == Some(&Value::Bool(true));
            let pointer = if body_ref {
                "/body".to_owned()
            } else {
                format!("/body/blocks/{index}/payload")
            };
            if body_ref {
                // Keep the native record beside the expanded display body within the same
                // size bound; exact-marker payloads can still collapse back to body_ref.
                let raw = block["payload"].get("raw").cloned();
                block["payload"] = original["body"].clone();
                let payload = block["payload"].as_object_mut().expect("native body");
                payload.remove("blocks");
                if let Some(raw) = raw {
                    payload.insert("raw".into(), raw);
                }
            }
            if block["kind"] == "image" && external_image(&block["payload"]) {
                block["kind"] = json!("image_link");
            } else if block["kind"] == "image" {
                let (media, size) = image_facts(&block["payload"]);
                block["continuation"] = continuation(
                    reference(source, basis, session_id, &original, &pointer),
                    media,
                    size,
                    "on-demand",
                );
                block["payload"] = json!({});
            } else {
                let images_replaced = image_refs(
                    &mut block["payload"],
                    &pointer,
                    source,
                    basis,
                    session_id,
                    &original,
                );
                let encoded = serde_json::to_vec(&block["payload"]).map_err(ApiError::internal)?;
                // A preview may have clipped a value to within the display window, so every
                // block of one keeps a way back to the complete native value.
                if encoded.len() > VALUE_BYTES
                    || original["_oversized_bytes"].is_number()
                    || preview
                {
                    block["continuation"] = continuation(
                        reference(source, basis, session_id, &original, &pointer),
                        "application/json",
                        if let Some(size) = original["_oversized_payload_bytes"][index].as_u64() {
                            Some(size as usize)
                        } else if preview {
                            // A preview never held the complete value, so it cannot size it.
                            None
                        } else if body_ref || images_replaced {
                            // body_ref previews omit the blocks array, and image refs
                            // replace native bytes. Size describes the owner value,
                            // not either of those display-only transformations.
                            Some(
                                serde_json::to_vec(
                                    original.pointer(&pointer).expect("native block payload"),
                                )
                                .map_err(ApiError::internal)?
                                .len(),
                            )
                        } else {
                            Some(encoded.len())
                        },
                        "size-limit",
                    );
                    bound(&mut block["payload"]);
                }
            }
        }
    }
    // A task's agents link to their own conversations when those transcripts exist
    // (contract §1b q2); prepare_one applies this to first and keyset continuation pages.
    if session.conversation_blocks
        && let Some(blocks) = body.get_mut("blocks").and_then(Value::as_array_mut)
    {
        for block in blocks
            .iter_mut()
            .filter(|block| block["kind"] == "tool_output" && block["view"]["type"] == "task")
        {
            let Some(agents) = block
                .get_mut("view")
                .and_then(|view| view.get_mut("agents"))
                .and_then(Value::as_array_mut)
            else {
                continue;
            };
            for agent in agents {
                let Some(task_id) = agent.get("id").and_then(Value::as_str).map(str::to_owned)
                else {
                    continue;
                };
                let Some(agent) = agent.as_object_mut() else {
                    continue;
                };
                match crate::subagent_sessions::child_transcript(source, &task_id) {
                    Some(_) => {
                        agent.insert(
                            "conversation".into(),
                            json!({"session_id": crate::subagent_sessions::child_session_id(
                                source,
                                &task_id,
                            )}),
                        );
                    }
                    None => {
                        agent.remove("conversation");
                    }
                }
            }
        }
    }
    // Keep open display objects (including assistant text/reasoning metadata) bounded.
    // JSON continuations can cover the whole body, but an image's continuation must keep
    // pointing to its pixels. Its clipped display objects get their own references.
    if let Some(blocks) = body.get_mut("blocks").and_then(Value::as_array_mut) {
        for (index, block) in blocks.iter_mut().enumerate() {
            let pixels = block["kind"] == "image" && block.get("continuation").is_some();
            for key in ["metadata", "view"] {
                if let Some(value) = block.get_mut(key)
                    && (bound_open_value(value)
                        || (preview && (!pixels || has_preview_marker(value))))
                {
                    let pointer = if !pixels && block.get("continuation").is_some() {
                        "/body".to_owned()
                    } else {
                        format!("/body/blocks/{index}/{key}")
                    };
                    let more = continuation(
                        reference(source, basis, session_id, &original, &pointer),
                        "application/json",
                        if preview {
                            None
                        } else {
                            Some(
                                serde_json::to_vec(
                                    original.pointer(&pointer).expect("native open value"),
                                )
                                .map_err(ApiError::internal)?
                                .len(),
                            )
                        },
                        "size-limit",
                    );
                    if pixels {
                        block[key]["continuation"] = more;
                    } else {
                        block["continuation"] = more;
                    }
                }
            }
        }
    }
    if let Some(block) = body
        .get("blocks")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.first())
        && block["kind"] == "unknown"
    {
        let label = body
            .get("text")
            .and_then(Value::as_str)
            .and_then(|text| text.lines().next())
            .unwrap_or("[unknown]");
        body["text"] = json!(format!("{label}\n{}", block["payload"]));
    }
    if let Some(block) = body
        .get("blocks")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.first())
        && block["kind"] == "image_link"
    {
        body["text"] = json!(format!(
            "[external image link · open explicitly]\n{}",
            block["payload"]
        ));
    }
    // Fallback bodies use known v0 types and also move native pixels to owner fetch refs.
    for key in ["text", "arguments", "content"] {
        if let Some(value) = body.get_mut(key) {
            image_refs(
                value,
                &format!("/body/{key}"),
                source,
                basis,
                session_id,
                &original,
            );
            if let Value::String(text) = value {
                *text = clipped(text);
            } else {
                bound(value);
            }
        }
    }
    // Assistant projections also need handles for short chat replies. Preserve a separate
    // text continuation without changing ordinary native clients' display-block layout.
    if session.conversation_blocks && original["body"]["media_type"] == "text/plain"
        && original["body"]["text"].as_str().is_some_and(|text| text.len() > VALUE_BYTES || session.assistant_content_handles)
        && body.get("blocks").and_then(Value::as_array)
            .is_none_or(|blocks| session.assistant_content_handles || blocks.iter().all(|block| block["kind"] == "source_record"))
    {
        let blocks = body.entry("blocks").or_insert_with(|| json!([])).as_array_mut().expect("blocks array");
        blocks.push(json!({"id":format!("{}/text",original["id"].as_str().unwrap_or("content")),
            "kind":"raw_text","source_type":"content","payload":{"body_ref":true},
            "continuation":continuation(reference(source,basis,session_id,&original,"/body"),
                "application/json",None,"size-limit")}));
    }
    // Error/status fallback fields use the same clipped display convention as
    // text/arguments/content. A negotiated body_ref continuation fetches the
    // complete original body; legacy clients receive the visible marker only.
    for key in ["message", "details", "detail"] {
        if let Some(value) = body.get_mut(key) {
            bound_open_value(value);
        }
    }
    let mut fallback = original["body"].clone();
    fallback
        .as_object_mut()
        .expect("native body")
        .remove("blocks");
    if let Some(blocks) = body.get_mut("blocks").and_then(Value::as_array_mut) {
        for (index, block) in blocks.iter_mut().enumerate() {
            if original["body"]["blocks"][index]["payload"] == fallback
                || original["body"]["blocks"][index]["payload"] == json!({"body_ref":true})
            {
                block["payload"] = json!({"body_ref": true});
            }
        }
    }
    item.as_object_mut().expect("native item").remove("_source");
    item.as_object_mut()
        .expect("native item")
        .remove("_oversized_bytes");
    item.as_object_mut()
        .expect("native item")
        .remove("_oversized_payload_bytes");
    item.as_object_mut()
        .expect("native item")
        .remove("_preview_of_bytes");
    if serde_json::to_vec(item).map_err(ApiError::internal)?.len()
        > CLIENT_MAX_RESPONSE_BYTES - 128_000
    {
        let body = item["body"].as_object_mut().expect("native body");
        for (key, value) in body.iter_mut() {
            if key != "blocks" {
                bound_open_value(value);
                if !value.is_string() {
                    bound(value);
                }
            }
        }
        // Retained open-object keys can still exceed the transport budget.
        // Replace only this entry, preserving its identity and ordering.
        let size = serde_json::to_vec(item).map_err(ApiError::internal)?.len();
        if size > CLIENT_MAX_RESPONSE_BYTES - 128_000 {
            if item["type"] != "error" {
                item["type"] = json!("error");
                item["role"] = json!("system");
            }
            item["body"] = json!({
                "code":"native-entry-too-large",
                "message":format!("[st truncated this native timeline value: size limit; {size} bytes]"),
                "retryable":false,
                "details":{"size":size},
                "blocks":[{
                    "id":"native-entry-too-large",
                    "kind":"error",
                    "source_type":"native-entry-too-large",
                    "payload":{"body_ref":true},
                    "continuation":continuation(
                        reference(source, basis, session_id, &original, "/body"),
                        "application/json",
                        Some(serde_json::to_vec(&original["body"]).map_err(ApiError::internal)?.len()),
                        "size-limit",
                    )
                }]
            });
        }
    }
    if !session.conversation_blocks {
        item["body"]
            .as_object_mut()
            .expect("native body")
            .remove("blocks");
    }
    Ok(())
}

/// Keep negotiated data out of legacy relayed clients without changing their known enum.
pub fn legacy(value: &mut Value, session: &ClientSession) {
    if session.conversation_blocks {
        return;
    }
    if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(body) = item["body"].as_object_mut() {
                body.remove("blocks");
            }
        }
    }
    if let Some(value) = value.as_object_mut() {
        value.remove("header");
    }
}

/// Canonical stored-message text has no transcript locator. Authenticate its immutable
/// claim identity and session separately, then reuse the assistant grant wrapper above it.
pub fn message_reference(session: &str, claim: &str) -> String {
    use hmac::Mac as _;
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&(session, claim)).expect("message locator encodes"));
    let mut mac = <hmac::Hmac<sha2::Sha256> as hmac::Mac>::new_from_slice(&*REF_KEY).expect("reference key");
    mac.update(b"st-conversation-message-v1:");
    mac.update(payload.as_bytes());
    format!("message.{}.{}", hex::encode(mac.finalize().into_bytes()), payload)
}

fn message_text(state: &AppState, session: &str, wanted: &str, max_bytes: usize) -> Result<(String, Value), ApiError> {
    use hmac::Mac as _;
    let (signature, payload) = wanted.strip_prefix("message.").and_then(|value| value.split_once('.')).ok_or_else(invalidated)?;
    if wanted.len() > 4096 { return Err(invalidated()); }
    let mut mac = <hmac::Hmac<sha2::Sha256> as hmac::Mac>::new_from_slice(&*REF_KEY).expect("reference key");
    mac.update(b"st-conversation-message-v1:");
    mac.update(payload.as_bytes());
    mac.verify_slice(&hex::decode(signature).map_err(|_| invalidated())?).map_err(|_| invalidated())?;
    let (bound_session, claim): (String, String) = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).map_err(|_| invalidated())?)
        .map_err(|_| invalidated())?;
    if bound_session != session { return Err(invalidated()); }
    let (_, text) = state.store.conversation_message_text_bounded(&claim, max_bytes).map_err(ApiError::internal)?
        .ok_or_else(invalidated)?;
    let text = text.ok_or_else(|| ApiError {
        status: StatusCode::PAYLOAD_TOO_LARGE, code: "response-budget-exceeded".into(),
        message: "the referenced message text exceeds the assistant read budget".into(), details: Box::default(),
    })?;
    Ok((text, json!(1)))
}

/// Read authenticated canonical-message or native-record plain text within `record_bytes`.
/// Oversized text, attachments, images, and structured payloads are never copied to the caller.
pub fn assistant_text(
    state: &AppState,
    session_id: &str,
    wanted: &str,
    record_bytes: usize,
) -> Result<(String, Value), ApiError> {
    if wanted.starts_with("message.") { return message_text(state, session_id, wanted, record_bytes); }
    let locator = locator(wanted, session_id)?;
    if locator.image {
        return Err(not_text());
    }
    let native: crate::external_sessions::NativeLocator =
        serde_json::from_value(locator.native.clone()).map_err(|_| invalidated())?;
    let slots = read_slots(&locator.source.transcript);
    let _slot = read_slot(slots)?;
    // Each read resolves the managed owner at the current frontier, as chunk_local does.
    let current_source = || {
        let index = state.store.index().map_err(ApiError::internal)?;
        let managed = super::super::managed_session_owner_at(&state.store, index, session_id)
            .map_err(ApiError::internal)?;
        source(state, session_id, managed.as_ref())
    };
    let found = current_source()?;
    if basis(&found)? != locator.basis {
        return Err(invalidated());
    }
    let value = crate::external_sessions::normalized_record_bounded(&found, &native, record_bytes)
        .map_err(|error| {
            if error.chain().any(|cause| cause.is::<crate::external_sessions::RecordBudgetExceeded>()) {
                ApiError {
                    status: StatusCode::PAYLOAD_TOO_LARGE,
                    code: "response-budget-exceeded".into(),
                    message: "the referenced native record exceeds the assistant read budget".into(),
                    details: Box::default(),
                }
            } else {
                invalidated()
            }
        })?
        .into_iter()
        .find(|item| item["id"] == locator.entry && item["revision"] == locator.revision)
        .and_then(|mut item| item.pointer_mut(&locator.pointer).map(Value::take))
        .ok_or_else(invalidated)?;
    if basis(&current_source()?)? != locator.basis {
        return Err(invalidated());
    }
    let Value::Object(mut body) = value else {
        return Err(not_text());
    };
    if body.get("media_type").and_then(Value::as_str) != Some("text/plain") {
        return Err(not_text());
    }
    match body.remove("text") {
        Some(Value::String(text)) => Ok((text, locator.revision)),
        _ => Err(not_text()),
    }
}

fn not_text() -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "content-not-text".into(),
        message: "assistant content reads return plain text only, never attachments or structured payloads".into(),
        details: Box::default(),
    }
}


