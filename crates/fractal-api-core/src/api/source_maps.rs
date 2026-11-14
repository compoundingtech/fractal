//! Reads of `source.mapped` claims and the file texts they name.
use super::*;

#[derive(Deserialize)]
pub struct MapQuery {
    subject: String,
    revision: Option<String>,
    /// Also answer `texts: {blob_hash: text}` for every file the map names.
    #[serde(default)]
    texts: bool,
}

/// `{subject, revision, map[, texts]}`; 404 `source-map-missing` when that revision has none.
/// A `step-run/…` subject answers its mission at the revision its run pinned, with `step` and
/// the step's KDL node path as `node`.
pub async fn get(
    State(state): State<AppState>,
    Query(query): Query<MapQuery>,
) -> Result<Json<Value>, ApiError> {
    let read = state
        .store
        .source_map_read(&query.subject, query.revision.as_deref())
        .map_err(ApiError::bad)?;
    let Some(map) = read.map else {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            code: "source-map-missing".into(),
            message: format!("no source map recorded for {}@{}", read.subject, read.revision),
            details: Box::default(),
        });
    };
    let mut response = json!({"subject": read.subject, "revision": read.revision, "map": map});
    if let (Some(step), Some(node)) = (read.step, read.node) {
        response["step"] = json!(step);
        response["node"] = json!(node);
    }
    if query.texts {
        let mut texts = serde_json::Map::new();
        for file in map["files"].as_array().into_iter().flatten() {
            let hash = file["blob_hash"].as_str().ok_or_else(|| ApiError::internal("source map file without blob_hash"))?;
            // The claim and the blobs it names are admitted together, so a missing text is a defect.
            let bytes = state
                .store
                .source_blob(hash)
                .map_err(ApiError::bad)?
                .ok_or_else(|| ApiError::internal(format!("source blob {hash} named by the map is not stored")))?;
            texts.insert(hash.to_owned(), Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        }
        response["texts"] = Value::Object(texts);
    }
    Ok(Json(response))
}

/// The UTF-8 text stored under a sha256 a source map names.
pub async fn blob(
    State(state): State<AppState>,
    AxumPath(hash): AxumPath<String>,
) -> Result<Response, ApiError> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad(FractalError::new("invalid-source-blob", "a source blob is a 64-character sha256")));
    }
    let bytes = state
        .store
        .source_blob(&hash)
        .map_err(ApiError::bad)?
        .ok_or_else(|| ApiError::not_found(format!("no source blob {hash}")))?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        bytes,
    )
        .into_response())
}
