//! D-4: durable, mission-claim-bound authority for the existing device commands.
//!
//! Command consumption is persisted before admission. Results reuse that consumption;
//! they never spend a second command or confer any message authority. The process lock
//! serializes budget admission on this member; claims survive daemon restarts.
use super::client_control_enrollment::enrolled;
use super::client_v0::{acting_party, revalidate_session, validation, ClientSession};
use super::*;
use std::collections::BTreeSet;
use std::sync::{LazyLock, Mutex};

pub const DEFAULT_MAX_COMMANDS: u64 = 50;
pub const MAX_GRANT_MS: u128 = 2 * 60 * 60 * 1000;
const GRANTED: &str = "custom.client.control-granted";
const REVOKED: &str = "custom.client.control-revoked";
const ENDED: &str = "custom.client.control-ended";
const CONSUMED: &str = "custom.client.control-consumed";
static ADMISSION: Mutex<()> = Mutex::new(());
static WATCHERS: LazyLock<Mutex<BTreeSet<(String, String)>>> = LazyLock::new(Mutex::default);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueGrantRequest {
    pub run_id: String,
    pub step_run: String,
    pub device: String,
    pub actor_agent: String,
    pub verbs: Vec<String>,
    pub screens: Vec<String>,
    pub item_id: Option<String>,
    pub max_commands: Option<u64>,
    /// Daemon-clock Unix milliseconds; defaults to the bounded claim/run deadline.
    pub expires_at: Option<u128>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlGrant {
    pub grant_id: String,
    pub run_id: String,
    pub device: String,
    pub session: String,
    pub actor_agent: String,
    pub verbs: Vec<String>,
    pub screens: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub max_commands: u64,
    pub expires_at: u128,
    pub issued_at: u128,
    pub enrollment_id: String,
    pub binding: ClaimBinding,
}

/// Identity of the particular step claim, not merely an agent assignment.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimBinding {
    pub step_run: String,
    pub generation: String,
    pub attempt: u32,
    pub incarnation: Option<String>,
    pub claim_id: String,
}

impl ControlGrant {
    pub fn wire(&self, state: &str) -> fractal_schema::client_control::ClientControlGrant {
        fractal_schema::client_control::ClientControlGrant {
            grant_id: self.grant_id.clone(), run_id: self.run_id.clone(),
            device: self.device.clone(), session: self.session.clone(),
            actor_agent: self.actor_agent.clone(), verbs: self.verbs.clone(),
            screens: self.screens.clone(), max_commands: self.max_commands,
            item_id: self.item_id.clone(),
            expires_at: client_timestamp(self.expires_at), state: state.into(),
        }
    }
    /// The generated ClientControlGrant wire shape, without internal admission data.
    pub fn resource(&self, state: &str) -> Value {
        let mut resource = json!({"grant_id":self.grant_id,"run_id":self.run_id,"device":self.device,
            "session":self.session,"actor_agent":self.actor_agent,"verbs":self.verbs,
            "screens":self.screens,"max_commands":self.max_commands,
            "expires_at":client_timestamp(self.expires_at),"state":state});
        if let Some(item_id) = &self.item_id { resource["item_id"] = json!(item_id); }
        resource["id"] = json!(self.grant_id);
        resource["kind"] = json!("client-control-grant");
        resource["revision"] = json!(format!("{}:{state}", self.grant_id));
        resource["updated_at"] = json!(client_timestamp(self.issued_at));
        resource
    }
}

fn refusal(code: &'static str) -> ApiError {
    ApiError { status: StatusCode::FORBIDDEN, code: code.into(),
        message: format!("client control refused: {code}"), details: Box::default() }
}
fn unavailable() -> ApiError { ApiError::internal("client control admission lock unavailable") }
fn verb_allowed(verb: &str) -> bool { matches!(verb, "phone.inspect" | "phone.navigate") }
fn screen_allowed(screen: &str) -> bool { matches!(screen, "NowRoot" | "Attention") }

fn append(state: &AppState, subject: &str, kind: &str, actor: &str, fields: Value) -> Result<ClaimRecord, ApiError> {
    state.store.append_client_operation_claim(&crate::model::ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(fields).map_err(ApiError::internal)?,
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).map_err(ApiError::internal)
}
fn trusted(state: &AppState, claim: &ClaimRecord) -> Result<bool, ApiError> {
    Ok(claim.origin == state.store.origin()
        && state.store.client_operation_is_trusted(claim).map_err(ApiError::internal)?)
}
fn claims(state: &AppState, subject: &str, kind: &str) -> Result<Vec<ClaimRecord>, ApiError> {
    let mut admitted = Vec::new();
    for claim in state.store.claims_for(subject, Some(kind)).map_err(ApiError::internal)? {
        if trusted(state, &claim)? { admitted.push(claim); }
    }
    Ok(admitted)
}

/// Look up immutable grant data. This is not an authorization decision: callers must
/// check a command or result window before using it.
pub fn lookup(state: &AppState, grant_id: &str) -> Result<ControlGrant, ApiError> {
    if !grant_id.starts_with("client-control-grant/") { return Err(refusal("grant-missing")); }
    let records = claims(state, grant_id, GRANTED)?;
    // There is precisely one issuance per unpredictable grant id. Conflicting claims
    // are not a way to replace an existing authority record.
    if records.len() != 1 { return Err(refusal("grant-missing")); }
    let grant: ControlGrant = serde_json::from_value(records[0].body["fields"].clone())
        .map_err(|_| refusal("grant-missing"))?;
    if grant.grant_id != grant_id { return Err(refusal("grant-missing")); }
    Ok(grant)
}

