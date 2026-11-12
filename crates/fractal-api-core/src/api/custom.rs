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
