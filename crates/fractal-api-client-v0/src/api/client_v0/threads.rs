use super::*;
use crate::store::thread_metadata::{self, HistoryPage, Summary, SummaryPage};

type ReadResult<T> = Result<(Extension<ClientSnapshot>, Json<T>), ApiError>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    agent_id: String,
    cursor: Option<String>,
    limit: Option<usize>,
}

pub(super) async fn list(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<ListQuery>,
) -> ReadResult<SummaryPage> {
    with_person(state, session, move |store, person, _| {
        let valid = fractal_schema::registry()
            .validate_subject(&query.agent_id)
            .is_ok_and(|spec| spec.family == "agent");
        if !valid {
            return Err(validation("thread discovery requires one concrete agent"));
        }
        thread_metadata::list_conversation(
            store,
            person,
            &query.agent_id,
            query.cursor.as_deref(),
            query.limit.unwrap_or(40),
        )
        .map_err(store_error)
    })
    .await
}

pub(super) async fn detail(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>,
) -> ReadResult<Summary> {
    with_owner(state, session, id, |store, person, subject, _| {
        thread_metadata::summary(store, person, subject).map_err(store_error)
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct TimelineQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

pub(super) async fn timeline(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TimelineQuery>,
) -> ReadResult<HistoryPage> {
    with_owner(state, session, id, move |store, person, subject, _| {
        thread_metadata::history(
            store,
            person,
            subject,
            query.cursor.as_deref(),
            query.limit.unwrap_or(40),
        )
        .map_err(store_error)
    })
    .await
}

fn store_error(error: FractalError) -> ApiError {
    match error.code {
        "thread-unavailable" => ApiError::not_found("the thread is unavailable"),
        "stale-thread-cursor" | "stale-thread-title" | "stale-subject" => stale(&error.message),
        "thread-authority-expired" => forbidden(&error.message),
        _ => ApiError::bad(error),
    }
}

async fn with_owner<T: Send + 'static>(
    state: AppState,
    session: ClientSession,
    id: String,
    read: impl FnOnce(&Store, &str, &str, u64) -> Result<T, ApiError> + Send + 'static,
) -> ReadResult<T> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError::not_found("the thread is unavailable"));
    }
    let subject = format!("thread/{id}");
    with_person(state, session, move |store, person, index| {
        read(store, person, &subject, index)
    })
    .await
}

async fn with_person<T: Send + 'static>(
    state: AppState,
    session: ClientSession,
    read: impl FnOnce(&Store, &str, u64) -> Result<T, ApiError> + Send + 'static,
) -> ReadResult<T> {
    blocking_store(move || {
        state.store.read_snapshot(|index| {
            let value = (|| {
                let current = current_session(&state, &session)?;
                require_scope(&current, "read.projections")?;
                let person = &current.authority_actor;
                if current.local_peer
                    || current.assistant_write.is_some()
                    || !person.starts_with("person/")
                    || person.matches('/').count() != 1
                {
                    return Err(forbidden("thread reads require a concrete person"));
                }
                let value = read(&state.store, person, index)?;
                Ok((Extension(client_snapshot_at(&state, index)), Json(value)))
            })();
            Ok(value)
        })
    })
    .await?
}

