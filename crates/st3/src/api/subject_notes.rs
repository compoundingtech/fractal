//! Authored subject context; authenticated identity, never body fields, supplies authorship.
use super::*;
use axum::extract::Path;

fn reader(session: &client_v0::ClientSession, write: bool) -> Result<&str, ApiError> {
    client_v0::require_scope(session, if write { "control.notes" } else { "read.projections" })?;
    let actor = session.authority_actor.as_str();
    if !actor.starts_with("person/") && !actor.starts_with("agent/") {
        return Err(ApiError::bad(St3Error::new("note-forbidden", "notes require an identified person or agent")));
    }
    Ok(actor)
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoteQuery {
    target: Option<String>,
    author: Option<String>,
    recipient: Option<String>,
}

pub(super) async fn read(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Query(query): Query<NoteQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    let actor = reader(&session, false)?.to_owned();
    if query.target.is_some() && query.author.is_some() {
        return Err(ApiError::bad(St3Error::new("invalid-subject-note", "select target or author, not both")));
    }
    if query.recipient.as_deref().is_some_and(|recipient| !recipient.starts_with("agent/")
        || (actor.starts_with("agent/") && actor != recipient)) {
        return Err(ApiError::bad(St3Error::new("note-forbidden", "agents may refresh only their own digest")));
    }
    let (snapshot, notes) = blocking_store(move || state.store.read_snapshot(|index| {
        let notes = if let Some(recipient) = query.recipient {
            crate::notes_digest::current_notes(&state.store, &recipient)?
        } else {
            state.store.list_subject_notes(query.target.as_deref(), query.author.as_deref(), &actor)?
        };
        Ok((client_snapshot_at(&state, index), notes))
    })).await?;
    Ok((Extension(snapshot), Json(json!({"notes": notes}))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoteCreate {
    target: String,
    text: String,
    session: Option<String>,
    expires_at: Option<String>,
}

pub(super) async fn create(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Json(input): Json<NoteCreate>,
) -> Result<Json<Value>, ApiError> {
    let actor = reader(&session, true)?.to_owned();
    let id = format!("note/{actor}/{}", uuid::Uuid::now_v7());
    let (id, note) = blocking_action(move || {
        if state.store.latest_claim(&input.target, None).map_err(|error| St3Error::new("internal", error.to_string()))?.is_none() {
            return Err(St3Error::new("note-target-access", "note target is absent"));
        }
        let scope = match input.session.as_deref() {
            Some("current") => Some(state.store.current_subject_note_incarnation(&input.target).map_err(|error| St3Error::new("internal", error.to_string()))?
                .ok_or_else(|| St3Error::new("invalid-subject-note", "target has no current running incarnation"))?),
            _ => input.session,
        };
        let note = state.store.set_subject_note(&id, &actor, &input.target, Some(&input.text),
            scope.as_deref(), input.expires_at.as_deref())?;
        for agent in state.store.subject_note_agents(&input.target).map_err(|error| St3Error::new("internal", error.to_string()))? {
            crate::notes_digest::emit_changed(&state.store, &agent).map_err(|error| St3Error::new("internal", error.to_string()))?;
        }
        signal_changed(&state);
        Ok((id, note))
    }).await?;
    Ok(Json(json!({"id": id, "note": note})))
}

pub(super) async fn update(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let actor = reader(&session, true)?.to_owned();
    st3_schema::subject_notes::validate_actor(&id, st3_schema::subject_notes::KIND, Some(&actor))
        .map_err(|error| ApiError::bad(St3Error::new(error.code, error.message)))?;
    let fields = input.as_object().ok_or_else(|| ApiError::bad(St3Error::new("invalid-subject-note", "expected an update object")))?;
    if fields.keys().any(|field| !matches!(field.as_str(), "text" | "session" | "expires_at")) {
        return Err(ApiError::bad(St3Error::new("invalid-subject-note", "update accepts text, session and expires_at only")));
    }
    if fields.get("text").is_some_and(|text| !text.is_null() && !text.is_string()) {
        return Err(ApiError::bad(St3Error::new("invalid-subject-note", "text must be a string or null")));
    }
    for field in ["session", "expires_at"] {
        if fields.get(field).is_some_and(|value| !value.is_string() && !value.is_null()) {
            return Err(ApiError::bad(St3Error::new("invalid-subject-note", format!("invalid {field}"))));
        }
    }
    let (id, note) = blocking_action(move || {
        // Authorship was checked before this lookup. Retain write access to expired and
        // archived revisions without making them current on a read surface.
        let previous = state.store.latest_claim(&id, Some(st3_schema::subject_notes::KIND))
            .map_err(|error| St3Error::new("internal", error.to_string()))?
            .ok_or_else(|| St3Error::new("subject-note-not-found", "note is absent"))?;
        let old = previous.body.get("fields").ok_or_else(|| St3Error::new("internal", "note revision has no fields"))?;
        let target = old.get("target").and_then(Value::as_str).ok_or_else(|| St3Error::new("internal", "note revision has no target"))?;
        let fields = input.as_object().expect("validated update object");
        let session = fields.get("session").map_or_else(|| old.get("session").and_then(Value::as_str), Value::as_str);
        let expiry = fields.get("expires_at").map_or_else(|| old.get("expires_at").and_then(Value::as_str), Value::as_str);
        let text = fields.get("text").map_or_else(|| old.get("text").and_then(Value::as_str), Value::as_str);
        let note = state.store.set_subject_note(&id, &actor, target, text, session, expiry)?;
        for agent in state.store.subject_note_agents(target).map_err(|error| St3Error::new("internal", error.to_string()))? {
            crate::notes_digest::emit_changed(&state.store, &agent).map_err(|error| St3Error::new("internal", error.to_string()))?;
        }
        signal_changed(&state);
        Ok((id, note))
    }).await?;
    Ok(Json(json!({"id": id, "note": note})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn subject_note_write_requires_identified_author_and_rejects_body_impersonation() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        for body in [json!({"target":"agent/example/worker","text":"context"}),
            json!({"target":"agent/example/worker","text":"context","actor":"person/example"})] {
            let response = router(state.clone()).oneshot(Request::builder().method("POST")
                .uri("/v1/subject-notes").header("content-type", "application/json")
                .body(Body::from(body.to_string())).unwrap()).await.unwrap();
            assert!(!response.status().is_success());
        }
    }

    #[test]
    fn subject_note_paired_write_requires_explicit_scope() {
        let session = client_v0::ClientSession::for_tests_scoped("client/example", "person/example", &["read.projections"]);
        assert!(reader(&session, true).is_err());
        let session = client_v0::ClientSession::for_tests_scoped("client/example", "person/example", &["control.notes"]);
        assert_eq!(reader(&session, true).unwrap(), "person/example");
    }

    #[tokio::test]
    async fn subject_note_api_roundtrip_uses_agent_context_and_preserves_authority() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let fleet = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
        state.store.bind_fleet(fleet).unwrap();
        state.store.pin_fleet_anchor(key.public()).unwrap();
        state.store.set_member_key(Some(key.clone())).unwrap();
        state.store.admit_fleet_anchor(fleet, key.public(), "listening").unwrap();
        for (subject, kind, fields) in [
            ("daemon/node", "daemon.started", json!({"status":"running","features":{"subject_notes":1}})),
            ("agent/example/worker", "harness.observed", json!({"state":"ready"})),
        ] {
            state.store.append_claim(&ClaimInput { subject: subject.into(), kind: kind.into(), actor: None,
                fields: serde_json::from_value(fields).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None }).unwrap();
        }
        async fn call(state: &AppState, method: &str, uri: &str, actor: &str, body: Value) -> (StatusCode, Value) {
            let request = Request::builder().method(method).uri(uri).header("x-st3-person", actor)
                .header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
            let response = router(state.clone()).oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
            (status, serde_json::from_slice(&bytes).unwrap())
        }
        let (status, created) = call(&state, "POST", "/v1/subject-notes", "person/example",
            json!({"target":"agent/example/worker","text":"Context reminder"})).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["value"]["id"], created["value"]["note"]["id"]);
        assert_eq!(created["value"]["note"]["audience"], "agent");
        let id = created["value"]["note"]["id"].as_str().unwrap();
        let (status, agent) = call(&state, "GET", "/v1/client/subject-notes", "agent/example/worker", Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{agent}");
        assert_eq!(agent["value"]["notes"][0]["text"], "Context reminder");
        let path = format!("/v1/subject-notes/{}", urlencoding::encode(id));
        let (status, denied) = call(&state, "PUT", &path, "person/other", json!({"text":"not mine"})).await;
        assert!(!status.is_success(), "{denied}");
        for actor in ["person/example", "agent/example/worker"] {
            for audience in ["self", "agent", "future-level"] {
                let response = router(state.clone()).oneshot(Request::builder().method("POST")
                    .uri("/v1/subject-notes").header("x-st3-person", actor)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"target":"agent/example/worker","text":"Context","audience":audience}).to_string())).unwrap()).await.unwrap();
                assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            }
        }
        for audience in ["self", "agent", "future-level"] {
            let (status, rejected) = call(&state, "PUT", &path, "person/example", json!({"audience":audience})).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        }
        let (status, updated) = call(&state, "PUT", &path, "person/example", json!({"text":"Updated context"})).await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        assert_eq!(updated["value"]["note"]["audience"], "agent");
        let (_, agent) = call(&state, "GET", "/v1/client/subject-notes", "agent/example/worker", Value::Null).await;
        assert_eq!(agent["value"]["notes"][0]["text"], "Updated context");
        let (status, archived) = call(&state, "PUT", &path, "person/example", json!({"text":null})).await;
        assert_eq!(status, StatusCode::OK, "{archived}");
        assert!(archived["value"]["note"].is_null());
        assert_eq!(archived["value"]["id"], id);
        let (status, created_by_agent) = call(&state, "POST", "/v1/subject-notes", "agent/example/worker",
            json!({"target":"agent/example/worker","text":"Agent context"})).await;
        assert_eq!(status, StatusCode::OK, "{created_by_agent}");
        assert_eq!(created_by_agent["value"]["note"]["audience"], "agent");
        let (status, expired) = call(&state, "POST", "/v1/subject-notes", "person/example",
            json!({"target":"agent/example/worker","text":"past reminder","expires_at":"2000-01-01T00:00:00Z"})).await;
        assert_eq!(status, StatusCode::OK, "{expired}");
        assert!(expired["value"]["note"].is_null());
        let expired_id = expired["value"]["id"].as_str().unwrap();
        let (status, archived) = call(&state, "PUT", &format!("/v1/subject-notes/{}", urlencoding::encode(expired_id)),
            "person/example", json!({"text":null})).await;
        assert_eq!(status, StatusCode::OK, "{archived}");
        for audience in ["self", "agent", "future-level"] {
            let historical_id = format!("note/person/example/{}", uuid::Uuid::now_v7());
            state.store.append_claim(&ClaimInput {
                subject: historical_id.clone(), kind: st3_schema::subject_notes::KIND.into(),
                actor: Some("person/example".into()),
                fields: serde_json::from_value(json!({"target":"agent/example/worker",
                    "text":"Historical context","audience":audience})).unwrap(),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
            for actor in ["person/other", "agent/example/worker"] {
                let (status, notes) = call(&state, "GET", "/v1/client/subject-notes?target=agent%2Fexample%2Fworker",
                    actor, Value::Null).await;
                assert_eq!(status, StatusCode::OK, "{notes}");
                assert!(notes["value"]["notes"].as_array().unwrap().iter()
                    .any(|note| note["id"] == historical_id && note["audience"] == audience));
            }
            let (status, digest) = call(&state, "GET", "/v1/client/subject-notes?recipient=agent%2Fexample%2Fworker",
                "agent/example/worker", Value::Null).await;
            assert_eq!(status, StatusCode::OK, "{digest}");
            assert!(digest["value"]["notes"].as_array().unwrap().iter().any(|note| note["id"] == historical_id));
            let historical_path = format!("/v1/subject-notes/{}", urlencoding::encode(&historical_id));
            let (status, updated) = call(&state, "PUT", &historical_path, "person/example",
                json!({"text":"Updated historical context"})).await;
            assert_eq!(status, StatusCode::OK, "{updated}");
            assert_eq!(updated["value"]["note"]["audience"], "agent");
        }
    }
}
