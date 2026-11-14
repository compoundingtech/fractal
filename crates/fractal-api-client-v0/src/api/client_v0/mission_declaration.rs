use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationQuery {
    mission: String,
    revision: Option<String>,
}

pub async fn get(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<DeclarationQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    if !session.allows("read.declarations") {
        return Err(forbidden("Fractal does not show this declaration to this device"));
    }
    if !query.mission.starts_with("mission/") || query.mission.len() == "mission/".len() {
        return Err(validation("mission must be a mission/ subject"));
    }
    let (snapshot, value) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            let value = state.store.mission_declaration(&query.mission, query.revision.as_deref(), index)?;
            let value = value.map(|(spec, published_at)| {
                let rendered = crate::mission_render::render_mission_kdl(&spec)?;
                let mut value = json!({"mission": spec.subject, "revision": spec.revision,
                    "published_at": published_at, "text": rendered.text, "legacy": rendered.legacy});
                // The source a revision was rendered from, as its first publication recorded it.
                if let Some(provenance) = state.store.mission_provenance(&spec.id, &spec.revision)
                    .map_err(|error| FractalError::new("internal", error.to_string()))?
                {
                    value["provenance"] = json!({
                        "repository": provenance.source.repository,
                        "commit": provenance.source.commit,
                        "path": provenance.source.path,
                        "renderer": provenance.source.renderer,
                    });
                }
                Ok::<_, FractalError>(value)
            }).transpose()?;
            Ok((client_snapshot_at(&state, index), value))
        })
    }).await?;
    let value = value.ok_or_else(|| ApiError::not_found("the mission revision is not published"))?;
    if serde_json::to_vec(&value).map_err(ApiError::internal)?.len() > CLIENT_MAX_RESPONSE_BYTES - 4096 {
        return Err(validation("the mission declaration exceeds the client response limit"));
    }
    Ok((Extension(snapshot), Json(value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(state: &AppState, goal: &str) -> String {
        let source = format!("version 2\nmission \"example/declaration\" state=\"ready\" {{\n goal \"{goal}\"\n gate \"check\" {{ exec \"true\"; host \"declaration-node\"; workspace \".\"; env {{ TOKEN \"secret-value\"; }}; }}\n}}\n");
        let intent = crate::graph::parse_intent(&source, "declaration-node").unwrap();
        let preview = state.store.mission(&intent, crate::model::IntentInput {
            kdl: source, source_name: None,
        }).unwrap();
        state.store.apply_as(&intent, &preview.subject_tokens, goal, Some("person/operator")).unwrap();
        intent.missions["example/declaration"].revision.clone()
    }

    async fn read(state: &AppState, revision: Option<String>) -> Result<Value, ApiError> {
        get(State(state.clone()), Extension(ClientSession::for_tests_scoped(
            "client/test", "person/operator", &["read.declarations"],
        )), Query(DeclarationQuery { mission: "mission/example/declaration".into(), revision }))
            .await.map(|(_, Json(value))| value)
    }

    #[tokio::test]
    async fn mission_declaration_reads_immutable_revisions_and_not_found() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "declaration-node");
        assert_eq!(read(&state, None).await.unwrap_err().status, StatusCode::NOT_FOUND);
        let first = publish(&state, "First goal");
        let original = read(&state, Some(first.clone())).await.unwrap();
        assert_eq!(original["mission"], "mission/example/declaration");
        assert_eq!(original["revision"], first);
        assert!(original["published_at"].as_u64().unwrap() > 0);
        assert_eq!(original["legacy"], false);
        assert!(original["text"].as_str().unwrap().contains("First goal"));
        let second = publish(&state, "Second goal");
        assert!(!original["text"].as_str().unwrap().contains("secret-value"));
        assert!(original["text"].as_str().unwrap().contains("<redacted>"));
        assert_eq!(read(&state, Some(first)).await.unwrap(), original);
        assert_eq!(read(&state, None).await.unwrap()["revision"], second);
        assert_eq!(read(&state, Some("missing".into())).await.unwrap_err().status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn mission_declaration_carries_the_recorded_provenance_when_there_is_one() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "declaration-node");
        let first = publish(&state, "No source");
        assert!(read(&state, Some(first)).await.unwrap().get("provenance").is_none());
        let source = "version 2\nmission \"example/declaration\" state=\"ready\" {\n provenance {\n  reason \"Rendered from TypeScript.\"\n  source {\n   repository \"https://example.invalid/fleet/missions\"\n   commit \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n   path \"missions/declaration.ts\"\n   renderer \"fractal-declarations\"\n  }\n }\n goal \"Sourced goal\"\n}\n";
        let intent = crate::graph::parse_intent(source, "declaration-node").unwrap();
        let preview = state.store.mission(&intent, crate::model::IntentInput {
            kdl: source.into(), source_name: None,
        }).unwrap();
        state.store.apply_as(&intent, &preview.subject_tokens, "sourced", Some("person/operator")).unwrap();
        let sourced = read(&state, None).await.unwrap();
        assert_eq!(sourced["provenance"], json!({
            "repository": "https://example.invalid/fleet/missions",
            "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "path": "missions/declaration.ts",
            "renderer": "fractal-declarations",
        }));
        assert!(!sourced["text"].as_str().unwrap().contains("Rendered from TypeScript."), "{sourced}");
    }

    #[tokio::test]
    async fn mission_declaration_requires_declarations_grant_before_reading() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "declaration-node");
        let error = get(State(state), Extension(ClientSession::for_tests_scoped(
            "client/test", "person/operator", &["read.projections"],
        )), Query(DeclarationQuery { mission: "mission/missing".into(), revision: None })).await.unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert_eq!(error.message, "Fractal does not show this declaration to this device");
    }
}
