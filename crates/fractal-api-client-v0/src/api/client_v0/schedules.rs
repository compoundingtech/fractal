//! Declaration-only schedule listing. A timer, run history, or a remote read is never consulted.
use super::*;
use base64::Engine as _;
use hmac::{Hmac, Mac as _};
use std::sync::LazyLock;
use crate::store::schedule_declarations::ScheduleDeclarationRow;

const DEFAULT_UNHEARD_MS: u64 = 5 * 60 * 1000;
// JSON-safe in every generated native/browser model.
const MAX_UNHEARD_MS: u64 = 9_007_199_254_740_991;
const CURSOR_AGE_MS: u128 = 15 * 60 * 1000;
const MAX_HOSTS: usize = 200;
const ITEM_BUDGET: usize = CLIENT_MAX_RESPONSE_BYTES - 128_000;
const FOOTER_BUDGET: usize = 64 * 1024;
const CURSOR_PREFIX: &str = "schedule-page/";

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulesQuery {
    limit: Option<usize>,
    cursor: Option<String>,
    unheard_after_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    instance: String,
    actor: String,
    authority: String,
    grant: Option<String>,
    after: String,
    limit: usize,
    unheard_after_ms: u64,
    expires_at: u128,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedPosition { position: Position, mac: String }
static CURSOR_KEY: LazyLock<[u8; 32]> = LazyLock::new(|| {
    let mut key = [0; 32];
    getrandom::fill(&mut key).expect("creating schedule cursor authentication key");
    key
});

fn cursor_error() -> ApiError {
    ApiError::bad(St3Error::new("cursor-expired", "the schedule page cursor is invalid or expired; restart the listing"))
}
fn instance(state: &AppState) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(&(
        &state.node, state.state_dir.to_string_lossy(),
    )).expect("instance identity serializes")))
}
fn mac(position: &Position) -> Result<Hmac<Sha256>, ApiError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&*CURSOR_KEY).expect("HMAC accepts a 32-byte key");
    mac.update(&serde_json::to_vec(position).map_err(ApiError::internal)?);
    Ok(mac)
}
fn encode(position: Position) -> Result<String, ApiError> {
    let signature = hex::encode(mac(&position)?.finalize().into_bytes());
    let bytes = serde_json::to_vec(&SignedPosition { position, mac: signature }).map_err(ApiError::internal)?;
    let cursor = format!("{CURSOR_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes));
    if cursor.len() > 4096 {
        return Err(ApiError::bad(St3Error::new("validation-failed", "a schedule page boundary exceeds the cursor size limit")));
    }
    Ok(cursor)
}
fn decode(cursor: &str, state: &AppState, session: &ClientSession, now: u128) -> Result<Position, ApiError> {
    if cursor.len() > 4096 { return Err(cursor_error()); }
    let encoded = cursor.strip_prefix(CURSOR_PREFIX).ok_or_else(cursor_error)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded).map_err(|_| cursor_error())?;
    let signed: SignedPosition = serde_json::from_slice(&bytes).map_err(|_| cursor_error())?;
    mac(&signed.position)?.verify_slice(&hex::decode(signed.mac).map_err(|_| cursor_error())?)
        .map_err(|_| cursor_error())?;
    let p = signed.position;
    if p.instance != instance(state) || p.actor != session.actor
        || p.authority != session.authority_actor || p.grant != session.pairing_grant
        || p.expires_at <= now || !p.after.starts_with("schedule/")
        || !(1..=200).contains(&p.limit) || p.unheard_after_ms == 0
        || p.unheard_after_ms > MAX_UNHEARD_MS {
        return Err(cursor_error());
    }
    Ok(p)
}

