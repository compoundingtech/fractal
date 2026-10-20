//! Closed-tool MCP adapter for the paired client gateway.
//! HTTP and stdio share the same handler and input contracts; neither exposes daemon control.
use axum::{Router, http::StatusCode, routing::get};
use rmcp::{ServerHandler, RoleServer, ErrorData, model::*, service::RequestContext};
use rmcp::transport::streamable_http_server::{StreamableHttpService, StreamableHttpServerConfig, session::local::LocalSessionManager};
use serde_json::{Value, json};
use std::{io::{self, Read}, path::Path, os::unix::fs::{OpenOptionsExt, MetadataExt}};

pub mod cli;

/// Wire-compatible adapter profiles. Authority always comes from the paired
/// credential's daemon grant, never from this local presentation setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Profile {
    server_name: &'static str,
    assistant_principal: &'static str,
    tool_descriptions: &'static [(&'static str, &'static str)],
    structured_output: bool,
}
impl Profile {
    // Keep profile selection spelled consistently with the original variants.
    #[allow(non_upper_case_globals)]
    pub const ChatGpt: Self = Self {
        server_name: "st-chatgpt",
        assistant_principal: "assistant/chatgpt",
        tool_descriptions: &[],
        structured_output: false,
    };
    #[allow(non_upper_case_globals)]
    pub const Mcp: Self = Self {
        server_name: "st",
        assistant_principal: "assistant/mcp",
        tool_descriptions: &[
            ("st_message_send", "Send one exact message to an existing agent under write.assistant.messages and the paired assistant/mcp grant. Obtain human confirmation before calling. Person recipients are refused."),
            ("st_alert_answer", "Execute one typed person-ask answer under write.assistant.answers and the paired assistant/mcp grant using the alert's current episode and fence. Obtain human confirmation before calling."),
        ],
        structured_output: true,
    };
    pub const fn server_name(self) -> &'static str { self.server_name }
    pub const fn assistant_principal(self) -> &'static str { self.assistant_principal }
    const fn structured_output(self) -> bool { self.structured_output }
    fn description(self, name: &str, legacy: &'static str) -> &'static str {
        self.tool_descriptions.iter().find(|(tool, _)| *tool == name)
            .map_or(legacy, |(_, description)| *description)
    }
}

#[derive(Clone)]
pub struct Adapter { client: reqwest::Client, bearer: String, profile: Profile }
impl Adapter {
    pub fn new(socket: &Path, credential_file: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_profile(socket, credential_file, Profile::ChatGpt)
    }
    pub fn with_profile(socket: &Path, credential_file: &Path, profile: Profile) -> Result<Self, Box<dyn std::error::Error>> {
        if socket.file_name().and_then(|name| name.to_str()) != Some("st3-client.sock") {
            return Err("only the paired gateway st3-client.sock is permitted".into());
        }
        let bearer = read_credential(credential_file)?;
        let client = reqwest::Client::builder().unix_socket(socket).build()?;
        Ok(Self { client, bearer, profile })
    }
}

/// Read an owner-only regular file without following a final-component symlink.
/// Check the opened descriptor, not path metadata, and bound reads even if the file grows.
fn read_credential(path: &Path) -> io::Result<String> {
    const MAX_BYTES: u64 = 4096;
    let file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid takes no arguments and has no memory-safety preconditions.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 || metadata.len() > MAX_BYTES {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "credential must be an owner-only regular file of at most 4096 bytes"));
    }
    let mut source = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut source)?;
    let bearer = source.trim();
    if source.len() > MAX_BYTES as usize || bearer.is_empty()
        || !bearer.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "credential must contain one nonempty ASCII bearer token"));
    }
    Ok(bearer.to_owned())
}

