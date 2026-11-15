//! Person availability over client-v0. Reads show the person everything and anyone else only
//! the effective values; writes always act as the session's own person, never one the body names.
use crate::DirectiveNotesStore;
use crate::store::availability::{
    AvailabilityChange, AvailabilityStore, DeadlineClock, DeviceReport, DeviceState, Interruptions, Status,
};
use super::*;
use serde::Deserializer;

fn is_person(actor: &str) -> bool {
    actor.strip_prefix("person/")
        .is_some_and(|name| !name.is_empty() && !name.contains('/') && !name.contains(char::is_whitespace))
}

/// The person a write acts as: the session's own concrete person holding `scope`. Agents are
/// refused even though a local agent session holds every scope, and so is a local harness or
/// peer naming a person: only the person writes their own state.
pub(super) fn person_writer(session: &client_v0::ClientSession, scope: &str) -> Result<String, ApiError> {
    client_v0::require_scope(session, scope)?;
    if !is_person(&session.authority_actor) {
        return Err(client_v0::forbidden("only a person writes their own availability, presence or note; agents are refused"));
    }
    if session.local_peer {
        return Err(client_v0::forbidden("local harnesses and peers cannot write a person's availability, presence or note"));
    }
    Ok(session.authority_actor.clone())
}

/// The source a person-state write records, assigned by the daemon and never by the body: the
/// session's exact pairing grant, or this member for a local caller.
pub(super) fn writer_source(state: &AppState, session: &client_v0::ClientSession) -> String {
    session.pairing_grant.clone().unwrap_or_else(|| client_host_id(state.store.origin()))
}

/// Store refusals in the client vocabulary: bad input fails validation, a foreign writer is forbidden.
pub(super) fn client_refusal(error: ApiError) -> ApiError {
    match error.code.as_str() {
        "invalid-availability" | "invalid-presence" | "invalid-directive-note" => client_v0::validation(error.message),
        "availability-forbidden" | "directive-note-forbidden" => client_v0::forbidden(error.message),
        _ => error,
    }
}

/// A UTC RFC 3339 timestamp as Unix milliseconds.
fn unix_ms(value: &str, name: &str) -> Result<u128, ApiError> {
    fractal_schema::directive_notes::expiry(value).ok()
        .and_then(|time| u128::try_from(time.timestamp_millis()).ok())
        .filter(|ms| *ms > 0)
        .ok_or_else(|| client_v0::validation(format!("{name} must be a UTC RFC 3339 timestamp")))
}

