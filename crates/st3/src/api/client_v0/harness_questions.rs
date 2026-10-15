//! Metadata-only reads and person-only native answer settlement.
use super::*;
pub use crate::store::harness_questions::HarnessQuestion;
use st3_schema::harness_questions::{Answer, Receipt, Status, OBSERVED, MAX_QUESTIONS, MAX_OPTIONS};

/// A native adapter's capability refusal. No adapter is installed by default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HarnessQuestionAdapterError { Missing, Unavailable }

/// Injected at the daemon boundary; implementations settle the exact native call, never terminal text.
/// The returned answers preserve the harness's authoritative timeout bits.
pub trait HarnessQuestionSettleSink: Send + Sync {
    fn availability(&self, question: &HarnessQuestion) -> Result<(), HarnessQuestionAdapterError>;
    fn settle(&self, question: &HarnessQuestion, answers: &BTreeMap<String, Answer>)
        -> Result<BTreeMap<String, Answer>, HarnessQuestionAdapterError>;
}

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

pub(super) fn capability(state: &AppState, session: &ClientSession) -> Value {
    if state.harness_question_sink.is_none() {
        return json!({"id":"harness.answer","version":0,"state":"unavailable","reason":"adapter_missing"});
    }
    let person = !session.actor.starts_with("agent/")
        && session.authority_actor.starts_with("person/")
        && session.authority_actor.matches('/').count() == 1;
    json!({"id":"harness.answer","version":0,
        "state":if person && session.allows("terminal.control") { "granted" } else { "ungranted" }})
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
pub(in crate::api) async fn list(State(state): State<AppState>, Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<ClientSession>, Query(query): Query<ClientListQuery>)
    -> Result<ClientPageResponse, ApiError> {
    require_scope(&session, "read.projections")?;
    let mut items = Vec::new();
    if query.cursor.is_none() {
        let status = state.store.status_for_claim_kind_at(OBSERVED, None, false).map_err(ApiError::internal)?;
        let now = client_now_ms();
        for subject in status.subjects {
            if let Some(question) = state.store.harness_question(&subject.subject, now).map_err(ApiError::internal)? {
                items.push(metadata(&state, &question));
            }
        }
    }
    let page = client_page(&state, &snapshot, "harness-questions", items, &query)?;
    Ok((Extension(snapshot), Json(page)))
}
pub(in crate::api) async fn get(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    Ok(Json(metadata(&state, &held(&state, &client_detail_id("harness-question", &id))?)))
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
    validate_action_admission(session, request)?;
    if session.actor.starts_with("agent/") || !session.authority_actor.starts_with("person/") || session.authority_actor.matches('/').count() != 1 {
        return Err(forbidden("native harness answers require a concrete person; agents cannot answer"));
    }
    let gate = action_gate(state, session, &request.idempotency_key);
    let _guard = gate.lock_owned().await;
    let mut provenance = request.fence.clone();
    provenance.subject_revisions.clear();
    validate_fence(state, &provenance)?;
    let id = parameter_string(&request.parameters, "question")?;
    // Serialize different people and different keys settling the same native call as well.
    let mut owner = session.clone();
    owner.actor = "harness-question-settlement".into();
    let question_gate = action_gate(state, &owner, &id);
    let _question_guard = question_gate.lock_owned().await;
    let question = held(state, &id)?;
    live_input_session(state, &question.agent, &question.runtime_incarnation)?;
    let digest = action_request_digest(request)?;
    let actor_key = st3_schema::harness_questions::actor_key(&session.actor, &request.idempotency_key);
    // Receipts live only on retained resolutions. Reusing this actor's key for a different
    // question is a conflict too; expiry removes the receipt together with its question.
    let resolved = state.store.status_for_claim_kind_at(st3_schema::harness_questions::RESOLVED, None, false)
        .map_err(ApiError::internal)?;
    let now = client_now_ms();
    for subject in resolved.subjects {
        if subject.subject == id { continue; }
        if let Some(other) = state.store.harness_question(&subject.subject, now).map_err(ApiError::internal)?
            && other.resolution.as_ref().and_then(|resolution| resolution.receipt.as_ref())
                .is_some_and(|receipt| receipt.actor_key == actor_key) {
            return Err(ApiError { status: StatusCode::CONFLICT, code:"idempotency-conflict".into(),
                message:"the idempotency key already settled a different question".into(), details:Box::default() });
        }
    }
    if let Some(receipt) = question.resolution.as_ref().and_then(|resolution| resolution.receipt.as_ref())
        .filter(|receipt| receipt.actor_key == actor_key) {
        if receipt.request_digest != digest {
            return Err(ApiError { status: StatusCode::CONFLICT, code: "idempotency-conflict".into(),
                message: "the idempotency key was already used for a different action".into(), details: Box::default() });
        }
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
    sink.availability(&question).map_err(unavailable)?;
    let answers = validate_answers(&request.parameters, &question)?;
    let sink = Arc::clone(sink);
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
    use st3_schema::harness_questions::Question;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        let state = super::super::tests::test_state_named(root.path(), "tester");
        state.store.append_claim(&ClaimInput {
            subject: "agent/ada".into(), kind: "runtime.observed".into(), actor: Some("agent/ada".into()),
            fields: serde_json::from_value(json!({"status":"running","runtime_id":"tester","incarnation_id":"tester","terminal":true})).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let question = state.store.observe_harness_question(&QuestionObservation {
            agent:"agent/ada".into(), session:"tester".into(), native_id:"tester".into(), runtime_incarnation:"tester".into(),
            questions:vec![Question { id:"tester".into(), options:2, multi:false, required:true }],
        }).unwrap();
        let session = ClientSession::for_tests_scoped("person/alex", "person/alex", &["terminal.control", "read.projections"]);
        let request = ActionRequest { api_version:CLIENT_API_VERSION.into(), id:"action/tester".into(), action_type:"harness.answer".into(),
            idempotency_key:"tester-operator-key".into(), fence:Fence { snapshot_id:new_client_snapshot(&state).id,
                subject_revisions:BTreeMap::from([(question.id.clone(), question.revision.clone())]),
                runtime_incarnation:Some("tester".into()), ..Fence::default() },
            parameters:json!({"question":question.id,"answers":{"tester":{"positions":[1]}}}) };
        (root, state, session, question, request)
    }
    async fn invoke(state: &AppState, session: &ClientSession, request: &ActionRequest) -> Result<Json<Value>, ApiError> {
        action_inner(State(state.clone()), Extension(new_client_snapshot(state)), Extension(session.clone()), Json(request.clone()), None).await
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
        changed.parameters["answers"]["tester"]["positions"] = json!([0]);
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
            fields:serde_json::from_value(json!({"status":"running","runtime_id":"tester","incarnation_id":"operator","terminal":true})).unwrap(),
            evidence:Vec::new(), expected_subject:None, idempotency_key:None,
        }).unwrap();
        assert_eq!(invoke(&state, &session, &request).await.unwrap_err().code, "stale-incarnation");
    }
    #[tokio::test]
    async fn new_key_after_resolution_precedes_capability_and_answer_validation() {
        let (_root, mut state, session, question, request) = fixture();
        state.harness_question_sink = Some(Arc::new(Sink { calls:AtomicUsize::new(0), timed_out:true }));
        let accepted = invoke(&state, &session, &request).await.unwrap().0;
        assert_eq!(accepted["harness_answer"]["resolution"]["answers"]["tester"]["timed_out"], true);
        state.harness_question_sink = None;
        let mut next = request.clone();
        next.idempotency_key = "operator-reviewer-key".into();
        next.parameters["answers"] = json!({"unknown":{"note":"forbidden"}});
        let before = state.store.index().unwrap();
        assert_eq!(invoke(&state, &session, &next).await.unwrap().0["harness_answer"]["status"], "already_resolved");
        assert_eq!(state.store.index().unwrap(), before);
        let public = metadata(&state, &held(&state, &question.id).unwrap());
        assert_eq!(public["resolution"]["answers"]["tester"]["timed_out"], true);
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
            (json!({"tester":{"positions":[2]}}), "unknown_option"),
            (json!({"tester":{"positions":[0,0]}}), "too_many"),
            (json!({"tester":{"positions":[0,1]}}), "too_many"),
            (json!({}), "required_missing"),
            (json!({"tester":{"positions":[]}}), "empty_single"),
            (json!({"operator":{"positions":[0]}}), "unknown_question"),
            (json!({"tester":{"positions":vec![0;65]}}), "too_long"),
            (json!({"tester":{"positions":[0],"other":"text"}}), "other_not_allowed"),
            (json!({"tester":{"positions":[0],"note":"text"}}), "note_not_allowed"),
        ] {
            let error = validate_answers(&json!({"question":question.id,"answers":answers}), &question).unwrap_err();
            assert_eq!(error.code, "invalid-answer");
            assert_eq!(error.details["reason"], reason);
        }
        for answers in [json!([]), json!({"tester":{"positions":[-1]}}), json!({"tester":{"positions":[1.5]}}), json!({"tester":{"positions":[4294967296_u64]}}), json!({"tester":{"timed_out":false,"positions":[0]}})] {
            assert_eq!(validate_answers(&json!({"question":question.id,"answers":answers}), &question).unwrap_err().code, "validation-failed");
        }
        let mut multi = question.clone();
        multi.questions[0].multi = true;
        assert_eq!(validate_answers(&json!({"answers":{"tester":{"positions":[]}}}), &multi).unwrap_err().details["reason"], "required_missing");
        multi.questions[0].required = false;
        assert!(validate_answers(&json!({"answers":{"tester":{"positions":[]}}}), &multi).is_ok());
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
            assert!(row.get("text").is_none());
            assert!(row.get("receipt").is_none());
        }
        let before = state.store.index().unwrap();
        let response = app.oneshot(Request::builder().method("POST").uri("/v1/client/actions")
            .header(LOCAL_PERSON_HEADER, "person/alex").header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&request).unwrap())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(state.store.index().unwrap(), before);
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
        request.parameters["answers"] = json!({"tester":{"positions":[999]}});
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
            agent:"agent/ada".into(), session:"tester".into(), native_id:"operator".into(), runtime_incarnation:"tester".into(),
            questions:vec![Question { id:"tester".into(), options:2, multi:false, required:true }],
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
            json!({"status":"running","runtime_id":"tester","incarnation_id":"tester","terminal":false}),
            json!({"status":"stopped","runtime_id":"tester","incarnation_id":"tester","terminal":true}),
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

    #[tokio::test(flavor = "current_thread")]
    async fn native_settlement_runs_off_the_async_executor() {
        struct WorkerOnly(std::thread::ThreadId);
        impl HarnessQuestionSettleSink for WorkerOnly {
            fn availability(&self, _: &HarnessQuestion) -> Result<(), HarnessQuestionAdapterError> { Ok(()) }
            fn settle(&self, _: &HarnessQuestion, answers: &BTreeMap<String, Answer>) -> Result<BTreeMap<String, Answer>, HarnessQuestionAdapterError> {
                assert_ne!(std::thread::current().id(), self.0);
                Ok(answers.clone())
            }
        }
        let (_root, mut state, session, question, request) = fixture();
        state.harness_question_sink = Some(Arc::new(WorkerOnly(std::thread::current().id())));
        assert_eq!(invoke(&state, &session, &request).await.unwrap().0["harness_answer"]["status"], "accepted");
        assert!(held(&state, &question.id).unwrap().resolution.is_some());
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
        let mut state = super::super::tests::test_state_named(root.path(), "tester");
        let person = ClientSession::for_tests_scoped("person/alex", "person/alex", &["terminal.control"]);
        let find = |state: &AppState, session: &ClientSession| capabilities(state, session).into_iter()
            .find(|item| item["id"] == "harness.answer").unwrap();
        assert_eq!(find(&state, &person), json!({"id":"harness.answer","version":0,"state":"unavailable","reason":"adapter_missing"}));
        state.harness_question_sink = Some(Arc::new(Available));
        assert_eq!(find(&state, &person)["state"], "granted");
        let agent = ClientSession::for_tests_scoped("agent/ada", "agent/ada", &["terminal.control"]);
        assert_eq!(find(&state, &agent)["state"], "ungranted");
        let unscoped = ClientSession::for_tests_scoped("person/alex", "person/alex", &[]);
        assert_eq!(find(&state, &unscoped)["state"], "ungranted");
        state.harness_question_sink = None;
        let value = client_capabilities(State(state.clone()), Extension(new_client_snapshot(&state)), Extension(person)).await.unwrap().0;
        let answer = value["capabilities"].as_array().unwrap().iter().find(|item| item["id"] == "harness.answer").unwrap();
        assert_eq!(answer, &json!({"id":"harness.answer","version":0,"state":"unavailable","reason":"adapter_missing"}));
    }
}