const TOOLS: &[(&str, &str, bool)] = &[
    ("st_alerts_list", "List bounded owner alerts with actionable question/options, episode, target and evidence when present. Set agent_id to an exact canonical agent ID to select its published alerts. An empty page does not mean no work. Use st_alert_get only when further exact detail is needed.", true),
    ("st_alert_get", "Read one exact owner alert, including its episode and named answer options.", true),
    ("st_alert_answer", "Execute one typed person-ask answer under write.assistant.answers using the alert's current episode and fence.", false),
    ("st_missions_list", "List mission metadata.", true),
    ("st_agents_list", "List agent metadata. For a known canonical agent ID, set agent_id to avoid paging through the fleet. This does not read priorities or conversation content.", true),
    ("st_conversations_list", "Discover source-bounded conversation metadata and read handles without conversation content. Each value.items entry supplies agent_id and session_id for st_conversation_timeline and a read object containing all st_conversation_read arguments (null when the bounded current timeline page has no readable text handle; use timeline to continue). Pass read unchanged to st_conversation_read; use value.page.next_cursor to page this list. Pages fit max_response_bytes; an item that cannot fit alone is named in the error.", true),
    ("st_conversation_detail", "Read one agent's current conversation with bounded chat excerpts and revision-pinned read handles. Use agent_id from st_conversations_list; pass each non-null timeline item's read object unchanged to st_conversation_read. Content may be sensitive.", true),
    ("st_conversation_timeline", "Page bounded chat excerpts and revision-pinned read handles. Copy agent_id and session_id from st_conversations_list to select the discovered session; omit session_id only to select the current session. Pass each non-null value.items[].read object unchanged to st_conversation_read, and use value.page.next_cursor only to continue this timeline. Content may be sensitive.", true),
    ("st_conversation_search", "Search bounded plain conversation text snippets. Content may be sensitive.", true),
    ("st_conversation_read", "Read one bounded revision-pinned conversation content range. Pass a non-null value.items[].read object from st_conversations_list or st_conversation_timeline unchanged; conversation detail also supplies timeline read handles. Each read object contains agent_id, session_id, reference, revision, start and end. Never invent handles or substitute a pagination cursor for a content reference. Content may contain sensitive material.", true),
    ("st_message_send", "Send one exact message to any existing agent, authorized by write.assistant.messages and the ChatGPT confirmation card. Person recipients are refused.", false),
];

// Discovery and local admission share these rules; ASCII keys also satisfy daemon byte bounds.
const ACTION_ID_PREFIX: &str = "action/";
const IDEMPOTENCY_KEY_MIN_LENGTH: usize = 16;
const IDEMPOTENCY_KEY_MAX_LENGTH: usize = 256;
const IDEMPOTENCY_KEY_CHAR_RANGE: std::ops::RangeInclusive<u8> = b' '..=b'~';

fn idempotency_key_pattern() -> String {
    format!("^[{}-{}]{{{IDEMPOTENCY_KEY_MIN_LENGTH},{IDEMPOTENCY_KEY_MAX_LENGTH}}}$",
        char::from(*IDEMPOTENCY_KEY_CHAR_RANGE.start()), char::from(*IDEMPOTENCY_KEY_CHAR_RANGE.end()))
}

