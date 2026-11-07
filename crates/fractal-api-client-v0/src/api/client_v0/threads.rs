use super::*;
use crate::store::thread_metadata::{self, Summary, SummaryPage, TimelinePage};

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
) -> ReadResult<TimelinePage> {
    with_owner(state, session, id, move |store, person, subject, _| {
        thread_metadata::mixed_timeline(
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
        "stale-thread-cursor" | "stale-thread-title" | "stale-subject" | "thread-closed"
        | "thread-state-unchanged" => stale(&error.message),
        "thread-authority-expired" | "device-key-not-enrolled" => forbidden(&error.message),
        "thread-rollout-unavailable" => ApiError {
            status: StatusCode::CONFLICT,
            code: "unsupported-capability".into(),
            message: error.message,
            details: Box::default(),
        },
        "invalid-thread-cursor" | "invalid-thread-message" | "invalid-thread-start"
        | "invalid-thread-decision" | "invalid-thread-title" => validation(&error.message),
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
    use thread_metadata::Edit;
    let current = current_session(state, session)?;
    require_scope(&current, "control.messages")?;
    let person = &current.authority_actor;
    let authority = edit_authority(&current)?;
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

fn edit_authority(
    current: &ClientSession,
) -> Result<thread_metadata::PersonAuthority<'_>, ApiError> {
    use thread_metadata::PersonAuthority;
    let person = &current.authority_actor;
    if current.local_peer
        || current.assistant_write.is_some()
        || !person.starts_with("person/")
        || person.matches('/').count() != 1
    {
        return Err(forbidden("thread edits require a concrete person"));
    }
    Ok(
        match (
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
        },
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartParameters {
    target_id: String,
    agent_id: String,
    origin_ask: String,
    origin_ask_episode: String,
    title: String,
    content: String,
    signature: Option<smallclaims::principal::ClaimSignature>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyParameters {
    target_id: String,
    content: String,
    thread_quote_id: Option<String>,
    signature: Option<smallclaims::principal::ClaimSignature>,
}

pub(super) async fn message_action(
    state: AppState,
    session: ClientSession,
    request: ActionRequest,
) -> Result<Json<Value>, ApiError> {
    validate_action_admission(&session, &request)?;
    let gate = action_gate(&state, &session, &request.idempotency_key);
    let _guard = gate.lock_owned().await;
    blocking_store(move || Ok(run_message(&state, &session, &request))).await?
}

fn run_message(
    state: &AppState,
    session: &ClientSession,
    request: &ActionRequest,
) -> Result<Json<Value>, ApiError> {
    use crate::api::{ThreadMessageDraft, prepare_inline_thread_message};
    let current = current_session(state, session)?;
    require_scope(&current, "control.messages")?;
    let person = &current.authority_actor;
    let authority = if request.action_type == "thread.reply"
        && current.transport == "unix"
        && current.assistant_write.is_none()
        && let Some(bound) = current.bound_agent.as_deref()
        && person.starts_with("agent/")
    {
        thread_metadata::PersonAuthority::BoundAgent { actor: bound }
    } else {
        edit_authority(&current)?
    };
    validate_snapshot_fence(state, &request.fence)?;
    if !request.fence.subject_revisions.is_empty() {
        return Err(validation(
            "thread messages use their typed origin or quote fence",
        ));
    }
    let (thread, message, claim, appended) = if request.action_type == "thread.start" {
        let p: StartParameters = serde_json::from_value(request.parameters.clone())
            .map_err(|error| validation(error.to_string()))?;
        creation_string(&p.title, "title", 512)?;
        let message = prepare_inline_thread_message(
            state,
            ThreadMessageDraft {
                key: &request.idempotency_key,
                thread: &p.target_id,
                from: person,
                to: &p.agent_id,
                content: &p.content,
                title: Some(&p.title),
                quote: None,
                origin: Some((&p.origin_ask, &p.origin_ask_episode)),
            },
            p.signature.as_ref(),
        )?;
        let key = format!("client-thread-start:{}", creation_key(&current, request));
        let (claim, appended) = thread_metadata::start_from_ask_as(
            &state.store,
            thread_metadata::StartFromAsk {
                thread: &p.target_id,
                title: &p.title,
                person,
                ask: &p.origin_ask,
                episode: &p.origin_ask_episode,
                key: &key,
                first_message: &message,
            },
            authority,
            p.signature.as_ref(),
        )
        .map_err(store_error)?;
        (p.target_id, message.subject, claim, appended)
    } else {
        let p: ReplyParameters = serde_json::from_value(request.parameters.clone())
            .map_err(|error| validation(error.to_string()))?;
        let recipient =
            crate::store::thread_delivery::reply_recipient(&state.store, &p.target_id, person)
                .map_err(store_error)?;
        let message = prepare_inline_thread_message(
            state,
            ThreadMessageDraft {
                key: &request.idempotency_key,
                thread: &p.target_id,
                from: person,
                to: &recipient,
                content: &p.content,
                title: None,
                quote: p.thread_quote_id.as_deref(),
                origin: None,
            },
            p.signature.as_ref(),
        )?;
        let (claim, appended) = crate::store::thread_delivery::append_reply(
            &state.store,
            &message,
            authority,
            p.signature.as_ref(),
        )
        .map_err(store_error)?;
        (p.target_id, message.subject, claim, appended)
    };
    if appended {
        signal_message_changed(state, "message.sent", false);
    }
    Ok(Json(json!({
        "kind": "action-result", "action_id": request.id, "operation_id": claim.operation_id,
        "status": "completed", "affected_ids": [thread, message],
        "snapshot_id": new_client_snapshot(state).id,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskLinkParameters {
    target_id: String,
    ask_id: String,
    ask_episode: String,
    decision: Option<DecisionLink>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionLink {
    predecessor_ask: String,
    predecessor_episode: String,
    reason: String,
}

pub(super) async fn link_ask_action(
    state: AppState,
    session: ClientSession,
    request: ActionRequest,
) -> Result<Json<Value>, ApiError> {
    validate_action_admission(&session, &request)?;
    let gate = action_gate(&state, &session, &request.idempotency_key);
    let _guard = gate.lock_owned().await;
    blocking_store(move || {
        Ok((|| {
            let current = current_session(&state, &session)?;
            require_scope(&current, "control.work")?;
            let bound = current
                .bound_agent
                .as_deref()
                .filter(|_| {
                    current.transport == "unix"
                        && current.assistant_write.is_none()
                        && current.authority_actor.starts_with("agent/")
                })
                .ok_or_else(|| forbidden("only the bound requesting agent can attach an ask"))?;
            validate_snapshot_fence(&state, &request.fence)?;
            if !request.fence.subject_revisions.is_empty() {
                return Err(validation("ask attachment uses the exact ask episode"));
            }
            let p: AskLinkParameters = serde_json::from_value(request.parameters.clone())
                .map_err(|error| validation(error.to_string()))?;
            let decision =
                p.decision
                    .as_ref()
                    .map(|value| crate::store::thread_links::DecisionHistory {
                        predecessor: &value.predecessor_ask,
                        episode: &value.predecessor_episode,
                        reason: &value.reason,
                    });
            let key = format!("client-thread-link:{}", creation_key(&current, &request));
            let (claim, appended) = crate::store::thread_links::attach_ask(
                &state.store,
                bound,
                crate::store::thread_links::AttachAsk {
                    thread: &p.target_id,
                    ask: &p.ask_id,
                    episode: &p.ask_episode,
                    key: &key,
                    decision,
                },
            )
            .map_err(store_error)?;
            if appended {
                signal_visible_change(&state);
            }
            Ok(Json(json!({
                "kind":"action-result", "action_id":request.id, "operation_id":claim.operation_id,
                "status":"completed", "affected_ids":[p.target_id,p.ask_id],
                "snapshot_id":new_client_snapshot(&state).id,
            })))
        })())
    })
    .await?
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
        state.store.configure_thread_write_rollout([]);
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
    #[tokio::test]
    async fn real_inbox_ask_starts_and_retries_through_typed_action() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-http-start");
        state.store.configure_thread_write_rollout([]);
        let person = "person/operator";
        let agent = "agent/garden/reviewer";
        let thread = "thread/http-start";
        let intent = crate::graph::parse_intent(
            r#"version 2
agent "garden/reviewer" { workspace "/work/garden"; harness "omp" {} }
"#,
            state.store.origin(),
        ).unwrap();
        state.store.apply_internal(&intent, "declare-start-agent").unwrap();
        let app = crate::api::router(state.clone());
        let post = |actor: &'static str, bound: Option<&'static str>, action: &'static str, parameters: Value| {
            let app = app.clone();
            let snapshot = new_client_snapshot(&state).id;
            async move {
                let request = json!({
                    "api_version": CLIENT_API_VERSION, "id": format!("action/{action}"),
                    "type": action, "idempotency_key": format!("thread-fixture-{action}"),
                    "fence": {"snapshot_id": snapshot}, "parameters": parameters,
                });
                let mut request = Request::builder().method("POST").uri("/v1/client/actions")
                    .header(LOCAL_PERSON_HEADER, actor).header("Content-Type", "application/json")
                    .body(Body::from(request.to_string())).unwrap();
                if let Some(bound) = bound {
                    request.extensions_mut().insert(crate::api::BoundAgent(bound.into()));
                }
                let response = app.oneshot(request).await.unwrap();
                let status = response.status();
                let body = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
                let value: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(status, StatusCode::OK, "{value}");
                value["value"].clone()
            }
        };
        let asked = post(agent, Some(agent), "work.ask", json!({
            "person_id": person, "title": "Review CI", "reason": "Can we proceed?",
        })).await;
        let ask = asked["affected_ids"][0].as_str().unwrap();
        let episode = state.store.newest_claim_for_subject_kind(ask, "work.person-asked")
            .unwrap().unwrap();
        let parameters = json!({
            "target_id": thread, "agent_id": agent, "origin_ask": ask,
            "origin_ask_episode": episode.id, "title": "Review CI", "content": "Check worker tests.",
        });
        let start = post(person, None, "thread.start", parameters.clone()).await;
        let retry = post(person, None, "thread.start", parameters).await;
        assert_eq!(retry["operation_id"], start["operation_id"]);
        let message = start["affected_ids"][1].as_str().unwrap();
        let sent = state.store.claims_for(message, Some("message.sent")).unwrap();
        assert_eq!(sent.len(), 1);
        let created = state.store.newest_claim_for_subject_kind(thread, "thread.created").unwrap().unwrap();
        assert_eq!(sent[0].batch_id, created.batch_id);
        assert_eq!(sent[0].body["fields"]["thread_id"], thread);
        assert_eq!(state.store.latest_claim_id(ask).unwrap().as_deref(), Some(episode.id.as_str()));
    }

    #[tokio::test]
    async fn thread_reply_keys_are_disjoint_between_two_owners() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-reply-keys");
        state.store.configure_thread_write_rollout([]);
        let agent = "agent/example";
        for (person, subject) in [
            ("person/operator", "thread/first"),
            ("person/other", "thread/second"),
        ] {
            thread_metadata::apply(
                &state.store,
                person,
                subject,
                subject,
                thread_metadata::Edit::Create {
                    agent,
                    title: "Review",
                },
            )
            .unwrap();
        }
        let app = crate::api::router(state.clone());
        let post = |person: &'static str, thread: &'static str| {
            let app = app.clone();
            let snapshot = new_client_snapshot(&state).id;
            async move {
                let request = json!({
                    "api_version": CLIENT_API_VERSION,
                    "id": "action/thread-reply",
                    "type": "thread.reply",
                    "idempotency_key": "same-thread-reply-key",
                    "fence": {"snapshot_id": snapshot},
                    "parameters": {"target_id": thread, "content": "Reply"},
                });
                let response = app
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/v1/client/actions")
                            .header(LOCAL_PERSON_HEADER, person)
                            .header("Content-Type", "application/json")
                            .body(Body::from(request.to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
                    .await
                    .unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                (status, body)
            }
        };
        let (status, first) = post("person/operator", "thread/first").await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let (status, second) = post("person/other", "thread/second").await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_ne!(
            first["value"]["affected_ids"][1],
            second["value"]["affected_ids"][1]
        );
        assert_ne!(
            first["value"]["operation_id"],
            second["value"]["operation_id"]
        );
        assert_eq!(
            post("person/other", "thread/first").await.0,
            StatusCode::NOT_FOUND
        );
        let (_, retry) = post("person/operator", "thread/first").await;
        assert_eq!(
            retry["value"]["operation_id"],
            first["value"]["operation_id"]
        );
    }
    #[tokio::test]
    async fn paired_reply_requires_the_exact_enrolled_device_key() {
        use crate::api::{ThreadMessageDraft, prepare_inline_thread_message};
        use smallclaims::principal::{ClaimSignature, FIELDS_FORMAT, fields_signing_bytes};

        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-signed");
        state.store.configure_thread_write_rollout([]);
        let person = "person/operator";
        let agent = "agent/example";
        let thread = "thread/fixture";
        let request_key = "paired-thread-reply";
        let credential = "fixture-bearer-thread";
        let pair_subject = "custom/client/pairing-thread-device";
        thread_metadata::apply(
            &state.store,
            person,
            thread,
            "create",
            thread_metadata::Edit::Create {
                agent,
                title: "Review",
            },
        )
        .unwrap();
        state
            .store
            .set_node_key(std::sync::Arc::new(
                smallclaims::fleet::MemberKey::generate().unwrap().0,
            ))
            .unwrap();
        let (device, _) = smallclaims::fleet::MemberKey::generate().unwrap();
        let (other, _) = smallclaims::fleet::MemberKey::generate().unwrap();
        let chain = state
            .store
            .enroll_device_key(person, device.public(), "thread phone")
            .unwrap();
        let other_chain = state
            .store
            .enroll_device_key(person, other.public(), "different phone")
            .unwrap();
        state
            .store
            .append_claim(&ClaimInput {
                subject: pair_subject.into(),
                kind: "custom.client.pairing-completed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("person_id".into(), json!(person)),
                    ("session_actor".into(), json!("client/paired/thread-device")),
                    ("device_public_key".into(), json!(device.public())),
                    (
                        "credential_hash".into(),
                        json!(credential_digest(credential)),
                    ),
                    (
                        "expires_at_unix_ms".into(),
                        json!(client_now_ms() as u64 + 60_000),
                    ),
                    ("scopes".into(), json!(["control.messages"])),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let input = prepare_inline_thread_message(
            &state,
            ThreadMessageDraft {
                key: request_key,
                thread,
                from: person,
                to: agent,
                content: "Please confirm.",
                title: None,
                quote: None,
                origin: None,
            },
            None,
        )
        .unwrap();
        let sign = |key: &smallclaims::fleet::MemberKey, chain: Vec<String>| {
            let mut signature =
                ClaimSignature::sign(key, "", person, None, chain, client_now_ms() as u64);
            signature.format = Some(FIELDS_FORMAT.into());
            signature.signed_fields = input
                .fields
                .keys()
                .filter(|field| field.as_str() != "status")
                .cloned()
                .collect();
            signature.signature = key.sign(&fields_signing_bytes(
                &input.subject,
                &input.kind,
                input.actor.as_deref(),
                &json!(input.fields),
                &signature.signed_fields,
                &signature.signer,
                None,
                &signature.key,
                &signature.chain,
                &signature.nonce,
                signature.signed_at_unix_ms,
            ));
            signature
        };
        let signature = sign(&device, chain);
        let wrong_device = sign(&other, other_chain);
        // This fixture opens a bare store. The daemon normally runs the signature
        // inventory migration before a signed request can establish nonce absence.
        for _ in 0..20 {
            let report = state.store.advance_signature_migration().unwrap();
            if report.done() {
                break;
            }
            if report.phase == "judging" {
                state.store.advance_signature_judgment().unwrap();
            }
        }
        assert!(!state.store.signature_nonce_used(&wrong_device.key, &wrong_device.nonce).unwrap());
        let app = crate::api::router(state.clone());
        let post = |signature: ClaimSignature| {
            let app = app.clone();
            let snapshot = new_client_snapshot(&state).id;
            async move {
                let request = json!({
                    "api_version": CLIENT_API_VERSION, "id": "action/paired-thread-reply",
                    "type": "thread.reply", "idempotency_key": request_key,
                    "fence": {"snapshot_id": snapshot},
                    "parameters": {"target_id": thread, "content":"Please confirm.", "signature": signature},
                });
                let response = app
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/v1/client/actions")
                            .header(AUTHORIZATION, format!("Bearer {credential}"))
                            .header("Content-Type", "application/json")
                            .body(Body::from(request.to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
                    .await
                    .unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                (status, body)
            }
        };
        let (status, wrong) = post(wrong_device).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{wrong}");
        assert_eq!(wrong["code"], "forbidden", "{wrong}");
        assert_eq!(wrong["message"], "the thread message signing key is unavailable", "{wrong}");
        assert!(
            state
                .store
                .claims_for(&input.subject, None)
                .unwrap()
                .is_empty()
        );
        let (status, sent) = post(signature.clone()).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let (status, retry) = post(signature).await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(
            sent["value"]["operation_id"],
            retry["value"]["operation_id"]
        );
        assert_eq!(
            state
                .store
                .claims_for(&input.subject, Some("message.sent"))
                .unwrap()
                .len(),
            1
        );
    }
    #[tokio::test]
    async fn bound_agent_reply_is_retained_in_the_closed_thread() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "thread-agent-reply");
        let person = "person/operator";
        let agent = "agent/garden/reviewer";
        let thread = "thread/fixture";
        let intent = crate::graph::parse_intent(
            r#"version 2
agent "garden/reviewer" { workspace "/work/garden"; harness "omp" {} }
agent "garden/other" { workspace "/work/garden"; harness "omp" {} }
"#,
            state.store.origin(),
        )
        .unwrap();
        state
            .store
            .apply_internal(&intent, "declare-agent")
            .unwrap();
        let (created, _) = thread_metadata::apply(
            &state.store,
            person,
            thread,
            "create",
            thread_metadata::Edit::Create {
                agent,
                title: "Review",
            },
        )
        .unwrap();
        let (closed, _) = thread_metadata::apply(
            &state.store,
            person,
            thread,
            "close",
            thread_metadata::Edit::Close {
                revision: &created.id,
            },
        )
        .unwrap();
        let app = crate::api::router(state.clone());
        let post = |bound: Option<&'static str>| {
            let app = app.clone();
            let snapshot = new_client_snapshot(&state).id;
            async move {
                let request = json!({
                    "api_version": CLIENT_API_VERSION, "id": "action/agent-thread-reply",
                    "type": "thread.reply", "idempotency_key": "agent-thread-reply-key",
                    "fence": {"snapshot_id": snapshot},
                    "parameters": {"target_id": thread, "content":"Worker tests passed."},
                });
                let mut request = Request::builder()
                    .method("POST")
                    .uri("/v1/client/actions")
                    .header(LOCAL_PERSON_HEADER, agent)
                    .header("Content-Type", "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap();
                if let Some(bound) = bound {
                    request
                        .extensions_mut()
                        .insert(crate::api::BoundAgent(bound.into()));
                }
                let response = app.oneshot(request).await.unwrap();
                let status = response.status();
                let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
                    .await
                    .unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                (status, body)
            }
        };
        assert_eq!(post(None).await.0, StatusCode::FORBIDDEN);
        assert_ne!(post(Some("agent/garden/other")).await.0, StatusCode::OK);
        let (status, body) = post(Some(agent)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "unsupported-capability", "{body}");
        assert_eq!(body["message"], "thread writes require the verified fleet reader rollout", "{body}");
        state.store.configure_thread_write_rollout([]);
        let (status, reply) = post(Some(agent)).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        let message = reply["value"]["affected_ids"][1].as_str().unwrap();
        let claim = state
            .store
            .newest_claim_for_subject_kind(message, "message.sent")
            .unwrap()
            .unwrap();
        assert_eq!(claim.actor.as_deref(), Some(agent));
        assert_eq!(claim.body["fields"]["to"], person);
        assert_eq!(claim.body["fields"]["thread_id"], thread);
        assert_eq!(
            state.store.latest_claim_id(thread).unwrap().as_deref(),
            Some(closed.id.as_str())
        );
        assert!(
            state
                .store
                .claims_for_kind_at("work.person-asked", None, true, 10)
                .unwrap()
                .claims
                .is_empty()
        );
    }
}
