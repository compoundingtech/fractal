use super::*;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoResolvedQuery {}

pub async fn read(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    Query(_query): Query<AutoResolvedQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    let person = session.authority_actor;
    if !person.starts_with("person/") {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden".into(),
            message: "only a person can read their auto-resolved asks".into(),
            details: Box::default(),
        });
    }
    let (snapshot, rows) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            Ok((client_snapshot_at(&state, index), state.store.auto_resolved_asks(&person, client_now_ms())?))
        })
    }).await?;
    Ok((Extension(snapshot), Json(rows)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    #[tokio::test]
    async fn read_is_person_only_and_rejects_unknown_queries() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        for (actor, path, expected) in [
            ("person/avery", "/v1/client/auto-resolved", StatusCode::OK),
            ("person/blair", "/v1/client/auto-resolved", StatusCode::OK),
            ("agent/example", "/v1/client/auto-resolved", StatusCode::FORBIDDEN),
            ("person/avery", "/v1/client/auto-resolved?person=person/blair", StatusCode::BAD_REQUEST),
        ] {
            let response = router(state.clone()).oneshot(Request::builder().uri(path)
                .header("x-st3-person", actor).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), expected);
            if expected == StatusCode::OK {
                let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["value"], json!({"items": [], "complete": true}));
            }
        }
    }
}
