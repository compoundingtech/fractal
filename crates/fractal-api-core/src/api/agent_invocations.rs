//! Generic invocation evidence and private retained input/transcript reads.
use super::*;
use super::client_v0::{ClientSession, require_scope};
use fractal_store::invocation_artifacts::{self, InvocationArtifact};

#[derive(Deserialize)]
pub struct ArtifactQuery { invocation: String, kind: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRequest {
    pub agent: String,
    pub incarnation: String,
    pub invocation_id: String,
    pub message_id: String,
    pub text: String,
}

/// Only the authenticated provider's agent can attest to its native input envelope.
pub async fn input(
    State(state): State<AppState>,
    bound: Option<BoundActor>,
    peer: Option<Extension<NativeDeliveryPeer>>,
    session: Option<Extension<ClientSession>>,
    Json(request): Json<InputRequest>,
) -> Result<Json<Value>, ApiError> {
    let actor = bound.as_ref().map(BoundActor::actor)
        .or_else(|| peer.as_ref().map(|Extension(peer)| peer.agent.as_str()))
        .or_else(|| session.as_ref().map(|Extension(session)| session.authority_actor.as_str()));
    if actor != Some(request.agent.as_str()) || matches!(bound, Some(BoundActor::Run(_)))
        || peer.as_ref().is_some_and(|Extension(peer)| peer.agent != request.agent)
        || session.as_ref().is_some_and(|Extension(session)| session.authority_actor != request.agent)
    {
        return Err(ApiError { status: StatusCode::FORBIDDEN, code: "foreign-agent-actor".into(),
            message: "only the invocation's authenticated agent may capture native input".into(),
            details: Box::default() });
    }
    if let Some(Extension(session)) = session.as_ref() {
        require_scope(session, "control.runtimes")?;
    }
    let receipts = state.store.invocation_receipts(&request.invocation_id).map_err(ApiError::internal)?;
    if request.incarnation.is_empty() || request.message_id.is_empty()
        || !receipts.iter().any(|receipt| receipt.agent == request.agent
            && receipt.incarnation.as_deref() == Some(request.incarnation.as_str())
            && receipt.event_key.as_deref() == Some(request.message_id.as_str()))
    {
        return Err(ApiError { status: StatusCode::CONFLICT, code: "invocation-input-mismatch".into(),
            message: "input must match an exact invocation receipt's agent, incarnation and message".into(),
            details: Box::default() });
    }
    // Retry the original immutable capture rather than replacing it with a later envelope.
    let prior = state.store.claims_for(&request.agent, Some("harness.invocation-artifact"))
        .map_err(ApiError::internal)?.into_iter().find(|claim|
            claim.body["fields"]["invocation_id"] == request.invocation_id
                && claim.body["fields"]["kind"] == "input"
                && claim.body["fields"]["artifact"]["message_id"] == request.message_id);
    if let Some(prior) = prior {
        let artifact: InvocationArtifact = serde_json::from_value(prior.body["fields"]["artifact"].clone())
            .map_err(ApiError::internal)?;
        if !invocation_artifacts::matches_input(&artifact, &request.text) {
            return Err(ApiError { status: StatusCode::CONFLICT, code: "invocation-input-conflict".into(),
                message: "this invocation message already has a different immutable input".into(),
                details: Box::default() });
        }
        return Ok(Json(json!({"status":"retained","artifact":artifact})));
    }
    let mut artifact = invocation_artifacts::capture_input(&state.state_dir,
        &request.invocation_id, &request.text, now_ms()).map_err(ApiError::internal)?;
    artifact.incarnation = Some(request.incarnation);
    artifact.message_id = Some(request.message_id);
    let artifact = invocation_artifacts::record_exact(&state.store, &request.agent, &artifact)
        .map_err(ApiError::internal)?.ok_or_else(|| ApiError {
            status: StatusCode::CONFLICT, code: "invocation-input-conflict".into(),
            message: "this invocation message already has a different immutable input".into(),
            details: Box::default() })?;
    signal_changed(&state);
    Ok(Json(json!({"status":"retained","artifact":artifact})))
}

fn invocation_agent(store: &Store, invocation: &str) -> Result<String, ApiError> {
    let receipts = store.invocation_receipts(invocation).map_err(ApiError::internal)?;
    receipts.first().map(|receipt| receipt.agent.clone())
        .ok_or_else(|| ApiError::not_found(format!("invocation `{invocation}` does not exist")))
}

pub async fn get(
    State(state): State<AppState>, AxumPath(invocation): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    blocking_store(move || {
        let receipts = store.invocation_receipts(&invocation)?;
        let Some(first) = receipts.first() else { return Ok(None); };
        let artifacts = store.claims_for(&first.agent, Some("harness.invocation-artifact"))?
            .into_iter().filter(|claim| claim.body["fields"]["invocation_id"] == invocation)
            .collect::<Vec<_>>();
        Ok(Some(json!({"invocation_id":invocation,"receipts":receipts,"artifacts":artifacts})))
    }).await?.map(Json).ok_or_else(|| ApiError::not_found("invocation does not exist"))
}

fn authorize(state: &AppState, session: &ClientSession, agent: &str) -> Result<(), ApiError> {
    require_scope(session, "read.projections")?;
    if session.authority_actor == agent
        || state.owning_person.as_deref() == Some(session.authority_actor.as_str())
    { return Ok(()); }
    Err(ApiError { status: StatusCode::FORBIDDEN, code:"forbidden".into(),
        message:"invocation artifacts are private to their agent and the configured owning person".into(),
        details:Box::default() })
}

fn retained(store: &Store, agent: &str, invocation: &str, kind: &str, hash: &str) -> Result<InvocationArtifact, ApiError> {
    let latest = store.claims_for(agent, Some("harness.invocation-artifact")).map_err(ApiError::internal)?
        .into_iter().rev().find(|claim| claim.body["fields"]["invocation_id"] == invocation
            && claim.body["fields"]["kind"] == kind
            && (claim.body["fields"]["sha256"] == hash || claim.body["fields"]["artifact"]["sha256"] == hash))
        .ok_or_else(|| ApiError::not_found("invocation artifact does not exist"))?;
    if latest.body["fields"]["status"] != "retained" {
        return Err(ApiError {status:StatusCode::GONE,code:"artifact-deleted".into(),
            message:"the invocation artifact reference was deleted".into(),details:Box::default()});
    }
    serde_json::from_value(latest.body["fields"]["artifact"].clone()).map_err(ApiError::internal)
}

pub async fn artifact(
    State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    AxumPath(hash): AxumPath<String>, Query(query): Query<ArtifactQuery>,
) -> Result<Response, ApiError> {
    let agent = invocation_agent(&state.store, &query.invocation)?;
    authorize(&state, &session, &agent)?;
    let artifact = retained(&state.store, &agent, &query.invocation, &query.kind, &hash)?;
    let bytes = invocation_artifacts::read(&state.state_dir, &artifact, now_ms()).map_err(ApiError::internal)?
        .ok_or_else(|| ApiError {status:StatusCode::GONE,code:"artifact-expired".into(),
            message:"invocation artifact bytes are unavailable after retention; metadata remains".into(),details:Box::default()})?;
    Ok(([(axum::http::header::CONTENT_TYPE,"application/octet-stream"),
         (axum::http::header::CACHE_CONTROL,"private, no-store")],bytes).into_response())
}

pub async fn delete_artifact(
    State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    AxumPath(hash): AxumPath<String>, Query(query): Query<ArtifactQuery>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "control.runtimes")?;
    let agent = invocation_agent(&state.store, &query.invocation)?;
    authorize(&state, &session, &agent)?;
    let artifact = retained(&state.store, &agent, &query.invocation, &query.kind, &hash)?;
    invocation_artifacts::delete(&state.state_dir, &state.store, &agent, &artifact, now_ms()).map_err(ApiError::internal)?;
    signal_changed(&state);
    Ok(Json(json!({"status":"deleted","invocation_id":query.invocation,"sha256":hash})))
}