fn action_id_schema() -> Value {
    json!({"type":"string","pattern":format!("^{ACTION_ID_PREFIX}"),
        "description":"Unique per intended action, e.g. action/<short-slug>; reuse the same value on a retry"})
}
fn idempotency_key_schema() -> Value {
    json!({"type":"string","pattern":idempotency_key_pattern(),
        "description":format!("{IDEMPOTENCY_KEY_MIN_LENGTH}-{IDEMPOTENCY_KEY_MAX_LENGTH} printable ASCII chars only (stricter than daemon byte-length bounds); reuse the exact value on retries of the same action")})
}
fn fence_schema() -> Value {
    json!({"type":"object","description":"Use the snapshot returned by the preceding read. Copy subject revisions from that snapshot to fence writes against changed resources.",
        "properties":{
            "snapshot_id":{"type":"string","description":"The snapshot id from the immediately preceding read result."},
            "subject_revisions":{"type":"object","description":"Resource subject identifiers mapped to their opaque revision strings; omitted means no subject-specific revisions.","additionalProperties":{"type":"string"}}
        },"required":["snapshot_id"],"additionalProperties":false})
}
fn schema(name: &str) -> Value {
    let props = match name {
        "st_alert_answer" => json!({"action_id":action_id_schema(),"idempotency_key":idempotency_key_schema(),"fence":fence_schema(),"alert_id":{"type":"string"},"episode":{"type":"string"},"option":{"type":"string"},"summary":{"type":"string"}}),
        "st_message_send" => json!({"action_id":action_id_schema(),"idempotency_key":idempotency_key_schema(),"fence":fence_schema(),"to":{"type":"string"},"title":{"type":"string"},"content":{"type":"string"}}),
        "st_alert_get" => json!({"alert_id":{"type":"string"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        "st_conversation_detail" => json!({"agent_id":{"type":"string"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        "st_conversation_timeline" => json!({"agent_id":{"type":"string","description":"Copy value.items[].agent_id from st_conversations_list."},"session_id":{"type":"string","description":"Copy the same conversation item's session_id to pin the discovered session; omitted selects the current session."},"cursor":{"type":"string","description":"Copy value.page.next_cursor from the preceding timeline page; omit for the first page."},"limit":{"type":"integer","minimum":1},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        "st_conversation_search" => json!({"text":{"type":"string"},"agent_id":{"type":"string"},"cursor":{"type":"string"},"limit":{"type":"integer","minimum":1},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        "st_conversation_read" => json!({"agent_id":{"type":"string","description":"Copy read.agent_id from a discovered conversation or timeline item."},"session_id":{"type":"string","description":"Copy read.session_id from the same handle; do not infer from agent_id."},"reference":{"type":"string","description":"Copy read.reference, an opaque content reference, not a pagination cursor."},"revision":{"type":"string","description":"Copy read.revision paired with this content reference to pin the source version."},"start":{"type":"integer","minimum":0,"description":"Copy read.start, the inclusive range start."},"end":{"type":"integer","minimum":0,"description":"Copy read.end, the exclusive range end."},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        "st_alerts_list" => json!({"agent_id":{"type":"string","pattern":"^agent/[A-Za-z0-9/._:@+=~-]+$","maxLength":256,"description":"Exact canonical agent ID, not an alias or display name."},"limit":{"type":"integer","minimum":1},"cursor":{"type":"string"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        "st_agents_list" => json!({"agent_id":{"type":"string","pattern":"^agent/[A-Za-z0-9/._:@+=~-]+$","maxLength":256,"description":"Exact canonical agent ID, not an alias or display name."},"limit":{"type":"integer","minimum":1},"cursor":{"type":"string"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
        _ => json!({"limit":{"type":"integer","minimum":1},"cursor":{"type":"string"},"max_response_bytes":{"type":"integer","minimum":1,"maximum":16384}}),
    };
    let required = match name {
        "st_alert_answer" => vec!["action_id","idempotency_key","fence","alert_id","episode","option","summary"],
        "st_message_send" => vec!["action_id","idempotency_key","fence","to","title","content"],
        "st_alert_get" => vec!["alert_id"],
        "st_conversation_detail" | "st_conversation_timeline" => vec!["agent_id"],
        "st_conversation_search" => vec!["text"],
        "st_conversation_read" => vec!["agent_id","session_id","reference","revision","start","end"],
        _ => vec![],
    };
    json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
}

/// Validate the structural JSON Schema vocabulary used by the published contracts.
/// Action string constraints are checked by validate_arguments with rule-specific errors.
fn matches_schema(value: &Value, contract: &Value) -> bool {
    match contract["type"].as_str() {
        Some("string") => value.is_string(),
        Some("integer") => value.as_u64().is_some_and(|number| {
            contract["minimum"].as_u64().is_none_or(|minimum| number >= minimum)
                && contract["maximum"].as_u64().is_none_or(|maximum| number <= maximum)
        }),
        Some("object") => value.as_object().is_some_and(|object| {
            let required_present = contract["required"].as_array().is_none_or(|required| {
                required.iter().all(|key| key.as_str().is_some_and(|key| object.contains_key(key)))
            });
            required_present && object.iter().all(|(key, value)| {
                if let Some(property) = contract["properties"].get(key) {
                    matches_schema(value, property)
                } else {
                    match &contract["additionalProperties"] {
                        Value::Bool(false) => false,
                        Value::Object(_) => matches_schema(value, &contract["additionalProperties"]),
                        _ => false,
                    }
                }
            })
        }),
        _ => false,
    }
}
fn validate_arguments(name: &str, args: &Value) -> Result<(), Value> {
    if !TOOLS.iter().any(|(tool, _, _)| *tool == name) {
        return Err(json!({"code":"unknown-tool"}));
    }
    if !matches_schema(args, &schema(name)) {
        return Err(json!({"code":"invalid-arguments","message":"Use the exact published tool schema."}));
    }
    if matches!(name, "st_alerts_list" | "st_agents_list")
        && let Some(id) = args.get("agent_id").and_then(Value::as_str)
        && (!(7..=256).contains(&id.len()) || !id.starts_with("agent/")
            || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"/._:-@+=~".contains(&byte))) {
        return Err(json!({"code":"invalid-arguments","message":"agent_id must be an exact canonical agent identifier, not an alias."}));
    }
    if matches!(name, "st_alert_answer" | "st_message_send") {
        let action_id = args["action_id"].as_str().expect("validated action ID string");
        if !action_id.starts_with(ACTION_ID_PREFIX) {
            return Err(json!({"code":"invalid-arguments",
                "message":format!("action_id must match ^{ACTION_ID_PREFIX} (start with {ACTION_ID_PREFIX}).")}));
        }
        let key = args["idempotency_key"].as_str().expect("validated idempotency key string");
        if !(IDEMPOTENCY_KEY_MIN_LENGTH..=IDEMPOTENCY_KEY_MAX_LENGTH).contains(&key.len())
            || !key.bytes().all(|byte| IDEMPOTENCY_KEY_CHAR_RANGE.contains(&byte)) {
            return Err(json!({"code":"invalid-arguments",
                "message":format!("idempotency_key must match {}: {IDEMPOTENCY_KEY_MIN_LENGTH}-{IDEMPOTENCY_KEY_MAX_LENGTH} printable ASCII chars only.", idempotency_key_pattern())}));
        }
    }
    Ok(())
}
fn tool_definitions(profile: Profile) -> Vec<Tool> {
    TOOLS.iter().map(|(name,description,read)| {
        let mut definition = json!({
            "name":name,"description":profile.description(name, description),"inputSchema":schema(name),
            "annotations":{"readOnlyHint":read,"destructiveHint":*name=="st_alert_answer","idempotentHint":true,"openWorldHint":*name=="st_message_send"}
        });
        if profile.structured_output() { definition["outputSchema"] = output_schema(name); }
        serde_json::from_value(definition).expect("static tool definition")
    }).collect()
}

/// Describe the gateway envelope, retaining its provenance rather than wrapping it
/// in a second success envelope. Item records remain extensible daemon contracts.
fn output_schema(name: &str) -> Value {
    let page = json!({"type":"object","properties":{
        "limit":{"type":["integer","null"]},"has_more":{"type":"boolean"},
        "next_cursor":{"type":["string","null"]},"cursor_expires_at":{"type":["string","null"]}
    },"required":["limit","has_more","next_cursor"]});
    let value = match name {
        "st_alert_get" => json!({"type":"object","properties":{
            "id":{"type":["string","null"]},"episode":{"type":["string","null"]},
            "kind":{"type":["string","null"]},"title":{},"detail":{},"request":{}
        },"required":["id","episode","kind","title","detail","request"]}),
        "st_alert_answer" | "st_message_send" => json!({"type":"object","properties":{
            "kind":{"const":"action-result"},"action_id":{"type":"string"},
            "operation_id":{"type":"string"},"status":{"type":"string"},
            "affected_ids":{"type":"array","items":{"type":"string"}},"snapshot_id":{"type":"string"}
        },"required":["kind","action_id","status"]}),
        "st_conversation_read" => json!({"type":"object","properties":{
            "kind":{"const":"assistant-conversation-content"},"agent_id":{"type":"string"},
            "session_id":{"type":"string"},"reference":{"type":"string"},"revision":{"type":"string"},
            "start":{"type":"integer","minimum":0},"end":{"type":"integer","minimum":0},
            "text":{"type":"string"},"returned_bytes":{"type":"integer","minimum":0},
            "truncated":{"type":"boolean"},"next_range":{"type":["object","null"]}
        },"required":["kind","agent_id","session_id","reference","revision","start","end","text","returned_bytes","truncated","next_range"]}),
        _ => {
            let mut result = json!({"type":"object","properties":{
                "kind":{"type":"string"},"items":{"type":"array","items":{"type":"object"}},"page":page
            },"required":["kind","items","page"]});
            if matches!(name, "st_conversation_detail" | "st_conversation_timeline") {
                result["properties"]["kind"] = json!({"const":if name == "st_conversation_detail" {"conversation-detail"} else {"conversation-timeline"}});
                for key in ["agent_id", "session_id", "current_session_id"] {
                    result["properties"][key] = json!({"type":"string"});
                    result["required"].as_array_mut().expect("array").push(json!(key));
                }
            }
            else {
                result["properties"]["kind"] = json!({"const":if name == "st_conversation_search" {"conversation-search"} else {"page"}});
            }
            result
        }
    };
    json!({"type":"object","title":format!("{name} result"),"oneOf":[
        {"type":"object","properties":{
            "api_version":{"const":"st3.client.v0"},"request_id":{"type":"string"},
            "snapshot":{"type":"object","properties":{"id":{"type":"string"},"host_id":{"type":"string"},"store_index":{"type":"integer","minimum":0}}},
            "value":value
        },"required":["api_version","request_id","snapshot","value"]},
        {"type":"object","properties":{
            "code":{"enum":["unknown-tool","invalid-arguments","unavailable","invalid-daemon-response","incomplete","daemon-denied"]},
            "message":{"type":"string"},"http_status":{"type":"integer"},"daemon":{}
        },"required":["code"],"additionalProperties":false}
    ]})
}

fn tool_result(profile: Profile, value: Value, error: bool) -> CallToolResult {
    if profile.structured_output() {
        // MCP 2025-11-25 tools/structured-content SHOULD also serialize the
        // JSON in a TextContent block for older clients. rmcp 3.5.1's structured
        // constructors implement that exact mirror for success and errors.
        if error { CallToolResult::structured_error(value) } else { CallToolResult::structured(value) }
    } else if error { CallToolResult::error(vec![ContentBlock::text(value.to_string())]) }
    else { CallToolResult::success(vec![ContentBlock::text(value.to_string())]) }
}
/// Only closed tool names and locally selected codes enter telemetry. A dropped
/// dispatch future emits cancellation too, without retaining any tool arguments.
struct CallObservation {
    tool: &'static str,
    profile: Profile,
    started: std::time::Instant,
    outcome: &'static str,
    code: &'static str,
    request_id: Option<String>,
    response_bytes: Option<u64>,
}
impl CallObservation {
    fn new(name: &str, profile: Profile) -> Self {
        Self {
            profile,
            tool: TOOLS.iter().find(|(tool, _, _)| *tool == name)
                .map_or("unknown", |(tool, _, _)| *tool),
            started: std::time::Instant::now(),
            outcome: "cancelled",
            code: "cancelled",
            request_id: None,
            response_bytes: None,
        }
    }
    fn response(&mut self, value: &Value) {
        // Daemon IDs are request/<UUID>. Reject arbitrary text even in this
        // metadata field: a compromised/malformed gateway must not inject logs.
        if let Some(id) = value["request_id"].as_str()
            && let Some(uuid) = id.strip_prefix("request/")
            && uuid.len() == 36
            && uuid.bytes().enumerate().all(|(i, byte)| {
                if matches!(i, 8 | 13 | 18 | 23) { byte == b'-' }
                else { byte.is_ascii_hexdigit() }
            })
        {
            self.request_id = Some(id.to_owned());
        }
    }
}
impl Drop for CallObservation {
    fn drop(&mut self) {
        tracing::info!(target: "st_chatgpt::call",
            tool = self.tool, outcome = self.outcome, code = self.code,
            daemon_request_id = self.request_id.as_deref(),
            duration_ms = self.started.elapsed().as_secs_f64() * 1000.0,
            response_bytes = self.response_bytes,
            "{}", if self.profile.structured_output() { "MCP tool call finished" } else { "ChatGPT tool call finished" });
    }
}
async fn call(adapter: &Adapter, name: &str, args: &Value) -> Result<Value,Value> {
    let mut observation = CallObservation::new(name, adapter.profile);
    let result = dispatch(adapter, name, args, &mut observation).await;
    observation.outcome = if result.is_ok() { "success" } else { "error" };
    observation.code = match &result {
        Ok(_) => "ok",
        Err(value) => match value["code"].as_str() {
            Some("unknown-tool") => "unknown-tool",
            Some("invalid-arguments") => "invalid-arguments",
            Some("unavailable") => "unavailable",
            Some("invalid-daemon-response") => "invalid-daemon-response",
            Some("incomplete") => "incomplete",
            _ => "daemon-denied",
        },
    };
    result
}
async fn dispatch(adapter: &Adapter, name: &str, args: &Value, observation: &mut CallObservation) -> Result<Value,Value> {
    validate_arguments(name, args)?;
    let object = args.as_object().expect("validated tool object");
    let write = matches!(name,"st_alert_answer"|"st_message_send");
    let route = match name {
        "st_alerts_list" => "attention-rich",
        "st_alert_get" => "attention-content",
        "st_missions_list" => "missions-rich",
        "st_agents_list" => "agents-rich",
        "st_conversations_list" => "conversations",
        "st_conversation_detail" => "conversation-detail",
        "st_conversation_timeline" => "conversation-timeline",
        "st_conversation_search" => "conversation-search",
        "st_conversation_read" => "conversation-content",
        _ => "propose",
    };
    let url = format!("http://localhost/v1/client/assistant/{route}");
    let request = if write {
        let parameters = if name == "st_alert_answer" {
            json!({"target_id":args["alert_id"],"episode":args["episode"],"summary":args["summary"],"answer":{"id":args["option"]}})
        } else {json!({"to":args["to"],"title":args["title"],"content":args["content"]})};
        adapter.client.post(url).json(&json!({"api_version":"st3.client.v0","id":args["action_id"],"type":if name=="st_alert_answer" {"work.done"} else {"message.send"},"idempotency_key":args["idempotency_key"],"fence":args["fence"],"parameters":parameters}))
    } else {
        let query: Vec<_> = object.iter().map(|(k,v)| (k.as_str(),v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string()))).collect();
        adapter.client.get(url).query(&query)
    };
    let response = request.bearer_auth(&adapter.bearer).send().await.map_err(|_| json!({"code":"unavailable","message":"The paired st daemon is unavailable."}))?;
    let status = response.status();
    // reqwest's JSON decoder already buffers the body. Reuse that exact buffer
    // for byte accounting, without reserializing content or trusting Content-Length.
    let bytes = response.bytes().await.map_err(|_|json!({"code":"invalid-daemon-response"}))?;
    observation.response_bytes = Some(bytes.len() as u64);
    let value: Value = serde_json::from_slice(&bytes).map_err(|_|json!({"code":"invalid-daemon-response"}))?;
    if adapter.profile.structured_output() && !value.is_object() {
        return Err(json!({"code":"invalid-daemon-response"}));
    }
    observation.response(&value);
    if status.is_success() {Ok(value)} else {Err(json!({"code":if status.as_u16()==413 {"incomplete"} else {"daemon-denied"},"http_status":status.as_u16(),"daemon":value}))}
}
impl ServerHandler for Adapter {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(self.profile.server_name(),env!("CARGO_PKG_VERSION")))
    }
    async fn list_tools(&self, _:Option<PaginatedRequestParams>, _:RequestContext<RoleServer>) -> Result<ListToolsResult,ErrorData> {
        Ok(ListToolsResult { tools: tool_definitions(self.profile), ..Default::default() })
    }
    async fn call_tool(&self, request:CallToolRequestParams, _:RequestContext<RoleServer>) -> Result<CallToolResponse,ErrorData> {
        let args=request.arguments.map(Value::Object).unwrap_or_else(||json!({}));
        let (value,error)=match call(self,&request.name,&args).await {Ok(v)=>(v,false),Err(v)=>(v,true)};
        let result=tool_result(self.profile,value,error);
        Ok(result.into())
    }
}
/// Stateless Streamable HTTP: only POST /mcp is a protocol endpoint.
pub fn http_router(adapter: Adapter) -> Router {
    let service=StreamableHttpService::new(move ||Ok(adapter.clone()),std::sync::Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_legacy_session_mode(false).with_json_response(true));
    Router::new().route("/mcp",get(||async {StatusCode::METHOD_NOT_ALLOWED}).post_service(service))
        .fallback(||async {StatusCode::NOT_FOUND})
}

#[cfg(test)]
mod tests;
