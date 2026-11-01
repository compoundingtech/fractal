//! Metadata-only reads and person-only native answer settlement.
use super::*;
use crate::HarnessQuestionsStore;
pub use crate::store::harness_questions::HarnessQuestion;
use fractal_schema::harness_questions::{Answer, Receipt, Status, OBSERVED, MAX_QUESTIONS, MAX_OPTIONS};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum InvalidAnswerReason {
    UnknownOption, TooMany, RequiredMissing, EmptySingle, TooLong, UnknownQuestion,
    OtherNotAllowed, NoteNotAllowed,
}
fn invalid(reason: InvalidAnswerReason) -> ApiError {
    ApiError { status: StatusCode::UNPROCESSABLE_ENTITY, code: "invalid-answer".into(),
        message: "the answer does not match the native question".into(),
        details: Box::new(serde_json::Map::from_iter([("reason".into(), json!(reason))])) }
}
fn unavailable(error: HarnessQuestionAdapterError) -> ApiError {
    ApiError { status: StatusCode::CONFLICT, code: "not-answerable".into(),
        message: "the native question adapter is unavailable".into(),
        details: Box::new(serde_json::Map::from_iter([("reason".into(), json!(match error {
            HarnessQuestionAdapterError::Missing => "adapter_missing",
            HarnessQuestionAdapterError::Unavailable => "adapter_unavailable",
        }))])) }
}

