//! Person-owned arrangements use the shared register projection, never glass privacy or history folds.
use crate::ArrangementsStore;
use super::*;

pub fn person(
    session: &ClientSession,
    requested: Option<&str>,
    write: bool,
) -> Result<String, ApiError> {
    require_scope(session, if write { "control.arrangements" } else { "read.arrangements" })?;
    let person = requested.ok_or_else(|| validation("arrangements require an explicit person/NAME owner"))?;
    if !person.starts_with("person/") || person.matches('/').count() != 1 || person.len() == 7 {
        return Err(validation("arrangement owner must be person/NAME"));
    }
    st3_schema::registry().validate_subject(person).map_err(|error| validation(error.message))?;
    if !acting_party(session) {
        return Err(forbidden("arrangements require a concrete person or trusted local fleet agent"));
    }
    if session.authority_actor.starts_with("person/") && session.authority_actor != person {
        return Err(forbidden("cross-person arrangements require delegated authority"));
    }
    Ok(person.to_owned())
}

/// Keep whole resources; byte bounds may shorten a count-bounded authoritative window.
pub fn window_end(items: &[Value], offset: usize, end: usize) -> Result<usize, ApiError> {
    let budget = CLIENT_MAX_RESPONSE_BYTES - 128_000;
    let mut used = 0;
    let mut kept = offset;
    for item in items.get(offset..end).unwrap_or_default() {
        let bytes = serde_json::to_vec(item).map_err(ApiError::internal)?.len() + 1;
        if used + bytes > budget {
            if kept == offset {
                return Err(validation("the arrangement exceeds the bounded client response budget"));
            }
            break;
        }
        used += bytes;
        kept += 1;
    }
    Ok(kept)
}

pub async fn list(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(request_snapshot): Extension<ClientSnapshot>,
    Query(query): Query<ClientListQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<ClientResourcePage>), ApiError> {
    let owner = person(&session, query.person.as_deref(), false)?;
    if query.actor.is_some() || query.history {
        return Err(validation("arrangements select a person and current state only"));
    }
    if query.cursor.is_some() {
        let page = client_page(&state, &request_snapshot, "arrangements", vec![], &query)?;
        return Ok((Extension(request_snapshot), Json(page)));
    }
    let read_state = state.clone();
    let (snapshot, items) = blocking_store(move || {
        read_state.store.read_snapshot(|index| {
            Ok((client_snapshot_at(&read_state, index), read_state.store.arrangements(&owner, index)?))
        })
    }).await?;
    let page = client_page(&state, &snapshot, "arrangements", items, &query)?;
    Ok((Extension(snapshot), Json(page)))
}