fn claim_window<'a>(run: &'a crate::model::MissionRunView, step_run: &str, actor: &str, now: u128)
    -> Result<(&'a crate::model::StepRunView, u128), ApiError>
{
    if run.status != "running" || run.ended().is_some()
        || run.deadline_at_unix_ms.is_some_and(|deadline| now >= deadline)
    { return Err(refusal("grant-expired")); }
    let step = run.steps.iter().find(|step| step.subject == step_run)
        .ok_or_else(|| refusal("actor-not-granted"))?;
    let deadline = step.claim_expires_at_unix_ms.ok_or_else(|| refusal("actor-not-granted"))?;
    if deadline <= now { return Err(refusal("grant-expired")); }
    if step.claimant.as_deref() != Some(actor)
        || !matches!(step.status.as_str(), "claimed" | "working" | "verifying")
    { return Err(refusal("actor-not-granted")); }
    Ok((step, deadline.min(run.deadline_at_unix_ms.unwrap_or(u128::MAX))))
}

fn live_claim(state: &AppState, run_id: &str, step_run: &str, actor: &str, now: u128)
    -> Result<(ClaimBinding, u128), ApiError>
{
    let run = state.store.mission_run(run_id).map_err(ApiError::internal)?
        .ok_or_else(|| refusal("actor-not-granted"))?;
    let (step, deadline) = claim_window(&run, step_run, actor, now)?;
    let claim = state.store.claims_for(step_run, Some("work.claimed")).map_err(ApiError::internal)?
        .into_iter().filter(|claim| claim.origin == state.store.origin())
        .max_by_key(|claim| claim.store_index).ok_or_else(|| refusal("actor-not-granted"))?;
    Ok((ClaimBinding { step_run: step.subject.clone(), generation: step.generation.clone(),
        attempt: step.attempt, incarnation: step.claim_incarnation.clone(), claim_id: claim.id },
        deadline))
}

fn validate_scope(verbs: &[String], screens: &[String], budget: u64) -> Result<(), ApiError> {
    if verbs.is_empty() || verbs.iter().any(|verb| !verb_allowed(verb)) {
        return Err(refusal("verb-not-granted"));
    }
    if screens.is_empty() || screens.iter().any(|screen| !screen_allowed(screen)) {
        return Err(refusal("screen-not-granted"));
    }
    if budget == 0 { return Err(refusal("budget-exhausted")); }
    Ok(())
}

pub fn issue_grant(state: &AppState, session: &ClientSession, request: IssueGrantRequest) -> Result<ControlGrant, ApiError> {
    let session = revalidate_session(state, session)?;
    if session.transport != "unix" || !acting_party(&session)
        || !request.actor_agent.starts_with("agent/")
        || session.authority_actor != request.actor_agent
    { return Err(refusal("actor-not-granted")); }
    let _guard = ADMISSION.lock().map_err(|_| unavailable())?;
    let enrollment = enrolled(state, &request.device)?;
    let now = client_now_ms();
    let max_commands = request.max_commands.unwrap_or(DEFAULT_MAX_COMMANDS);
    validate_scope(&request.verbs, &request.screens, max_commands)?;
    if request.screens.iter().any(|screen| screen == "Attention") {
        let item = request.item_id.as_deref().filter(|item| !item.is_empty())
            .ok_or_else(|| refusal("screen-not-granted"))?;
        if state.store.claims_for_subject_kind_at(item, "work.person-asked", None, true, 1)
            .map_err(ApiError::internal)?.claims.is_empty()
        { return Err(refusal("screen-not-granted")); }
    }
    let (binding, deadline) = live_claim(state, &request.run_id, &request.step_run, &request.actor_agent, now)?;
    let cap = now.saturating_add(MAX_GRANT_MS).min(deadline);
    let expires_at = request.expires_at.unwrap_or(cap);
    if expires_at <= now { return Err(refusal("grant-expired")); }
    if expires_at > cap { return Err(validation("grant expiry exceeds the two-hour, run or claim deadline")); }
    let grant = ControlGrant {
        grant_id: format!("client-control-grant/{}", crate::model::new_request_id()),
        run_id: request.run_id, device: request.device, session: enrollment.session.actor,
        actor_agent: request.actor_agent, verbs: request.verbs, screens: request.screens,
        item_id: request.item_id,
        max_commands, expires_at, issued_at: now, enrollment_id: enrollment.enrollment_id, binding,
    };
    append(state, &grant.grant_id, GRANTED, &grant.actor_agent,
        serde_json::to_value(&grant).map_err(ApiError::internal)?)?;
    ensure_lifecycle_watcher(state, &grant);
    super::signal_changed(state);
    Ok(grant)
}

fn check_identity(grant: &ControlGrant, run_id: &str, device: &str, actor: &str) -> Result<(), ApiError> {
    if grant.actor_agent != actor { return Err(refusal("actor-not-granted")); }
    if grant.run_id != run_id || grant.device != device { return Err(refusal("grant-missing")); }
    Ok(())
}
fn check_deadline(grant: &ControlGrant, now: u128) -> Result<(), ApiError> {
    if now < grant.issued_at || now >= grant.expires_at { return Err(refusal("grant-expired")); }
    Ok(())
}
fn active(state: &AppState, grant: &ControlGrant, now: u128) -> Result<(), ApiError> {
    check_deadline(grant, now)?;
    if !claims(state, &grant.grant_id, REVOKED)?.is_empty() { return Err(refusal("grant-expired")); }
    if !claims(state, &grant.grant_id, ENDED)?.is_empty() { return Err(refusal("grant-expired")); }
    let enrollment = enrolled(state, &grant.device)?;
    if enrollment.enrollment_id != grant.enrollment_id || enrollment.session.actor != grant.session {
        return Err(refusal("grant-expired"));
    }
    let (binding, _) = live_claim(state, &grant.run_id, &grant.binding.step_run, &grant.actor_agent, now)
        .map_err(|error| if error.code == "actor-not-granted" { refusal("grant-expired") } else { error })?;
    if binding != grant.binding { return Err(refusal("grant-expired")); }
    Ok(())
}

pub fn check_screen(grant: &ControlGrant, screen: &str) -> Result<(), ApiError> {
    if !screen_allowed(screen) || !grant.screens.iter().any(|allowed| allowed == screen) {
        return Err(refusal("screen-not-granted"));
    }
    Ok(())
}
/// An item-bound grant can inspect or navigate only the seeded native ask. Parent
/// result verification applies this same rule to the signed visible item's id.
pub fn check_item(grant: &ControlGrant, item_id: &str) -> Result<(), ApiError> {
    if grant.item_id.as_deref() != Some(item_id) {
        return Err(refusal("screen-not-granted"));
    }
    Ok(())
}
fn check_command(grant: &ControlGrant, kind: &str, args: &Value) -> Result<(), ApiError> {
    if !verb_allowed(kind) || !grant.verbs.iter().any(|allowed| allowed == kind) {
        return Err(refusal("verb-not-granted"));
    }
    if kind == "phone.navigate" {
        let screen = args["screen"].as_str().unwrap_or_default();
        check_screen(grant, screen)?;
        if screen == "Attention" {
            check_item(grant, args.pointer("/params/id").and_then(Value::as_str).unwrap_or_default())?;
        }
    }
    super::client_commands::validate_args(kind, args)
}
fn consumption_fields(grant: &ControlGrant, command_id: &str, kind: &str, args: &Value) -> Value {
    json!({"grantId":grant.grant_id,"runId":grant.run_id,"device":grant.device,
        "actorAgent":grant.actor_agent,"commandId":command_id,"commandKind":kind,"args":args})
}
fn check_budget(grant: &ControlGrant, used: usize) -> Result<(), ApiError> {
    if u64::try_from(used).unwrap_or(u64::MAX) >= grant.max_commands { return Err(refusal("budget-exhausted")); }
    Ok(())
}

/// Admit exactly one new command. Parent must validate its normal command capacity
/// and arguments first and use a fresh command id. A failed persistence never admits.
#[allow(clippy::too_many_arguments)]
pub fn check_and_consume(state: &AppState, grant_id: &str, run_id: &str, device: &str,
    actor_agent: &str, kind: &str, args: &Value, command_id: &str) -> Result<ControlGrant, ApiError>
{
    let _guard = ADMISSION.lock().map_err(|_| unavailable())?;
    let grant = lookup(state, grant_id)?;
    check_identity(&grant, run_id, device, actor_agent)?;
    active(state, &grant, client_now_ms())?;
    check_command(&grant, kind, args)?;
    let used = claims(state, grant_id, CONSUMED)?;
    if used.iter().any(|claim| claim.body["fields"]["commandId"] == command_id) {
        return Err(refusal("budget-exhausted"));
    }
    check_budget(&grant, used.len())?;
    append(state, grant_id, CONSUMED, actor_agent, consumption_fields(&grant, command_id, kind, args))?;
    Ok(grant)
}

/// Recheck the live grant and the original admission without consuming more budget.
/// The caller separately verifies the command TTL and purpose-specific result signature.
#[allow(clippy::too_many_arguments)]
pub fn check_result_window(state: &AppState, grant_id: &str, run_id: &str, device: &str,
    actor_agent: &str, kind: &str, args: &Value, command_id: &str) -> Result<ControlGrant, ApiError>
{
    let _guard = ADMISSION.lock().map_err(|_| unavailable())?;
    let grant = lookup(state, grant_id)?;
    check_identity(&grant, run_id, device, actor_agent)?;
    active(state, &grant, client_now_ms())?;
    check_command(&grant, kind, args)?;
    let expected = consumption_fields(&grant, command_id, kind, args);
    if !claims(state, grant_id, CONSUMED)?.iter().any(|claim| claim.body["fields"] == expected) {
        return Err(refusal("grant-missing"));
    }
    Ok(grant)
}

pub fn revoke_grant(state: &AppState, session: &ClientSession, grant_id: &str) -> Result<(), ApiError> {
    let session = revalidate_session(state, session)?;
    let _guard = ADMISSION.lock().map_err(|_| unavailable())?;
    let grant = lookup(state, grant_id)?;
    let own_client = super::client_commands::paired(&session) && session.actor == grant.session
        && enrolled(state, &grant.device)?.enrollment_id == grant.enrollment_id;
    let operator = session.transport == "unix" && acting_party(&session)
        && (session.authority_actor == grant.actor_agent || session.authority_actor.starts_with("person/"));
    if !own_client && !operator { return Err(refusal("actor-not-granted")); }
    if terminal_claim(state, &grant)?.is_none() {
        if let Some(reason) = termination_reason(state, &grant, client_now_ms())? {
            append_ended(state, &grant, reason)?;
        } else {
            append(state, grant_id, REVOKED, &session.authority_actor,
                json!({"grantId":grant_id,"revokedAt":client_now_ms(),"reason":"revoked"}))?;
        }
    }
    super::signal_changed(state);
    Ok(())
}

fn terminal_claim(state: &AppState, grant: &ControlGrant) -> Result<Option<ClaimRecord>, ApiError> {
    let mut terminal = claims(state, &grant.grant_id, REVOKED)?;
    terminal.extend(claims(state, &grant.grant_id, ENDED)?);
    Ok(terminal.into_iter().min_by_key(|claim| claim.store_index))
}

fn termination_reason(state: &AppState, grant: &ControlGrant, now: u128)
    -> Result<Option<&'static str>, ApiError>
{
    if now < grant.issued_at || now >= grant.expires_at { return Ok(Some("expired")); }
    match enrolled(state, &grant.device) {
        Ok(enrollment) if enrollment.enrollment_id == grant.enrollment_id
            && enrollment.session.actor == grant.session => {},
        Ok(_) => return Ok(Some("enrollment-revoked")),
        Err(error) if error.status.is_server_error() => return Err(error),
        Err(_) => return Ok(Some("enrollment-revoked")),
    }
    let run = state.store.mission_run(&grant.run_id).map_err(ApiError::internal)?;
    let Some(run) = run else { return Ok(Some("run-ended")); };
    if run.status != "running" || run.ended().is_some()
        || run.deadline_at_unix_ms.is_some_and(|deadline| now >= deadline)
    { return Ok(Some("run-ended")); }
    match live_claim(state, &grant.run_id, &grant.binding.step_run, &grant.actor_agent, now) {
        Ok((binding, _)) if binding == grant.binding => Ok(None),
        Ok(_) => Ok(Some("claim-ended")),
        Err(error) if error.status.is_server_error() => Err(error),
        Err(_) => Ok(Some("claim-ended")),
    }
}

fn append_ended(state: &AppState, grant: &ControlGrant, reason: &str) -> Result<ClaimRecord, ApiError> {
    let claim = append(state, &grant.grant_id, ENDED, &grant.actor_agent,
        json!({"grantId":grant.grant_id,"reason":reason,"endedAt":client_now_ms()}))?;
    super::signal_changed(state);
    Ok(claim)
}

fn lifecycle_resource(grant: &ControlGrant, claim: &ClaimRecord) -> Result<Value, ApiError> {
    use fractal_schema::client_control::{ClientControlGrantLifecycleEvent, ClientControlGrantLifecycleState};
    // Generated clients use JSON numbers, whose integer range is narrower than u64.
    if claim.store_index > (1_u64 << 53) - 1 {
        return Err(ApiError::internal("client control lifecycle sequence exceeds the wire integer range"));
    }
    let (state, reason) = match claim.kind.as_str() {
        GRANTED => (ClientControlGrantLifecycleState::Started, "granted"),
        REVOKED => (ClientControlGrantLifecycleState::Revoked, "revoked"),
        ENDED => (ClientControlGrantLifecycleState::Ended,
            claim.body["fields"]["reason"].as_str().ok_or_else(|| ApiError::internal("missing grant end reason"))?),
        _ => return Err(ApiError::internal("invalid client control lifecycle claim")),
    };
    let event = ClientControlGrantLifecycleEvent {
        grant_id: grant.grant_id.clone(), run_id: grant.run_id.clone(),
        device: grant.device.clone(), session: grant.session.clone(), item_id: grant.item_id.clone(),
        state, reason: reason.into(), sequence: claim.store_index,
    };
    let mut resource = serde_json::to_value(event).map_err(ApiError::internal)?;
    resource["id"] = json!(format!("{}/lifecycle", grant.grant_id));
    resource["kind"] = json!("client-control-grant-lifecycle");
    resource["revision"] = json!(format!("{}:{}", grant.grant_id, claim.store_index));
    resource["updated_at"] = json!(client_timestamp(claim.accepted_at_unix_ms));
    Ok(resource)
}

/// Reconcile and read one durable lifecycle transition without issuing a command.
/// Both subscriber snapshots and idle watchers use this serialized surface.
pub fn refresh_lifecycle(state: &AppState, grant_id: &str) -> Result<Value, ApiError> {
    let _guard = ADMISSION.lock().map_err(|_| unavailable())?;
    let grant = lookup(state, grant_id)?;
    let record = if let Some(terminal) = terminal_claim(state, &grant)? { terminal }
        else if let Some(reason) = termination_reason(state, &grant, client_now_ms())? {
            append_ended(state, &grant, reason)?
        } else {
            claims(state, grant_id, GRANTED)?.into_iter().next().ok_or_else(|| refusal("grant-missing"))?
        };
    lifecycle_resource(&grant, &record)
}

fn next_lifecycle_deadline(state: &AppState, grant: &ControlGrant) -> Result<u128, ApiError> {
    let mut deadline = grant.expires_at;
    match live_claim(state, &grant.run_id, &grant.binding.step_run, &grant.actor_agent, client_now_ms()) {
        Ok((_, claim_deadline)) => deadline = deadline.min(claim_deadline),
        Err(error) if error.status.is_server_error() => return Err(error),
        Err(_) => return Ok(client_now_ms()),
    }
    let enrollment = match enrolled(state, &grant.device) {
        Ok(enrollment) => enrollment,
        Err(error) if error.status.is_server_error() => return Err(error),
        Err(_) => return Ok(client_now_ms()),
    };
    if let Some(pairing_deadline) = enrollment.session.grant_expires_at {
        deadline = deadline.min(pairing_deadline);
    }
    Ok(deadline)
}

struct LifecycleWatcher((String, String));
impl Drop for LifecycleWatcher {
    fn drop(&mut self) {
        if let Ok(mut watchers) = WATCHERS.lock() { watchers.remove(&self.0); }
    }
}
fn ensure_lifecycle_watcher(state: &AppState, grant: &ControlGrant) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return; };
    let key = (state.node.clone(), grant.grant_id.clone());
    let Ok(mut watchers) = WATCHERS.lock() else { return; };
    if !watchers.insert(key.clone()) { return; }
    drop(watchers);
    let guard = LifecycleWatcher(key);
    let state = state.clone();
    let grant = grant.clone();
    // Subscribe before the initial read so a mutation cannot fall between them.
    let mut changes = state.event_notify.subscribe();
    runtime.spawn(async move {
        let _guard = guard;
        loop {
            let event = match refresh_lifecycle(&state, &grant.grant_id) {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(grant_id = %grant.grant_id, error = %error.message, "client control lifecycle refresh failed");
                    super::signal_changed(&state);
                    return;
                }
            };
            if event["state"] != "started" { return; }
            let deadline = match next_lifecycle_deadline(&state, &grant) {
                Ok(deadline) => deadline,
                Err(error) => {
                    tracing::warn!(grant_id = %grant.grant_id, error = %error.message, "client control lifecycle deadline failed");
                    super::signal_changed(&state);
                    return;
                }
            };
            tokio::select! {
                changed = changes.changed() => { if changed.is_err() { return; } },
                _ = tokio::time::sleep(Duration::from_millis(
                    u64::try_from(deadline.saturating_sub(client_now_ms())).unwrap_or(u64::MAX),
                )) => {},
            }
        }
    });
}

