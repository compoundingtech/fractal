//! Custom schema administration and generic client reads; paired devices cannot register kinds.
//! Raw notice subjects stay unavailable here until recipient-authorized native reads exist.
use crate::CustomStore;
use super::*;
use crate::store::custom::{RegistrationRequest, ReplyRequest};

pub(super) fn notices_unavailable() -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "unsupported-capability".into(),
        message: "Notices require recipient-authorized native reads; generic reads are unavailable.".into(),
        details: Box::new(serde_json::Map::from_iter([
            ("reason".into(), json!("notices-unavailable")),
        ])),
    }
}

pub async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegistrationRequest>,
) -> Result<Json<Value>, ApiError> {
    person_or_agent_actor(&request.actor, "invalid-custom-actor")?;
    let store = state.store.clone();
    let result = blocking_action(move || store.register_custom_kind(&request)).await?;
    signal_changed(&state);
    Ok(Json(result))
}
pub async fn registrations(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"items":state.store.custom_registrations().map_err(ApiError::internal)?}),
    ))
}
pub async fn reply(
    State(state): State<AppState>,
    Json(request): Json<ReplyRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    person_or_agent_actor(&request.actor, "invalid-custom-actor")?;
    let store = state.store.clone();
    let c = blocking_action(move || store.reply_custom_subject(&request)).await?;
    signal_changed(&state);
    Ok(Json(c))
}
#[derive(Default, Deserialize)]
pub struct ListQuery {
    kind: Option<String>,
    version: Option<u32>,
    cursor: Option<String>,
    limit: Option<usize>,
}
pub async fn list(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Query(query): Query<ListQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    if query
        .kind
        .as_deref()
        .is_some_and(fractal_schema::notice::is_reserved_kind)
    {
        return Err(notices_unavailable());
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let cursor = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor
                .strip_prefix("custom-page/")
                .and_then(|s| {
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(s)
                        .ok()
                })
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .ok_or_else(|| client_v0::validation("invalid custom page cursor"))
        })
        .transpose()?;
    let (snapshot, page) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            if let Some(value) = &cursor
                && (value["index"] != index || value["kind"] != json!(query.kind)
                    || value["version"] != json!(query.version)
                    || value["limit"] != limit || value["host"] != state.node
                    || !value["after"].is_string()) {
                return Ok(None);
            }
            let after = cursor.as_ref().and_then(|v| v["after"].as_str());
            // The store excludes reserved rows before LIMIT, preserving generic pagination.
            let mut items = state.store.custom_subjects(query.kind.as_deref(), query.version, after, limit + 1)?;
            let more = items.len() > limit;
            items.truncate(limit);
            let next = if more {
                Some(format!("custom-page/{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&json!({"host":state.node,"index":index,"kind":query.kind,"version":query.version,"limit":limit,"after":items.last().unwrap()["id"]}))?
                )))
            } else { None };
            let page = json!({"kind":"page","collection":"custom-subjects","filters":{},"items":items,"page":{"has_more":more,"next_cursor":next,"limit":limit}});
            Ok(Some((client_snapshot_at(&state, index), page)))
        })
    }).await?.ok_or_else(|| ApiError {
        status: StatusCode::GONE, code: "page-cursor-expired".into(),
        message: "custom collection changed; restart pagination".into(), details: Box::default(),
    })?;
    Ok((Extension(snapshot), Json(page)))
}
pub async fn read(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    AxumPath(id): AxumPath<String>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    if fractal_schema::notice::is_reserved_subject(&id) {
        return Err(notices_unavailable());
    }
    let result = blocking_store(move || {
        state.store.read_snapshot(|index| {
            Ok(state
                .store
                .custom_subject(&id)?
                .map(|value| (client_snapshot_at(&state, index), value)))
        })
    })
    .await?;
    let (snapshot, value) =
        result.ok_or_else(|| ApiError::not_found("custom source is unavailable"))?;
    Ok((Extension(snapshot), Json(value)))
}
#[derive(Deserialize)]
pub struct BasisQuery {
    subject: String,
    kinds: String,
}
pub async fn basis(
    State(state): State<AppState>,
    Query(query): Query<BasisQuery>,
) -> Result<Json<Value>, ApiError> {
    if fractal_schema::threads::is_reserved_subject(&query.subject)
        || query.kinds.split(',').any(fractal_schema::threads::is_reserved_kind)
    {
        return Err(ApiError::not_found("the thread is unavailable"));
    }
    if fractal_schema::notice::is_reserved_subject(&query.subject)
        || query.kinds.split(',').any(fractal_schema::notice::is_reserved_kind)
    {
        return Err(notices_unavailable());
    }
    let kinds = query
        .kinds
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if kinds.is_empty()
        || kinds.len() > 32
        || kinds.iter().any(|k| !fractal_schema::is_custom_claim_kind(k))
    {
        return Err(client_v0::validation(
            "basis needs 1..32 custom claim kinds",
        ));
    }
    fractal_schema::registry()
        .validate_subject(&query.subject)
        .map_err(|e| client_v0::validation(e.message))?;
    Ok(Json(
        json!({"subject":query.subject,"kinds":kinds,"revision":state.store.custom_basis_revision(&query.subject,&kinds).map_err(ApiError::internal)?}),
    ))
}

