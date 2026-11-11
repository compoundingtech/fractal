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

/// `{subject, revision, current?, map, kdl, revisions}` for one agent or mission revision (the
/// current one when none is named); not found when that revision has no recorded map. `kdl` is
/// the revision's canonical redacted KDL, the text the map's node paths name.
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
            let Some(map) = map else { return Ok((client_snapshot_at(&state, index), None)) };
            let kdl = revision_kdl(&state, &subject, &revision, index)?;
            let revisions = state.store.source_map_revisions(&subject)?;
            let mut value = json!({"subject": subject, "revision": revision, "map": map, "kdl": kdl, "revisions": revisions.revisions});
            if let Some(current) = revisions.current {
                value["current"] = json!(current);
            }
            Ok((client_snapshot_at(&state, index), Some(value)))
        })
    })
    .await?;
    let value = value.ok_or_else(|| ApiError::not_found("no source map is recorded for this revision"))?;
    if serde_json::to_vec(&value).map_err(ApiError::internal)?.len() > CLIENT_MAX_RESPONSE_BYTES - 4096 {
        return Err(validation("the source map exceeds the client response limit"));
    }
    Ok((Extension(snapshot), Json(value)))
}

/// Canonical KDL of an exact subject revision, environment values redacted. Agents are keyed by
/// their content revision here, not by the declaration claim id `agent-declaration.get` names.
fn revision_kdl(state: &AppState, subject: &str, revision: &str, index: u64) -> Result<String, FractalError> {
    let missing = || FractalError::new("internal", format!("{subject}@{revision} has a source map but no declaration"));
    if subject.starts_with("mission/") {
        let (spec, _) = state.store.mission_declaration(subject, Some(revision), index)?.ok_or_else(missing)?;
        return Ok(crate::mission_render::render_mission_kdl(&spec)?.text);
    }
    let mut tree = state.store.agent_declaration_at(subject, revision)?.ok_or_else(missing)?;
    crate::graph::redact_agent_env_values(&mut tree);
    let uris = state.store.declared_resource_uris(&crate::graph::declared_resources(&tree))
        .map_err(|error| FractalError::new("internal", error.to_string()))?;
    crate::graph::render_agent_desired_kdl(&tree, &uris)
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
            Ok((client_snapshot_at(&state, index), bytes))
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

#[cfg(test)]
mod tests {
    use super::*;

    const SUBJECT: &str = "mission/example/mapped";
    const NAMED: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// Publish the mission, then record one `source.mapped` claim for its revision naming
    /// `NAMED`, plus a stored blob (`OTHER`) that no map names. Returns the revision.
    fn record(state: &AppState) -> String {
        let source = "version 2\nmission \"example/mapped\" state=\"ready\" {\n goal \"Mapped goal\"\n}\n";
        let intent = crate::graph::parse_intent(source, "source-node").unwrap();
        let preview = state.store.mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None }).unwrap();
        state.store.apply_as(&intent, &preview.subject_tokens, "mapped", Some("person/operator")).unwrap();
        let revision = intent.missions["example/mapped"].revision.clone();
        let revision = revision.as_str();
        let mut connection = state.store.connection.write();
        let tx = connection.transaction().unwrap();
        for (hash, text) in [(NAMED, "export default mission({})\n"), (OTHER, "private bytes\n")] {
            tx.execute("INSERT OR IGNORE INTO blobs(hash, bytes, size) VALUES (?1, ?2, ?3)", rusqlite::params![hash, text.as_bytes(), text.len() as u64]).unwrap();
        }
        let fields = json!({
            "subject": SUBJECT, "revision": revision, "set_revision": "owned-set/factory@s1",
            "repository": "schickling/factory", "ref": "main", "sha": "a".repeat(40), "sequence": 1,
            "files": [{"repository": "schickling/factory", "path": "graph/example/mapped/mission.ts", "blob": "b0", "blob_hash": NAMED}],
            "declaration": 0, "inputs": [0],
            "nodes": [{"kdl": "mission[example/mapped]", "kdlLines": [2, 4], "span": {"file": 0, "start": {"line": 1, "column": 1}, "end": {"line": 1, "column": 26}}, "frames": []}],
        });
        smallclaims::store::append_claim_record_tx(
            &tx, &state.store.origin, &fractal_schema::source_map::claim_subject(SUBJECT, revision), "source.mapped",
            None, &json!({ "fields": fields }), &[], None,
        ).unwrap();
        tx.commit().unwrap();
        revision.to_owned()
    }

    fn session(scopes: &[&str]) -> ClientSession {
        ClientSession::for_tests_scoped("client/test", "person/operator", scopes)
    }

    async fn map(state: &AppState, scopes: &[&str], revision: &str) -> Result<Value, ApiError> {
        get(State(state.clone()), Extension(session(scopes)), Query(SourceMapQuery { subject: SUBJECT.into(), revision: Some(revision.into()) }))
            .await.map(|(_, Json(value))| value)
    }

    async fn text(state: &AppState, scopes: &[&str], hash: &str, revision: &str) -> Result<Value, ApiError> {
        blob(State(state.clone()), Extension(session(scopes)), AxumPath(hash.into()), Query(SourceBlobQuery { subject: SUBJECT.into(), revision: revision.into() }))
            .await.map(|(_, Json(value))| value)
    }

    #[tokio::test]
    async fn source_reads_require_the_declarations_grant() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "source-node");
        let r1 = record(&state);
        for error in [map(&state, &["read.projections"], &r1).await.unwrap_err(), text(&state, &["read.projections"], NAMED, &r1).await.unwrap_err()] {
            assert_eq!(error.status, StatusCode::FORBIDDEN);
            assert_eq!(error.message, "Fractal does not show this source to this device");
        }
    }

    #[tokio::test]
    async fn source_map_answers_mapped_revisions_and_not_found_otherwise() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "source-node");
        let r1 = record(&state);
        let value = map(&state, &["read.declarations"], &r1).await.unwrap();
        assert_eq!(value["subject"], SUBJECT);
        assert_eq!(value["revision"], r1);
        assert_eq!(value["current"], r1);
        assert_eq!(value["map"]["files"][0]["blob_hash"], NAMED);
        assert!(value["kdl"].as_str().unwrap().contains("goal \"Mapped goal\""), "{value}");
        let current = get(State(state.clone()), Extension(session(&["read.declarations"])), Query(SourceMapQuery { subject: SUBJECT.into(), revision: None }))
            .await.map(|(_, Json(value))| value).unwrap();
        assert_eq!(current, value);
        assert_eq!(map(&state, &["read.declarations"], "unmapped").await.unwrap_err().status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn source_blob_serves_only_files_the_revision_map_names() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "source-node");
        let r1 = record(&state);
        let named = text(&state, &["read.declarations"], NAMED, &r1).await.unwrap();
        assert_eq!(named, json!({"hash": NAMED, "text": "export default mission({})\n"}));
        // Stored, but named by no map of this revision; and a named file under an unmapped revision.
        assert_eq!(text(&state, &["read.declarations"], OTHER, &r1).await.unwrap_err().status, StatusCode::NOT_FOUND);
        assert_eq!(text(&state, &["read.declarations"], NAMED, "unmapped").await.unwrap_err().status, StatusCode::NOT_FOUND);
        assert_eq!(text(&state, &["read.declarations"], "NOT-A-HASH", &r1).await.unwrap_err().status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
