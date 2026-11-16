//! Native continuity previews and explicitly authenticated evidence recovery.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    run: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverRequest {
    run: String,
    preview_hash: String,
    #[serde(default)]
    confirmed_legacy_steps: Vec<String>,
}

pub async fn preview(
    State(state): State<AppState>,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<Value>, ApiError> {
    blocking_action(move || state.store.mission_continuity_preview(&request.run)).await.map(Json)
}

pub async fn startup_preview(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    blocking_action(move || state.store.mission_supersession_preview()).await.map(Json)
}

pub async fn recover(
    State(state): State<AppState>,
    session: Option<Extension<client_v0::ClientSession>>,
    principal: Option<Extension<write_principal::WritePrincipal>>,
    Json(request): Json<RecoverRequest>,
) -> Result<Json<Value>, ApiError> {
    let refused = || ApiError {
        status: StatusCode::FORBIDDEN,
        code: "forbidden".into(),
        message: "continuity recovery requires an authenticated actor; legacy confirmations require person authority".into(),
        details: Box::default(),
    };
    let session = session.ok_or_else(refused)?.0;
    let session = client_v0::revalidate_session(&state, &session)?;
    client_v0::require_scope(&session, "control.work")?;
    // Serving-boundary attribution wins over a local identity header. Neither a
    // request body nor an agent's person-shaped header can authenticate a person.
    let actor = principal.as_ref().map_or(session.authority_actor.as_str(), |principal| principal.0.actor()).to_owned();
    if !(actor.starts_with("person/") || actor.starts_with("agent/") || actor.starts_with("mission-run/"))
        || (!request.confirmed_legacy_steps.is_empty()
            && (session.local_peer || session.assistant_write.is_some()
                || !actor.starts_with("person/") || actor != session.authority_actor))
    {
        return Err(refused());
    }
    let store = state.store.clone();
    let result = blocking_action(move || store.recover_mission_continuity(
        &request.run, &request.preview_hash, &actor, &request.confirmed_legacy_steps,
    )).await?;
    signal_changed(&state);
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    async fn post(app: Router, path: &str, person: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder().method("POST").uri(path)
            .header("content-type", "application/json");
        if let Some(person) = person { request = request.header("x-st3-person", person); }
        let response = app.oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn recovery_requires_authenticated_authority_and_rejects_actor_fields() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let path = "/v1/mission-runs/continuity/recover";
        let before = state.store.index().unwrap();
        for (person, confirmations, expected) in [
            (None, json!([]), StatusCode::FORBIDDEN),
            (Some("agent/helper"), json!(["approval"]), StatusCode::FORBIDDEN),
        ] {
            let (status, _) = post(router(state.clone()), path, person, json!({
                "run":"absent", "preview_hash":"stale", "confirmed_legacy_steps":confirmations,
            })).await;
            assert_eq!(status, expected);
        }
        let (status, _) = post(router(state.clone()), path, Some("person/operator"), json!({
            "run":"absent", "preview_hash":"stale", "actor":"person/other",
        })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn authenticated_recovery_preserves_core_refusals() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let error = state.store.recover_mission_continuity("absent", "stale", "person/operator", &[]).unwrap_err();
        let expected = ApiError::bad(error);
        let before = state.store.index().unwrap();
        for actor in ["person/operator", "agent/helper"] {
            let (status, value) = post(router(state.clone()), "/v1/mission-runs/continuity/recover",
                Some(actor), json!({"run":"absent", "preview_hash":"stale"})).await;
            assert_eq!(status, expected.status);
            assert_eq!(value["code"], expected.code);
        }
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn harness_person_header_cannot_confirm_legacy_sources() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let app = router(state.clone());
        let before = state.store.index().unwrap();
        let response = app.oneshot(Request::builder().method("POST")
            .uri("/v1/mission-runs/continuity/recover")
            .header("content-type", "application/json")
            .header("x-st3-person", "person/operator")
            .extension(BoundAgent("agent/helper".into()))
            .body(Body::from(json!({"run":"absent", "preview_hash":"stale",
                "confirmed_legacy_steps":["approval"]}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn previews_use_store_results_without_mutating_evidence() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let expected = state.store.mission_supersession_preview().unwrap();
        let before = state.store.index().unwrap();
        let response = router(state.clone()).oneshot(Request::builder()
            .uri("/v1/mission-runs/continuity/startup-preview")
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
        let (status, value) = post(router(state.clone()), "/v1/mission-runs/continuity/preview", None,
            json!({"run":"absent"})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(value["code"], state.store.mission_continuity_preview("absent").unwrap_err().code);
        assert_eq!(state.store.index().unwrap(), before);
    }
}