/// The latest typed lifecycle resource for each grant on this exact paired session.
/// Claim store indexes provide restart-stable, strictly increasing transition sequences.
pub fn collection_items(state: &AppState, session: &ClientSession) -> Result<Vec<Value>, ApiError> {
    let session = revalidate_session(state, session)?;
    if !super::client_commands::paired(&session) { return Ok(Vec::new()); }
    let mut before = None;
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        let page = state.store.claims_for_kind_at(GRANTED, before, true, 256).map_err(ApiError::internal)?;
        for claim in page.claims {
            if !trusted(state, &claim)? { continue; }
            let Ok(grant) = serde_json::from_value::<ControlGrant>(claim.body["fields"].clone()) else { continue; };
            if grant.session != session.actor || !seen.insert(grant.grant_id.clone()) { continue; }
            let event = refresh_lifecycle(state, &grant.grant_id)?;
            // Re-opened sessions restart deadline watchers only for live grants.
            if event["state"] == "started" { ensure_lifecycle_watcher(state, &grant); }
            result.push(event);
        }
        let Some(cursor) = page.next_cursor else { break; };
        before = Some(cursor);
    }
    result.sort_by_key(|resource| resource["sequence"].as_u64().unwrap_or_default());
    Ok(result)
}

pub async fn issue_grant_route(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    Json(request): Json<IssueGrantRequest>) -> Result<Json<Value>, ApiError>
{
    let grant = issue_grant(&state, &session, request)?;
    super::signal_changed(&state);
    ensure_lifecycle_watcher(&state, &grant);
    Ok(Json(grant.resource("active")))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeGrantRequest { pub grant_id: String }
pub async fn revoke_grant_route(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    Json(request): Json<RevokeGrantRequest>) -> Result<Json<Value>, ApiError>
{
    revoke_grant(&state, &session, &request.grant_id)?;
    super::signal_changed(&state);
    Ok(Json(lookup(&state, &request.grant_id)?.resource("revoked")))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grant() -> ControlGrant {
        ControlGrant { grant_id: "client-control-grant/test".into(), run_id: "mission-run/test".into(),
            device: "device/test".into(), session: "client-session/test".into(), actor_agent: "agent/test".into(),
            verbs: vec!["phone.inspect".into(), "phone.navigate".into()],
            screens: vec!["NowRoot".into(), "Attention".into()], item_id: Some("step-run/ask-seed/ask".into()), max_commands: DEFAULT_MAX_COMMANDS,
            expires_at: 200, issued_at: 100, enrollment_id: "enrollment/test".into(),
            binding: ClaimBinding { step_run: "step-run/test".into(), generation: "generation/test".into(),
                attempt: 1, incarnation: Some("incarnation/test".into()), claim_id: "claim/test".into() } }
    }
    #[test]
    fn all_d4_refusals_have_exact_codes() {
        let grant = grant();
        assert_eq!(check_identity(&grant, "another-run", &grant.device, &grant.actor_agent).unwrap_err().code, "grant-missing");
        assert_eq!(check_identity(&grant, &grant.run_id, "device/other", &grant.actor_agent).unwrap_err().code, "grant-missing");
        assert_eq!(check_identity(&grant, &grant.run_id, &grant.device, "agent/other").unwrap_err().code, "actor-not-granted");
        assert_eq!(check_deadline(&grant, 200).unwrap_err().code, "grant-expired");
        assert_eq!(check_screen(&grant, "Missions").unwrap_err().code, "screen-not-granted");
        assert_eq!(check_command(&grant, "phone.message.read", &json!({"message_id":"message/seed"})).unwrap_err().code, "verb-not-granted");
        assert_eq!(check_budget(&grant, 50).unwrap_err().code, "budget-exhausted");
    }
    #[test]
    fn message_authority_is_never_grantable() {
        for verb in ["message.send", "phone.message.read", "client.message.read", "control.messages",
            "answer", "close", "read-state", "phone.attention.answer", "phone.attention.close"] {
            assert_eq!(validate_scope(&[verb.into()], &["NowRoot".into()], 50).unwrap_err().code, "verb-not-granted");
            assert_eq!(check_command(&grant(), verb, &json!({})).unwrap_err().code, "verb-not-granted");
        }
    }
    #[test]
    fn closed_screens_and_verb_subsets_are_enforced() {
        let mut grant = grant();
        assert!(check_command(&grant, "phone.inspect", &json!({})).is_ok());
        assert!(check_command(&grant, "phone.navigate", &json!({"screen":"Attention","params":{"id":"step-run/ask-seed/ask"}})).is_ok());
        assert_eq!(check_command(&grant, "phone.navigate", &json!({"screen":"Conversation","params":{"target":"person/seed"}})).unwrap_err().code, "screen-not-granted");
        grant.screens = vec!["NowRoot".into()];
        assert_eq!(check_command(&grant, "phone.navigate", &json!({"screen":"Attention","params":{"id":"step-run/ask-seed/ask"}})).unwrap_err().code, "screen-not-granted");
        grant.verbs = vec!["phone.inspect".into()];
        assert_eq!(check_command(&grant, "phone.navigate", &json!({"screen":"NowRoot"})).unwrap_err().code, "verb-not-granted");
        assert_eq!(validate_scope(&["phone.inspect".into()], &["Agents".into()], 50).unwrap_err().code, "screen-not-granted");
    }
    #[test]
    fn budget_and_windows_are_exact() {
        let grant = grant();
        assert_eq!(grant.max_commands, 50);
        assert_eq!(MAX_GRANT_MS, 7_200_000);
        assert!(check_budget(&grant, 49).is_ok());
        assert!(check_budget(&grant, 51).is_err());
        assert!(check_deadline(&grant, 100).is_ok());
        assert!(check_deadline(&grant, 199).is_ok());
        assert_eq!(check_deadline(&grant, 99).unwrap_err().code, "grant-expired");
        assert_eq!(validate_scope(&grant.verbs, &grant.screens, 0).unwrap_err().code, "budget-exhausted");
    }
    #[test]
    fn durable_schema_preserves_claim_and_enrollment_identity() {
        let grant = grant();
        let encoded = serde_json::to_value(&grant).unwrap();
        assert_eq!(encoded["grantId"], grant.grant_id);
        let decoded: ControlGrant = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.binding, grant.binding);
        assert_eq!(decoded.enrollment_id, grant.enrollment_id);
        let first = consumption_fields(&grant, "command/one", "phone.inspect", &json!({}));
        assert_ne!(first, consumption_fields(&grant, "command/two", "phone.inspect", &json!({})));
        assert_ne!(first, consumption_fields(&grant, "command/one", "message.send", &json!({})));
        let mut another_run = grant.clone(); another_run.run_id = "mission-run/other".into();
        assert_ne!(first, consumption_fields(&another_run, "command/one", "phone.inspect", &json!({})));
    }
}

