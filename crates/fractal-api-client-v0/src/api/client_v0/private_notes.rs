use super::*;
use crate::private_notes::{NotesFence, NotesWrite, WriteContext, operation_key};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteParameters {
    uri: String,
    markdown: String,
}

pub(super) async fn detail(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<ClientSession>,
    AxumPath(uri): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let reader = state.clone();
    let notes = super::super::blocking_action(move || {
        reader.private_notes.read(&reader.node, &uri)
    }).await?;
    let mut actions = Vec::new();
    if session.allows("control.work") && acting_party(&session) {
        actions.push(json!({"id":"private-notes.write", "fence":{"snapshot_id":snapshot.id, "subject_revisions":{}, "private_notes":notes.fence}}));
    }
    Ok(Json(json!({"ref":notes.uri,"family":"private-notes","schema":"fractal.private-notes@1","data":notes,"actions":actions,"live":null})))
}

fn reuse_observation_transport(state: &AppState, input: &mut ClaimInput) -> Result<(), St3Error> {
    let key = input.idempotency_key.as_deref().expect("notes observations have stable keys");
    let existing = smallclaims::store::operation_tx(
        &state.store.readers.get(), &smallclaims::store::operation_id_for_key(key),
    ).map_err(|_| St3Error::new("private-notes-indeterminate", "the notes audit metadata is unavailable"))?;
    let Some((_, claim_id, operation_state)) = existing else { return Ok(()) };
    if operation_state != "active" {
        return Err(St3Error::new("private-notes-indeterminate", "the notes audit operation has conflicting authority"));
    }
    let claim = state.store.claim_by_id(&claim_id).map_err(|_| St3Error::new("private-notes-indeterminate", "the notes audit metadata is unavailable"))?
        .ok_or_else(|| St3Error::new("private-notes-indeterminate", "the original notes audit metadata is unavailable"))?;
    let fields = claim.body.get("fields").and_then(Value::as_object)
        .ok_or_else(|| St3Error::new("private-notes-indeterminate", "the persisted notes observation has no fields"))?;
    if claim.subject != input.subject || claim.kind != input.kind || claim.actor != input.actor
        || fields.len() != input.fields.len()
        || input.fields.iter().any(|(key, value)| key != "transport" && fields.get(key) != Some(value)) {
        return Err(St3Error::new("idempotency-conflict", "the notes key names a different attributed observation"));
    }
    let transport = fields.get("transport")
        .filter(|value| matches!(value.as_str(), Some("unix" | "fabric-loopback")))
        .ok_or_else(|| St3Error::new("private-notes-indeterminate", "the persisted notes observation has no normal transport"))?;
    // The same ordinary paired session may retry through either listener.
    // Preserve the original durable intent's transport, not the recovery hop.
    input.fields.insert("transport".into(), transport.clone());
    Ok(())
}

fn publish_observation(state: &AppState, input: &mut ClaimInput) -> Result<(), St3Error> {
    reuse_observation_transport(state, input)?;
    if let Err(error) = state.store.append_claim(input) {
        if error.code != "idempotency-mismatch" { return Err(error); }
        // Another daemon process may have published this key after our lookup.
        reuse_observation_transport(state, input)?;
        state.store.append_claim(input)?;
    }
    signal_changed(state);
    Ok(())
}

