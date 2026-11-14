use crate::DirectiveNotesStore;
use super::*;

fn forbidden(message: &str) -> ApiError {
    ApiError {
        status: StatusCode::FORBIDDEN,
        code: "forbidden".into(),
        message: message.into(),
        details: Box::default(),
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteQuery {
    actor: Option<String>,
}

pub async fn read(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Query(query): Query<NoteQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    let actor = session.authority_actor;
    if !(actor.starts_with("person/") || actor.starts_with("agent/"))
        || query.actor.as_deref().is_some_and(|requested| requested != actor)
    {
        return Err(forbidden("identify the person or agent reading its directive notes"));
    }
    let (snapshot, notes) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            Ok((client_snapshot_at(&state, index), state.store.directive_notes(&actor)?))
        })
    }).await?;
    Ok((Extension(snapshot), Json(json!({"notes": notes}))))
}

/// `text` is required: a string sets the note, null clears it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteSet {
    #[serde(deserialize_with = "super::availability::required")]
    text: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

/// Set or clear the session person's own directive note; the body never names the person.
pub async fn set(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Json(input): Json<NoteSet>,
) -> Result<Json<Value>, ApiError> {
    let person = super::availability::person_writer(&session, "write.notes")?;
    let source = super::availability::writer_source(&state, &session);
    let store = state.store.clone();
    let note = blocking_action(move || store.set_directive_note(
        &person, &person, &source, input.text.as_deref(), input.expires_at.as_deref(),
    )).await.map_err(super::availability::client_refusal)?;
    signal_visible_change(&state);
    Ok(Json(json!({"note": note})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    async fn read_as(state: &AppState, actor: Option<&str>, requested: Option<&str>) -> (StatusCode, Value) {
        let path = requested.map_or_else(|| "/v1/client/notes".into(), |actor| {
            format!("/v1/client/notes?actor={}", urlencoding::encode(actor))
        });
        let mut request = Request::builder().uri(path);
        if let Some(actor) = actor { request = request.header("x-st3-person", actor); }
        let response = router(state.clone()).oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn note_read_requires_identified_actor_and_rejects_actor_switching() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let (status, _) = read_as(&state, None, Some("person/ada")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = read_as(&state, Some("agent/worker"), Some("person/ada")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, value) = read_as(&state, Some("person/ada"), None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["value"]["notes"], json!([]));
        let (status, _) = read_as(&state, Some("agent/unknown"), None).await;
        assert_eq!(status, StatusCode::OK);
    }
}