#[cfg(test)]
mod mission_binding_tests {
    use super::*;

    fn running_run() -> crate::model::MissionRunView {
        serde_json::from_value(json!({
            "subject":"mission-run/test","id":"test","mission":"mission/test",
            "generation":"generation/test","initial_revision":"revision/test",
            "revision":"revision/test","root_revision":"revision/test",
            "root_mission_run":"mission-run/test","workspace":"workspace/test",
            "requester":"agent/test","mode":"execute","status":"running","phase":"executing",
            "created_at_unix_ms":10,"updated_at_unix_ms":20,"deadline_at_unix_ms":200,
            "steps":[{
                "subject":"step-run/test","run":"mission-run/test","generation":"generation/test",
                "step":"test","definition_hash":"hash","status":"working","attempt":1,
                "assigned_to":"agent/test","agentless":false,"worker_reported":false,
                "claimant":"agent/test","claim_incarnation":"incarnation/test",
                "claim_expires_at_unix_ms":250,"readiness_epoch":1,
                "created_at_unix_ms":10,"updated_at_unix_ms":20
            }]
        })).unwrap()
    }

    #[test]
    fn issuance_requires_a_running_run_and_claimed_owner_not_assignment() {
        let mut run = running_run();
        assert_eq!(claim_window(&run, "step-run/test", "agent/test", 100).unwrap().1, 200);
        assert_eq!(claim_window(&run, "step-run/test", "agent/other", 100).unwrap_err().code, "actor-not-granted");
        assert_eq!(claim_window(&run, "step-run/other", "agent/test", 100).unwrap_err().code, "actor-not-granted");
        run.steps[0].claimant = None;
        assert_eq!(claim_window(&run, "step-run/test", "agent/test", 100).unwrap_err().code, "actor-not-granted");
        run.steps[0].claimant = Some("agent/test".into());
        run.steps[0].status = "ready".into();
        assert_eq!(claim_window(&run, "step-run/test", "agent/test", 100).unwrap_err().code, "actor-not-granted");
    }