pub async fn list(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<SchedulesQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    require_scope(&session, "read.declarations")?;
    let now = client_now_ms();
    let position = query.cursor.as_deref().map(|cursor| decode(cursor, &state, &session, now)).transpose()?;
    let limit = query.limit.or_else(|| position.as_ref().map(|p| p.limit)).unwrap_or(50);
    let unheard = query.unheard_after_ms.or_else(|| position.as_ref().map(|p| p.unheard_after_ms)).unwrap_or(DEFAULT_UNHEARD_MS);
    if !(1..=200).contains(&limit) || unheard == 0 || unheard > MAX_UNHEARD_MS {
        return Err(validation("schedule limit must be 1 through 200 and unheard_after_ms must be 1 through 9007199254740991 milliseconds"));
    }
    if position.as_ref().is_some_and(|p| p.limit != limit || p.unheard_after_ms != unheard) {
        return Err(cursor_error());
    }
    let after = position.map(|p| p.after).unwrap_or_default();
    let (snapshot, page) = blocking_store(move || {
        let store = state.store.clone();
        store.read_snapshot(|index| Ok(read_page(&state, &session, index, &after, limit, unheard, now)))
    }).await??;
    Ok((Extension(snapshot), Json(page)))
}

fn child<'a>(declaration: &'a Value, name: &str) -> Option<&'a Value> {
    declaration.get("children")?.as_array()?.iter().find(|child| child.get("name").and_then(Value::as_str) == Some(name))
}
fn primary(declaration: &Value, origin: Option<&str>) -> Option<String> {
    if declaration["name"] != "schedule" || declaration.get("children").and_then(Value::as_array).is_none() { return None; }
    let host = match child(declaration, "host") {
        Some(node) => node.get("arguments")?.as_array()?.first()?.as_str()?,
        None => "local",
    };
    let host = if host == "local" { origin? } else { host };
    (!host.is_empty() && !host.contains('/') && !host.chars().any(char::is_whitespace)).then(|| format!("host/{host}"))
}
fn stopped(declaration: &Value) -> bool {
    declaration.as_str() == Some("stopped") || declaration.get("children").and_then(Value::as_array)
        .is_some_and(|children| children.len() == 1 && children[0]["name"] == "stop")
}
fn standby(declaration: &Value) -> (&'static str, Option<Value>) {
    if stopped(declaration) { return ("not-declared", None); }
    if declaration["name"] != "schedule" || declaration.get("children").and_then(Value::as_array).is_none() { return ("unknown", None); }
    match child(declaration, "standby") {
        Some(node) => ("declared", Some(node.clone())),
        None => ("not-declared", None),
    }
}
fn next_fire(declaration: &Value, origin: Option<&str>, now: u128) -> (Option<String>, &'static str) {
    if stopped(declaration) { return (None, "stopped"); }
    // A future scheduling setting can change the nominal clock. Keep its raw fields without
    // pretending this reader understands its next tick. Standby is presence-only here.
    if declaration["name"] != "schedule" { return (None, "unknown"); }
    let known = ["host", "at", "every", "anchor", "calendar", "catch-up", "max-catch-up", "work", "standby"];
    if declaration.get("children").and_then(Value::as_array).is_none_or(|nodes| nodes.iter().any(|node|
        node.get("name").and_then(Value::as_str).is_none_or(|name| !known.contains(&name)))) {
        return (None, "unknown");
    }
    let Some(spec) = crate::graph::schedule_spec(declaration, origin.unwrap_or("")) else { return (None, "unknown"); };
    let Ok(now) = i64::try_from(now) else { return (None, "unknown"); };
    let at = if let Some(at) = spec.at_unix_ms {
        if at < now { return (None, "past-one-shot"); }
        Some(at)
    } else if let Some(calendar) = spec.calendar {
        crate::reconcile::schedule_declaration_clock::next_declared_calendar_at(&calendar, now).ok()
    } else if let (Some(every), Some(anchor)) = (spec.every_ms.filter(|every| *every > 0), spec.anchor_unix_ms) {
        let delta = i128::from(now).saturating_sub(i128::from(anchor)).max(0);
        let every = i128::from(every);
        let steps = delta / every + i128::from(delta % every != 0);
        steps.checked_mul(every).and_then(|offset| i128::from(anchor).checked_add(offset)).and_then(|at| i64::try_from(at).ok())
    } else { None };
    match at.filter(|at| *at >= now && *at >= 0) {
        Some(at) => (Some(client_timestamp(at as u128)), "future"),
        None => (None, "unknown"),
    }
}