fn until(value: Option<&str>, name: &str) -> Result<Option<u128>, ApiError> {
    value.map(|value| unix_ms(value, name)).transpose()
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityQuery {
    person: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Query(query): Query<AvailabilityQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    // A local harness or peer naming a person is still no person: it reads only coarsely.
    let person_reader = is_person(&session.authority_actor);
    let own_reader = person_reader && !session.local_peer;
    let reader = session.authority_actor;
    if !person_reader && !reader.starts_with("agent/") {
        return Err(client_v0::forbidden("identify the person or agent reading availability"));
    }
    if query.person.as_deref().is_some_and(|person| !is_person(person)) {
        return Err(client_v0::validation("person must be an exact person/NAME"));
    }
    let now = client_now_ms();
    let (snapshot, people) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            // An agent sees the people whose directive notes it reads, and only coarsely.
            let people = match query.person {
                Some(person) => vec![person],
                None if person_reader => vec![reader.clone()],
                None => state.store.directive_note_people(&reader)?.into_iter().collect(),
            };
            let mut views = Vec::with_capacity(people.len());
            for person in people {
                let view = state.store.person_availability(&person, now)?;
                views.push(if own_reader && person == reader { view } else { view.without_presence() });
            }
            Ok((client_snapshot_at(&state, index), views))
        })
    }).await?;
    Ok((Extension(snapshot), Json(json!({"people": people}))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    #[serde(default)]
    person: Option<String>,
    from: String,
    to: String,
    bucket_seconds: u64,
}

/// One person's availability over a past range in buckets, folded from the same claims and by
/// the same rules as the current read. The person sees their presence; anyone else does not.
pub async fn history(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Query(query): Query<HistoryQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    let person_reader = is_person(&session.authority_actor);
    if !person_reader && !session.authority_actor.starts_with("agent/") {
        return Err(client_v0::forbidden("identify the person or agent reading availability"));
    }
    let person = match query.person {
        Some(person) if is_person(&person) => person,
        Some(_) => return Err(client_v0::validation("person must be an exact person/NAME")),
        None if person_reader => session.authority_actor.clone(),
        None => return Err(client_v0::validation("an agent names the person whose availability history it reads")),
    };
    // A local harness or peer naming a person is still no person: it reads only coarsely.
    let own = person_reader && !session.local_peer && person == session.authority_actor;
    let (from, to) = (unix_ms(&query.from, "from")?, unix_ms(&query.to, "to")?);
    let now = client_now_ms();
    let (snapshot, history) = blocking_api(move || {
        let (snapshot, history) = state.store.read_snapshot(|index| Ok((
            client_snapshot_at(&state, index),
            state.store.person_availability_history(&person, from, to, query.bucket_seconds, now),
        ))).map_err(ApiError::internal)?;
        Ok((snapshot, history.map_err(ApiError::bad).map_err(client_refusal)?))
    }).await?;
    let history = if own { history } else { history.without_presence() };
    Ok((Extension(snapshot), Json(serde_json::to_value(history).map_err(ApiError::internal)?)))
}

/// A present key is required even when null: null clears the status override or the note.
pub(super) fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusChange {
    #[serde(deserialize_with = "required")]
    value: Option<Status>,
    #[serde(default)]
    until: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisChange<T> {
    value: T,
    #[serde(default)]
    until: Option<String>,
}

/// Absent axes stay unchanged.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilitySet {
    #[serde(default)]
    status: Option<StatusChange>,
    #[serde(default)]
    interruptions: Option<AxisChange<Interruptions>>,
    #[serde(default)]
    deadline_clock: Option<AxisChange<DeadlineClock>>,
}

pub async fn set(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Json(input): Json<AvailabilitySet>,
) -> Result<Json<Value>, ApiError> {
    let person = person_writer(&session, "write.availability")?;
    let mut changes = Vec::with_capacity(3);
    if let Some(change) = input.status {
        changes.push(AvailabilityChange::Status(change.value, until(change.until.as_deref(), "status.until")?));
    }
    if let Some(change) = input.interruptions {
        changes.push(AvailabilityChange::Interruptions(change.value, until(change.until.as_deref(), "interruptions.until")?));
    }
    let holds = input.deadline_clock.is_some();
    if let Some(change) = input.deadline_clock {
        changes.push(AvailabilityChange::DeadlineClock(change.value, until(change.until.as_deref(), "deadline_clock.until")?));
    }
    let source = writer_source(&state, &session);
    let now = client_now_ms();
    let store = state.store.clone();
    let availability = blocking_action(move || store.set_person_availability(&person, &source, &changes, now))
        .await.map_err(client_refusal)?;
    // The deadline clock moves asks' effective deadlines, which the reconciler enforces.
    if holds { signal_changed(&state) } else { signal_visible_change(&state) }
    Ok(Json(serde_json::to_value(availability).map_err(ApiError::internal)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceReport {
    state: DeviceState,
    #[serde(default)]
    last_input_at: Option<String>,
    observed_at: String,
}

pub async fn report(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Json(input): Json<PresenceReport>,
) -> Result<Json<Value>, ApiError> {
    let person = person_writer(&session, "write.presence")?;
    let source = writer_source(&state, &session);
    let report = DeviceReport {
        state: input.state,
        last_input_at: until(input.last_input_at.as_deref(), "last_input_at")?,
        observed_at: unix_ms(&input.observed_at, "observed_at")?,
    };
    let now = client_now_ms();
    let store = state.store.clone();
    let reported = blocking_action(move || store.report_person_presence(&person, &source, report, now))
        .await.map_err(client_refusal)?;
    signal_visible_change(&state);
    Ok(Json(serde_json::to_value(reported).map_err(ApiError::internal)?))
}

#[cfg(test)]
mod tests;
