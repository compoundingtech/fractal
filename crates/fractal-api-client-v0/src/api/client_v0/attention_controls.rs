//! A person's own controls over their open attention rows, shared by every client:
//! `attention.seen`, `attention.snooze` and `attention.dismiss`. Only the session's own person
//! may use them: agents and assistant grants are refused whatever their scopes, and no
//! parameter names an actor. Seen and snooze change only that person's view of a row; dismiss
//! sets an ask aside with the person's own `dismissed` outcome, never one of the asker's answers.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::PersonWorkStore;
use super::*;
use fractal_store::store::attention_controls::{Control, Controls};

/// What one control did: the subjects it touched, and for seen and snooze the row's controls
/// after it.
pub struct Controlled {
    pub affected: Vec<String>,
    pub state: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seen {
    attention_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snooze {
    attention_id: String,
    until: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dismiss {
    attention_id: String,
    #[serde(default)]
    text: Option<String>,
}

/// The session's own person, the only party a control is for.
fn person(session: &ClientSession) -> Result<String, ApiError> {
    if session.assistant_write.is_some() || !session.authority_actor.starts_with("person/") {
        return Err(forbidden("attention controls are the person's own: agents and assistants cannot use them"));
    }
    Ok(session.authority_actor.clone())
}

fn parameters<T: serde::de::DeserializeOwned>(request: &ActionRequest) -> Result<T, ApiError> {
    serde_json::from_value(request.parameters.clone()).map_err(|error| validation(error.to_string()))
}

/// The person's open row `id`, read as their clients read it.
async fn open_row(state: &AppState, person: &str, id: &str) -> Result<Value, ApiError> {
    if !fractal_schema::attention_controls::valid_attention_id(id) {
        return Err(validation("attention_id must be an open attention row id"));
    }
    let store = state.store.clone();
    let person = person.to_owned();
    let id = id.to_owned();
    blocking_store(move || {
        Ok(client_attention_resources(&store, Some(&person), false)?
            .into_iter()
            .find(|row| row["id"] == id.as_str()))
    })
    .await?
    .ok_or_else(|| ApiError::not_found("no open attention row of this person has that id"))
}

fn state_value(id: &str, controls: Controls) -> Value {
    let time = |at: Option<u128>| at.map_or(Value::Null, |at| json!(client_timestamp(at)));
    json!({"attention_id": id, "seen_at": time(controls.seen_at_ms), "snoozed_until": time(controls.snoozed_until_ms)})
}

pub(super) async fn control(
    state: &AppState,
    session: &ClientSession,
    request: &ActionRequest,
) -> Result<Controlled, ApiError> {
    let person = person(session)?;
    // The client the person acted from: the exact pairing grant, or this member for a local caller.
    let source = session
        .pairing_grant
        .clone()
        .filter(|_| !session.local_peer)
        .unwrap_or_else(|| client_host_id(state.store.origin()));
    // The claim key follows the action's idempotency key, so a retry after a lost receipt
    // replays the same claim.
    let key = format!("{}:{}", request.action_type, creation_key(session, request));
    let (id, control) = match request.action_type.as_str() {
        "attention.seen" => {
            let Seen { attention_id } = parameters(request)?;
            (attention_id, Control::Seen)
        }
        "attention.snooze" => {
            let Snooze { attention_id, until } = parameters(request)?;
            let until = until
                .map(|until| {
                    chrono::DateTime::parse_from_rfc3339(&until)
                        .ok()
                        .and_then(|at| u128::try_from(at.timestamp_millis()).ok())
                        .filter(|at| *at > client_now_ms())
                        .ok_or_else(|| validation("until must be a future RFC 3339 instant, or null to end the snooze"))
                })
                .transpose()?;
            (attention_id, Control::Snooze(until))
        }
        _ => return dismiss(state, session, &person, request).await,
    };
    open_row(state, &person, &id).await?;
    let store = state.store.clone();
    let (claim, controls) = {
        let (person, id) = (person.clone(), id.clone());
        blocking_action(move || store.record_attention_control(&person, &id, control, &source, key)).await?
    };
    if claim.is_some() {
        signal_visible_change(state);
    }
    Ok(Controlled {
        affected: claim.map(|claim| claim.subject).into_iter().collect(),
        state: Some(state_value(&id, controls)),
    })
}

/// Dismiss sets an ask aside with the person's own outcome, `dismissed`, and tells the asker;
/// an update is read. Every other kind keeps its own actions.
async fn dismiss(state: &AppState, session: &ClientSession, person: &str, request: &ActionRequest) -> Result<Controlled, ApiError> {
    let Dismiss { attention_id, text } = parameters(request)?;
    let row = open_row(state, person, &attention_id).await?;
    // A mission step assigned to the person has no asker: it is their work, not an ask.
    if row["attention_kind"] != "person-step" || row.get("requester_id").is_none() {
        return Err(ApiError::bad(FractalError::new(
            "attention-dismiss-unsupported",
            format!(
                "a {} row is not an ask: use its own actions ({})",
                row["attention_kind"].as_str().unwrap_or("non-ask"),
                row["actions"].as_array().map_or_else(String::new, |actions| {
                    actions.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                }),
            ),
        )));
    }
    // An ask that offers its own `dismiss` answer keeps it (person_request::answer), so this
    // action could only pick the asker's answer: the person answers it with work.done instead.
    let own_dismiss = row["request"]["answers"].as_array().is_some_and(|answers| {
        answers.iter().any(|answer| answer["id"] == crate::person_request::DISMISS)
    });
    if own_dismiss {
        return Err(ApiError::bad(FractalError::new(
            "attention-dismiss-unsupported",
            "this ask offers its own `dismiss` answer: answer it with work.done",
        )));
    }
    let update = row.get("update").is_some();
    if update && text.is_some() {
        return Err(validation("an update is only read: dismissing it takes no text"));
    }
    let target = row["source_id"].as_str().ok_or_else(|| ApiError::internal("ask row has no source"))?.to_owned();
    let episode = row["episode"].as_str().ok_or_else(|| ApiError::internal("ask row has no episode"))?.to_owned();
    let answer = if update {
        json!({"id": "read"})
    } else {
        json!({"id": crate::person_request::DISMISS, "text": text})
    };
    let response = PersonStepResponse {
        delegation: None,
        subject: target,
        actor: person.to_owned(),
        summary: String::new(),
        evidence: Vec::new(),
        episode: Some(episode),
        idempotency_key: request.idempotency_key.clone(),
        answer: Some(serde_json::from_value(answer).map_err(ApiError::internal)?),
    };
    // A browser-pairing approval also checks the paired device, as work.done does.
    let device = session.pairing_grant.clone().filter(|_| !session.local_peer).map(|grant| (grant, session.actor.clone()));
    let store = state.store.clone();
    let view = blocking_action(move || {
        let device = device.as_ref().map(|(grant, actor)| (grant.as_str(), actor.as_str()));
        store.finish_person_step_kind(&response, false, None, false, device)
    })
    .await?;
    // The step that waits on the ask resumes.
    signal_changed(state);
    Ok(Controlled { affected: vec![view.subject], state: None })
}
