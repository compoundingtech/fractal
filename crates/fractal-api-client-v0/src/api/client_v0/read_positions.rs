//! Person read positions: the session person's `read-positions` list and the forward-only
//! `read-position.advance` action. Positions are person workspace state, authorized with the
//! arrangements scopes; a person session reads and writes only its own.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::ReadPositionsStore;
use super::*;
use fractal_schema::read_positions as schema;

/// The owner a session may read (`write = false`) or advance (`write = true`): an explicit
/// `person/NAME` equal to a person session's own, or any person for a trusted local fleet agent.
pub fn person(
    session: &ClientSession,
    requested: Option<&str>,
    write: bool,
) -> Result<String, ApiError> {
    require_scope(session, if write { "control.arrangements" } else { "read.arrangements" })?;
    let person = requested.ok_or_else(|| validation("read positions require an explicit person/NAME owner"))?;
    if !person.starts_with("person/") || person.matches('/').count() != 1 || person.len() == 7 {
        return Err(validation("read-position owner must be person/NAME"));
    }
    fractal_schema::registry().validate_subject(person).map_err(|error| validation(error.message))?;
    if !acting_party(session) {
        return Err(forbidden("read positions require a concrete person or trusted local fleet agent"));
    }
    if session.authority_actor.starts_with("person/") && session.authority_actor != person {
        return Err(forbidden("a person reads and advances only their own read positions"));
    }
    Ok(person.to_owned())
}

/// `GET /v1/client/read-positions?person=person/NAME`: every position of the person, ordered by
/// conversation, paged from one read snapshot.
pub async fn list(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(request_snapshot): Extension<ClientSnapshot>,
    Query(query): Query<ClientListQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<ClientResourcePage>), ApiError> {
    let owner = person(&session, query.person.as_deref(), false)?;
    if query.history || query.actor.is_some() || query.owner_run.is_some()
        || query.status.is_some() || query.owner.is_some() || query.state.is_some() || query.native_only
    {
        return Err(validation("read positions select a person and current state only"));
    }
    if query.cursor.is_some() {
        let page = client_page(&state, &request_snapshot, schema::COLLECTION, vec![], &query)?;
        return Ok((Extension(request_snapshot), Json(page)));
    }
    let read_state = state.clone();
    let (snapshot, items) = blocking_store(move || {
        read_state.store.read_snapshot(|index| {
            Ok((client_snapshot_at(&read_state, index), read_state.store.read_positions(&owner, index)?))
        })
    }).await?;
    let page = client_page(&state, &snapshot, schema::COLLECTION, items, &query)?;
    Ok((Extension(snapshot), Json(page)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Advance {
    owner: String,
    conversation: String,
    at: String,
}

/// What `read-position.advance` did: the position's resource ID, the claim it appended when
/// the position moved forward, and the stored position after the merge.
pub struct Advanced {
    pub id: String,
    pub claim: Option<ClaimRecord>,
    pub at: String,
}

/// Advance the owner's position in one conversation. A position that is not beyond the stored
/// one completes without a claim; either way the result names the stored position.
pub(super) async fn advance(
    state: &AppState,
    session: &ClientSession,
    request: &ActionRequest,
) -> Result<Advanced, ApiError> {
    let parameters: Advance = serde_json::from_value(request.parameters.clone())
        .map_err(|error| validation(error.to_string()))?;
    let owner = person(session, Some(&parameters.owner), true)?;
    if !schema::valid_conversation(&parameters.conversation) {
        return Err(validation("conversation must be FAMILY/ID without whitespace, at most 512 bytes"));
    }
    let at_ms = chrono::DateTime::parse_from_rfc3339(&parameters.at)
        .ok()
        .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
        .ok_or_else(|| validation("at must be an RFC 3339 instant at or after the Unix epoch"))?;
    let actor = session_claim_actor(session);
    // The claim key follows the action's idempotency key, so a retry after a lost receipt
    // replays the same claim; a later advance with a fresh key is a fresh claim.
    let key = format!("read-position-advance:{}", creation_key(session, request));
    let id = format!("{}/{}", schema::subject(&owner), parameters.conversation);
    let store = state.store.clone();
    let conversation = parameters.conversation;
    let advanced = blocking_action(move || {
        store.advance_read_position(&owner, &conversation, at_ms, &actor, key)
    })
    .await?;
    Ok(Advanced { id, claim: advanced.claim, at: client_timestamp(u128::from(advanced.at_ms)) })
}

#[cfg(test)]
mod tests;
