//! Versioned plugin replay, intentionally outside the frozen client-v0 contract.
use super::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayQuery {
    pub source: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn replay(
    State(state): State<AppState>,
    Query(query): Query<ReplayQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.unwrap_or(64);
    if !(1..=128).contains(&limit) || query.source.is_empty() || query.source.len() > 142
        || query.cursor.as_ref().is_some_and(|cursor| cursor.len() > 4096) {
        return Err(ApiError::bad(FractalError::new(
            "validation-failed", "capture replay requires a source, limit 1 to 128 and bounded cursor",
        )));
    }
    let result = read_deadline::spawn_blocking(move || {
        crate::reconcile::github_capture::capture_replay(
            &state.store, &state.state_dir, &query.source, query.cursor.as_deref(), limit,
        )
    }).await.map_err(ApiError::internal)?.map_err(ApiError::internal)?;
    Ok(Json(match result {
        Some(page) => serde_json::json!({"state":"ready","page":page}),
        None => serde_json::json!({"state":"projection-pending","page":null}),
    }))
}