fn resolution(question: &HarnessQuestion) -> Value {
    question.resolution.as_ref().map_or(Value::Null, |resolved| json!({
        "status": resolved.status, "revision": resolved.revision,
        "answers": resolved.answers, "resolved_at": resolved.resolved_at_unix_ms,
    }))
}
fn metadata(state: &AppState, question: &HarnessQuestion) -> Value {
    let capability = if question.resolution.is_some() {
        json!({"state":"unavailable","reason":"resolved"})
    } else {
        match state.harness_question_sink.as_ref().map_or(Err(HarnessQuestionAdapterError::Missing),
            |sink| sink.availability(question)) {
            Ok(()) => json!({"state":"available"}),
            Err(error) => json!({"state":"unavailable","reason":match error {
                HarnessQuestionAdapterError::Missing => "adapter_missing",
                HarnessQuestionAdapterError::Unavailable => "adapter_unavailable",
            }}),
        }
    };
    json!({"kind":"harness-question","id":question.id,"revision":question.revision,
        "agent":question.agent,"session":question.session,"native_id":question.native_id,
        "runtime_incarnation":question.runtime_incarnation,"questions":question.questions,
        "updated_at":client_timestamp(question.resolution.as_ref().map_or(question.observed_at_unix_ms, |resolved| resolved.resolved_at_unix_ms)),
        "observed_at":question.observed_at_unix_ms,"resolution":resolution(question),"answer":capability})
}
fn held(state: &AppState, id: &str) -> Result<HarnessQuestion, ApiError> {
    state.store.harness_question(id, client_now_ms()).map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("st holds no such harness question"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    limit: Option<usize>,
    cursor: Option<String>,
}

pub async fn list(State(state): State<AppState>, Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<ClientSession>, Query(query): Query<ListQuery>)
    -> Result<ClientPageResponse, ApiError> {
    require_scope(&session, "read.projections")?;
    let query = ClientListQuery { limit: query.limit, cursor: query.cursor, ..ClientListQuery::default() };
    let (snapshot, page) = tokio::task::spawn_blocking(move || {
        let mut items = Vec::new();
        if query.cursor.is_none() {
            let status = state.store.status_for_claim_kind_at(
                OBSERVED, None, false, crate::store::ClaimsReduction::Runtime,
            ).map_err(ApiError::internal)?;
            let now = client_now_ms();
            for subject in status.subjects {
                if let Some(question) = state.store.harness_question(&subject.subject, now).map_err(ApiError::internal)? {
                    items.push(metadata(&state, &question));
                }
            }
        }
        client_page(&state, &snapshot, "harness-questions", items, &query)
            .map(|page| (snapshot, page))
    }).await.map_err(ApiError::internal)??;
    Ok((Extension(snapshot), Json(page)))
}
pub async fn get(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let value = tokio::task::spawn_blocking(move || {
        held(&state, &client_detail_id("harness-question", &id))
            .map(|question| metadata(&state, &question))
    }).await.map_err(ApiError::internal)??;
    Ok(Json(value))
}

fn validate_answers(parameters: &Value, question: &HarnessQuestion) -> Result<BTreeMap<String, Answer>, ApiError> {
    let envelope = parameters.as_object().ok_or_else(|| validation("answer parameters must be an object"))?;
    if envelope.contains_key("other") { return Err(invalid(InvalidAnswerReason::OtherNotAllowed)); }
    if envelope.contains_key("note") { return Err(invalid(InvalidAnswerReason::NoteNotAllowed)); }
    if envelope.keys().any(|key| !matches!(key.as_str(), "question" | "answers")) {
        return Err(validation("answer parameters contain an unknown field"));
    }
    let answers = parameters.get("answers").and_then(Value::as_object)
        .ok_or_else(|| validation("answers must be an object keyed by question id"))?;
    if answers.len() > MAX_QUESTIONS { return Err(invalid(InvalidAnswerReason::TooLong)); }
    let mut result = BTreeMap::new();
    for (id, value) in answers {
        let fields = value.as_object().ok_or_else(|| validation("each answer must be an object"))?;
        if fields.contains_key("other") { return Err(invalid(InvalidAnswerReason::OtherNotAllowed)); }
        if fields.contains_key("note") { return Err(invalid(InvalidAnswerReason::NoteNotAllowed)); }
        if fields.keys().any(|key| key != "positions") { return Err(validation("each answer contains only positions")); }
        let positions: Vec<u32> = serde_json::from_value(fields.get("positions").cloned().unwrap_or(Value::Null))
            .map_err(|_| validation("positions must be an array of unsigned integers"))?;
        let definition = question.questions.iter().find(|item| item.id == *id)
            .ok_or_else(|| invalid(InvalidAnswerReason::UnknownQuestion))?;
        if positions.len() > usize::from(MAX_OPTIONS) { return Err(invalid(InvalidAnswerReason::TooLong)); }
        if positions.iter().any(|position| *position >= u32::from(definition.options)) {
            return Err(invalid(InvalidAnswerReason::UnknownOption));
        }
        if positions.iter().collect::<BTreeSet<_>>().len() != positions.len() || !definition.multi && positions.len() > 1 {
            return Err(invalid(InvalidAnswerReason::TooMany));
        }
        if !definition.multi && positions.is_empty() { return Err(invalid(InvalidAnswerReason::EmptySingle)); }
        if definition.required && positions.is_empty() { return Err(invalid(InvalidAnswerReason::RequiredMissing)); }
        result.insert(id.clone(), Answer { positions: positions.into_iter().map(|position| u8::try_from(position).expect("bounded option")).collect(), timed_out: false });
    }
    if question.questions.iter().any(|item| item.required && !result.contains_key(&item.id)) {
        return Err(invalid(InvalidAnswerReason::RequiredMissing));
    }
    Ok(result)
}
fn result(state: &AppState, request: &ActionRequest, question: &HarnessQuestion, status: &str, digest: &str) -> Json<Value> {
    Json(json!({"kind":"action-result","action_id":request.id,
        "operation_id":format!("operation/client-{}", &digest[..24]),
        "status":"completed","affected_ids":[question.id],"snapshot_id":new_client_snapshot(state).id,
        "harness_answer":{"status":status,"resolution":resolution(question)}}))
}
pub(super) async fn answer(state: &AppState, session: &ClientSession, request: &ActionRequest) -> Result<Json<Value>, ApiError> {
    if session.actor.starts_with("agent/") || !session.authority_actor.starts_with("person/") || session.authority_actor.matches('/').count() != 1 {
        return Err(forbidden("native harness answers require a concrete person; agents cannot answer"));
    }
    validate_action_admission(session, request)?;
    let gate = action_gate(state, session, &request.idempotency_key);
    let _guard = gate.lock_owned().await;
    let mut provenance = request.fence.clone();
    provenance.subject_revisions.clear();
    validate_fence(state, &provenance)?;
    let digest = action_request_digest(request)?;
    let actor_key = fractal_schema::harness_questions::actor_key(&session.actor, &request.idempotency_key);
    retained_action_receipt(state, session, request, &digest, &actor_key)?;
    let id = parameter_string(&request.parameters, "question")?;
    // Serialize different people and different keys settling the same native call as well.
    let mut owner = session.clone();
    owner.actor = "harness-question-settlement".into();
    let question_gate = action_gate(state, &owner, &id);
    let _question_guard = question_gate.lock_owned().await;
    let question = held(state, &id)?;
    live_input_session(state, &question.agent, &question.runtime_incarnation).map_err(|error| {
        if error.code == "stale-incarnation" { stale(error.message) } else { error }
    })?;
    if trusted_harness_receipt(state, &question)
        .is_some_and(|receipt| receipt.actor_key == actor_key) {
        return Ok(result(state, request, &question, "accepted", &digest));
    }
    if request.fence.runtime_incarnation.as_deref() != Some(question.runtime_incarnation.as_str()) {
        return Err(stale("the question runtime incarnation fence is stale"));
    }
    if request.fence.subject_revisions.get(&id) != Some(&question.revision) {
        return Err(stale("the question content revision fence is stale"));
    }
    validate_fence(state, &request.fence)?;
    if question.resolution.is_some() { return Ok(result(state, request, &question, "already_resolved", &digest)); }
    let sink = state.harness_question_sink.as_ref().ok_or_else(|| unavailable(HarnessQuestionAdapterError::Missing))?;
    let sink = Arc::clone(sink);
    let (question, sink) = tokio::task::spawn_blocking(move || {
        sink.availability(&question)?;
        Ok::<_, HarnessQuestionAdapterError>((question, sink))
    }).await.map_err(ApiError::internal)?.map_err(unavailable)?;
    let answers = validate_answers(&request.parameters, &question)?;
    // A native adapter may block on its harness. Both gates remain held while it settles;
    // move the question through the worker instead of copying its metadata.
    let (question, answers) = tokio::task::spawn_blocking(move || {
        let answers = sink.settle(&question, &answers)?;
        Ok::<_, HarnessQuestionAdapterError>((question, answers))
    }).await.map_err(ApiError::internal)?.map_err(unavailable)?;
    let receipt = Receipt { actor_key,
        request_digest: digest, action_id: request.id.clone() };
    let (resolved, written) = state.store.resolve_harness_question(&id, Status::Answered, &question.revision, &answers, Some(&receipt)).map_err(ApiError::bad)?;
    if written { signal_changed(state); }
    Ok(result(state, request, &resolved, if written { "accepted" } else { "already_resolved" }, &receipt.request_digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::harness_questions::QuestionObservation;
    use fractal_schema::harness_questions::Question;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn harness_answer_requires_its_explicit_scope_and_unknown_harness_actions_stay_unavailable() {
        let (_root, mut state, _session, _question, mut request) = fixture();
        state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false }));
        assert_eq!(action_scope("harness.answer"), Some("terminal.control"));
        assert_eq!(action_scope("harness.unknown"), None);
        let reader = ClientSession::for_tests_scoped("person/alex", "person/alex", &["read.projections"]);
        assert_eq!(invoke(&state, &reader, &request).await.unwrap_err().code, "forbidden");
        let granted = ClientSession::for_tests_scoped("person/alex", "person/alex", &["terminal.control"]);
        let advertised = capabilities(&state, &granted);
        assert!(advertised.iter().any(|item| item == &json!({"id":"harness.answer","version":0,"state":"granted"})));
        assert!(!advertised.iter().any(|item| item["id"] == "harness.unknown"));
        request.action_type = "harness.unknown".into();
        assert_eq!(invoke(&state, &granted, &request).await.unwrap_err().code, "validation-failed");
    }

    #[tokio::test]
    async fn an_agent_with_terminal_control_cannot_answer_as_a_person() {
        let (_root, state, _session, _question, request) = fixture();
        let agent = ClientSession::for_tests_scoped("agent/ada", "person/alex", &["terminal.control"]);
        let before = state.store.index().unwrap();
        assert_eq!(invoke(&state, &agent, &request).await.unwrap_err().code, "forbidden");
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[test]
    fn harness_question_refusals_keep_their_typed_code_and_reason_in_client_envelopes() {
        for (error, code, reason) in [
            (unavailable(HarnessQuestionAdapterError::Missing), "not-answerable", "adapter_missing"),
            (invalid(InvalidAnswerReason::UnknownOption), "invalid-answer", "unknown_option"),
        ] {
            let raw = json!({"code": error.code, "message": error.message, "details": error.details});
            let envelope = client_error_envelope(error.status, &raw, "request/tester");
            assert_eq!(envelope["code"], code);
            assert_eq!(envelope["details"]["reason"], reason);
            assert_eq!(envelope["retryable"], false);
        }
    }

    struct Sink { calls: AtomicUsize, timed_out: bool }
    impl HarnessQuestionSettleSink for Sink {
        fn availability(&self, _: &HarnessQuestion) -> Result<(), HarnessQuestionAdapterError> { Ok(()) }
        fn settle(&self, _: &HarnessQuestion, answers: &BTreeMap<String, Answer>) -> Result<BTreeMap<String, Answer>, HarnessQuestionAdapterError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut answers = answers.clone();
            for answer in answers.values_mut() { answer.timed_out = self.timed_out; }
            Ok(answers)
        }
    }
    fn fixture() -> (tempfile::TempDir, AppState, ClientSession, HarnessQuestion, ActionRequest) {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "q0");
        state.store.append_claim(&ClaimInput {
            subject: "agent/ada".into(), kind: "runtime.observed".into(), actor: Some("agent/ada".into()),
            fields: serde_json::from_value(json!({"status":"running","runtime_id":"q0","incarnation_id":"q0","terminal":true})).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let question = state.store.observe_harness_question(&QuestionObservation {
            agent:"agent/ada".into(), session:"q0".into(), native_id:"q0".into(), runtime_incarnation:"q0".into(),
            questions:vec![Question { id:"q0".into(), options:2, multi:false, required:true }],
        }).unwrap();
        let session = ClientSession::for_tests_scoped("person/alex", "person/alex", &["terminal.control", "read.projections"]);
        let request = ActionRequest { api_version:CLIENT_API_VERSION.into(), id:"action/tester".into(), action_type:"harness.answer".into(),
            idempotency_key:"tester-operator-key".into(), fence:Fence { snapshot_id:new_client_snapshot(&state).id,
                subject_revisions:BTreeMap::from([(question.id.clone(), question.revision.clone())]),
                runtime_incarnation:Some("q0".into()), ..Fence::default() },
            parameters:json!({"question":question.id,"answers":{"q0":{"positions":[1]}}}) };
        (root, state, session, question, request)
    }
    async fn invoke(state: &AppState, session: &ClientSession, request: &ActionRequest) -> Result<Json<Value>, ApiError> {
        action_inner(State(state.clone()), Extension(new_client_snapshot(state)), Extension(session.clone()), Json(request.clone()), None).await
    }
    #[tokio::test]
    async fn oversized_receipt_action_ids_refuse_before_settlement() {
        let (_root, mut state, session, question, mut request) = fixture();
        let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
        state.harness_question_sink = Some(sink.clone());
        let before = state.store.index().unwrap();
        for suffix in ["a".repeat(250), "é".repeat(125)] {
            request.id = format!("action/{suffix}");
            assert_eq!(request.id.len(), 257);
            assert_eq!(invoke(&state, &session, &request).await.unwrap_err().code, "validation-failed");
            assert_eq!(state.store.index().unwrap(), before);
            assert!(held(&state, &question.id).unwrap().resolution.is_none());
            assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
        }
        request.id = format!("action/{}", "a".repeat(249));
        assert_eq!(request.id.len(), 256);
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        let accepted = state.store.index().unwrap();
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        assert_eq!(state.store.index().unwrap(), accepted);
        assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn answer_map_keys_are_data_but_nested_identity_fields_are_forbidden() {
        let (_root, mut state, session, _question, mut request) = fixture();
        let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
        state.harness_question_sink = Some(sink.clone());
        let before = state.store.index().unwrap();
        for key in ["actor", "credential", "fleet_secret", "shared_secret"] {
            request.parameters["answers"] = json!({key:{"positions":[1]}});
            assert_eq!(invoke(&state, &session, &request).await.unwrap_err().details["reason"], "unknown_question");
            let mut invalid = request.clone();
            invalid.parameters[key] = json!("person/alex");
            assert_eq!(invoke(&state, &session, &invalid).await.unwrap_err().code, "validation-failed");
            let mut invalid = request.clone();
            invalid.parameters["answers"][key][key] = json!("person/alex");
            assert_eq!(invoke(&state, &session, &invalid).await.unwrap_err().code, "validation-failed");
        }
        assert_eq!(state.store.index().unwrap(), before);
        assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
        request.parameters["answers"] = json!({"q0":{"positions":[1]}});
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
    }

    #[tokio::test]
    async fn harness_and_private_notes_share_action_keys_in_both_orders() {
        for harness_first in [false, true] {
            let (root, mut state, _session, _question, request) = fixture();
            let session = ClientSession::for_tests_scoped("person/alex", "person/alex",
                &["terminal.control", "read.projections", "control.work"]);
            let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
            state.harness_question_sink = Some(sink.clone());
            let catalog = root.path().join("catalog");
            let subject = catalog.join("agents/q0/ada");
            std::fs::create_dir_all(subject.join("resources")).unwrap();
            let uri = "dev.schickling.agent-private-notes://q0/ada";
            std::fs::write(subject.join("agent.kdl"), format!(
                "agent \"ada\" {{\n host \"q0\"\n resource \"notes\" uri=\"{uri}\"\n}}\n"
            )).unwrap();
            state.private_notes = Arc::new(crate::private_notes::Authority { catalogs:vec![catalog] });
            let notes = super::super::private_notes::detail(
                State(state.clone()), Extension(new_client_snapshot(&state)),
                Extension(session.clone()), AxumPath(uri.into()),
            ).await.unwrap().0;
            let notes_request: ActionRequest = serde_json::from_value(json!({
                "api_version":CLIENT_API_VERSION, "id":"action/notes-test",
                "type":"private-notes.write", "idempotency_key":request.idempotency_key,
                "fence":notes["actions"][0]["fence"],
                "parameters":{"uri":uri,"markdown":"Owner-local context.\n"},
            })).unwrap();
            let (first, second) = if harness_first {
                (&request, &notes_request)
            } else {
                (&notes_request, &request)
            };
            let _ = invoke(&state, &session, first).await.unwrap();
            let before = state.store.index().unwrap();
            assert_eq!(invoke(&state, &session, second).await.unwrap_err().code,
                "idempotency-conflict");
            assert_eq!(state.store.index().unwrap(), before);
            assert_eq!(sink.calls.load(Ordering::SeqCst), usize::from(harness_first));
            // The losing family cannot poison exact-key recovery in the winning family.
            let _ = invoke(&state, &session, first).await.unwrap();
            assert_eq!(state.store.index().unwrap(), before);
            assert_eq!(sink.calls.load(Ordering::SeqCst), usize::from(harness_first));
        }
    }

    #[tokio::test]
    async fn legacy_action_receipts_refuse_harness_answers_before_settlement() {
        for actor in [None, Some("person/alex".to_owned())] {
            let (_root, mut state, session, _question, request) = fixture();
            let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
            state.harness_question_sink = Some(sink.clone());
            let digest = hex::encode(Sha256::digest(
                format!("{}:{}", session.actor, request.idempotency_key).as_bytes()));
            state.store.append_claim(&ClaimInput {
                subject: format!("custom/client/action-{}", &digest[..32]),
                kind: "custom.client.action-result".into(),
                actor,
                fields: BTreeMap::from([
                    ("request_digest".into(), json!(action_request_digest(&request).unwrap())),
                    ("result".into(), json!({"status":"completed"})),
                ]),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
            let before = state.store.index().unwrap();
            assert_eq!(invoke(&state, &session, &request).await.unwrap_err().code,
                "legacy-action-receipt-untrusted");
            assert_eq!(state.store.index().unwrap(), before);
            assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn peer_harness_receipts_neither_replay_nor_reserve_a_local_action_key() {
        let (_root, mut state, _session, question, request) = fixture();
        let session = ClientSession::for_tests_scoped("person/alex", "person/alex",
            &["terminal.control", "read.projections", "control.arrangements"]);
        let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
        state.harness_question_sink = Some(sink.clone());
        let peer = Store::open_memory("peer").unwrap();
        const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
        peer.bind_fleet(FLEET).unwrap();
        state.store.bind_fleet(FLEET).unwrap();
        peer.append_claim(&ClaimInput {
            subject: "host/peer".into(), kind: "transport.observed".into(), actor: None,
            fields: BTreeMap::from([("status".into(), json!("up"))]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        peer.append_claim(&ClaimInput {
            subject: question.id.clone(), kind: fractal_schema::harness_questions::RESOLVED.into(),
            actor: None,
            fields: BTreeMap::from([
                ("status".into(), json!("answered")),
                ("revision".into(), json!(question.revision)),
                ("answers".into(), json!({"q0":{"positions":[1],"timed_out":false}})),
                ("receipt".into(), json!(Receipt {
                    actor_key: fractal_schema::harness_questions::actor_key(&session.actor, &request.idempotency_key),
                    request_digest: action_request_digest(&request).unwrap(),
                    action_id: request.id.clone(),
                })),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let exchange = peer.export_replication_exchange(FLEET, &state.store.replication_inventory().unwrap()).unwrap();
        state.store.receive_replication_exchange("peer", FLEET, &exchange).unwrap();
        assert!(state.store.validate_replication_backlog().unwrap().valid > 0);
        state.store.project_replication_backlog().unwrap();
        let question = held(&state, &question.id).unwrap();
        assert_eq!(question.resolution.as_ref().unwrap().origin, "peer");
        assert!(trusted_harness_receipt(&state, &question).is_none());
        let before = state.store.index().unwrap();
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "already_resolved");
        assert_eq!(state.store.index().unwrap(), before);
        assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
        let generic = ActionRequest {
            api_version: CLIENT_API_VERSION.into(), id: "action/alex".into(),
            action_type: "arrangement.edit".into(), idempotency_key: request.idempotency_key,
            fence: Fence { snapshot_id: new_client_snapshot(&state).id, ..Fence::default() },
            parameters: json!({"subject":"arrangement/person/alex/019a0000-0000-7000-8000-000000000001",
                "owner":"person/alex","operations":[{"op":"create","name":"Sidebar"}]}),
        };
        assert_eq!(invoke(&state, &session, &generic).await.unwrap().0["status"], "completed");
    }

    #[test]
    fn harness_receipt_provenance_requires_local_origin_and_no_actor() {
        let (_root, state, _session, mut question, request) = fixture();
        state.store.resolve_harness_question(&question.id, Status::Answered, &question.revision,
            &BTreeMap::new(), Some(&Receipt {
                actor_key: "a".repeat(64), request_digest: "b".repeat(64), action_id: request.id,
            })).unwrap();
        question = held(&state, &question.id).unwrap();
        assert!(trusted_harness_receipt(&state, &question).is_some());
        question.resolution.as_mut().unwrap().actor = Some("person/alex".into());
        assert!(trusted_harness_receipt(&state, &question).is_none());
        question.resolution.as_mut().unwrap().actor = None;
        question.resolution.as_mut().unwrap().origin = "peer".into();
        assert!(trusted_harness_receipt(&state, &question).is_none());
    }

    #[tokio::test]
    async fn retained_keys_conflict_across_generic_and_harness_receipts_in_both_orders() {
        for harness_first in [false, true] {
            let (_root, mut state, _session, _question, request) = fixture();
            let session = ClientSession::for_tests_scoped("person/alex", "person/alex",
                &["terminal.control", "read.projections", "control.arrangements"]);
            let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
            state.harness_question_sink = Some(sink.clone());
            let arrangement = "arrangement/person/alex/019a0000-0000-7000-8000-000000000001";
            let generic = ActionRequest {
                api_version:CLIENT_API_VERSION.into(), id:"action/operator".into(),
                action_type:"arrangement.edit".into(), idempotency_key:request.idempotency_key.clone(),
                fence:Fence { snapshot_id:new_client_snapshot(&state).id, ..Fence::default() },
                parameters:json!({"subject":arrangement,"owner":"person/alex","operations":[{"op":"create","name":"Sidebar"}]}),
            };
            let (first, second) = if harness_first { (&request, &generic) } else { (&generic, &request) };
            let accepted = invoke(&state, &session, first).await.unwrap().0;
            let before = state.store.index().unwrap();
            assert_eq!(invoke(&state, &session, second).await.unwrap_err().code, "idempotency-conflict");
            assert_eq!(state.store.index().unwrap(), before);
            assert_eq!(sink.calls.load(Ordering::SeqCst), usize::from(harness_first));
            assert_eq!(invoke(&state, &session, first).await.unwrap().0["operation_id"], accepted["operation_id"]);
            assert_eq!(state.store.index().unwrap(), before);
            assert_eq!(sink.calls.load(Ordering::SeqCst), usize::from(harness_first));
            assert_eq!(held(&state, request.parameters["question"].as_str().unwrap()).unwrap().resolution.is_some(), harness_first);
            if harness_first {
                assert!(state.store.claims_for(arrangement, Some("arrangement.edited")).unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn harness_answer_without_adapter_is_unavailable_and_writes_nothing() {
        let (_root, state, session, question, request) = fixture();
        let before = state.store.index().unwrap();
        let error = invoke(&state, &session, &request).await.unwrap_err();
        assert_eq!(error.code, "not-answerable");
        assert_eq!(error.details["reason"], "adapter_missing");
        assert_eq!(state.store.index().unwrap(), before);
        assert!(held(&state, &question.id).unwrap().resolution.is_none());
        assert_eq!(metadata(&state, &question)["answer"], json!({"state":"unavailable","reason":"adapter_missing"}));
    }
    #[tokio::test]
    async fn every_agent_is_refused_even_unix_free_mode() {
        let (_root, state, _session, _question, request) = fixture();
        for transport in ["unix", "fabric-loopback"] {
            let mut agent = ClientSession::for_tests("agent/ada", "agent/ada", transport);
            agent.scopes.insert("terminal.control".into());
            let before = state.store.index().unwrap();
            assert_eq!(invoke(&state, &agent, &request).await.unwrap_err().code, "forbidden");
            assert_eq!(state.store.index().unwrap(), before);
        }
    }
    #[tokio::test]
    async fn accepted_receipts_survive_reopen_without_an_adapter_or_duplicate_writes() {
        let (root, mut state, session, question, request) = fixture();
        state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false }));
        let accepted = invoke(&state, &session, &request).await.unwrap().0;
        let before = state.store.index().unwrap();
        drop(state);
        let reopened = super::super::tests::test_state_named(root.path(), "q0");
        let replay = invoke(&reopened, &session, &request).await.unwrap().0;
        assert_eq!(replay["operation_id"], accepted["operation_id"]);
        assert_eq!(replay["harness_answer"], accepted["harness_answer"]);
        assert_eq!(reopened.store.index().unwrap(), before);
        let mut new_key = request.clone();
        new_key.idempotency_key = "alex-new-answer-key".into();
        new_key.parameters["answers"] = json!({"q0":{"positions":[999]}});
        assert_eq!(invoke(&reopened, &session, &new_key).await.unwrap().0["harness_answer"]["status"], "already_resolved");
        assert!(held(&reopened, &question.id).unwrap().resolution.is_some());
        assert_eq!(reopened.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn metadata_pages_preserve_snapshot_cursors_and_require_projection_scope() {
        let (_root, state, session, question, _request) = fixture();
        state.store.observe_harness_question(&QuestionObservation {
            agent:"agent/ada".into(), session:"q0".into(), native_id:"second-call".into(),
            runtime_incarnation:"q0".into(),
            questions:vec![Question { id:"q0".into(), options:2, multi:false, required:true }],
        }).unwrap();
        let snapshot = new_client_snapshot(&state);
        let denied = ClientSession::for_tests_scoped("person/alex", "person/alex", &[]);
        assert_eq!(list(State(state.clone()), Extension(snapshot.clone()), Extension(denied),
            Query(ListQuery { limit:Some(1), cursor:None })).await.unwrap_err().code, "forbidden");
        let (_, Json(first)) = list(State(state.clone()), Extension(snapshot.clone()), Extension(session.clone()),
            Query(ListQuery { limit:Some(1), cursor:None })).await.unwrap();
        assert_eq!(first.collection, "harness-questions");
        assert_eq!(first.items.len(), 1);
        assert!(first.page.has_more);
        let (_, Json(second)) = list(State(state.clone()), Extension(snapshot), Extension(session),
            Query(ListQuery { limit:Some(1), cursor:first.page.next_cursor })).await.unwrap();
        assert_eq!(second.items.len(), 1);
        assert!(!second.page.has_more);
        assert_ne!(first.items[0]["id"], second.items[0]["id"]);
        assert!(first.items.iter().chain(&second.items).any(|item| item["id"] == question.id));
        assert!(serde_json::from_value::<ListQuery>(json!({"status":"pending"})).is_err());
    }

    #[tokio::test]
    async fn receipt_replays_but_changed_action_id_or_parameters_conflict() {
        let (_root, mut state, session, question, request) = fixture();
        let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
        state.harness_question_sink = Some(sink.clone());
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        let before = state.store.index().unwrap();
        let mut retry = request.clone();
        retry.fence.subject_revisions.insert(question.id.clone(), "stale".into());
        assert_eq!(invoke(&state, &session, &retry).await.unwrap().0["harness_answer"]["status"], "accepted");
        let mut changed_id = request.clone();
        changed_id.id = "action/operator".into();
        assert_eq!(invoke(&state, &session, &changed_id).await.unwrap_err().code, "idempotency-conflict");
        let mut changed = request.clone();
        changed.parameters["answers"]["q0"]["positions"] = json!([0]);
        assert_eq!(invoke(&state, &session, &changed).await.unwrap_err().code, "idempotency-conflict");
        assert_eq!(state.store.index().unwrap(), before);
        assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn harness_answer_authority_fences_replays_and_resolution_order() {
        let (_root, mut state, session, _question, request) = fixture();
        state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false }));
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        let mut retry = request.clone();
        retry.fence.snapshot_id = "snapshot/operator/999999999/future".into();
        assert_eq!(invoke(&state, &session, &retry).await.unwrap_err().code, "stale-fence");
        state.store.append_claim(&ClaimInput {
            subject:"agent/ada".into(), kind:"runtime.observed".into(), actor:Some("agent/ada".into()),
            fields:serde_json::from_value(json!({"status":"running","runtime_id":"q0","incarnation_id":"operator","terminal":true})).unwrap(),
            evidence:Vec::new(), expected_subject:None, idempotency_key:None,
        }).unwrap();
        assert_eq!(invoke(&state, &session, &request).await.unwrap_err().code, "stale-fence");
    }
    #[tokio::test]
    async fn new_key_after_resolution_precedes_capability_and_answer_validation() {
        let (_root, mut state, session, question, request) = fixture();
        state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:true }));
        let accepted = invoke(&state, &session, &request).await.unwrap().0;
        assert_eq!(accepted["harness_answer"]["resolution"]["answers"]["q0"]["timed_out"], true);
        state.harness_question_sink = None;
        let mut next = request.clone();
        next.idempotency_key = "operator-reviewer-key".into();
        next.parameters["answers"] = json!({"unknown":{"note":"forbidden"}});
        let before = state.store.index().unwrap();
        assert_eq!(invoke(&state, &session, &next).await.unwrap().0["harness_answer"]["status"], "already_resolved");
        assert_eq!(state.store.index().unwrap(), before);
        let public = metadata(&state, &held(&state, &question.id).unwrap());
        assert_eq!(public["resolution"]["answers"]["q0"]["timed_out"], true);
        assert!(public["resolution"].get("receipt").is_none());
        assert_eq!(public["revision"], question.revision);
        validate_fence(&state, &request.fence).unwrap();
    }
    #[tokio::test]
    async fn stale_revision_and_incarnation_refuse_before_settlement() {
        let (_root, mut state, session, question, request) = fixture();
        let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
        state.harness_question_sink = Some(sink.clone());
        let mut stale_revision = request.clone();
        stale_revision.fence.subject_revisions.insert(question.id, "operator".into());
        assert_eq!(invoke(&state, &session, &stale_revision).await.unwrap_err().code, "stale-fence");
        let mut stale_incarnation = request.clone();
        stale_incarnation.fence.runtime_incarnation = Some("operator".into());
        assert_eq!(invoke(&state, &session, &stale_incarnation).await.unwrap_err().code, "stale-fence");
        assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn answer_validation_has_typed_reasons_and_rejects_malformed_envelopes() {
        let (_root, _state, _session, question, _request) = fixture();
        for (answers, reason) in [
            (json!({"q0":{"positions":[2]}}), "unknown_option"),
            (json!({"q0":{"positions":[0,0]}}), "too_many"),
            (json!({"q0":{"positions":[0,1]}}), "too_many"),
            (json!({}), "required_missing"),
            (json!({"q0":{"positions":[]}}), "empty_single"),
            (json!({"operator":{"positions":[0]}}), "unknown_question"),
            (json!({"q0":{"positions":vec![0;65]}}), "too_long"),
            (json!({"q0":{"positions":[0],"other":"text"}}), "other_not_allowed"),
            (json!({"q0":{"positions":[0],"note":"text"}}), "note_not_allowed"),
        ] {
            let error = validate_answers(&json!({"question":question.id,"answers":answers}), &question).unwrap_err();
            assert_eq!(error.code, "invalid-answer");
            assert_eq!(error.details["reason"], reason);
        }
        for answers in [json!([]), json!({"q0":{"positions":[-1]}}), json!({"q0":{"positions":[1.5]}}), json!({"q0":{"positions":[4294967296_u64]}}), json!({"q0":{"timed_out":false,"positions":[0]}})] {
            assert_eq!(validate_answers(&json!({"question":question.id,"answers":answers}), &question).unwrap_err().code, "validation-failed");
        }
        let mut multi = question.clone();
        multi.questions[0].multi = true;
        assert_eq!(validate_answers(&json!({"answers":{"q0":{"positions":[]}}}), &multi).unwrap_err().details["reason"], "required_missing");
        multi.questions[0].required = false;
        assert!(validate_answers(&json!({"answers":{"q0":{"positions":[]}}}), &multi).is_ok());
    }

    #[tokio::test]
    async fn metadata_routes_and_action_route_use_the_real_daemon_boundary() {
        let (_root, state, _session, question, request) = fixture();
        let app = super::super::super::router(state.clone());
        for uri in ["/v1/client/harness-questions".to_owned(),
            format!("/v1/client/harness-questions/{}", question.id)] {
            let response = app.clone().oneshot(Request::builder().uri(uri).header(LOCAL_PERSON_HEADER, "person/alex").body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap()).unwrap();
            let row = if body["value"]["items"].is_array() { &body["value"]["items"][0] } else { &body["value"] };
            assert_eq!(row["id"], question.id);
            assert_eq!(row["answer"]["reason"], "adapter_missing");
            assert_eq!(row["questions"], json!([{"id":"q0","options":2,"multi":false,"required":true}]));
            assert_eq!(row["resolution"], Value::Null);
        }
        let before = state.store.index().unwrap();
        let response = app.oneshot(Request::builder().method("POST").uri("/v1/client/actions")
            .header(LOCAL_PERSON_HEADER, "person/alex").header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&request).unwrap())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn resolved_metadata_wire_shape_excludes_retained_receipt() {
        let (_root, mut state, session, question, request) = fixture();
        state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:true }));
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        let resolved = held(&state, &question.id).unwrap();
        let resolution = resolved.resolution.as_ref().unwrap();
        assert!(resolution.receipt.is_some());
        let expected = json!({
            "kind":"harness-question", "id":question.id, "revision":question.revision,
            "agent":"agent/ada", "session":"q0",
            "native_id":fractal_schema::harness_questions::native_id_digest("q0"), "runtime_incarnation":"q0",
            "questions":[{"id":"q0","options":2,"multi":false,"required":true}],
            "updated_at":client_timestamp(resolution.resolved_at_unix_ms),
            "observed_at":question.observed_at_unix_ms,
            "resolution":{"status":"answered", "revision":question.revision,
                "answers":{"q0":{"positions":[1],"timed_out":true}},
                "resolved_at":resolution.resolved_at_unix_ms},
            "answer":{"state":"unavailable","reason":"resolved"},
        });
        let app = super::super::super::router(state.clone());
        for uri in ["/v1/client/harness-questions".to_owned(),
            format!("/v1/client/harness-questions/{}", question.id)] {
            let response = app.clone().oneshot(Request::builder().uri(uri)
                .header(LOCAL_PERSON_HEADER, "person/alex").body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap()).unwrap();
            let row = if body["value"]["items"].is_array() { &body["value"]["items"][0] } else { &body["value"] };
            assert_eq!(row, &expected);
        }
    }

    #[tokio::test]
    async fn metadata_list_clamps_limits_and_rejects_mismatched_cursors() {
        let (_root, state, session, _question, _request) = fixture();
        state.store.observe_harness_question(&QuestionObservation {
            agent:"agent/ada".into(), session:"q0".into(), native_id:"second-call".into(),
            runtime_incarnation:"q0".into(),
            questions:vec![Question { id:"q0".into(), options:2, multi:false, required:true }],
        }).unwrap();
        let snapshot = new_client_snapshot(&state);
        let (_, Json(first)) = list(State(state.clone()), Extension(snapshot.clone()), Extension(session.clone()),
            Query(ListQuery { limit:Some(0), cursor:None })).await.unwrap();
        assert_eq!(first.page.limit, 1);
        assert_eq!(first.items.len(), 1);
        let cursor = first.page.next_cursor.unwrap();
        for (limit, cursor, expected) in [
            (Some(1), "not-a-page-cursor".to_owned(), "validation-failed"),
            (Some(2), cursor.clone(), "page-cursor-expired"),
        ] {
            assert_eq!(list(State(state.clone()), Extension(snapshot.clone()), Extension(session.clone()),
                Query(ListQuery { limit, cursor:Some(cursor) })).await.unwrap_err().code, expected);
        }
        let mut other_snapshot = snapshot.clone();
        other_snapshot.id.push_str("-different");
        assert_eq!(list(State(state.clone()), Extension(other_snapshot), Extension(session.clone()),
            Query(ListQuery { limit:None, cursor:Some(cursor.clone()) })).await.unwrap_err().code, "page-cursor-expired");
        let (_, Json(continued)) = list(State(state.clone()), Extension(snapshot.clone()), Extension(session.clone()),
            Query(ListQuery { limit:None, cursor:Some(cursor) })).await.unwrap();
        assert_eq!(continued.page.limit, 1);
        assert_eq!(continued.items.len(), 1);
        assert_ne!(continued.items[0]["id"], first.items[0]["id"]);
        let (_, Json(all)) = list(State(state.clone()), Extension(snapshot), Extension(session),
            Query(ListQuery { limit:Some(usize::MAX), cursor:None })).await.unwrap();
        assert_eq!(all.page.limit, CLIENT_MAX_PAGE_ITEMS);
        assert_eq!(all.items.len(), 2);
        assert!(!all.page.has_more);
    }

    #[tokio::test]
    async fn unavailable_adapter_and_invalid_answers_never_write_receipts() {
        struct Unavailable;
        impl HarnessQuestionSettleSink for Unavailable {
            fn availability(&self, _: &HarnessQuestion) -> Result<(), HarnessQuestionAdapterError> { Err(HarnessQuestionAdapterError::Unavailable) }
            fn settle(&self, _: &HarnessQuestion, _: &BTreeMap<String, Answer>) -> Result<BTreeMap<String, Answer>, HarnessQuestionAdapterError> { unreachable!("unavailable adapter must never settle") }
        }
        let (_root, mut state, session, question, mut request) = fixture();
        state.harness_question_sink = Some(Arc::new(Unavailable));
        request.parameters["answers"] = json!({"q0":{"positions":[999]}});
        let before = state.store.index().unwrap();
        assert_eq!(invoke(&state, &session, &request).await.unwrap_err().details["reason"], "adapter_unavailable");
        assert_eq!(metadata(&state, &question)["answer"]["reason"], "adapter_unavailable");
        let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
        state.harness_question_sink = Some(sink.clone());
        assert_eq!(invoke(&state, &session, &request).await.unwrap_err().details["reason"], "unknown_option");
        assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn changed_question_parameter_conflicts_with_retained_receipt() {
        let (_root, mut state, session, _question, request) = fixture();
        state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false }));
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        let other = state.store.observe_harness_question(&QuestionObservation {
            agent:"agent/ada".into(), session:"q0".into(), native_id:"operator".into(), runtime_incarnation:"q0".into(),
            questions:vec![Question { id:"q0".into(), options:2, multi:false, required:true }],
        }).unwrap();
        let mut changed = request.clone();
        changed.parameters["question"] = json!(other.id);
        changed.fence.subject_revisions = BTreeMap::from([(other.id.clone(), other.revision.clone())]);
        let before = state.store.index().unwrap();
        assert_eq!(invoke(&state, &session, &changed).await.unwrap_err().code, "idempotency-conflict");
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test]
    async fn live_terminal_authority_and_scope_are_required_before_replay() {
        for fields in [
            json!({"status":"running","runtime_id":"q0","incarnation_id":"q0","terminal":false}),
            json!({"status":"stopped","runtime_id":"q0","incarnation_id":"q0","terminal":true}),
        ] {
            let (_root, mut state, session, _question, request) = fixture();
            state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false }));
            assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
            let no_scope = ClientSession::for_tests_scoped("person/alex", "person/alex", &[]);
            assert_eq!(invoke(&state, &no_scope, &request).await.unwrap_err().code, "forbidden");
            let agent_credential = ClientSession::for_tests_scoped("agent/ada", "person/alex", &["terminal.control"]);
            assert_eq!(invoke(&state, &agent_credential, &request).await.unwrap_err().code, "forbidden");
            state.store.append_claim(&ClaimInput {
                subject:"agent/ada".into(), kind:"runtime.observed".into(), actor:Some("agent/ada".into()),
                fields:serde_json::from_value(fields).unwrap(), evidence:Vec::new(), expected_subject:None, idempotency_key:None,
            }).unwrap();
            assert!(invoke(&state, &session, &request).await.is_err());
        }
    }

    #[tokio::test]
    async fn concurrent_generic_and_harness_actions_exclude_the_same_key() {
        for harness_first in [false, true] {
            let (_root, mut state, _session, question, request) = fixture();
            let session = ClientSession::for_tests_scoped("person/alex", "person/alex",
                &["terminal.control", "read.projections", "control.arrangements"]);
            let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
            state.harness_question_sink = Some(sink.clone());
            let arrangement = "arrangement/person/alex/019a0000-0000-7000-8000-000000000001";
            let generic = ActionRequest {
                api_version:CLIENT_API_VERSION.into(), id:"action/concurrent".into(),
                action_type:"arrangement.edit".into(), idempotency_key:request.idempotency_key.clone(),
                fence:Fence { snapshot_id:new_client_snapshot(&state).id, ..Fence::default() },
                parameters:json!({"subject":arrangement,"owner":"person/alex","operations":[{"op":"create","name":"Sidebar"}]}),
            };
            // Queue both families behind the shared gate before allowing either to mutate.
            let gate = action_gate(&state, &session, &request.idempotency_key);
            let guard = gate.lock_owned().await;
            let mut harness = Box::pin(invoke(&state, &session, &request));
            let mut other = Box::pin(invoke(&state, &session, &generic));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            if harness_first {
                assert!(Future::poll(harness.as_mut(), &mut context).is_pending());
                assert!(Future::poll(other.as_mut(), &mut context).is_pending());
            } else {
                assert!(Future::poll(other.as_mut(), &mut context).is_pending());
                assert!(Future::poll(harness.as_mut(), &mut context).is_pending());
            }
            drop(guard);
            let (answer, edit) = tokio::join!(harness, other);
            let (winner, loser) = if harness_first { (answer, edit) } else { (edit, answer) };
            let accepted = winner.unwrap().0;
            assert_eq!(loser.unwrap_err().code, "idempotency-conflict");
            assert_eq!(sink.calls.load(Ordering::SeqCst), usize::from(harness_first));
            assert_eq!(held(&state, &question.id).unwrap().resolution.is_some(), harness_first);
            assert_eq!(state.store.claims_for(arrangement, Some("arrangement.edited")).unwrap().is_empty(), harness_first);
            let before = state.store.index().unwrap();
            let replay = invoke(&state, &session, if harness_first { &request } else { &generic }).await.unwrap().0;
            assert_eq!(replay["operation_id"], accepted["operation_id"]);
            assert_eq!(state.store.index().unwrap(), before);
        }
    }

    #[tokio::test]
    async fn daemon_router_refuses_unix_free_mode_agent_answers() {
        let (_root, mut state, _session, question, request) = fixture();
        let sink = Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:false });
        state.harness_question_sink = Some(sink.clone());
        let local_agent = ClientSession::local(Some("agent/ada")).unwrap();
        assert!(local_agent.allows("terminal.control"));
        let before = state.store.index().unwrap();
        let response = super::super::super::router(state.clone()).oneshot(
            Request::builder().method("POST").uri("/v1/client/actions")
                .header(LOCAL_PERSON_HEADER, "agent/ada").header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request).unwrap())).unwrap()
        ).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap()).unwrap();
        assert_eq!(body["code"], "forbidden");
        assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
        assert!(held(&state, &question.id).unwrap().resolution.is_none());
        assert_eq!(state.store.index().unwrap(), before);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_availability_and_settlement_run_off_the_async_executor() {
        struct WorkerOnly(std::thread::ThreadId, AtomicUsize);
        impl HarnessQuestionSettleSink for WorkerOnly {
            fn availability(&self, _: &HarnessQuestion) -> Result<(), HarnessQuestionAdapterError> {
                assert_ne!(std::thread::current().id(), self.0);
                self.1.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            fn settle(&self, _: &HarnessQuestion, answers: &BTreeMap<String, Answer>) -> Result<BTreeMap<String, Answer>, HarnessQuestionAdapterError> {
                assert_ne!(std::thread::current().id(), self.0);
                Ok(answers.clone())
            }
        }
        let (_root, mut state, session, question, request) = fixture();
        let sink = Arc::new(WorkerOnly(std::thread::current().id(), AtomicUsize::new(0)));
        state.harness_question_sink = Some(sink.clone());
        let Json(detail) = get(State(state.clone()), Extension(session.clone()), AxumPath(question.id.clone())).await.unwrap();
        assert_eq!(detail["answer"], json!({"state":"available"}));
        let (_, Json(page)) = list(State(state.clone()), Extension(new_client_snapshot(&state)),
            Extension(session.clone()), Query(ListQuery { limit:Some(1), cursor:None })).await.unwrap();
        assert_eq!(page.items[0]["answer"], json!({"state":"available"}));
        assert_eq!(sink.1.load(Ordering::SeqCst), 2);
        // Capability discovery is adapter-presence based, not a native availability probe.
        let Json(advertised) = client_capabilities(State(state.clone()),
            Extension(new_client_snapshot(&state)), Extension(session.clone())).await.unwrap();
        assert!(advertised["capabilities"].as_array().unwrap().iter().any(|item|
            item == &json!({"id":"harness.answer","version":0,"state":"granted"})));
        assert_eq!(sink.1.load(Ordering::SeqCst), 2);
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        assert!(held(&state, &question.id).unwrap().resolution.is_some());
        assert_eq!(sink.1.load(Ordering::SeqCst), 3);
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    struct Available;
    impl HarnessQuestionSettleSink for Available {
        fn availability(&self, _: &HarnessQuestion) -> Result<(), HarnessQuestionAdapterError> { Ok(()) }
        fn settle(&self, _: &HarnessQuestion, answers: &BTreeMap<String, Answer>) -> Result<BTreeMap<String, Answer>, HarnessQuestionAdapterError> { Ok(answers.clone()) }
    }

    #[tokio::test]
    async fn harness_answer_capability_tracks_injected_sink_and_person_authority() {
        let root = tempfile::tempdir().unwrap();
        let mut state = super::super::tests::test_state_named(root.path(), "q0");
        let person = ClientSession::for_tests_scoped("person/alex", "person/alex", &["terminal.control"]);
        let find = |state: &AppState, session: &ClientSession| capabilities(state, session).into_iter()
            .find(|item| item["id"] == "harness.answer").unwrap();
        assert_eq!(find(&state, &person), json!({"id":"harness.answer","version":0,"state":"unavailable"}));
        state.harness_question_sink = Some(Arc::new(Available));
        assert_eq!(find(&state, &person)["state"], "granted");
        let agent = ClientSession::for_tests_scoped("agent/ada", "agent/ada", &["terminal.control"]);
        assert_eq!(find(&state, &agent)["state"], "ungranted");
        let unscoped = ClientSession::for_tests_scoped("person/alex", "person/alex", &[]);
        assert_eq!(find(&state, &unscoped)["state"], "ungranted");
        state.harness_question_sink = None;
        let value = client_capabilities(State(state.clone()), Extension(new_client_snapshot(&state)), Extension(person)).await.unwrap().0;
        let answer = value["capabilities"].as_array().unwrap().iter().find(|item| item["id"] == "harness.answer").unwrap();
        assert_eq!(answer, &json!({"id":"harness.answer","version":0,"state":"unavailable"}));
    }
}