fn reserve_key(state: &AppState, authority: &str, subject: &str, digest: &str) -> Result<(), St3Error> {
    let reservation = ClaimInput {
        subject: subject.into(),
        kind: "client.action-result".into(),
        actor: None,
        fields: BTreeMap::from([
            ("authority_actor".into(), json!(authority)),
            ("request_digest".into(), json!(digest)),
            ("result".into(), json!({"status":"reserved"})),
        ]),
        evidence: Vec::new(),
        expected_subject: Some(None),
        idempotency_key: None,
    };
    match state.store.append_claim(&reservation) {
        Ok(_) => Ok(()),
        Err(error) if error.code == "stale-subject" => {
            let receipt = state.store.observations_for(subject, "client.action-result")
                .map_err(|_| St3Error::new("private-notes-indeterminate", "the shared action key reservation is unavailable"))?
                .into_iter()
                .rfind(|receipt| receipt.origin == state.store.origin() && receipt.actor.is_none());
            if receipt.as_ref().and_then(|receipt| receipt.body.pointer("/fields/request_digest"))
                .and_then(Value::as_str) == Some(digest)
            {
                Ok(())
            } else {
                Err(St3Error::new("idempotency-conflict", "the idempotency key was already used for a different action"))
            }
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn write(state: &AppState, session: &ClientSession, request: &mut ActionRequest, receipt_subject: &str, request_digest: &str) -> Result<Value, ApiError> {
    require_scope(session, "control.work")?;
    if !acting_party(session) {
        return Err(forbidden("client mutations require a concrete person or a local agent"));
    }
    let parameters: WriteParameters = serde_json::from_value(std::mem::take(&mut request.parameters)).map_err(|_| validation("notes write requires only a canonical URI and complete Markdown"))?;
    fractal_schema::private_notes::PrivateNotesUri::parse(&parameters.uri).map_err(|error| validation(error.message))?;
    let fence: NotesFence = request.fence.private_notes.take().ok_or_else(|| validation("notes write requires its carrier generation and revision fence"))?;
    let write = NotesWrite { uri: parameters.uri, markdown: parameters.markdown, fence };
    let operation = operation_key(&session.actor, &request.idempotency_key).map_err(ApiError::bad)?;
    // No supported write can change bytes without a durable attributed intent.
    // A graph failure after local completion remains an error; exact-key retry
    // recovers the private receipt and idempotently publishes completion.
    let audit = ClaimInput {
        subject: format!("resource/{}", crate::declaration_syntax::uri_reference_name(&write.uri)),
        kind: "resource.observed".into(),
        actor: Some(session_claim_actor(session)),
        fields: BTreeMap::from([
            ("kind".into(), json!("uri.reference")),
            // Only the URI is a resource fact; attribution is observation metadata.
            ("facts".into(), json!({"uri":write.uri})),
            ("action".into(), json!("private-notes.write")),
            ("operation_id".into(), json!(format!("operation/private-notes-{}", &operation[..24]))),
            ("stage".into(), json!("requested")),
            ("authority_actor".into(), json!(session.authority_actor)),
            ("session_actor".into(), json!(session.actor)),
            ("transport".into(), json!(session.transport)),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(format!("private-notes:{operation}:requested")),
    };
    let audit = std::cell::RefCell::new(audit);
    let writer = state.clone();
    let actor = session.actor.clone();
    let key = request.idempotency_key.clone();
    let authority = session.authority_actor.clone();
    let receipt_subject = receipt_subject.to_owned();
    let request_digest = request_digest.to_owned();
    let mut result = super::super::blocking_action(move || {
        writer.private_notes.write_with_audit(
            WriteContext { node: &writer.node, state_dir: &writer.state_dir, actor: &actor, key: &key }, &write,
            || {
                // Reserve the shared key before any private mutation. Even an indeterminate
                // completion must exclude other actions until exact-key recovery succeeds.
                reserve_key(&writer, &authority, &receipt_subject, &request_digest)?;
                publish_observation(&writer, &mut audit.borrow_mut())
            },
            || {
                let mut completed = audit.borrow_mut();
                completed.fields.insert("stage".into(), json!("completed"));
                completed.idempotency_key = Some(format!("private-notes:{operation}:completed"));
                publish_observation(&writer, &mut completed)
            },
        )
    }).await?;
    result["action_id"] = request.id.clone().into();
    result["snapshot_id"] = new_client_snapshot(state).id.into();
    Ok(result)
}


#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn state(root: &std::path::Path) -> AppState {
        let catalog = root.join("catalog");
        let subject = catalog.join("agents/example/worker");
        std::fs::create_dir_all(subject.join("resources")).unwrap();
        std::fs::write(subject.join("agent.kdl"), format!(
            "agent \"worker\" {{\n host \"example\"\n resource \"notes\" uri=\"{URI}\"\n}}\n"
        )).unwrap();
        let mut state = super::super::tests::test_state_named(root, "example");
        state.private_notes = Arc::new(crate::private_notes::Authority { catalogs: vec![catalog] });
        state
    }

    const URI: &str = "dev.schickling.agent-private-notes://example/worker";

    async fn read(state: &AppState, session: &ClientSession) -> Value {
        detail(State(state.clone()), Extension(new_client_snapshot(state)),
            Extension(session.clone()), AxumPath(URI.into())).await.unwrap().0
    }

    fn action(notes: &Value, key: &str, markdown: &str) -> ActionRequest {
        serde_json::from_value(json!({
            "api_version":"fractal.client.v0", "id":"action/private-notes-test", "type":"private-notes.write",
            "idempotency_key":key, "fence":notes["actions"][0]["fence"],
            "parameters":{"uri":URI,"markdown":markdown},
        })).unwrap()
    }

    async fn invoke(state: &AppState, session: &ClientSession, request: ActionRequest) -> Result<Value, ApiError> {
        super::super::action(State(state.clone()), Extension(new_client_snapshot(state)),
            Extension(session.clone()), Json(request)).await.map(|value| value.0)
    }

    #[tokio::test]
    async fn ordinary_person_agent_and_paired_client_share_fenced_private_notes_permissions() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let read_only = ClientSession::for_tests_scoped("person/ada", "person/ada", &["read.projections"]);
        assert_eq!(read(&state, &read_only).await["actions"], json!([]));
        for (index, (actor, authority)) in [
            ("person/ada", "person/ada"),
            ("agent/example/worker", "agent/example/worker"),
            ("client/paired-example", "person/ada"),
        ].into_iter().enumerate() {
            let session = ClientSession::for_tests_scoped(actor, authority, &["read.projections", "control.work"]);
            let notes = read(&state, &session).await;
            let request = action(&notes, &format!("notes-write-{index}-00000001"), &format!("Private owner context {index}.\n"));
            assert_eq!(invoke(&state, &read_only, request.clone()).await.unwrap_err().status, StatusCode::FORBIDDEN);
            let written = invoke(&state, &session, request.clone()).await.unwrap();
            assert_eq!(written["affected_ids"], json!([]));
            let retried = invoke(&state, &session, request.clone()).await.unwrap();
            assert_eq!(written["private_notes"], retried["private_notes"]);
            let mut changed = request.clone();
            changed.parameters["markdown"] = json!("different private bytes");
            assert_eq!(super::super::action_request_digest(&request).unwrap(),
                super::super::action_request_digest(&changed).unwrap());
            assert_eq!(invoke(&state, &session, changed).await.unwrap_err().code, "idempotency-conflict");
            let mut stale = request;
            stale.idempotency_key = format!("notes-stale-{index}-00000001");
            stale.parameters["markdown"] = "stale replacement".into();
            assert_eq!(invoke(&state, &session, stale).await.unwrap_err().code, "stale-fence");
            let resource = format!("resource/{}", crate::declaration_syntax::uri_reference_name(URI));
            let claims = state.store.claims_for(&resource, Some("resource.observed")).unwrap();
            let completed = claims.iter().filter(|claim| claim.body["fields"]["stage"] == "completed"
                && claim.body["fields"]["session_actor"] == actor).collect::<Vec<_>>();
            assert_eq!(completed.len(), 1);
            assert_eq!(completed[0].actor.as_deref(), Some(authority));
            let wire = serde_json::to_string(&claims).unwrap();
            for private in ["Private owner context", "stale replacement", "carrier_generation", "revision", "markdown"] {
                assert!(!wire.contains(private), "private audit field: {private}");
            }
        }
    }

    #[tokio::test]
    async fn notes_and_other_actions_share_idempotency_conflicts_without_private_receipts() {
        for notes_first in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let state = state(root.path());
            let session = ClientSession::local(Some("person/ada")).unwrap();
            let notes = read(&state, &session).await;
            let request = action(&notes, "notes-cross-action-key-0001", "Private shared-key context.\n");
            let send = ActionRequest {
                api_version: CLIENT_API_VERSION.into(),
                id: "action/notes-cross-action-send".into(),
                action_type: "message.send".into(),
                idempotency_key: request.idempotency_key.clone(),
                fence: Fence { snapshot_id: new_client_snapshot(&state).id, ..Default::default() },
                parameters: json!({"to":"agent/worker","content":"ordinary message"}),
            };
            let (first, second) = if notes_first { (request.clone(), send) } else { (send, request) };
            invoke(&state, &session, first).await.unwrap();
            let index = state.store.index().unwrap();
            let before = read(&state, &session).await["data"].clone();
            let error = invoke(&state, &session, second).await.unwrap_err();
            assert_eq!(error.code, "idempotency-conflict");
            assert_eq!(error.status, StatusCode::CONFLICT);
            assert_eq!(state.store.index().unwrap(), index);
            assert_eq!(read(&state, &session).await["data"], before);
            let receipt_digest = hex::encode(Sha256::digest(
                format!("{}:notes-cross-action-key-0001", session.actor).as_bytes(),
            ));
            let receipt_subject = format!("custom/client/action-{}", &receipt_digest[..32]);
            let receipts = state.store.observations_for(&receipt_subject, "client.action-result").unwrap();
            assert_eq!(receipts.len(), 1);
            if notes_first {
                let wire = serde_json::to_string(&receipts).unwrap();
                for private in ["Private shared-key context", "private_notes", "carrier_generation", "revision", "markdown"] {
                    assert!(!wire.contains(private), "private receipt field: {private}");
                }
            }
        }
    }

    #[tokio::test]
    async fn audit_failure_requires_exact_recovery_with_original_transport() {
        use std::os::unix::fs::MetadataExt as _;
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let mut session = ClientSession::for_tests_scoped("client/paired-example", "person/ada", &["read.projections", "control.work", "control.messages"]);
        let notes = read(&state, &session).await;
        let request = action(&notes, "notes-audit-recovery-0001", "Private durable context.\n");
        let graph = rusqlite::Connection::open(root.path().join("graph.db")).unwrap();
        graph.execute_batch("CREATE TRIGGER refuse_notes_audit BEFORE INSERT ON claims
            WHEN NEW.kind='resource.observed' AND json_extract(NEW.body,'$.fields.stage')='requested'
            BEGIN SELECT RAISE(FAIL, 'requested audit refusal'); END;").unwrap();
        assert!(invoke(&state, &session, request.clone()).await.is_err());
        let carrier = root.path().join("catalog/agents/example/worker/resources/private-notes.md");
        assert!(!carrier.exists());
        graph.execute_batch("DROP TRIGGER refuse_notes_audit;
            CREATE TRIGGER refuse_notes_audit BEFORE INSERT ON claims
            WHEN NEW.kind='resource.observed' AND json_extract(NEW.body,'$.fields.stage')='completed'
            BEGIN SELECT RAISE(FAIL, 'completed audit refusal'); END;").unwrap();
        assert_eq!(invoke(&state, &session, request.clone()).await.unwrap_err().code, "private-notes-indeterminate");
        assert_eq!(std::fs::read_to_string(&carrier).unwrap(), "Private durable context.\n");
        let conflicting = ActionRequest {
            api_version: CLIENT_API_VERSION.into(),
            id: "action/pending-notes-cross-action".into(),
            action_type: "message.send".into(),
            idempotency_key: request.idempotency_key.clone(),
            fence: Fence { snapshot_id: new_client_snapshot(&state).id, ..Default::default() },
            parameters: json!({"to":"agent/worker","content":"must not steal a pending notes key"}),
        };
        let index = state.store.index().unwrap();
        assert_eq!(invoke(&state, &session, conflicting).await.unwrap_err().code, "idempotency-conflict");
        assert_eq!(state.store.index().unwrap(), index);
        let before = std::fs::metadata(&carrier).unwrap();
        let newer = read(&state, &session).await;
        let other = action(&newer, "notes-other-operation-0001", "must not bypass pending completion");
        assert_eq!(invoke(&state, &session, other).await.unwrap_err().code, "private-notes-indeterminate");
        graph.execute_batch("DROP TRIGGER refuse_notes_audit;").unwrap();
        session.transport = "fabric-loopback";
        let recovered = invoke(&state, &session, request.clone()).await.unwrap();
        let repeated = invoke(&state, &session, request).await.unwrap();
        assert_eq!(recovered["private_notes"], repeated["private_notes"]);
        let after = std::fs::metadata(&carrier).unwrap();
        assert_eq!((before.ino(), before.mtime(), before.mtime_nsec()), (after.ino(), after.mtime(), after.mtime_nsec()));
        let resource = format!("resource/{}", crate::declaration_syntax::uri_reference_name(URI));
        let claims = state.store.claims_for(&resource, Some("resource.observed")).unwrap();
        assert_eq!(claims.len(), 2);
        for claim in claims {
            assert_eq!(claim.body["fields"]["transport"], "unix");
            assert_eq!(claim.body["fields"]["session_actor"], session.actor);
            assert_eq!(claim.actor.as_deref(), Some("person/ada"));
        }
    }
}
