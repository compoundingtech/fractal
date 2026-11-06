//! Custom schema administration and generic client reads; paired devices cannot register kinds.
//! Raw notice subjects stay unavailable here until recipient-authorized native reads exist.
use crate::CustomStore;
use super::*;
use crate::store::custom::{RegistrationRequest, ReplyRequest};

fn notices_unavailable() -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "unsupported-capability".into(),
        message: "Notices require recipient-authorized native reads; generic custom reads are unavailable.".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests_shared_client_v0_tests::get_request as client_json;

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

}
