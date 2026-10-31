//! Foreground phone commands are member-local, bounded, ephemeral correlation records.
//! No claims, durable receipts, replication, or replay participate in this protocol.
use super::client_v0::{acting_party, forbidden, paired_client_session, require_scope, revalidate_session, validation, ClientSession};
use super::*;
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

pub const MAX_BODY_BYTES: usize = 16 * 1024;
pub const MAX_PENDING_PER_DEVICE: usize = 8;
pub const MAX_COMMANDS: usize = 2048;
pub const RETENTION_MS: u128 = 5 * 60 * 1000;
static ROWS: LazyLock<Mutex<BTreeMap<(String, String), Command>>> = LazyLock::new(Mutex::default);
static CHANGES: LazyLock<tokio::sync::broadcast::Sender<String>> = LazyLock::new(|| tokio::sync::broadcast::channel(64).0);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCommandRequest {
    pub api_version: String,
    pub device: String,
    pub kind: String,
    pub args: Value,
    pub ttl_ms: Option<u64>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceResultRequest {
    pub command_id: String,
    pub outcome: String,
    pub result: Option<Value>,
    pub error_code: Option<String>,
}
#[derive(Clone)]
struct Command {
    resource: Value,
    expires: u128,
    terminal_at: Option<u128>,
    person: String,
    grant: String,
    issuer_grant: Option<String>,
}
impl Command {
    fn terminal(&self) -> bool { self.terminal_at.is_some() }
    fn stamp(&mut self, at: u128) {
        self.resource["revision"] = json!(format!("{}:{}", self.resource["id"].as_str().unwrap_or_default(), self.resource["state"].as_str().unwrap_or_default()));
        self.resource["updated_at"] = json!(client_timestamp(at));
    }
    fn expire(&mut self, now: u128, foreground: bool) {
        if !self.terminal() && now >= self.expires {
            self.resource["state"] = json!(if foreground { "expired" } else { "not-foreground" });
            self.terminal_at = Some(self.expires);
            self.stamp(self.expires);
        }
    }
    fn finish(&mut self, request: &DeviceResultRequest, now: u128, foreground: bool) {
        self.expire(now, foreground);
        if self.terminal() { return; }
        self.resource["state"] = json!(request.outcome);
        if let Some(result) = &request.result { self.resource["result"] = result.clone(); }
        if let Some(code) = &request.error_code { self.resource["error_code"] = json!(code); }
        self.terminal_at = Some(now);
        self.stamp(now);
    }
}
pub fn subscribe() -> tokio::sync::broadcast::Receiver<String> { CHANGES.subscribe() }
fn changed(member: &str) { let _ = CHANGES.send(member.to_owned()); }
fn storage_error() -> ApiError { ApiError::internal("local device command memory is unavailable") }
fn missing() -> ApiError { ApiError { status: StatusCode::NOT_FOUND, code: "not-found".into(), message: "device command not found".into(), details: Box::default() } }
fn capacity() -> ApiError { ApiError { status: StatusCode::TOO_MANY_REQUESTS, code: "resource-exhausted".into(), message: "device command capacity exceeded".into(), details: Box::default() } }
pub fn scope(kind: &str) -> Result<&'static str, ApiError> {
    match kind { "phone.inspect" | "phone.navigate" => Ok("read.projections"), "phone.message.read" => Ok("control.messages"), _ => Err(validation("unknown phone command kind")) }
}
pub fn paired(session: &ClientSession) -> bool { session.pairing_grant.is_some() && session.pairing_issued_here && session.transport != "unix" }
pub fn capable(session: &ClientSession) -> bool { paired(session) || acting_party(session) && (session.allows("read.projections") || session.allows("control.messages")) }
fn bounded_string(value: &Value) -> bool { value.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)) }
fn fields(value: &Value, required: &[&str], optional: &[&str]) -> bool {
    value.as_object().is_some_and(|o| required.iter().all(|k| o.contains_key(*k)) && o.keys().all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str())))
}
/// Validate closed, correlated argument shapes before any row is admitted.
pub fn validate_args(kind: &str, args: &Value) -> Result<(), ApiError> {
    scope(kind)?;
    let valid = match kind {
        "phone.inspect" => fields(args, &[], &[]),
        "phone.message.read" => fields(args, &["attention_id"], &[]) && bounded_string(&args["attention_id"]) && args["attention_id"].as_str().is_some_and(|s| s.starts_with("attention/") && s.len() > "attention/".len()),
        "phone.navigate" if fields(args, &["screen"], &["params"]) => {
            let empty = json!({});
            let params = args.get("params").unwrap_or(&empty);
            match args["screen"].as_str().unwrap_or_default() {
                "NowRoot" | "Agents" | "Missions" | "Usage" | "History" => !args.as_object().is_some_and(|o| o.contains_key("params")),
                "Documents" => fields(params, &[], &["prefix"]) && params.get("prefix").is_none_or(bounded_string),
                "Mission" | "Attention" | "Launch" => fields(params, &["id"], &[]) && bounded_string(&params["id"]),
                "Conversation" => fields(params, &["target"], &["sessionId"]) && bounded_string(&params["target"]) && params.get("sessionId").is_none_or(bounded_string),
                "Terminal" => fields(params, &["terminalId"], &[]) && bounded_string(&params["terminalId"]),
                "Document" => fields(params, &["reference", "trail"], &[]) && bounded_string(&params["reference"]) && params["trail"].as_array().is_some_and(|a| a.len() <= 32 && a.iter().all(bounded_string)),
                _ => false,
            }
        }
        _ => false,
    };
    if valid && serde_json::to_vec(args).map_err(|_| validation("invalid command args"))?.len() <= 8192 { Ok(()) } else { Err(validation("invalid phone command args")) }
}
fn sweep(rows: &mut BTreeMap<(String, String), Command>, member: &str, now: u128) {
    for ((node, _), row) in rows.iter_mut().filter(|((node, _), _)| node == member) {
        row.expire(now, super::client_presence::has_open_stream(node, row.resource["session_actor"].as_str().unwrap_or_default()));
    }
    rows.retain(|_, row| row.terminal_at.is_none_or(|at| now.saturating_sub(at) < RETENTION_MS));
}
fn admit(rows: &BTreeMap<(String, String), Command>, member: &str, device: &str) -> Result<(), ApiError> {
    if rows.len() >= MAX_COMMANDS || rows.iter().filter(|((node, _), row)|
        node == member && row.resource["device_id"] == device && !row.terminal()
    ).count() >= MAX_PENDING_PER_DEVICE { Err(capacity()) } else { Ok(()) }
}
fn visible(row: &Command, session: &ClientSession) -> bool {
    if session.pairing_grant.is_some() { paired(session) && row.resource["session_actor"] == session.actor && session.pairing_grant.as_deref() == Some(row.grant.as_str()) }
    else { row.resource["issued_by"] == session.actor && row.issuer_grant.is_none() && (session.authority_actor == row.person || session.transport == "unix" && session.authority_actor.starts_with("agent/") && acting_party(session)) }
}
fn authorize_read(row: &Command, session: &ClientSession) -> Result<(), ApiError> {
    if !visible(row, session) { return Err(missing()); }
    if !paired(session) { require_scope(session, scope(row.resource["command_kind"].as_str().unwrap_or_default())?)?; }
    Ok(())
}
/// Synchronous admission surface used by HTTP and integration tests; the daemon owns the clock.
/// Requires a running Tokio runtime to own deadline and retention wakeups.
pub fn issue(state: &AppState, session: &ClientSession, request: DeviceCommandRequest) -> Result<Value, ApiError> {
    let session = revalidate_session(state, session)?;
    if !super::wire_names::api_version(&request.api_version) { return Err(validation("unsupported api_version")); }
    if !request.device.starts_with("device/") || request.device.len() <= "device/".len() || request.device.len() > 4096 { return Err(validation("invalid device selector")); }
    validate_args(&request.kind, &request.args)?;
    require_scope(&session, scope(&request.kind)?)?;
    if !acting_party(&session) { return Err(forbidden("device commands require a concrete person or local agent")); }
    let ttl = request.ttl_ms.unwrap_or(30_000);
    if !(1..=60_000).contains(&ttl) { return Err(validation("ttl_ms must be 1 through 60000")); }
    let mut before = None;
    let mut target: Option<ClaimRecord> = None;
    loop {
        let page = state.store.claims_for_kind_at("custom.client.pairing-completed", before, true, 256).map_err(ApiError::internal)?;
        for claim in page.claims {
            if claim.body.pointer("/fields/device_id").and_then(Value::as_str) == Some(request.device.as_str())
                && target.as_ref().is_none_or(|old| claim.store_index > old.store_index) { target = Some(claim); }
        }
        let Some(cursor) = page.next_cursor else { break; }; before = Some(cursor);
    }
    let target = target.ok_or_else(|| forbidden("device is not an active local paired phone"))?;
    if target.origin != state.store.origin() { return Err(forbidden("device pairing belongs to another member")); }
    let phone = paired_client_session(state, &target, "gateway", false)?;
    if session.authority_actor != phone.authority_actor && !(session.transport == "unix" && session.authority_actor.starts_with("agent/") && acting_party(&session)) { return Err(forbidden("cannot command another person's device")); }
    let now = client_now_ms();
    let mut suffix = crate::model::new_request_id();
    // The existing generator supplies random UUID bits; commands use the v8 namespace.
    suffix.replace_range(14..15, "8");
    let id = format!("device-command/{suffix}");
    let resource = json!({"id":id,"kind":"device-command","revision":format!("{id}:pending"),"updated_at":client_timestamp(now),"command_kind":request.kind,"args":request.args,"device_id":request.device,"session_actor":phone.actor,"issued_by":session.actor,"issued_at":client_timestamp(now),"expires_at":client_timestamp(now + u128::from(ttl)),"state":"pending"});
    {
        let mut rows = ROWS.lock().map_err(|_| storage_error())?;
        sweep(&mut rows, &state.node, now);
        admit(&rows, &state.node, &request.device)?;
        rows.insert((state.node.clone(), id.clone()), Command { resource: resource.clone(), expires: now + u128::from(ttl), terminal_at: None, person: phone.authority_actor, grant: target.subject, issuer_grant: session.pairing_grant.clone() });
    }
    changed(&state.node);
    let member = state.node.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(ttl)).await;
        if let Ok(mut rows) = ROWS.lock() { sweep(&mut rows, &member, client_now_ms()); }
        changed(&member);
        tokio::time::sleep(Duration::from_millis(RETENTION_MS as u64)).await;
        if let Ok(mut rows) = ROWS.lock() { rows.remove(&(member.clone(), id)); }
        changed(&member);
    });
    Ok(resource)
}
pub fn read(state: &AppState, session: &ClientSession, id: &str) -> Result<Value, ApiError> {
    let session = revalidate_session(state, session)?;
    let mut rows = ROWS.lock().map_err(|_| storage_error())?;
    sweep(&mut rows, &state.node, client_now_ms());
    let row = rows.get(&(state.node.clone(), id.to_owned())).ok_or_else(missing)?;
    // Paired callers may also be issuers. GET provides their correlation surface,
    // while their collection window remains strictly addressed to their own phone.
    if row.resource["issued_by"] == session.actor
        && row.issuer_grant == session.pairing_grant
        && (row.person == session.authority_actor || session.transport == "unix" && session.authority_actor.starts_with("agent/") && acting_party(&session))
    {
        require_scope(&session, scope(row.resource["command_kind"].as_str().unwrap_or_default())?)?;
    } else {
        authorize_read(row, &session)?;
    }
    Ok(row.resource.clone())
}
pub fn report(state: &AppState, session: &ClientSession, request: DeviceResultRequest) -> Result<Value, ApiError> {
    if !paired(session) { return Err(forbidden("device results require a paired session issued by this member")); }
    let session = revalidate_session(state, session)?;
    if !paired(&session) { return Err(forbidden("device results require a paired session issued by this member")); }
    if !matches!(request.outcome.as_str(), "completed" | "not-foreground" | "expired" | "refused" | "failed")
        || request.error_code.as_ref().is_some_and(|s| s.is_empty() || s.len() > 128 || !s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')))
        || request.result.as_ref().is_some_and(|v| serde_json::to_vec(v).map_or(true, |b| b.len() > 8192)) { return Err(validation("invalid device result")); }
    let resource = {
        let mut rows = ROWS.lock().map_err(|_| storage_error())?;
        let row = rows.get_mut(&(state.node.clone(), request.command_id.clone())).ok_or_else(missing)?;
        authorize_read(row, &session)?;
        row.finish(&request, client_now_ms(), super::client_presence::has_open_stream(&state.node, &session.actor));
        row.resource.clone()
    };
    changed(&state.node);
    Ok(resource)
}
/// A paired phone sees only its own exact grant; other issuers see only their own rows.
/// Reading a phone's window marks live rows delivered, never redelivering expired work.
pub fn collection_items(state: &AppState, session: &ClientSession) -> Result<Vec<Value>, ApiError> {
    let session = revalidate_session(state, session)?;
    if !capable(&session) { return Err(forbidden("phone commands are unavailable for this session")); }
    let mut rows = ROWS.lock().map_err(|_| storage_error())?;
    sweep(&mut rows, &state.node, client_now_ms());
    let mut items = Vec::new();
    let mut delivered = false;
    let mut selected = rows.iter_mut()
        .filter(|((member, _), row)| member == &state.node && visible(row, &session)
            && (!paired(&session) || !row.terminal()))
        .map(|(_, row)| row).collect::<Vec<_>>();
    // A bounded newest-first issuer window keeps retained correlation history from
    // becoming an oversized frame. Individual resources remain available through GET.
    selected.sort_by(|a, b| b.resource["issued_at"].as_str().cmp(&a.resource["issued_at"].as_str())
        .then_with(|| b.resource["id"].as_str().cmp(&a.resource["id"].as_str())));
    for row in selected.into_iter().take(32) {
        authorize_read(row, &session)?;
        if paired(&session) && row.resource["state"] == "pending" { row.resource["state"] = json!("delivered"); row.stamp(client_now_ms()); delivered = true; }
        items.push(row.resource.clone());
    }
    if delivered { changed(&state.node); }
    Ok(items)
}
pub async fn issue_route(State(state): State<AppState>, Extension(session): Extension<ClientSession>, Json(request): Json<DeviceCommandRequest>) -> Result<Json<Value>, ApiError> { issue(&state, &session, request).map(Json) }
pub async fn result_route(State(state): State<AppState>, Extension(session): Extension<ClientSession>, Json(request): Json<DeviceResultRequest>) -> Result<Json<Value>, ApiError> { report(&state, &session, request).map(Json) }
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadQuery { pub wait: Option<bool> }
pub async fn read_route(State(state): State<AppState>, Extension(session): Extension<ClientSession>, AxumPath(id): AxumPath<String>, Query(query): Query<ReadQuery>) -> Result<Json<Value>, ApiError> {
    let mut changes = subscribe();
    let mut claims = state.event_notify.subscribe();
    loop {
        let resource = read(&state, &session, &id)?;
        if query.wait != Some(true) || !matches!(resource["state"].as_str(), Some("pending" | "delivered")) { return Ok(Json(resource)); }
        let expiry = chrono::DateTime::parse_from_rfc3339(resource["expires_at"].as_str().unwrap_or_default()).map_err(ApiError::internal)?.timestamp_millis().max(0) as u128;
        let expiry = session.grant_expires_at.map_or(expiry, |grant| expiry.min(grant));
        tokio::select! {
            _ = changes.recv() => {},
            _ = claims.changed() => {},
            _ = tokio::time::sleep(Duration::from_millis(expiry.saturating_sub(client_now_ms()).min(u64::MAX as u128) as u64)) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command() -> Command { Command { resource: json!({"state":"pending"}), expires: 100, terminal_at: None, person: "person/ada".into(), grant: "grant/test".into(), issuer_grant: None } }
    fn result() -> DeviceResultRequest { DeviceResultRequest { command_id: "test".into(), outcome: "completed".into(), result: Some(json!({"ok":true})), error_code: None } }
    #[test]
    fn daemon_deadline_and_terminal_precedence() {
        let mut early = command(); early.finish(&result(), 99, false); early.expire(200, false); assert_eq!(early.resource["state"], "completed");
        let mut late = command(); late.finish(&result(), 100, true); assert_eq!(late.resource["state"], "expired"); assert!(late.resource.get("result").is_none());
        let mut absent = command(); absent.finish(&result(), 101, false); assert_eq!(absent.resource["state"], "not-foreground");
    }
    #[test]
    fn first_terminal_result_is_idempotent() {
        let mut row = command(); row.finish(&result(), 90, true); let first = row.resource.clone();
        let mut second = result(); second.outcome = "failed".into(); row.finish(&second, 91, true); assert_eq!(row.resource, first);
    }
    #[test]
    fn strict_args_and_payload_bounds() {
        assert!(validate_args("phone.inspect", &json!({})).is_ok());
        for args in [json!({"screen":"Unknown"}), json!({"screen":"Conversation","params":{}}), json!({"screen":"NowRoot","params":{"id":"x"}}), json!({"screen":"Document","params":{"reference":"doc/a","trail":[false]}})] { assert!(validate_args("phone.navigate", &args).is_err()); }
        assert!(validate_args("phone.inspect", &json!({"actor":"person/other"})).is_err());
        assert!(validate_args("phone.message.read", &json!({"attention_id":"x".repeat(9000)})).is_err());
    }
    #[test]
    fn terminal_retention_is_bounded() {
        let mut row = command(); row.finish(&result(), 90, true);
        let mut rows = BTreeMap::from([(("test-retention".into(), "test".into()), row)]);
        sweep(&mut rows, "test-retention", 90 + RETENTION_MS - 1); assert_eq!(rows.len(), 1);
        sweep(&mut rows, "test-retention", 90 + RETENTION_MS); assert!(rows.is_empty());
    }
    #[test]
    fn terminal_churn_cannot_exceed_memory_capacity() {
        let mut row = command(); row.finish(&result(), 90, true);
        let mut rows = BTreeMap::new();
        for index in 0..MAX_COMMANDS {
            admit(&rows, "capacity-test", "device/test").unwrap();
            rows.insert(("capacity-test".into(), index.to_string()), row.clone());
        }
        assert_eq!(admit(&rows, "capacity-test", "device/test").unwrap_err().status, StatusCode::TOO_MANY_REQUESTS);
        sweep(&mut rows, "capacity-test", 90 + RETENTION_MS);
        assert!(admit(&rows, "capacity-test", "device/test").is_ok());
    }
}
