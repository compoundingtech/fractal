//! Minimal assistant read projections. Each one reuses a native snapshot reader and copies
//! only allowlisted identifiers, closed enums, counts and timestamps out of it, so titles,
//! goals, previews, paths and other free text in source rows never reach an assistant.
//! Content reads return one bounded text range of one exactly named resource.
use crate::MissionsStore;
use crate::SeatsStore;
use crate::EvidenceStore;
use super::*;
use crate::api::published_attention::{
    published_assistant_attention_rows, published_view_not_ready,
};

#[path = "reads_paging.rs"]
mod paging;
use paging::page;

/// Default and ceiling of an assistant response, including its whole JSON envelope. The
/// route enforces the envelope; content sources read no more than this many bytes.
pub const DEFAULT_RESPONSE_BYTES: usize = 4 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024;

const ATTENTION_KINDS: &[&str] = &[
    "human-gate", "launch-approval", "revision-approval", "unread-message", "person-step",
    "agent-request", "fault", "harness-login", "harness-prompt",
];
const AGENT_STATES: &[&str] = &[
    "desired", "starting", "running", "waiting", "stopped", "failed", "suspended",
];
const AGENT_LIFECYCLES: &[&str] = &["standing", "owner", "bounded"];
const MISSION_PHASES: &[&str] = &[
    "draft", "ready", "running", "standing", "blocked", "completed", "failed", "cancelled",
    "retired",
];

const METADATA_FIELDS: &[&str] = &["cursor", "limit", "max_response_bytes"];
const SUMMARY_FIELDS: &[&str] = &["max_response_bytes"];
const CONVERSATION_FIELDS: &[&str] = &[
    "agent_id", "session_id", "reference", "revision", "start", "end", "max_response_bytes",
];
const TERMINAL_FIELDS: &[&str] = &[
    "agent_id", "terminal_id", "incarnation_id", "screen_revision", "max_response_bytes",
];

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
        "summary" => {
            allow(parameters, SUMMARY_FIELDS, max_bytes)?;
            summary(state).await
        }
        "attention" | "attention-rich" | "missions" | "missions-rich" | "agents" | "agents-rich" =>
            page(state, snapshot, projection, parameters, max_bytes, binding, context).await,
        "attention-content" => {
            allow(parameters, &["alert_id","max_response_bytes"], max_bytes)?;
            let id = required(parameters, "alert_id")?;
            let (cut, _, items) = published_assistant_attention_rows(
                &state.store, snapshot.store_index, &owner(state)?,
            ).ok_or_else(|| published_view_not_ready("attention"))?;
            let item = items.iter()
                .find(|item| item["source_id"] == id || item["id"] == id)
                .ok_or_else(|| ApiError::not_found("owner alert absent"))?;
            Ok((client_snapshot_at(state, cut), json!({
                "id": item["source_id"], "episode": item["episode"], "kind": item["attention_kind"],
                "title": item["title"], "detail": item["detail"], "request": attention_request(item),
            })))
        },
        "conversation-content" => Ok((snapshot.clone(), conversation(state, parameters, max_bytes).await?)),
        "terminal-content" => Ok((snapshot.clone(), terminal(state, parameters, max_bytes).await?)),
        _ => Err(ApiError::not_found("unknown assistant projection")),
    }
}

/// Every parameter is named by the projection; a stated budget must be the one enforced.
fn allow(
    parameters: &BTreeMap<String, String>,
    fields: &[&str],
    max_bytes: usize,
) -> Result<(), ApiError> {
    if !(1..=MAX_RESPONSE_BYTES).contains(&max_bytes) {
        return Err(validation(format!("max_response_bytes must be between 1 and {MAX_RESPONSE_BYTES}")));
    }
    if let Some(unknown) = parameters.keys().find(|key| !fields.contains(&key.as_str())) {
        return Err(validation(format!("query parameter `{unknown}` is not allowed here")));
    }
    if let Some(stated) = parameters.get("max_response_bytes")
        && stated.parse::<usize>().ok() != Some(max_bytes)
    {
        return Err(validation("max_response_bytes does not match the enforced budget"));
    }
    Ok(())
}