fn current_session(state: &AppState, session: &ClientSession) -> Result<ClientSession, ApiError> {
    let current = revalidate_session(state, session)?;
    if current.actor != session.actor || current.pairing_completion != session.pairing_completion {
        return Err(forbidden("the thread credential changed"));
    }
    Ok(current)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleParameters {
    target_id: String,
    title: Option<String>,
    title_revision: Option<String>,
}

pub(super) async fn lifecycle_action(
    state: AppState,
    session: ClientSession,
    request: ActionRequest,
) -> Result<Json<Value>, ApiError> {
    validate_action_admission(&session, &request)?;
    let gate = action_gate(&state, &session, &request.idempotency_key);
    let _guard = gate.lock_owned().await;
    blocking_store(move || Ok(run_lifecycle(&state, &session, &request))).await?
}

fn run_lifecycle(
    state: &AppState,
    session: &ClientSession,
    request: &ActionRequest,
) -> Result<Json<Value>, ApiError> {
    use thread_metadata::{Edit, PersonAuthority};
    let current = current_session(state, session)?;
    require_scope(&current, "control.messages")?;
    let person = &current.authority_actor;
    if current.local_peer
        || current.assistant_write.is_some()
        || !person.starts_with("person/")
        || person.matches('/').count() != 1
    {
        return Err(forbidden("thread edits require a concrete person"));
    }
    let authority = match (
        current.transport,
        current.pairing_grant.as_deref(),
        current.pairing_completion.as_deref(),
    ) {
        ("unix", None, None) => PersonAuthority::TrustedLocal,
        ("local-web", None, None) => PersonAuthority::LocalWeb {
            expires_at: current
                .grant_expires_at
                .ok_or_else(|| forbidden("the web credential is unavailable"))?,
        },
        (_, Some(grant), Some(completion)) => PersonAuthority::Paired {
            grant,
            completion,
            actor: &current.actor,
        },
        _ => return Err(forbidden("the thread credential is unavailable")),
    };
    validate_snapshot_fence(state, &request.fence)?;
    let parameters: LifecycleParameters = serde_json::from_value(request.parameters.clone())
        .map_err(|error| validation(error.to_string()))?;
    let subject = &parameters.target_id;
    if request.fence.subject_revisions.len() != 1 {
        return Err(validation(
            "only the target thread metadata revision is required",
        ));
    }
    let revision = request
        .fence
        .subject_revisions
        .get(subject)
        .ok_or_else(|| validation("a thread metadata revision is required"))?;
    let edit = match (
        request.action_type.as_str(),
        parameters.title.as_deref(),
        parameters.title_revision.as_deref(),
    ) {
        ("thread.rename", Some(title), Some(title_revision)) => {
            creation_string(title, "title", 512)?;
            Edit::Rename {
                title,
                title_revision,
                revision,
            }
        }
        ("thread.close", None, None) => Edit::Close { revision },
        ("thread.reopen", None, None) => Edit::Reopen { revision },
        _ => return Err(validation("invalid thread lifecycle parameters")),
    };
    let key = format!("client-thread:{}", creation_key(&current, request));
    let (claim, _) =
        thread_metadata::apply_as(&state.store, person, subject, &key, edit, authority)
            .map_err(store_error)?;
    Ok(Json(json!({
        "kind": "action-result", "action_id": request.id, "operation_id": claim.operation_id,
        "status": "completed", "affected_ids": [claim.subject], "metadata_revision": claim.id,
        "snapshot_id": new_client_snapshot(state).id,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;

    #[tokio::test]
    async fn exact_summary_enforces_owner_and_rejects_a_bound_agent_person_header() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-summary");
        let (created, _) = thread_metadata::apply(
            &state.store,
            "person/operator",
            "thread/fixture",
            "create",
            thread_metadata::Edit::Create {
                agent: "agent/example",
                title: "Review",
            },
        )
        .unwrap();
        let app = crate::api::router(state);
        for (person, bound, status) in [
            ("person/operator", false, StatusCode::OK),
            ("person/other", false, StatusCode::NOT_FOUND),
            ("person/operator", true, StatusCode::FORBIDDEN),
        ] {
            let mut request = Request::builder()
                .uri("/v1/client/threads/fixture")
                .header(LOCAL_PERSON_HEADER, person)
                .body(Body::empty())
                .unwrap();
            if bound {
                request
                    .extensions_mut()
                    .insert(crate::api::BoundAgent("agent/example".into()));
            }
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), status);
            let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            if status == StatusCode::OK {
                assert_eq!(
                    body["value"],
                    json!({
                        "id":"thread/fixture", "agent_id":"agent/example", "title":"Review",
                        "title_revision": created.id, "revision": created.id, "state":"open"
                    })
                );
            } else {
                assert!(!body.to_string().contains("Review"));
            }
        }
    }
    #[tokio::test]
    async fn owner_timeline_paginates_exact_rename_and_close_events() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-timeline");
        let person = "person/operator";
        let subject = "thread/fixture";
        let (created, _) = thread_metadata::apply(
            &state.store,
            person,
            subject,
            "create",
            thread_metadata::Edit::Create {
                agent: "agent/example",
                title: "Review",
            },
        )
        .unwrap();
        let (renamed, _) = thread_metadata::apply(
            &state.store,
            person,
            subject,
            "rename",
            thread_metadata::Edit::Rename {
                title: "New title",
                title_revision: &created.id,
                revision: &created.id,
            },
        )
        .unwrap();
        thread_metadata::apply(
            &state.store,
            person,
            subject,
            "close",
            thread_metadata::Edit::Close {
                revision: &renamed.id,
            },
        )
        .unwrap();
        let app = crate::api::router(state);
        let get = |uri: String| {
            let app = app.clone();
            async move {
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri(uri)
                            .header(LOCAL_PERSON_HEADER, person)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let body: Value = serde_json::from_slice(
                    &to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
                        .await
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(status, StatusCode::OK, "{body}");
                body["value"].clone()
            }
        };
        let first = get("/v1/client/threads/fixture/timeline?limit=1".into()).await;
        assert_eq!(first["items"][0]["kind"], "thread.closed");
        let cursor = first["next_cursor"].as_str().unwrap();
        let next = get(format!(
            "/v1/client/threads/fixture/timeline?limit=1&cursor={cursor}"
        ))
        .await;
        let rename = &next["items"][0];
        assert_eq!(rename["kind"], "thread.renamed");
        assert_eq!(rename["actor_id"], person);
        assert_eq!(rename["previous_title"], "Review");
        assert_eq!(rename["title"], "New title");
        assert!(next["next_cursor"].is_null());
    }
    #[tokio::test]
    async fn lifecycle_action_recovers_only_the_typed_receipt() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-action");
        let person = "person/operator";
        let subject = "thread/fixture";
        let (created, _) = thread_metadata::apply(
            &state.store,
            person,
            subject,
            "create",
            thread_metadata::Edit::Create {
                agent: "agent/example",
                title: "Review",
            },
        )
        .unwrap();
        let snapshot = new_client_snapshot(&state);
        let app = crate::api::router(state.clone());
        let request = json!({
            "api_version": CLIENT_API_VERSION,
            "id": "action/thread-close",
            "type": "thread.close",
            "idempotency_key": "thread-close-request",
            "fence": {"snapshot_id": snapshot.id, "subject_revisions": {(subject): created.id}},
            "parameters": {"target_id": subject}
        });
        let post = |actor: &'static str| {
            let app = app.clone();
            let request = request.clone();
            async move {
                app.oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/client/actions")
                        .header(LOCAL_PERSON_HEADER, actor)
                        .header("Content-Type", "application/json")
                        .body(Body::from(request.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        let response = post(person).await;
        let status = response.status();
        let first: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["value"]["affected_ids"], json!([subject]));
        assert_eq!(post("person/other").await.status(), StatusCode::NOT_FOUND);
        let response = post(person).await;
        assert_eq!(response.status(), StatusCode::OK);
        let retry: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            first["value"]["operation_id"],
            retry["value"]["operation_id"]
        );
        assert_eq!(
            state
                .store
                .claims_for(subject, Some("thread.state-set"))
                .unwrap()
                .len(),
            1
        );
    }
    #[tokio::test]
    async fn list_returns_only_the_selected_owner_and_agent_conversation() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-list");
        for (person, agent, thread) in [
            ("person/operator", "agent/example", "thread/fixture"),
            ("person/operator", "agent/other", "thread/other-agent"),
            ("person/other", "agent/example", "thread/other-owner"),
        ] {
            thread_metadata::apply(
                &state.store,
                person,
                thread,
                thread,
                thread_metadata::Edit::Create {
                    agent,
                    title: "Review",
                },
            )
            .unwrap();
        }
        let response = crate::api::router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/client/threads?agent_id=agent%2Fexample&limit=2")
                    .header(LOCAL_PERSON_HEADER, "person/operator")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["value"]["items"].as_array().unwrap().len(), 1);
        assert_eq!(body["value"]["items"][0]["id"], "thread/fixture");
        assert!(body["value"]["next_cursor"].is_null());
    }
}
