//! Keyed observations and explicitly person-authorized resolution; neither route sends.
use super::*;
use crate::store::effects::EffectRecord;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectResolutionOutcome { Confirmed, Abandoned }

impl EffectResolutionOutcome {
    fn as_str(&self) -> &'static str {
        match self { Self::Confirmed => "confirmed", Self::Abandoned => "abandoned" }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveEffectRequest {
    pub actor: String,
    pub outcome: EffectResolutionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<String>,
    pub evidence: Vec<String>,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<crate::model::DelegationProof>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EffectResponse {
    pub effect_id: String,
    pub state: String,
    pub record: EffectRecord,
}

#[derive(Deserialize)]
pub struct EffectKeyQuery { pub key: String }

fn response(state: String, record: EffectRecord) -> EffectResponse {
    EffectResponse { effect_id: record.effect_id.clone(), state, record }
}

fn missing() -> ApiError {
    ApiError { status: StatusCode::NOT_FOUND, code: "effect-not-found".into(),
        message: "no effect exists with this identity".into(), details: Box::default() }
}

pub async fn by_key(State(state): State<AppState>, Query(query): Query<EffectKeyQuery>) -> Result<Json<EffectResponse>, ApiError> {
    blocking_api(move || {
        let (phase, record) = state.store.effect_by_key(&query.key).map_err(ApiError::bad)?.ok_or_else(missing)?;
        Ok(Json(response(phase, record)))
    }).await
}

pub async fn resolve(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
    session: Option<Extension<client_v0::ClientSession>>,
    bound: Option<Extension<BoundAgent>>,
    Json(request): Json<ResolveEffectRequest>,
) -> Result<Json<EffectResponse>, ApiError> {
    let refused = || ApiError { status: StatusCode::FORBIDDEN, code: "forbidden".into(),
        message: "effect resolution requires authenticated person authority; agent delegation does not authorize it".into(),
        details: Box::default() };
    // ClientSession is installed only after authenticating transport/pairing authority.
    // A request actor (even a person-shaped one) is never its own proof.
    let session = session.ok_or_else(refused)?.0;
    let session = client_v0::revalidate_session(&state, &session)?;
    if bound.is_some() || session.local_peer || session.assistant_write.is_some()
        || request.delegation.is_some() || !session.authority_actor.starts_with("person/")
        || session.authority_actor != request.actor {
        return Err(refused());
    }
    client_v0::require_scope(&session, "control.work")?;
    let subject = path.strip_suffix("/resolve").filter(|subject| subject.starts_with("effect/"))
        .ok_or_else(missing)?.to_owned();
    let writes = state.clone();
    let result = blocking_api(move || {
        writes.store.resolve_effect_authorized(&subject, &request.actor, &session.authority_actor,
            request.outcome.as_str(), request.provider_ref.as_deref(), &request.evidence, &request.idempotency_key)
            .map_err(|error| if error.code == "not-found" { missing() } else { ApiError::bad(error) })?;
        let (phase, record) = writes.store.effect_state(&subject).map_err(ApiError::bad)?.ok_or_else(missing)?;
        Ok(Json(response(phase, record)))
    }).await?;
    signal_changed(&state);
    Ok(result)
}
