//! Client reads of `source.mapped` claims and the file texts they name, for the web Source,
//! KDL and Changes views. Same records as `/v1/source-maps`; this is the paired-client surface.
use super::*;

const SCOPE: &str = "read.declarations";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMapQuery {
    subject: String,
    revision: Option<String>,
}

fn not_shown() -> ApiError {
    forbidden("Fractal does not show this source to this device")
}

/// `{subject, revision, map, revisions}` for one agent or mission revision; not found when that
/// revision has no recorded map.
pub async fn get(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<SourceMapQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    if !session.allows(SCOPE) {
        return Err(not_shown());
    }
    let subject = query.subject;
    let named = subject.strip_prefix("agent/").or_else(|| subject.strip_prefix("mission/"));
    if named.is_none_or(|name| name.is_empty() || name.contains('@')) {
        return Err(validation("subject must be an agent/ or mission/ subject"));
    }
    let (snapshot, value) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            let (revision, map) = state.store.source_map(&subject, query.revision.as_deref())?;
            let value = match map {
                Some(map) => {
                    let revisions = state.store.source_map_revisions(&subject)?;
                    Some(json!({"subject": subject, "revision": revision, "map": map, "revisions": revisions.revisions}))
                }
                None => None,
            };
            Ok::<_, FractalError>((client_snapshot_at(&state, index), value))
        })
    })
    .await?;
    let value = value.ok_or_else(|| ApiError::not_found("no source map is recorded for this revision"))?;
    if serde_json::to_vec(&value).map_err(ApiError::internal)?.len() > CLIENT_MAX_RESPONSE_BYTES - 4096 {
        return Err(validation("the source map exceeds the client response limit"));
    }
    Ok((Extension(snapshot), Json(value)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBlobQuery {
    subject: String,
    revision: String,
}

/// `{hash, text}`: one file version, by sha256, only when `subject@revision`'s source map names
/// it. `blobs` also holds bytes other claims reference; this read never serves those.
pub async fn blob(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    AxumPath(hash): AxumPath<String>,
    Query(query): Query<SourceBlobQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    if !session.allows(SCOPE) {
        return Err(not_shown());
    }
    if hash.len() != 64 || !hash.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(validation("a source blob is a lowercase 64-character sha256"));
    }
    let (snapshot, bytes) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            let (_, map) = state.store.source_map(&query.subject, Some(&query.revision))?;
            let named = map.is_some_and(|map| {
                map["files"].as_array().into_iter().flatten().any(|file| file["blob_hash"].as_str() == Some(hash.as_str()))
            });
            let bytes = if named { state.store.source_blob(&hash)?.map(|bytes| (hash, bytes)) } else { None };
            Ok::<_, FractalError>((client_snapshot_at(&state, index), bytes))
        })
    })
    .await?;
    let (hash, bytes) = bytes.ok_or_else(|| ApiError::not_found("this revision's source map names no such file"))?;
    let text = String::from_utf8(bytes).map_err(ApiError::internal)?;
    let value = json!({"hash": hash, "text": text});
    if serde_json::to_vec(&value).map_err(ApiError::internal)?.len() > CLIENT_MAX_RESPONSE_BYTES - 4096 {
        return Err(validation("the source file exceeds the client response limit"));
    }
    Ok((Extension(snapshot), Json(value)))
}