fn row_value(row: &ScheduleDeclarationRow, now: u128) -> Value {
    let raw = row.body.as_deref().and_then(|body| serde_json::from_str::<Value>(body).ok());
    let owner_state = if row.owner_run.is_some() { "recorded" }
        else if row.owner_generation.is_some() || row.owner_step.is_some() { "unknown" } else { "not-recorded" };
    let (standby_state, standby_node) = raw.as_ref().map(standby).unwrap_or(("unknown", None));
    let (next, next_state) = raw.as_ref().map(|body| next_fire(body, row.origin.as_deref(), now)).unwrap_or((None, "unknown"));
    let host = raw.as_ref().filter(|body| !stopped(body))
        .and_then(|body| primary(body, row.origin.as_deref()));
    let problem = if row.body.is_none() { Some("declaration exceeds the 64 KiB materialization limit") }
        else if raw.is_none() { Some("declaration is not readable JSON") } else { None };
    json!({
        "id":row.subject,"revision":row.revision,"declaration_claim_id":row.claim,
        "declaration_origin_host_id":row.origin.as_ref().map(|name| format!("host/{name}")),
        "owner":{"run_id":row.owner_run,"generation_id":row.owner_generation,"step_id":row.owner_step,"state":owner_state},
        "primary_host_id":host,"standby_state":standby_state,"standby_node":standby_node,
        "state":if raw.as_ref().is_some_and(stopped) {"stopped"} else if raw.as_ref().is_some_and(Value::is_object) && next_state != "unknown" {"declared"} else {"unknown"},
        "declaration":raw,"declaration_bytes":row.body_bytes,"read_problem":problem,
        "next_declared_fire_at":next,"next_fire_state":next_state,"next_fire_basis":"declaration-only",
    })
}

fn heard(raw: Option<&str>, now: u128) -> Result<Option<(u128, u64)>, ()> {
    let Some(raw) = raw else { return Ok(None); };
    let at = raw.parse::<u128>().map_err(|_| ())?;
    if at > now || at > i64::MAX as u128 { return Err(()); }
    let age = u64::try_from(now - at).map_err(|_| ())?;
    Ok(Some((at, age)))
}
fn knowledge(origin: Option<&str>, local: &str, times: &BTreeMap<String, Option<String>>, now: u128) -> Value {
    let observation = origin.and_then(|origin| times.get(origin)).and_then(Option::as_deref);
    let (basis, at, age) = if origin == Some(local) { ("local-read", None, Some(0)) }
        else if let Ok(Some((at, age))) = heard(observation, now) { ("last-direct-exchange", Some(client_timestamp(at)), Some(age)) }
        else { ("unknown", None, None) };
    json!({"origin_host_id":origin.map(|host|format!("host/{host}")),"basis":basis,"last_exchange_at":at,"age_ms":age,"completeness":"unknown"})
}
fn coverage(state: &AppState, hosts: &BTreeSet<String>, times: &BTreeMap<String, Option<String>>, now: u128, threshold: u64) -> Result<Value, ApiError> {
    let considered = hosts.len().min(MAX_HOSTS);
    let mut unheard = Vec::new();
    for host in hosts.iter().take(MAX_HOSTS).filter(|host| *host != &state.node) {
        let raw = times.get(host).and_then(Option::as_deref);
        let (reason, at, age) = match heard(raw, now) {
            Ok(Some((at, age))) if age > threshold => ("older-than-threshold", Some(client_timestamp(at)), Some(age)),
            Ok(Some(_)) => continue,
            Ok(None) => ("never-recorded", None, None),
            Err(()) => ("clock-unknown", None, None),
        };
        unheard.push(json!({"host_id":format!("host/{host}"),"last_exchange_at":at,"age_ms":age,"reason":reason}));
    }
    let configured_omitted = state.configured_peers.len().saturating_sub(MAX_HOSTS);
    let mut byte_omitted = 0;
    // Names themselves can be large. Preserve a visible partial-coverage result, not an
    // unbounded footer or a silent assertion that an omitted host was heard.
    while serde_json::to_vec(&unheard).map_err(ApiError::internal)?.len() > FOOTER_BUDGET {
        unheard.pop(); byte_omitted += 1;
    }
    let omitted = hosts.len().saturating_sub(considered) + byte_omitted;
    Ok(json!({"scope":"configured-peers-and-page-hosts","unheard_after_ms":threshold,
        "unheard_hosts":unheard,"membership":"unknown",
        "covered_host_count":considered.saturating_sub(byte_omitted),
        "truncated":configured_omitted>0 || omitted>0,
        "omitted_configured_peer_count":configured_omitted,"omitted_covered_host_count":omitted}))
}