    #[test]
    fn grant_never_outlives_run_or_claim_or_cleanup() {
        let mut run = running_run();
        assert_eq!(claim_window(&run, "step-run/test", "agent/test", 200).unwrap_err().code, "grant-expired");
        run.steps[0].claim_expires_at_unix_ms = Some(150);
        assert_eq!(claim_window(&run, "step-run/test", "agent/test", 100).unwrap().1, 150);
        assert_eq!(claim_window(&run, "step-run/test", "agent/test", 150).unwrap_err().code, "grant-expired");
        run.steps[0].claim_expires_at_unix_ms = Some(250);
        for status in ["pending", "completed", "failed", "cancelled"] {
            run.status = status.into();
            assert_eq!(claim_window(&run, "step-run/test", "agent/test", 100).unwrap_err().code, "grant-expired");
        }
        run.status = "running".into();
        for phase in ["terminal", "cleanup-stopping", "cleanup-draining"] {
            run.phase = phase.into();
            assert_eq!(claim_window(&run, "step-run/test", "agent/test", 100).unwrap_err().code, "grant-expired");
        }
    }
}

/// Real persisted fixture shared with the signed-result integration tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use base64::Engine as _;
    use crate::PersonWorkStore as _;
    use ring::signature::{EcdsaKeyPair, KeyPair as _, ECDSA_P256_SHA256_FIXED_SIGNING};
    use super::super::client_control_enrollment::{self as enrollment, EnrollRequest, FIXTURE_PERSON};

    pub fn request(grant: &ControlGrant, max_commands: u64) -> IssueGrantRequest {
        IssueGrantRequest { run_id: grant.run_id.clone(), step_run: grant.binding.step_run.clone(),
            device: grant.device.clone(), actor_agent: grant.actor_agent.clone(),
            verbs: grant.verbs.clone(), screens: grant.screens.clone(), item_id: grant.item_id.clone(),
            max_commands: Some(max_commands), expires_at: None }
    }
    pub fn reenroll(state: &AppState, key: &EcdsaKeyPair, issuer: &ClientSession, consent: &str) {
        enrollment::enroll(state, issuer, EnrollRequest {
            device: "device/test".into(), pairing_subject: "custom/client/pairing-test".into(),
            public_key: format!("p256:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key().as_ref())),
            kind: "tui".into(), consent_ref: consent.into(), host: "test-host".into(),
            instance: "client-control-fixture".into(), packet_digest: "ab".repeat(32),
        }).unwrap();
    }
    pub fn fixture(root: &Path) -> (AppState, EcdsaKeyPair, ControlGrant, ClientSession) {
        let mut state = super::super::tests_shared::state(root);
        state.node = format!("control-test-{}", crate::model::new_request_id());
        state.store = Arc::new(Store::open(&root.join("control-grants.db"), "node").unwrap());
        state.state_dir = root.join(".st-instance/client-control-fixture");
        state.owning_person = Some(FIXTURE_PERSON.into());
        let source = r#"version 2
agent "test" { command "true" }
mission "client-control-test" state="ready" {
  goal "Exercise narrow client command authority."
  step "journey" { assigned-to "agent/node.test" }
}
"#;
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let plan = state.store.mission(&intent, crate::model::IntentInput {
            kdl: source.into(), source_name: None,
        }).unwrap();
        state.store.apply(&intent, &plan.subject_tokens, "control-test-definition").unwrap();
        let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "client-control-test".into(), revision: None,
            workspace: root.display().to_string(), requester: Some("agent/node.test".into()),
            mode: Some("run".into()), inputs: BTreeMap::new(), idempotency_key: "control-test-run".into(),
        }).unwrap();
        let step_run = run.steps[0].subject.clone();
        state.store.set_step_state(&step_run, "ready", None).unwrap();
        state.store.work_action(&step_run, "claim", &crate::model::WorkRequest {
            wait_ms: None, actor: Some("agent/node.test".into()), incarnation: Some("control-test-incarnation".into()),
            summary: None, reason: None, evidence: Vec::new(), idempotency_key: "control-test-claim".into(),
        }).unwrap();
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let public_key = format!("p256:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key().as_ref()));
        state.store.append_client_operation_claim(&crate::model::ClaimInput {
            subject: "custom/client/pairing-test".into(), kind: "custom.client.pairing-completed".into(),
            actor: Some(FIXTURE_PERSON.into()),
            fields: serde_json::from_value(json!({
                "device_id":"device/test","device_public_key":public_key,"pairing_id":"pairing/test",
                "person_id":FIXTURE_PERSON,"delegated_by":FIXTURE_PERSON,
                "session_actor":format!("{FIXTURE_PERSON}/session/test"),
                "scopes":["read.projections","control.commands"],"delegated_scopes":["read.projections","control.commands"],
                "expires_at_unix_ms":client_now_ms()+MAX_GRANT_MS
            })).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let issuer = ClientSession::local(Some("agent/node.test")).unwrap();
        reenroll(&state, &key, &issuer, "step-run/ask-control-test-one/ask");
        let ask = state.store.ask_person(&crate::model::PersonAskRequest {
            person: FIXTURE_PERSON.into(), actor: "agent/node.test".into(),
            title: "Client-control seed".into(), reason: "Inspect this native ask without answering.".into(),
            step: None, new_run: None, incarnation: None, legacy_request: None,
            idempotency_key: "control-test-seed-ask".into(), deadline_ms: None, default_answer: None,
            request: Some(json!({"version":1,"type":"decision","question":"Inspect the fixture?",
                "why_person":"Only the owner may answer.","answers":[
                    {"id":"approve","label":"Approve","outcome":"accept","consequence":"Record approval."},
                    {"id":"deny","label":"Deny","outcome":"decline","consequence":"Record refusal."}]})),
        }).unwrap();
        let grant = issue_grant(&state, &issuer, IssueGrantRequest {
            run_id: run.subject, step_run, device: "device/test".into(), actor_agent: issuer.actor.clone(),
            verbs: vec!["phone.inspect".into(), "phone.navigate".into()],
            screens: vec!["NowRoot".into(), "Attention".into()], item_id: Some(ask.subject), max_commands: None, expires_at: None,
        }).unwrap();
        (state, key, grant, issuer)
    }
}

#[cfg(test)]
mod durable_tests {
    use super::*;
    use super::test_support::{fixture, request, reenroll};
    use super::super::client_control_enrollment::{self as enrollment, RevokeRequest};

    fn consume(state: &AppState, grant: &ControlGrant, command_id: &str) -> Result<ControlGrant, ApiError> {
        check_and_consume(state, &grant.grant_id, &grant.run_id, &grant.device,
            &grant.actor_agent, "phone.inspect", &json!({}), command_id)
    }
    fn result(state: &AppState, grant: &ControlGrant, command_id: &str) -> Result<ControlGrant, ApiError> {
        check_result_window(state, &grant.grant_id, &grant.run_id, &grant.device,
            &grant.actor_agent, "phone.inspect", &json!({}), command_id)
    }
    #[test]
    fn real_running_claim_grant_and_all_six_refusals() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        assert_eq!(lookup(&state, &grant.grant_id).unwrap().binding, grant.binding);
        assert!(consume(&state, &grant, "command/accepted").is_ok());
        assert!(result(&state, &grant, "command/accepted").is_ok());
        assert_eq!(lookup(&state, "client-control-grant/missing").unwrap_err().code, "grant-missing");
        assert_eq!(result(&state, &grant, "command/never-admitted").unwrap_err().code, "grant-missing");
        assert_eq!(check_and_consume(&state, &grant.grant_id, &grant.run_id, &grant.device,
            "agent/other", "phone.inspect", &json!({}), "command/other").unwrap_err().code, "actor-not-granted");
        assert_eq!(check_and_consume(&state, &grant.grant_id, &grant.run_id, &grant.device,
            &grant.actor_agent, "phone.navigate", &json!({"screen":"Missions"}), "command/screen").unwrap_err().code, "screen-not-granted");
        assert_eq!(check_and_consume(&state, &grant.grant_id, &grant.run_id, &grant.device,
            &grant.actor_agent, "message.send", &json!({}), "command/message").unwrap_err().code, "verb-not-granted");
        let once = issue_grant(&state, &issuer, request(&grant, 1)).unwrap();
        consume(&state, &once, "command/one").unwrap();
        assert_eq!(consume(&state, &once, "command/two").unwrap_err().code, "budget-exhausted");
        // A final result for the last admitted command is still authorized at budget zero.
        assert!(result(&state, &once, "command/one").is_ok());
        revoke_grant(&state, &issuer, &grant.grant_id).unwrap();
        assert_eq!(consume(&state, &grant, "command/revoked").unwrap_err().code, "grant-expired");
        assert_eq!(result(&state, &grant, "command/accepted").unwrap_err().code, "grant-expired");
    }
    #[test]
    fn issuer_must_be_the_current_claim_owner() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, _) = fixture(root.path());
        let other = ClientSession::local(Some("agent/other")).unwrap();
        let mut wrong = request(&grant, 50); wrong.actor_agent = "agent/other".into();
        assert_eq!(issue_grant(&state, &other, wrong).unwrap_err().code, "actor-not-granted");
        assert_eq!(issue_grant(&state, &other, request(&grant, 50)).unwrap_err().code, "actor-not-granted");
    }
    #[test]
    fn consumed_budget_and_authority_survive_store_reopen() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        let once = issue_grant(&state, &issuer, request(&grant, 1)).unwrap();
        consume(&state, &once, "command/one").unwrap();
        drop(state);
        let mut reopened = super::super::tests_shared::state(root.path());
        reopened.store = Arc::new(Store::open(&root.path().join("control-grants.db"), "node").unwrap());
        reopened.state_dir = root.path().join(".st-instance/client-control-fixture");
        reopened.owning_person = Some(enrollment::FIXTURE_PERSON.into());
        assert_eq!(lookup(&reopened, &once.grant_id).unwrap().max_commands, 1);
        assert_eq!(consume(&reopened, &once, "command/two").unwrap_err().code, "budget-exhausted");
        assert!(result(&reopened, &once, "command/one").is_ok());
    }
    #[test]
    fn enrollment_revocation_and_fresh_consent_never_resurrect_old_grant() {
        let root = tempfile::tempdir().unwrap();
        let (state, key, grant, issuer) = fixture(root.path());
        consume(&state, &grant, "command/before").unwrap();
        enrollment::revoke(&state, &issuer, RevokeRequest { device: grant.device.clone() }).unwrap();
        assert_eq!(consume(&state, &grant, "command/revoked").unwrap_err().code, "device-not-test-client");
        assert_eq!(result(&state, &grant, "command/before").unwrap_err().code, "device-not-test-client");
        reenroll(&state, &key, &issuer, "step-run/ask-control-test-two/ask");
        assert_eq!(consume(&state, &grant, "command/new-enrollment").unwrap_err().code, "grant-expired");
        assert_eq!(result(&state, &grant, "command/before").unwrap_err().code, "grant-expired");
        let fresh = issue_grant(&state, &issuer, request(&grant, 50)).unwrap();
        assert_ne!(fresh.enrollment_id, grant.enrollment_id);
        assert!(consume(&state, &fresh, "command/fresh").is_ok());
    }
    #[test]
    fn claim_end_and_replacement_incarnation_end_old_grant() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        consume(&state, &grant, "command/before").unwrap();
        state.store.set_step_state(&grant.binding.step_run, "ready", Some("claim ended")).unwrap();
        assert_eq!(consume(&state, &grant, "command/released").unwrap_err().code, "grant-expired");
        assert_eq!(result(&state, &grant, "command/before").unwrap_err().code, "grant-expired");
        state.store.work_action(&grant.binding.step_run, "claim", &crate::model::WorkRequest {
            wait_ms: None, actor: Some(grant.actor_agent.clone()), incarnation: Some("replacement".into()),
            summary: None, reason: None, evidence: Vec::new(), idempotency_key: "replacement-claim".into(),
        }).unwrap();
        assert_eq!(consume(&state, &grant, "command/replacement").unwrap_err().code, "grant-expired");
        let fresh = issue_grant(&state, &issuer, request(&grant, 50)).unwrap();
        assert_ne!(fresh.binding.claim_id, grant.binding.claim_id);
        assert_ne!(fresh.binding.incarnation, grant.binding.incarnation);
        assert!(consume(&state, &fresh, "command/fresh").is_ok());
    }
    #[test]
    fn run_end_rechecked_on_both_command_and_result() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, _) = fixture(root.path());
        consume(&state, &grant, "command/before").unwrap();
        state.store.set_mission_run_state(&grant.run_id, "cancelled", "terminal", Some("run ended")).unwrap();
        assert_eq!(consume(&state, &grant, "command/after").unwrap_err().code, "grant-expired");
        assert_eq!(result(&state, &grant, "command/before").unwrap_err().code, "grant-expired");
    }
    #[test]
    fn expiry_and_generic_forgery_are_refused_from_persisted_records() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, mut grant, _) = fixture(root.path());
        grant.grant_id = "client-control-grant/forged".into();
        let input = crate::model::ClaimInput {
            subject: grant.grant_id.clone(), kind: GRANTED.into(), actor: Some(grant.actor_agent.clone()),
            fields: serde_json::from_value(serde_json::to_value(&grant).unwrap()).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        };
        state.store.append_claim(&input).unwrap();
        assert_eq!(consume(&state, &grant, "command/forged").unwrap_err().code, "grant-missing");
        grant.grant_id = "client-control-grant/expired".into();
        grant.expires_at = client_now_ms().saturating_sub(1);
        append(&state, &grant.grant_id, GRANTED, &grant.actor_agent, serde_json::to_value(&grant).unwrap()).unwrap();
        assert_eq!(consume(&state, &grant, "command/expired").unwrap_err().code, "grant-expired");
        assert_eq!(result(&state, &grant, "command/expired").unwrap_err().code, "grant-expired");
    }
    #[test]
    fn issuance_bounds_and_paired_revoke_lifecycle() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        assert_eq!(grant.max_commands, DEFAULT_MAX_COMMANDS);
        assert!(grant.expires_at <= grant.issued_at + MAX_GRANT_MS);
        let mut too_long = request(&grant, 50);
        too_long.expires_at = Some(client_now_ms() + MAX_GRANT_MS + 1);
        assert!(issue_grant(&state, &issuer, too_long).is_err());
        let mut expired = request(&grant, 50);
        expired.expires_at = Some(client_now_ms().saturating_sub(1));
        assert_eq!(issue_grant(&state, &issuer, expired).unwrap_err().code, "grant-expired");
        let mut message_read = request(&grant, 50);
        message_read.verbs.push("phone.message.read".into());
        assert_eq!(issue_grant(&state, &issuer, message_read).unwrap_err().code, "verb-not-granted");
        let mut other_screen = request(&grant, 50);
        other_screen.screens.push("Missions".into());
        assert_eq!(issue_grant(&state, &issuer, other_screen).unwrap_err().code, "screen-not-granted");
        let client = enrollment::enrolled(&state, &grant.device).unwrap();
        let resources = collection_items(&state, &client.session).unwrap();
        let active = resources.iter().find(|resource| resource["grant_id"] == grant.grant_id).unwrap();
        assert_eq!(active["kind"], "client-control-grant-lifecycle");
        assert_eq!(active["state"], "started");
        let started_sequence = active["sequence"].as_u64().unwrap();
        assert_eq!(active["revision"], format!("{}:{started_sequence}", grant.grant_id));
        assert!(collection_items(&state, &issuer).unwrap().is_empty());
        revoke_grant(&state, &client.session, &grant.grant_id).unwrap();
        let resources = collection_items(&state, &client.session).unwrap();
        let revoked = resources.iter().find(|resource| resource["grant_id"] == grant.grant_id).unwrap();
        assert_eq!(revoked["state"], "revoked");
        assert_eq!(revoked["id"], active["id"]);
        let revoked_sequence = revoked["sequence"].as_u64().unwrap();
        assert!(revoked_sequence > started_sequence);
        assert_eq!(revoked["revision"], format!("{}:{revoked_sequence}", grant.grant_id));
    }
    #[test]
    fn attention_requires_an_existing_bound_native_ask_but_root_does_not() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        let mut missing = request(&grant, 50);
        missing.item_id = None;
        assert_eq!(issue_grant(&state, &issuer, missing).unwrap_err().code, "screen-not-granted");
        let mut unknown = request(&grant, 50);
        unknown.item_id = Some("step-run/ask-unknown/ask".into());
        assert_eq!(issue_grant(&state, &issuer, unknown).unwrap_err().code, "screen-not-granted");
        let mut root_only = request(&grant, 50);
        root_only.screens = vec!["NowRoot".into()];
        root_only.item_id = None;
        assert!(issue_grant(&state, &issuer, root_only).unwrap().item_id.is_none());
        assert!(issue_grant(&state, &issuer, request(&grant, 50)).is_ok());
    }
    #[test]
    fn native_ask_item_binding_is_durable_and_conversation_is_never_granted() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        let bound = issue_grant(&state, &issuer, request(&grant, 50)).unwrap();
        let seed = grant.item_id.as_deref().unwrap();
        let loaded = lookup(&state, &bound.grant_id).unwrap();
        assert_eq!(loaded.item_id.as_deref(), Some(seed));
        assert_eq!(loaded.resource("active")["item_id"], seed);
        assert_eq!(loaded.wire("active").item_id, loaded.item_id);
        assert!(check_item(&loaded, seed).is_ok());
        assert_eq!(check_item(&loaded, "step-run/ask-other/ask").unwrap_err().code, "screen-not-granted");
        assert!(check_and_consume(&state, &bound.grant_id, &bound.run_id, &bound.device,
            &bound.actor_agent, "phone.navigate", &json!({"screen":"NowRoot"}), "command/root").is_ok());
        assert!(check_and_consume(&state, &bound.grant_id, &bound.run_id, &bound.device,
            &bound.actor_agent, "phone.navigate", &json!({"screen":"Attention","params":{"id":seed}}), "command/seed").is_ok());
        assert_eq!(check_and_consume(&state, &bound.grant_id, &bound.run_id, &bound.device,
            &bound.actor_agent, "phone.navigate", &json!({"screen":"Attention","params":{"id":"step-run/ask-other/ask"}}), "command/other").unwrap_err().code, "screen-not-granted");
        assert_eq!(check_and_consume(&state, &bound.grant_id, &bound.run_id, &bound.device,
            &bound.actor_agent, "phone.navigate", &json!({"screen":"Conversation","params":{"target":"person/seed"}}), "command/conversation").unwrap_err().code, "screen-not-granted");
        let mut conversation_request = request(&grant, 50);
        conversation_request.screens = vec!["NowRoot".into(), "Conversation".into()];
        assert_eq!(issue_grant(&state, &issuer, conversation_request).unwrap_err().code, "screen-not-granted");
        for verb in ["answer", "close", "read-state", "message.send", "phone.message.read"] {
            assert_eq!(check_and_consume(&state, &bound.grant_id, &bound.run_id, &bound.device,
                &bound.actor_agent, verb, &json!({}), "command/mutate").unwrap_err().code, "verb-not-granted");
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use super::test_support::{fixture, request};

    #[test]
    fn run_end_is_persisted_once_with_restart_stable_increasing_sequence() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        let started = refresh_lifecycle(&state, &grant.grant_id).unwrap();
        assert_eq!(started["state"], "started");
        assert_eq!(started["reason"], "granted");
        assert_eq!(started["kind"], "client-control-grant-lifecycle");
        assert_eq!(started["session"], grant.session);
        state.store.set_mission_run_state(&grant.run_id, "cancelled", "terminal", None).unwrap();
        let ended = refresh_lifecycle(&state, &grant.grant_id).unwrap();
        assert_eq!(ended["id"], started["id"]);
        assert_eq!(ended["state"], "ended");
        assert_eq!(ended["reason"], "run-ended");
        assert!(ended["sequence"].as_u64().unwrap() > started["sequence"].as_u64().unwrap());
        assert_eq!(claims(&state, &grant.grant_id, ENDED).unwrap().len(), 1);
        // A later revoke cannot replace the first terminal event.
        revoke_grant(&state, &issuer, &grant.grant_id).unwrap();
        assert_eq!(refresh_lifecycle(&state, &grant.grant_id).unwrap(), ended);
        assert!(claims(&state, &grant.grant_id, CONSUMED).unwrap().is_empty());
        drop(state);
        let mut reopened = super::super::tests_shared::state(root.path());
        reopened.store = Arc::new(Store::open(&root.path().join("control-grants.db"), "node").unwrap());
        reopened.state_dir = root.path().join(".st-instance/client-control-fixture");
        reopened.owning_person = Some(super::super::client_control_enrollment::FIXTURE_PERSON.into());
        assert_eq!(refresh_lifecycle(&reopened, &grant.grant_id).unwrap(), ended);
    }

    #[test]
    fn concurrent_end_observers_publish_one_terminal_transition() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, _) = fixture(root.path());
        state.store.set_step_state(&grant.binding.step_run, "ready", None).unwrap();
        let events = std::thread::scope(|scope| {
            let handles = (0..8).map(|_| {
                let state = &state;
                let grant = &grant;
                scope.spawn(move || refresh_lifecycle(state, &grant.grant_id).unwrap())
            }).collect::<Vec<_>>();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
        });
        assert!(events.iter().all(|event| event == &events[0]));
        assert_eq!(events[0]["reason"], "claim-ended");
        assert_eq!(claims(&state, &grant.grant_id, ENDED).unwrap().len(), 1);
    }

    #[test]
    fn generic_terminal_claims_cannot_end_a_live_grant() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, _) = fixture(root.path());
        let started = refresh_lifecycle(&state, &grant.grant_id).unwrap();
        for kind in [ENDED, REVOKED] {
            state.store.append_claim(&crate::model::ClaimInput {
                subject: grant.grant_id.clone(), kind: kind.into(), actor: Some(grant.actor_agent.clone()),
                fields: serde_json::from_value(json!({"grantId":grant.grant_id,"reason":"forged"})).unwrap(),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        assert_eq!(refresh_lifecycle(&state, &grant.grant_id).unwrap(), started);
        assert!(terminal_claim(&state, &grant).unwrap().is_none());
    }

    #[test]
    fn expiry_reconciliation_preserves_item_binding_and_unique_collection_ids() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, mut grant, _) = fixture(root.path());
        grant.grant_id = "client-control-grant/expired-lifecycle".into();
        grant.item_id = Some("step-run/ask-seed/ask".into());
        grant.expires_at = client_now_ms().saturating_sub(1);
        append(&state, &grant.grant_id, GRANTED, &grant.actor_agent, serde_json::to_value(&grant).unwrap()).unwrap();
        let ended = refresh_lifecycle(&state, &grant.grant_id).unwrap();
        assert_eq!(ended["state"], "ended");
        assert_eq!(ended["reason"], "expired");
        assert_eq!(ended["item_id"], "step-run/ask-seed/ask");
        let client = enrolled(&state, &grant.device).unwrap();
        let items = collection_items(&state, &client.session).unwrap();
        let ids = items.iter().map(|item| item["id"].as_str().unwrap()).collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), items.len());
        assert!(items.iter().all(|item| item["kind"] == "client-control-grant-lifecycle"));
        assert!(items.windows(2).all(|pair| pair[0]["sequence"].as_u64() < pair[1]["sequence"].as_u64()));
    }

    async fn wait_for_ended(state: &AppState, grant: &ControlGrant) -> ClaimRecord {
        let mut changes = state.event_notify.subscribe();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(claim) = claims(state, &grant.grant_id, ENDED).unwrap().into_iter().next() {
                    return claim;
                }
                changes.changed().await.unwrap();
            }
        }).await.expect("idle lifecycle watcher must persist termination without command polling")
    }

    #[tokio::test]
    async fn idle_expiry_watcher_persists_without_commands_or_collection_polling() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, grant, issuer) = fixture(root.path());
        let mut expires = request(&grant, 50);
        expires.expires_at = Some(client_now_ms() + 100);
        let short = issue_grant(&state, &issuer, expires).unwrap();
        let ended = wait_for_ended(&state, &short).await;
        assert_eq!(ended.body["fields"]["reason"], "expired");
        assert!(claims(&state, &short.grant_id, CONSUMED).unwrap().is_empty());
        assert_eq!(refresh_lifecycle(&state, &short.grant_id).unwrap()["sequence"], ended.store_index);
    }

    #[tokio::test]
    async fn idle_claim_and_run_events_wake_durable_watchers() {
        for run_end in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (state, _, grant, _) = fixture(root.path());
            if run_end {
                state.store.set_mission_run_state(&grant.run_id, "cancelled", "terminal", None).unwrap();
            } else {
                state.store.set_step_state(&grant.binding.step_run, "ready", None).unwrap();
            }
            super::super::signal_changed(&state);
            let ended = wait_for_ended(&state, &grant).await;
            assert_eq!(ended.body["fields"]["reason"], if run_end { "run-ended" } else { "claim-ended" });
            assert!(claims(&state, &grant.grant_id, CONSUMED).unwrap().is_empty());
        }
    }
}
