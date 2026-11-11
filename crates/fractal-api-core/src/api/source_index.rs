//! Reads across source maps: a subject's mapped revisions, the reverse index over live
//! subjects, and the canonical KDL of a revision for the Changes diff.
use super::*;
use fractal_schema::source_map::index::{Revisions, Where};

#[derive(Deserialize)]
pub struct RevisionsQuery {
    subject: String,
}

/// `{subject, current, revisions: [...]}`, newest first.
pub async fn revisions(
    State(state): State<AppState>,
    Query(query): Query<RevisionsQuery>,
) -> Result<Json<Revisions>, ApiError> {
    let store = state.store.clone();
    blocking_action(move || store.source_map_revisions(&query.subject)).await.map(Json)
}

#[derive(Deserialize)]
pub struct WhereQuery {
    path: String,
    repository: Option<String>,
    line: Option<u32>,
}

/// Which live agents and missions the file at `path` (line `line`) declares or feeds.
pub async fn where_used(
    State(state): State<AppState>,
    Query(query): Query<WhereQuery>,
) -> Result<Json<Where>, ApiError> {
    if query.path.trim().is_empty() || query.line == Some(0) {
        return Err(ApiError::bad(FractalError::new(
            "invalid-source-map-query",
            "`path` is a repository-relative file path and `line` is 1-based",
        )));
    }
    let store = state.store.clone();
    blocking_action(move || store.source_where(&query.path, query.repository.as_deref(), query.line))
        .await
        .map(Json)
}

#[derive(Deserialize)]
pub struct DeclarationQuery {
    subject: String,
    revision: String,
}

/// `{subject, revision, kdl}`: the canonical KDL of an agent or mission revision, environment
/// values redacted.
pub async fn declaration(
    State(state): State<AppState>,
    Query(query): Query<DeclarationQuery>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    let DeclarationQuery { subject, revision } = query;
    blocking_action(move || {
        let kdl = if let Some(id) = subject.strip_prefix("mission/") {
            store
                .mission_spec(id, Some(&revision))
                .map_err(|error| FractalError::new("store-read-failed", format!("{error:#}")))?
                .map(|spec| crate::mission_render::render_mission_kdl(&spec).map(|rendered| rendered.text))
                .transpose()?
        } else if subject.starts_with("agent/") {
            store
                .agent_declaration_at(&subject, &revision)?
                .map(|mut tree| {
                    crate::graph::redact_agent_env_values(&mut tree);
                    let uris = store
                        .declared_resource_uris(&crate::graph::declared_resources(&tree))
                        .map_err(|error| FractalError::new("store-read-failed", format!("{error:#}")))?;
                    crate::graph::render_agent_desired_kdl(&tree, &uris)
                })
                .transpose()?
        } else {
            return Err(FractalError::new(
                "invalid-source-map-query",
                "only agent and mission revisions have a declaration",
            ));
        };
        let kdl = kdl.ok_or_else(|| {
            FractalError::new("unknown-revision", format!("{subject} has no revision {revision}"))
        })?;
        Ok(json!({"subject": subject, "revision": revision, "kdl": kdl}))
    })
    .await
    .map(Json)
}