fn read_page(state: &AppState, session: &ClientSession, index: u64, after: &str, limit: usize, threshold: u64, now: u128) -> Result<(ClientSnapshot, Value), ApiError> {
    let rows = state.store.schedule_declaration_rows(after, limit).map_err(ApiError::internal)?;
    let mut has_more = rows.len() > limit;
    let mut items = Vec::new();
    let mut last = None;
    let mut bytes = 2usize;
    for row in rows.iter().take(limit) {
        let item = row_value(row, now);
        // Reserve for per-row knowledge before selecting the actual page boundary.
        let knowledge_reserve = 512usize.saturating_add(item["declaration_origin_host_id"].as_str().map_or(0, str::len));
        let size = serde_json::to_vec(&item).map_err(ApiError::internal)?.len().saturating_add(knowledge_reserve);
        if bytes.saturating_add(size) > ITEM_BUDGET {
            if items.is_empty() {
                return Err(validation("a schedule declaration's metadata exceeds the client response size limit"));
            }
            has_more = true;
            break;
        }
        bytes += size;
        last = Some(row.subject.clone());
        items.push(item);
    }
    let mut hosts = state.configured_peers.iter().take(MAX_HOSTS).cloned().collect::<BTreeSet<_>>();
    hosts.insert(state.node.clone());
    for item in &items {
        for key in ["declaration_origin_host_id", "primary_host_id"] {
            if let Some(host) = item[key].as_str().and_then(|host| host.strip_prefix("host/")) { hosts.insert(host.into()); }
        }
    }
    let times = state.store.schedule_exchange_times(&hosts).map_err(ApiError::internal)?;
    for item in &mut items {
        let origin = item["declaration_origin_host_id"].as_str().and_then(|host| host.strip_prefix("host/")).map(str::to_owned);
        item["knowledge"] = knowledge(origin.as_deref(), &state.node, &times, now);
    }
    let expiry = now.saturating_add(CURSOR_AGE_MS);
    let next = if has_more {
        Some(encode(Position { instance:instance(state),actor:session.actor.clone(),authority:session.authority_actor.clone(),grant:session.pairing_grant.clone(),after:last.ok_or_else(cursor_error)?,limit,unheard_after_ms:threshold,expires_at:expiry })?)
    } else { None };
    let page = json!({"kind":"page","collection":"schedules","as_of":client_timestamp(now),
        "basis":"local-replicated-declarations","items":items,
        "page":{"limit":limit,"has_more":has_more,"next_cursor":next,"cursor_expires_at":has_more.then(||client_timestamp(expiry))},
        "coverage":coverage(state,&hosts,&times,now,threshold)?});
    if serde_json::to_vec(&page).map_err(ApiError::internal)?.len() > CLIENT_MAX_RESPONSE_BYTES - 4096 {
        return Err(validation("the schedule page exceeds the client response size limit"));
    }
    Ok((client_snapshot_at(state, index), page))
}

#[cfg(test)]
mod tests;
