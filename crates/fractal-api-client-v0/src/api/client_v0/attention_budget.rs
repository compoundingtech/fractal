use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionBudgetQuery {
    mission: String,
    person: Option<String>,
    at: Option<u128>,
}

fn authority(session: &ClientSession, requested: Option<&str>, now: u128) -> Result<String, ApiError> {
    require_scope(session, "read.projections")?;
    let person = requested.unwrap_or(&session.authority_actor);
    let recipient = session.transport == "local-web" && session.authority_actor == person
        && session.grant_expires_at.is_some_and(|expiry| expiry > now)
        && !session.local_peer && session.assistant_write.is_none() && session.pairing_grant.is_none();
    // Alternative B (not enabled): replace `recipient` below with
    // `recipient || (session.allows("read.projections") && session.assistant_write.is_none() && (session.authority_actor.starts_with("agent/") || session.pairing_grant.is_some()))`.
    if !recipient {
        return Err(forbidden("attention budget requires the authenticated embedded-web recipient"));
    }
    if !person.strip_prefix("person/").is_some_and(|id| !id.is_empty()
        && !id.contains('/') && !id.chars().any(char::is_whitespace)) {
        return Err(validation("person must be a concrete person/ subject"));
    }
    Ok(person.to_owned())
}

pub async fn get(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<AttentionBudgetQuery>,
) -> Result<Json<Value>, ApiError> {
    authority(&session, query.person.as_deref(), client_now_ms())?;
    let current = revalidate_session(&state, &session)?;
    let person = authority(&current, query.person.as_deref(), client_now_ms())?;
    if !fractal_schema::registry().validate_subject(&query.mission)
        .is_ok_and(|subject| subject.family == "mission") {
        return Err(validation("mission must be a native mission/ subject"));
    }
    let at = query.at.unwrap_or_else(client_now_ms);
    blocking_store(move || {
        let report = state.store.attention_budget_report(&query.mission, &person, at)?;
        Ok(Json(serde_json::to_value(report)?))
    }).await
}

#[cfg(test)]
mod tests;