#[derive(Deserialize)]
pub struct DecisionInputsQuery {
    subject: String,
}

fn decision_inputs_error(error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<FractalError>().is_some() {
        error
    } else {
        crate::store::custom::fail(error.to_string()).into()
    }
}

pub async fn decision_inputs(
    State(state): State<AppState>,
    Query(query): Query<DecisionInputsQuery>,
) -> Result<Json<crate::store::custom::DecisionInputs>, ApiError> {
    fractal_schema::registry().validate_subject(&query.subject)
        .map_err(|error| client_v0::validation(error.message))?;
    let result = blocking_store(move || {
        state.store.decision_inputs(&query.subject).map_err(decision_inputs_error)
    }).await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests_shared_client_v0_tests::get_request as client_json;

    #[test]
    fn decision_inputs_preserves_typed_refusals() {
        let error = ApiError::internal(decision_inputs_error(
            FractalError::new("decision-inputs-too-large", "history exceeds limits").into(),
        ));
        assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.code, "decision-inputs-too-large");
        assert_eq!(error.message, "history exceeds limits");
    }

    #[test]
    fn decision_inputs_retains_invalid_claim_error_for_untyped_refusals() {
        let error = ApiError::internal(decision_inputs_error(
            anyhow::anyhow!("decision registration is unavailable"),
        ));
        assert_eq!(error.code, "invalid-custom-claim");
        assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    fn test_state(root: &std::path::Path) -> AppState {
        let mut state = crate::api::tests_shared::state(root);
        state.store = Arc::new(Store::open(&root.join("claims.sqlite3"), "node").unwrap());
        state
    }
    #[tokio::test]
    async fn notice_generic_detail_and_filters_are_explicitly_unavailable_without_writes() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        let before = state.store.index().unwrap();
        let app = router(state.clone());
        for subject in [
            "custom/attention",
            "custom/attention/source/absent",
            "custom/attention/snapshot/absent",
            "custom/attention/control/absent",
        ] {
            let (status, error) =
                client_json(app.clone(), &format!("/v1/client/custom-subjects/{subject}")).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
            assert_eq!(error["code"], "unsupported-capability");
            assert_eq!(error["details"]["reason"], "notices-unavailable");
        }
        for kind in [
            "attention.notice-source",
            "attention.notice-snapshot",
            "attention.notice-control",
            "custom.attention.notice-created",
            "custom.attention.snapshot-created",
            "custom.attention.snapshot-updated",
            "custom.attention.control-created",
            "custom.attention.control-updated",
        ] {
            let (status, error) = client_json(
                app.clone(),
                &format!("/v1/client/custom-subjects?kind={kind}&version=1&cursor=invalid"),
            ).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
            assert_eq!(error["details"]["reason"], "notices-unavailable");
        }
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn notice_legacy_rows_cannot_bypass_generic_list_pagination_or_detail() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        // Model cached rows from before namespace protection, including an alternate kind.
        let connection = rusqlite::Connection::open(root.path().join("claims.sqlite3")).unwrap();
        smallclaims::store::configure_projection_writer(&connection).unwrap();
        for (subject, kind) in [
            ("custom/attention/source/legacy", "garden.review"),
            ("custom/attention/snapshot/legacy", "garden.review"),
            ("custom/attention/control/legacy", "garden.review"),
            ("custom/garden/review/v1/one", "garden.review"),
            ("custom/garden/review/v1/two", "garden.review"),
        ] {
            let body = json!({
                "id":subject, "kind":"custom-subject", "registered_kind":kind,
                "schema_version":1, "registration":"legacy", "revision":"legacy",
                "state":"ready", "fields":{}
            });
            connection.execute(
                "INSERT INTO custom_sources (subject,registration,kind,revision,state,body,active,requested_at) VALUES (?1,'legacy',?2,'legacy','ready',?3,0,'0')",
                rusqlite::params![subject, kind, body.to_string()],
            ).unwrap();
        }
        drop(connection);
        let before = state.store.index().unwrap();
        let app = router(state.clone());
        for filter in ["", "kind=garden.review&version=1&", "version=1&"] {
            let route = format!("/v1/client/custom-subjects?{filter}limit=1");
            let (status, page) = client_json(app.clone(), &route).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["items"].as_array().unwrap().len(), 1);
            assert_eq!(page["items"][0]["id"], "custom/garden/review/v1/one");
            assert_eq!(page["page"]["has_more"], true);
            let cursor = page["page"]["next_cursor"].as_str().unwrap();
            let (status, next) = client_json(app.clone(), &format!("{route}&cursor={cursor}")).await;
            assert_eq!(status, StatusCode::OK, "{next}");
            assert_eq!(next["items"].as_array().unwrap().len(), 1);
            assert_eq!(next["items"][0]["id"], "custom/garden/review/v1/two");
            assert_eq!(next["page"]["has_more"], false);
        }
        let (status, error) = client_json(
            app, "/v1/client/custom-subjects/custom/attention/source/legacy",
        ).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["details"]["reason"], "notices-unavailable");
        assert_eq!(state.store.index().unwrap(), before);
    }

    fn legacy_claim(state: &AppState, subject: &str, kind: &str) -> ClaimRecord {
        // Model already-admitted old-peer history below today's notice write admission.
        let claim = state.store.connection.batched(|tx| crate::store::append_claim_tx(
            tx, state.store.origin(), subject, kind, None,
            &json!({"fields":{"private_marker":"notice-secret"}}), &[], None,
        )).unwrap().unwrap();
        state.store.committed_index.fetch_max(claim.store_index, std::sync::atomic::Ordering::Release);
        claim
    }

    #[tokio::test]
    async fn notice_paired_history_hides_legacy_subject_and_alternate_subject_kind() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        let credential = "notice-history-paired-credential";
        state.store.append_client_operation_claim(&ClaimInput {
            subject: "custom/client/notice-history-pair".into(),
            kind: "custom.client.pairing-completed".into(), actor: Some("person/example".into()),
            fields: BTreeMap::from([
                ("credential_hash".into(), json!(client_v0::credential_digest(credential))),
                ("session_actor".into(), json!("client/notice-history")),
                ("person_id".into(), json!("person/example")),
                ("scopes".into(), json!(["read.projections"])),
                ("expires_at_unix_ms".into(), json!(u64::MAX)),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let hidden = [
            legacy_claim(&state, "custom/attention/source/legacy-secret", "custom.example.created"),
            legacy_claim(&state, "custom/example/alternate-secret", "custom.attention.notice-created"),
            legacy_claim(&state, "custom/example/future-secret", "custom.attention.future-notice-kind"),
        ];
        let ordinary = legacy_claim(&state, "custom/example/visible", "custom.example.created");
        let app = router(state.clone());
        for path in std::iter::once("/v1/client/history?limit=500".to_owned())
            .chain(hidden.iter().map(|claim| format!("/v1/client/history/{}", claim.store_index)))
        {
            let response = app.clone().oneshot(axum::http::Request::builder()
                .uri(&path).header("authorization", format!("Bearer {credential}"))
                .body(Body::empty()).unwrap()).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            if path.contains('?') {
                assert_eq!(status, StatusCode::OK, "{value}");
                let items = value["value"]["items"].as_array().unwrap();
                assert!(items.iter().any(|item| item["revision"] == ordinary.id));
                for claim in &hidden {
                    assert!(!items.iter().any(|item| item["revision"] == claim.id));
                }
                assert!(!value.to_string().contains("secret"), "{value}");
            } else {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
                assert_eq!(value["code"], "unsupported-capability");
                assert_eq!(value["details"]["reason"], "notices-unavailable");
            }
        }
        for claim in &hidden {
            assert!(state.store.claim_by_id(&claim.id).unwrap().is_some());
        }
        for claim in &hidden {
            for suffix in ["", "&history=true", "&harness_only=true"] {
                let path = format!("/v1/status?subject={}{suffix}", urlencoding::encode(&claim.subject));
                let response = app.clone().oneshot(axum::http::Request::builder()
                    .uri(path).header("authorization", format!("Bearer {credential}"))
                    .body(Body::empty()).unwrap()).await.unwrap();
                assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
                let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
                let error: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(error["details"]["reason"], "notices-unavailable");
                assert!(!error.to_string().contains(&claim.id));
                assert!(!error.to_string().contains("notice-secret"));
            }
        }
        for (method, path) in [
            ("GET", "/v1/backup"),
            ("POST", "/v1/agents/rename-id/proof/export"),
            ("POST", "/v1/missions/rename-id/proof/export"),
            ("POST", "/v1/internal/replication/export"),
        ] {
            let response = app.clone().oneshot(axum::http::Request::builder()
                .method(method).uri(path).header("authorization", format!("Bearer {credential}"))
                .body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{path}");
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let error: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(error["details"]["reason"], "notices-unavailable", "{error}");
            let request = axum::http::Request::builder().method(method).uri(path)
                .body(Body::empty()).unwrap();
            let error = guard_bound_request(request, Some("agent/example/reader"), &state.store)
                .await.unwrap_err();
            if !path.starts_with(INTERNAL_ROUTE_PREFIX) {
                assert_eq!(error.details["reason"], "notices-unavailable");
            } else {
                assert_eq!(error.code, "foreign-agent-actor");
            }
        }
    }

    #[tokio::test]
    async fn notice_history_pagination_fills_visible_pages_and_preserves_snapshot_cursor() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        legacy_claim(&state, "custom/attention/source/oldest", "custom.example.created");
        let oldest = legacy_claim(&state, "custom/example/oldest", "custom.example.created");
        for index in 0..7 {
            legacy_claim(&state, &format!("custom/attention/source/middle-{index}"), "custom.example.created");
        }
        let middle = legacy_claim(&state, "custom/example/middle", "custom.example.created");
        legacy_claim(&state, "custom/example/hidden", "custom.attention.notice-created");
        let newest = legacy_claim(&state, "custom/example/newest", "custom.example.created");
        legacy_claim(&state, "custom/attention/source/newest", "custom.example.created");
        let snapshot = new_client_snapshot(&state);
        let first = client_reads::client_history(
            State(state.clone()), Extension(snapshot.clone()),
            Query(ClientListQuery { limit: Some(2), ..Default::default() }),
        ).await.unwrap().0;
        assert_eq!(first.items.iter().map(|item| item["revision"].as_str().unwrap()).collect::<Vec<_>>(),
            [newest.id.as_str(), middle.id.as_str()]);
        assert!(first.page.has_more);
        let cursor = first.page.next_cursor.unwrap();
        let decoded = decode_client_cursor(&cursor).unwrap();
        assert_eq!(decoded.offset, 2);
        assert_eq!(decoded.before_index, Some(middle.store_index));
        legacy_claim(&state, "custom/example/after-snapshot", "custom.example.created");
        let second = client_reads::client_history(
            State(state.clone()), Extension(snapshot),
            Query(ClientListQuery { cursor: Some(cursor), limit: Some(2), ..Default::default() }),
        ).await.unwrap().0;
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0]["revision"], oldest.id);
        assert!(!second.page.has_more);
        assert!(second.page.next_cursor.is_none());
        assert!(second.page.cursor_expires_at.is_none());
        let tail = records::generic_claims_page(&state.store, Default::default(), None, 0, Some(oldest.store_index + 1), true, 1, None).unwrap();
        assert_eq!(tail.claims[0].id, oldest.id);
        assert!(tail.next_cursor.is_none(), "hidden tail must not create another page");
    }

    #[tokio::test]
    async fn notice_generic_claims_events_and_basis_do_not_disclose_reserved_claims() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        let oldest = legacy_claim(&state, "custom/example/first", "custom.example.created");
        let hidden = [
            legacy_claim(&state, "custom/attention/source/raw", "custom.example.created"),
            legacy_claim(&state, "custom/example/alternate", "custom.attention.notice-created"),
        ];
        let newest = legacy_claim(&state, "custom/example/last", "custom.example.created");
        let app = router(state.clone());
        for (order, expected) in [("asc", [&oldest, &newest]), ("desc", [&newest, &oldest])] {
            let (status, page) = client_json(app.clone(), &format!("/v1/claims?limit=1&order={order}")).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["claims"][0]["id"], expected[0].id);
            assert_eq!(page["next_cursor"], expected[0].store_index);
            let filter = if order == "asc" { "after_index" } else { "before_index" };
            let (status, page) = client_json(app.clone(), &format!(
                "/v1/claims?limit=1&order={order}&{filter}={}", expected[0].store_index,
            )).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["claims"][0]["id"], expected[1].id);
            assert!(page["next_cursor"].is_null());
        }
        for claim in &hidden {
            let (status, _) = client_json(app.clone(), &format!("/v1/claims/by-id/{}", urlencoding::encode(&claim.id))).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            let (status, page) = client_json(app.clone(), &format!(
                "/v1/claims?subject={}", urlencoding::encode(&claim.subject),
            )).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["claims"], json!([]));
        }
        for path in [
            "/v1/custom/basis?subject=custom/attention/source/absent&kinds=custom.example.created",
            "/v1/custom/basis?subject=custom/example/alternate&kinds=custom.attention.notice-created",
        ] {
            let (status, error) = client_json(app.clone(), path).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
            assert_eq!(error["details"]["reason"], "notices-unavailable");
        }
        let (status, page) = client_json(app.clone(), "/v1/events/page?after=0&limit=2").await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["subject"], oldest.subject);
        assert_eq!(page["next_after"], hidden[0].store_index);
        assert_eq!(page["has_more"], true);
        let feed = client_v0::events(
            State(state), Extension(client_v0::ClientSession::for_tests_scoped("client/example", "person/example", &["read.projections"])),
            Query(client_v0::EventsQuery { after: Some("event-cursor/node/0".into()), limit: Some(500), ..Default::default() }),
        ).await.unwrap().0;
        assert_eq!(feed["items"].as_array().unwrap().len(), 2);
        assert!(!feed.to_string().contains("custom.attention"));
        assert!(!feed.to_string().contains("custom/attention"));
        assert!(!feed.to_string().contains("custom/example/alternate"));
    }

    #[tokio::test]
    async fn thread_metadata_is_hidden_from_generic_reads_without_losing_raw_history() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        let (thread, _) = crate::store::thread_metadata::apply(
            &state.store,
            "person/example",
            "thread/private-example",
            "create-private-example",
            crate::store::thread_metadata::Edit::Create {
                agent: "agent/example",
                title: "private thread title",
            },
        ).unwrap();
        let other = legacy_claim(&state, "custom/example/public", "custom.example.created");
        let app = router(state.clone());

        for path in [
            "/v1/claims",
            "/v1/claims?subject=thread/private-example",
            "/v1/events?after=0&wait=false",
            "/v1/events/page?after=0",
            "/v1/client/history",
            "/v1/status?history=true",
        ] {
            let (status, value) = client_json(app.clone(), path).await;
            assert_eq!(status, StatusCode::OK, "{path}: {value}");
            let rendered = value.to_string();
            assert!(!rendered.contains(&thread.id), "{path}: {rendered}");
            assert!(!rendered.contains("private-example"), "{path}: {rendered}");
            assert!(!rendered.contains("private thread title"), "{path}: {rendered}");
            if matches!(path, "/v1/claims" | "/v1/client/history" | "/v1/events/page?after=0") {
                assert!(rendered.contains(&other.subject), "{path}: {rendered}");
            }
        }
        for path in [
            format!("/v1/claims/by-id/{}", urlencoding::encode(&thread.id)),
            format!("/v1/client/history/{}", thread.store_index),
            "/v1/status?subject=thread/private-example&history=true".to_owned(),
            "/v1/status?subject=thread/private-example&harness_only=true".to_owned(),
            "/v1/custom/basis?subject=thread/private-example&kinds=custom.example.created".to_owned(),
        ] {
            let (status, value) = client_json(app.clone(), &path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {value}");
            assert!(!value.to_string().contains("private thread title"), "{path}: {value}");
        }
        let feed = client_v0::events(
            State(state.clone()),
            Extension(client_v0::ClientSession::for_tests_scoped(
                "client/example", "person/other", &["read.projections"],
            )),
            Query(client_v0::EventsQuery {
                after: Some("event-cursor/node/0".into()),
                limit: Some(500),
                ..Default::default()
            }),
        ).await.unwrap().0;
        assert!(feed.to_string().contains(&other.subject));
        assert!(!feed.to_string().contains("private-example"));
        assert!(!feed.to_string().contains("private thread title"));
        let raw = state.store.claim_by_id(&thread.id).unwrap().unwrap();
        assert_eq!(raw.body["fields"]["title"], "private thread title");
    }

    #[tokio::test]
    async fn receiving_member_hides_thread_routing_but_keeps_ordinary_messages() {
        const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let source_root = tempfile::tempdir().unwrap();
        let receiver_root = tempfile::tempdir().unwrap();
        let state = test_state(receiver_root.path());
        let source = Store::open(&source_root.path().join("source.db"), "newer-peer").unwrap();
        source.bind_fleet(FLEET).unwrap();
        state.store.bind_fleet(FLEET).unwrap();
        // Run the c71 claim vocabulary first against the exact same persisted receiver.
        let mut old_registry = fractal_schema::registry().clone();
        let fields = &mut old_registry.claims.get_mut("message.sent").unwrap().fields;
        for field in fractal_schema::threads::MESSAGE_RELATION_FIELDS {
            fields.remove(*field);
        }
        state.store.set_claim_registry(old_registry);
        source.set_member_key(Some(Arc::new(crate::fleet::MemberKey::generate().unwrap().0))).unwrap();

        let person = "person/example";
        let agent = "agent/example";
        let thread = "thread/private-message-fixture";
        let quoted = "message/original-thread-message";
        let private_ask = "step-run/private-link-fixture";
        let private_episode = "private-episode-fixture";
        let reply_body = json!({"fields": {
            "from": person, "to": agent, "status": "sent", "content": "ordinary visible text",
            "thread_id": thread, "thread_quote_id": quoted,
            "thread_origin_ask": private_ask, "thread_origin_episode": private_episode,
            "thread_reader_version": 2,
        }});
        let refused = state.store.append_client_claim(&ClaimInput {
            subject: "message/local-generic-thread-send".into(), kind: "message.sent".into(),
            actor: Some(person.into()), fields: serde_json::from_value(reply_body["fields"].clone()).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap_err();
        assert_eq!(refused.code, "thread-delivery-required");
        // Simulate a producer that knows the future fields at the claim seam. The receiving
        // member still runs real hash verification, replication admission, and projection.
        let reply = source.connection.batched(|tx| smallclaims::store::append_claim_record_tx(
            tx, source.origin(), "message/visible-thread-message", "message.sent",
            Some(person), &reply_body, &[], None,
        )).unwrap().unwrap();
        // The compatible ordinary message and the unknown relation share a signed envelope.
        // Keep the private relationship in the relation body, under a separate opaque subject.
        let (ordinary, separate) = source.connection.batched(|tx| -> anyhow::Result<_> {
            let ordinary = smallclaims::store::append_claim_record_tx(
                tx, source.origin(), "message/ordinary-unlinked", "message.sent", Some(person),
                &json!({"fields": {
                    "from": person, "to": agent, "status": "sent", "content": "ordinary control"
                }}), &[], None,
            )?;
            let separate = smallclaims::store::append_claim_record_tx(
                tx, source.origin(), "thread-link/opaque-relation-fixture", "thread.message-linked",
                Some(person), &reply_body, &[], Some(&ordinary.batch_id),
            )?;
            Ok((ordinary, separate))
        }).unwrap().unwrap();
        assert_eq!(ordinary.batch_id, separate.batch_id);
        source.committed_index.fetch_max(separate.store_index, std::sync::atomic::Ordering::Release);
        let exchange = source.export_replication_exchange(FLEET, &state.store.replication_inventory().unwrap()).unwrap();
        assert!(exchange.envelopes.iter().any(|envelope| envelope.signature.is_some()));
        state.store.receive_replication_exchange(source.origin(), FLEET, &exchange).unwrap();
        let admitted = state.store.validate_replication_backlog().unwrap();
        assert_eq!(admitted.unknown, 2);
        assert_eq!(admitted.invalid, 0);
        state.store.project_replication_backlog().unwrap();
        assert!(state.store.claim_by_id(&reply.id).unwrap().is_none());
        assert!(state.store.claim_by_id(&separate.id).unwrap().is_none());
        let records = state.store.replica_records(true).unwrap();
        let quarantine = |claim: &ClaimRecord| records.iter()
            .find(|record| record.claim_id.as_deref() == Some(claim.id.as_str())).unwrap();
        let inline_record = quarantine(&reply);
        assert_eq!(inline_record.state, "unknown");
        assert_eq!(inline_record.error_code.as_deref(), Some("unknown-claim-field"));
        let record = quarantine(&separate);
        assert_eq!(record.state, "unknown");
        assert_eq!(record.error_code.as_deref(), Some("unknown-claim-kind"));
        assert_eq!(state.store.replication_signatures_for(&[
            smallclaims::claim::ReplicaEnvelopeId {
                writer: record.writer.clone(), sequence: record.sequence,
                hash: record.envelope_hash.clone(),
            },
        ]).unwrap().len(), 1);
        let old_diagnostic_shape = serde_json::to_string(&record).unwrap();
        assert!(old_diagnostic_shape.contains(&separate.subject));
        assert!(old_diagnostic_shape.contains(&separate.id));
        assert!(old_diagnostic_shape.contains("thread.message-linked"));
        for hidden in [thread, quoted, private_ask, private_episode, "ordinary visible text"] {
            assert!(!old_diagnostic_shape.contains(hidden), "{old_diagnostic_shape}");
        }
        // Reopen as a reader-ready build. The SQLite reader floor advances before held
        // inline messages can enter the graph. The old receiver had no such routing rows.
        state.store.connection.write().pragma_update(None, "user_version", 20).unwrap();
        drop(state);
        let state = test_state(receiver_root.path());
        let floor: u32 = state.store.readers.get().pragma_query_value(None, "user_version",
            |row| row.get(0)).unwrap();
        assert_eq!(floor, fractal_schema::STORAGE_VERSION);
        let upgraded_admission = state.store.validate_replication_backlog().unwrap();
        assert!(upgraded_admission.valid > 0);
        assert_eq!(upgraded_admission.invalid, 0);
        state.store.project_replication_backlog().unwrap();
        let reply = state.store.claim_by_id(&reply.id).unwrap().unwrap();
        assert_eq!(reply.origin, source.origin());
        assert!(state.store.claim_by_id(&separate.id).unwrap().is_none());
        let app = router(state.clone());
        for path in [
            format!("/v1/claims/by-id/{}", urlencoding::encode(&reply.id)),
            format!("/v1/client/history/{}", reply.store_index),
        ] {
            let (status, body) = client_json(app.clone(), &path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        }
        let (status, diagnostic) = client_json(app.clone(),
            &format!("/v1/replication/records/{}", record.record_ref)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{diagnostic}");
        let (status, ordinary_read) = client_json(app.clone(),
            &format!("/v1/claims/by-id/{}", urlencoding::encode(&ordinary.id))).await;
        assert_eq!(status, StatusCode::OK, "{ordinary_read}");
        assert_eq!(ordinary_read["body"]["fields"]["content"], "ordinary control");
        let (status, claims) = client_json(app.clone(),
            "/v1/claims?subject=message/visible-thread-message").await;
        assert_eq!(status, StatusCode::OK, "{claims}");
        assert_eq!(claims["claims"], json!([]));
        // The local serving boundary permits these generic reads for a bound agent.
        // They must hide private message bodies in addition to routing metadata.
        let bound_app = app.clone()
            .layer(from_fn_with_state((state.store.clone(), None), guard_bound_actor))
            .layer(Extension(BoundAgent("agent/example/outsider".into())));
        let (status, events) = client_json(bound_app.clone(),
            "/v1/events/page?after=0&subject=message/visible-thread-message").await;
        assert_eq!(status, StatusCode::OK, "{events}");
        assert_eq!(events["items"], json!([]));
        let (status, ordinary_events) = client_json(bound_app.clone(),
            "/v1/events/page?after=0&subject=message/ordinary-unlinked").await;
        assert_eq!(status, StatusCode::OK, "{ordinary_events}");
        assert_eq!(ordinary_events["items"][0]["body"]["fields"]["content"], "ordinary control");
        let (status, status_body) = client_json(bound_app.clone(),
            "/v1/status?subject=message/visible-thread-message&history=true").await;
        assert_eq!(status, StatusCode::OK, "{status_body}");
        assert_eq!(status_body["subjects"], json!([]));
        let (status, harness_status) = client_json(bound_app.clone(),
            "/v1/status?subject=message/visible-thread-message&harness_only=true").await;
        assert_eq!(status, StatusCode::OK, "{harness_status}");
        // The harness-only shortcut gives the same empty-facts placeholder as an
        // unknown message. It does not read conversation content.
        assert_eq!(harness_status["subjects"][0]["actual"], Value::Null);
        assert_eq!(harness_status["subjects"][0]["claims"], json!([]));
        let (status, ordinary_status) = client_json(bound_app.clone(),
            "/v1/status?subject=message/ordinary-unlinked&history=true").await;
        assert_eq!(status, StatusCode::OK, "{ordinary_status}");
        assert_eq!(ordinary_status["subjects"][0]["actual"]["content"], "ordinary control");
        let (status, message) = client_json(bound_app.clone(),
            "/v1/messages/read/visible-thread-message").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{message}");
        let (status, ordinary_message) = client_json(bound_app,
            "/v1/messages/read/ordinary-unlinked").await;
        assert_eq!(status, StatusCode::OK, "{ordinary_message}");
        assert_eq!(ordinary_message["content"], "ordinary control");

        let raw = reply;
        assert_eq!(raw.body["fields"]["thread_id"], thread);
        assert_eq!(raw.body["fields"]["thread_quote_id"], quoted);
        assert_eq!(raw.body["fields"]["thread_origin_ask"], private_ask);
        assert_eq!(raw.body["fields"]["thread_origin_episode"], private_episode);
        let session = client_v0::ClientSession::local(Some(person)).unwrap();
        let header = client_v0::session_message_body(&state, &raw, true);
        let content = client_v0::session_message_content(&session, "session/example", &raw);
        assert_eq!(content["text"], "ordinary visible text");
        for value in [&events, &status_body, &harness_status, &message, &header, &content] {
            let encoded = value.to_string();
            for hidden in [thread, quoted, private_ask, private_episode] {
                assert!(!encoded.contains(hidden), "{value}");
            }
            for field in fractal_schema::threads::MESSAGE_RELATION_FIELDS {
                assert!(!encoded.contains(*field), "{value}");
            }
        }
    }

    #[test]
    fn version_one_replica_holds_initial_and_incremental_thread_messages_until_upgrade() {
        use crate::store::thread_metadata::{self, Edit, PersonAuthority};
        use fractal_schema::threads::{MESSAGE_READER_VERSION, MESSAGE_READER_VERSION_FIELD};

        const FLEET: &str = "af599eec-4f67-43b7-82c9-13f042f7d77c";
        const THREAD: &str = "thread/reader-floor";
        const PERSON: &str = "person/avery";
        const AGENT: &str = "agent/reader-floor";
        let source_root = tempfile::tempdir().unwrap();
        let receiver_root = tempfile::tempdir().unwrap();
        let source = Store::open(&source_root.path().join("source.db"), "writer").unwrap();
        source.bind_fleet(FLEET).unwrap();
        source.set_member_key(Some(Arc::new(crate::fleet::MemberKey::generate().unwrap().0))).unwrap();
        source.configure_thread_write_rollout([]);
        thread_metadata::apply(&source, PERSON, THREAD, "create-reader-floor",
            Edit::Create { agent: AGENT, title: "Reader floor" }).unwrap();

        let receiver = test_state(receiver_root.path());
        receiver.store.bind_fleet(FLEET).unwrap();
        let mut v1_registry = fractal_schema::registry().clone();
        let fields = &mut v1_registry.claims.get_mut("message.sent").unwrap().fields;
        assert!(fields.remove(MESSAGE_READER_VERSION_FIELD).is_some());
        assert!(fields.contains_key("thread_id"));
        receiver.store.set_claim_registry(v1_registry);

        let message = |subject: &str, content: &str| ClaimInput {
            subject: subject.into(), kind: "message.sent".into(), actor: Some(PERSON.into()),
            fields: serde_json::from_value(json!({
                "from": PERSON, "to": AGENT, "status": "sent", "content": content,
            })).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: Some(subject.into()),
        };
        let send = |subject: &str, content: &str| {
            let mut input = message(subject, content);
            input.fields.insert("thread_id".into(), json!(THREAD));
            let (claim, appended) = crate::store::thread_delivery::append_reply(
                &source, &input, PersonAuthority::TrustedLocal, None,
            ).unwrap();
            assert!(appended);
            assert_eq!(claim.body["fields"][MESSAGE_READER_VERSION_FIELD], MESSAGE_READER_VERSION);
            claim
        };
        let receive = |target: &Store| {
            let inventory = target.replication_inventory().unwrap();
            let exchange = source.export_replication_exchange(FLEET, &inventory).unwrap();
            assert!(exchange.envelopes.iter().any(|envelope| envelope.signature.is_some()));
            target.receive_replication_exchange(source.origin(), FLEET, &exchange).unwrap();
            let admission = target.validate_replication_backlog().unwrap();
            target.project_replication_backlog().unwrap();
            admission
        };

        let first = send("message/initial-v2", "private initial history");
        let ordinary = source.append_client_claim(&message("message/ordinary-control", "ordinary control")).unwrap();
        assert_eq!(receive(&receiver.store).unknown, 1);
        assert!(receiver.store.claim_by_id(&ordinary.id).unwrap().is_some());
        assert!(receiver.store.claim_by_id(&first.id).unwrap().is_none());
        let second = send("message/catch-up-v2", "private catch-up history");
        receive(&receiver.store);
        assert!(receiver.store.claim_by_id(&second.id).unwrap().is_none());
        let quarantined = receiver.store.replica_records(true).unwrap();
        for claim in [&first, &second] {
            let record = quarantined.iter().find(|record| record.claim_id.as_deref() == Some(claim.id.as_str())).unwrap();
            assert_eq!(record.state, "unknown");
            assert_eq!(record.error_code.as_deref(), Some("unknown-claim-field"));
            assert!(!serde_json::to_string(record).unwrap().contains("private "));
        }

        receiver.store.connection.write().pragma_update(None, "user_version", 21).unwrap();
        drop(receiver);
        let upgraded = test_state(receiver_root.path());
        assert_eq!(upgraded.store.readers.get().pragma_query_value(None, "user_version",
            |row| row.get::<_, u32>(0)).unwrap(), 22);
        let report = upgraded.store.validate_replication_backlog().unwrap();
        assert_eq!(report.valid, 2);
        assert_eq!(report.invalid, 0);
        upgraded.store.project_replication_backlog().unwrap();
        for claim in [&first, &second] {
            let admitted = upgraded.store.claim_by_id(&claim.id).unwrap().unwrap();
            assert_eq!(admitted.body, claim.body);
        }
        assert_eq!(upgraded.store.validate_replication_backlog().unwrap().valid, 0);

        // A preview-only sender's unmarked private message is invalid on the final reader.
        let mut unmarked = message("message/unsafe-preview", "preview private");
        unmarked.fields.insert("thread_id".into(), json!(THREAD));
        let unmarked = source.connection.batched(|tx| smallclaims::store::append_claim_record_tx(
            tx, source.origin(), &unmarked.subject, &unmarked.kind, unmarked.actor.as_deref(),
            &json!({"fields": unmarked.fields}), &[], None,
        )).unwrap().unwrap();
        source.committed_index.fetch_max(unmarked.store_index, std::sync::atomic::Ordering::Release);
        assert_eq!(receive(&upgraded.store).invalid, 1);
        assert!(upgraded.store.claim_by_id(&unmarked.id).unwrap().is_none());
    }

    #[test]
    fn generic_message_events_keep_ordinary_reply_metadata() {
        let mut event = EventRecord {
            store_index: 1, kind: "message.sent".into(), subject: "message/reply".into(),
            body: json!({"fields": {"in_reply_to": "message/ordinary", "content": "Reply"}}),
        };
        assert!(generic_event_visible(&event));
        event.body["fields"]["thread_id"] = json!("thread/example");
        assert!(!generic_event_visible(&event));
        event.body = json!({"thread_origin_episode": "private-old-shape"});
        assert!(!generic_event_visible(&event));
    }

    #[tokio::test]
    async fn notice_ordinary_history_pagination_keeps_exact_page_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        let claims = (0..4).map(|index| legacy_claim(
            &state, &format!("custom/example/ordinary-{index}"), "custom.example.created",
        )).collect::<Vec<_>>();
        let snapshot = new_client_snapshot(&state);
        let mut cursor = None;
        for (page_index, expected) in [[&claims[3], &claims[2]], [&claims[1], &claims[0]]].into_iter().enumerate() {
            let page = client_reads::client_history(
                State(state.clone()), Extension(snapshot.clone()),
                Query(ClientListQuery { cursor, limit: Some(2), ..Default::default() }),
            ).await.unwrap().0;
            assert_eq!(page.items.iter().map(|item| item["revision"].as_str().unwrap()).collect::<Vec<_>>(),
                expected.map(|claim| claim.id.as_str()));
            assert_eq!(page.page.has_more, page_index == 0);
            cursor = page.page.next_cursor;
            assert_eq!(cursor.is_some(), page_index == 0);
        }
    }

    #[tokio::test]
    async fn notice_status_excludes_reserved_fields_and_claim_ids_before_reduction() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path());
        let hidden = [
            legacy_claim(&state, "custom/attention/source/status-secret", "custom.example.created"),
            legacy_claim(&state, "custom/example/status-alternate", "custom.attention.notice-created"),
        ];
        let ordinary = state.store.connection.batched(|tx| crate::store::append_claim_tx(
            tx, state.store.origin(), "custom/example/status-visible", "custom.example.created", None,
            &json!({"fields":{"title":"Visible"}}), &[], None,
        )).unwrap().unwrap();
        state.store.committed_index.fetch_max(ordinary.store_index, std::sync::atomic::Ordering::Release);
        let cut = state.store.index().unwrap();
        let raw = state.store.status_history(None, None, Some(cut)).unwrap();
        assert!(serde_json::to_string(&raw).unwrap().contains("notice-secret"));
        let app = router(state.clone());
        for path in ["/v1/status".into(), "/v1/status?history=true".into(),
            format!("/v1/status?at_index={cut}"), format!("/v1/status?history=true&at_index={cut}")] {
            let (status, value) = client_json(app.clone(), &path).await;
            assert_eq!(status, StatusCode::OK, "{value}");
            let shown = value.to_string();
            assert!(!shown.contains("notice-secret"), "{shown}");
            for claim in &hidden {
                assert!(!shown.contains(&claim.id), "{shown}");
                assert!(!shown.contains(&claim.subject), "{shown}");
            }
        }
        for claim in &hidden {
            for suffix in ["".into(), "&history=true".into(), "&harness_only=true".into(),
                format!("&at_index={cut}"), "&harness_only=true&at_index=0".into()] {
                let (status, error) = client_json(app.clone(), &format!(
                    "/v1/status?subject={}{suffix}", urlencoding::encode(&claim.subject),
                )).await;
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
                assert_eq!(error["details"]["reason"], "notices-unavailable");
                assert!(!error.to_string().contains(&claim.id));
                assert!(state.store.claim_by_id(&claim.id).unwrap().is_some());
            }
        }
        let (_, visible) = client_json(app, &format!("/v1/status?subject={}", ordinary.subject)).await;
        assert!(visible.to_string().contains(&ordinary.id));
    }

}