fn required<'a>(parameters: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str, ApiError> {
    parameters
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| validation(format!("query parameter `{name}` is required")))
}

fn budget_exceeded(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        code: "response-budget-exceeded".into(),
        message: message.into(),
        details: Box::default(),
    }
}

/// An identifier copied out of a source row, or nothing. Identifiers are short tokens; a value
/// with spaces, controls or prose punctuation is source text, which never leaves.
fn identifier_allowed(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || b"/._:-@+=~".contains(&byte)
        })
}

fn ident(value: Option<&str>) -> Option<String> {
    value.filter(|value| identifier_allowed(value)).map(str::to_owned)
}

fn closed(value: Option<&str>, allowed: &[&'static str]) -> &'static str {
    value
        .and_then(|value| allowed.iter().find(|item| **item == value))
        .copied()
        .unwrap_or("unknown")
}

fn attention_kind(kind: &str) -> &'static str {
    if kind.starts_with("custom.") {
        "custom"
    } else {
        closed(Some(kind), ATTENTION_KINDS)
    }
}

/// A timestamp the native reader rendered, kept only if it is one.
fn timestamp(value: &Value) -> Value {
    value
        .as_str()
        .filter(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok())
        .map_or(Value::Null, |at| json!(at))
}

fn owner(state: &AppState) -> Result<String, ApiError> {
    state
        .owning_person
        .clone()
        .filter(|person| ident(Some(person.as_str())).is_some() && person.starts_with("person/"))
        .ok_or_else(|| forbidden("assistant reads require a configured owning person"))
}


async fn summary(state: &AppState) -> Result<(ClientSnapshot, Value), ApiError> {
    let person = owner(state)?;
    let reader = state.clone();
    blocking_store(move || {
        reader.store.clone().read_snapshot(|index| {
            let read = || -> Result<_, ApiError> {
            let snapshot = client_snapshot_at(&reader, index);
            let now = client_snapshot_time(&snapshot);
            let (_, _, attention) = published_assistant_attention_rows(&reader.store, index, &person)
                .ok_or_else(|| published_view_not_ready("attention"))?;
            let Some((cut, agents, _)) = reader.store.published_agent_roster(index, false) else {
                reader.store.request_agent_roster_refresh();
                return Err(published_view_not_ready("agents"));
            };
            if cut < index {
                reader.store.request_agent_roster_refresh();
            }
            let mut agent_states = BTreeMap::<&str, u64>::new();
            let local_host = client_host_id(reader.store.origin());
            for agent in agents.iter() {
                if ident(agent["id"].as_str()).is_some_and(|id| id.starts_with("agent/")) {
                    let reported = agent["harness_state"].as_str();
                    let age = agent["_status_source"]["observed_at_unix_ms"].as_u64()
                        .map(|at| u64::try_from(now.saturating_sub(u128::from(at))).unwrap_or(u64::MAX));
                    // The scalar state rules of overlay_agent_resources/overlay_down, followed
                    // by the same local delivery assessment; no card is built just to count it.
                    let mut runtime = agent["state"].as_str();
                    if age.is_some_and(|age| age > 90_000)
                        && matches!(reported, Some("ready" | "idle")) && runtime == Some("running")
                    {
                        runtime = Some("waiting");
                    }
                    if matches!(runtime, Some("desired" | "starting")) {
                        if age.is_some_and(|age| age <= HEARTBEAT_MS)
                            && !matches!(reported, None | Some("starting" | "ended" | "failed"))
                        {
                            runtime = Some(match reported {
                                Some("working") => "running",
                                Some("ready" | "idle") if age.is_some_and(|age| age <= 90_000) => "running",
                                _ => "waiting",
                            });
                        } else if agent["_launch_since_ms"].as_u64()
                            .is_some_and(|since| now.saturating_sub(u128::from(since)) > u128::from(LAUNCH_WINDOW_MS))
                        {
                            runtime = Some("down");
                        }
                    }
                    if agent_delivery_presence(agent, runtime, &local_host)
                        .is_some_and(|assessment| assessment.stale())
                    {
                        runtime = Some("waiting");
                    }
                    *agent_states.entry(closed(runtime, AGENT_STATES)).or_default() += 1;
                }
            }
            let mut mission_states = BTreeMap::<&str, u64>::new();
            for (phase, count) in reader.store.assistant_mission_state_counts_at(now).map_err(ApiError::internal)? {
                *mission_states.entry(closed(Some(&phase), MISSION_PHASES)).or_default() += count;
            }
            let value = json!({
                "kind": "assistant-summary",
                "as_of": client_timestamp(now),
                "counts": {
                    "attention": attention.len(),
                    "agent_states": agent_states,
                    "mission_states": mission_states,
                },
            });
            Ok((snapshot, value))
            };
            Ok(read())
        })
    })
    .await?
}

/// Native cards put informational person-step requests in `update`, not `request`.
/// Rich/content assistant reads retain that payload under their existing request field;
/// metadata never calls this helper or copies either source field.
fn attention_request(item: &Value) -> &Value {
    item.get("request").filter(|request| !request.is_null())
        .unwrap_or(&item["update"])
}

/// Translate one published native attention row; metadata never copies source prose.
fn attention_row(item: &Value, rich: bool) -> anyhow::Result<Value> {
    let episode = ident(item["episode"].as_str()).unwrap_or_default();
    let agent = [item["conversation_id"].as_str(), item["requester_id"].as_str()]
        .into_iter().flatten().find(|agent| agent.starts_with("agent/"));
    let mut row = json!({
        "id": client_attention_id(
            item["source_id"].as_str().ok_or_else(|| anyhow::anyhow!("attention source id absent"))?,
            item["person_id"].as_str().ok_or_else(|| anyhow::anyhow!("attention person id absent"))?,
            item["episode"].as_str().ok_or_else(|| anyhow::anyhow!("attention episode absent"))?,
        )?,
        "attention_kind": attention_kind(item["attention_kind"].as_str().unwrap_or_default()),
        "person_id": ident(item["person_id"].as_str()),
        "agent_id": ident(agent),
        "mission_id": ident(item["mission_id"].as_str()),
        "mission_run_id": ident(item["mission_run_id"].as_str()),
        "revision": episode,
        "episode": episode,
        "waiting_since": timestamp(&item["requested_at"]),
    });
    if rich {
        row["title"] = item["title"].clone();
        row["detail"] = item["detail"].clone();
        row["request"] = attention_request(item).clone();
        row["target_id"] = item["source_id"].clone();
        row["evidence"] = item["targets"].clone();
    }
    Ok(row)
}

/// Current missions: phase, the open (else newest) run, its step and blocked counts and the
/// agents it names. Goals, titles, step paths and status prose stay in the source.
fn mission_rows(missions: &[Value], rich: bool) -> anyhow::Result<Vec<Value>> {
    let mut rows = Vec::with_capacity(missions.len());
    for mission in missions {
        let Some(id) = ident(mission["id"].as_str()) else {
            continue;
        };
        let runs = mission["run_details"].as_array().map(Vec::as_slice).unwrap_or_default();
        let open = |run: &&Value| !matches!(run["status"].as_str(), Some("completed" | "failed" | "cancelled"));
        let run = runs.iter().rev().find(open).or_else(|| runs.last());
        let (run_id, total, completed, blocked, agents) = match run {
            Some(run) => {
                let steps = run["steps"]
                    .as_array()
                    .or_else(|| run["current_steps"].as_array())
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let agents = steps
                    .iter()
                    .flat_map(|step| [step["assignee"].as_str(), step["claimant"].as_str()])
                    .flatten()
                    .filter(|agent| agent.starts_with("agent/"))
                    .filter_map(|agent| ident(Some(agent)))
                    .collect::<BTreeSet<_>>();
                (
                    ident(run["id"].as_str()),
                    run["progress"]["total"].as_u64().unwrap_or(0),
                    run["progress"]["done"].as_u64().unwrap_or(0),
                    steps.iter().filter(|step| step["state"] == "blocked").count(),
                    agents,
                )
            }
            None => (None, 0, 0, 0, BTreeSet::new()),
        };
        let mut row = json!({
            "id": id,
            "run_id": run_id,
            "agent_ids": agents,
            "phase": closed(mission["state"].as_str(), MISSION_PHASES),
            "step_counts": {"total": total, "completed": completed},
            "blocked_count": blocked,
            "revision": ident(mission["revision"].as_str()),
            "updated_at": timestamp(&mission["updated_at"]),
        });
        if rich {
            row["title"] = mission["title"].clone();
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Allowlist only the selected page's native cards; never fold the whole roster.
fn agent_rows(agents: &[Value], rich: bool) -> Vec<Value> {
    agents
        .iter()
        .filter_map(|agent| {
            let id = ident(agent["id"].as_str()).filter(|id| id.starts_with("agent/"))?;
            let work = &agent["current_work"][0];
            let mut row = json!({
                "id": id,
                "lifecycle": agent["lifecycle"].as_str().map(|lifecycle| closed(Some(lifecycle), AGENT_LIFECYCLES)),
                "runtime_state": closed(agent["state"].as_str(), AGENT_STATES),
                "current_mission_id": ident(work["mission_id"].as_str()),
                "current_run_id": ident(work["mission_run_id"].as_str().or_else(|| agent["owner_run_id"].as_str())),
                "updated_at": timestamp(&agent["updated_at"]),
            });
            if rich {
                row["title"] = json!(id.trim_start_matches("agent/"));
            }
            Some(row)
        })
        .collect()
}

/// Escaped JSON string bytes of `text`, so a budget holds for the encoded response.
fn encoded_len(text: &str) -> usize {
    text.chars()
        .map(|character| match character {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            character if u32::from(character) < 0x20 => 6,
            character => character.len_utf8(),
        })
        .sum()
}

fn offset(parameters: &BTreeMap<String, String>, name: &str) -> Result<usize, ApiError> {
    required(parameters, name)?
        .parse()
        .map_err(|_| validation(format!("`{name}` must be a byte offset")))
}

/// One exact UTF-8 byte range of one authenticated conversation text. The native record
/// is opened only if it fits the response budget, so no larger body is ever read. The
/// range must lie on character boundaries inside the text and fit the budget; it is never
/// widened, split or continued automatically. `next_range` names the next equal range.
async fn conversation(
    state: &AppState,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
) -> Result<Value, ApiError> {
    allow(parameters, CONVERSATION_FIELDS, max_bytes)?;
    let agent_id = required(parameters, "agent_id")?.to_owned();
    let session_id = required(parameters, "session_id")?.to_owned();
    let reference = required(parameters, "reference")?.to_owned();
    let revision = required(parameters, "revision")?.to_owned();
    let (start, end) = (offset(parameters, "start")?, offset(parameters, "end")?);
    if !agent_id.starts_with("agent/") || ident(Some(agent_id.as_str())).is_none() {
        return Err(validation("agent_id must name an agent"));
    }
    if start >= end {
        return Err(validation("the content range must be nonempty"));
    }
    if end - start > max_bytes {
        return Err(budget_exceeded("the content range exceeds the response budget"));
    }
    if conversation_session_id(state, &agent_id)? != session_id {
        return Err(stale("the session is not the agent's current conversation"));
    }
    let reader = state.clone();
    let (session, wanted) = (session_id.clone(), reference.clone());
    let (text, pinned) = crate::api::read_deadline::spawn_blocking(move || {
        super::super::conversation_blocks::assistant_text(
            &reader, &session, &wanted, max_bytes,
        )
    })
    .await
    .map_err(ApiError::internal)??;
    let pinned = match pinned {
        Value::String(revision) => revision,
        other => other.to_string(),
    };
    if pinned != revision {
        return Err(stale("the content revision changed; read the current revision"));
    }
    if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return Err(validation(
            "the content range must lie on UTF-8 character boundaries within the text",
        ));
    }
    let slice = &text[start..end];
    if encoded_len(slice) > max_bytes {
        return Err(budget_exceeded("the encoded content range exceeds the response budget"));
    }
    let next_range = (end < text.len()).then(|| {
        let mut next = (end + (end - start)).min(text.len());
        while !text.is_char_boundary(next) {
            next += 1;
        }
        json!({"start": end, "end": next})
    });
    Ok(json!({
        "kind": "assistant-conversation-content",
        "agent_id": agent_id,
        "session_id": session_id,
        "reference": reference,
        "revision": revision,
        "start": start,
        "end": end,
        "text": slice,
        "returned_bytes": slice.len(),
        "truncated": end < text.len(),
        "next_range": next_range,
    }))
}

/// The current screen of one exactly named terminal incarnation and screen revision, as
/// text, cut on a character boundary to the budget. The shared screen is borrowed, never
/// rendered whole: only the returned characters are copied. One screen, never a stream.
pub async fn terminal(
    state: &AppState,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
) -> Result<Value, ApiError> {
    allow(parameters, TERMINAL_FIELDS, max_bytes)?;
    let agent_id = required(parameters, "agent_id")?;
    let terminal_id = required(parameters, "terminal_id")?;
    let incarnation_id = required(parameters, "incarnation_id")?;
    let screen_revision = required(parameters, "screen_revision")?;
    if !agent_id.starts_with("agent/") || ident(Some(agent_id)).is_none() {
        return Err(validation("agent_id must name an agent"));
    }
    if terminal_id != format!("terminal/{agent_id}") {
        return Err(validation("terminal_id must be the agent's terminal"));
    }
    let live = crate::api::read_deadline::query(
        &state.store,
        "/v1/client/assistant/terminal-content",
        || terminal_live_session(state, agent_id, Some(incarnation_id)),
    )?;
    if !live.terminal {
        return Err(validation("the requested runtime does not expose a terminal"));
    }
    let mut screens = crate::api::terminal_view::subscribe(
        &state.pty_root,
        &live.runtime_id,
        &live.incarnation_id,
    );
    let first = tokio::time::Instant::now() + TERMINAL_FIRST_SCREEN_TIMEOUT;
    let screen = crate::api::terminal_view::next_screen(&mut screens, None, first)
        .await
        .map_err(terminal_view_error)?
        .ok_or_else(|| terminal_unavailable("the terminal screen did not arrive"))?;
    if screen.revision() != screen_revision {
        return Err(stale("the terminal screen changed; read the current screen revision"));
    }
    let mut text = String::with_capacity(max_bytes.min(4 * 1024));
    let mut used = 0;
    let mut truncated = false;
    'lines: for (index, line) in screen.line_texts().enumerate() {
        let separator = if index == 0 { "" } else { "\n" };
        for character in separator.chars().chain(line.chars()) {
            let size = encoded_len(character.encode_utf8(&mut [0; 4]));
            if used + size > max_bytes {
                truncated = true;
                break 'lines;
            }
            used += size;
            text.push(character);
        }
    }
    Ok(json!({
        "kind": "assistant-terminal-content",
        "agent_id": agent_id,
        "terminal_id": terminal_id,
        "incarnation_id": incarnation_id,
        "screen_revision": screen_revision,
        "returned_bytes": text.len(),
        "text": text,
        "truncated": truncated,
    }))
}

#[cfg(test)]
#[path = "reads_tests.rs"]
pub mod tests;
