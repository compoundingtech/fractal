use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionBudgetQuery {
    mission: String,
    person: Option<String>,
    #[serde(default, deserialize_with = "query_timestamp")]
    at: Option<u128>,
}

// URL-encoded optional values arrive as strings; serde_urlencoded's optional
// value deserializer does not implement u128. Preserve the full timestamp range.
fn query_timestamp<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u128>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|value| value.parse().map_err(serde::de::Error::custom))
        .transpose()
}

fn authority(session: &ClientSession, requested: Option<&str>, now: u128) -> Result<String, ApiError> {
    // Same projection-reader predicate as client_v0::request_latency and
    // collection_stream; same private-person restriction as attention collections.
    require_scope(session, "read.projections")?;
    if session.grant_expires_at.is_some_and(|expiry| expiry <= now) {
        return Err(forbidden("the attention budget reader session expired"));
    }
    // Authenticated pairing provenance is independent of the Unix transport.
    // Only an unpaired Unix person header lacks recipient authentication.
    if session.transport == "unix" && session.authority_actor.starts_with("person/")
        && session.pairing_grant.is_none()
    {
        return Err(forbidden("a Unix person header does not authenticate an attention recipient"));
    }
    if session.transport == "local-web"
        && (!session.grant_expires_at.is_some_and(|expiry| expiry > now)
            || session.local_peer || session.assistant_write.is_some() || session.pairing_grant.is_some())
    {
        return Err(forbidden("attention budget requires an authenticated embedded-web recipient"));
    }
    let person = person_filter(session, requested)?
        .ok_or_else(|| validation("person is required when the session has no person"))?;
    if !person.strip_prefix("person/").is_some_and(|id| !id.is_empty()
        && !id.contains('/') && !id.chars().any(char::is_whitespace)) {
        return Err(validation("person must be a concrete person/ subject"));
    }
    Ok(person)
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
