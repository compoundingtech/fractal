//! OMP observation ingestion. This route cannot submit or acknowledge a person's answer.
use super::*;
use crate::HarnessQuestionsStore;
use crate::store::harness_questions::{QuestionGeneration, QuestionObservation, QuestionWitness};
use fractal_drivers::omp_questions::{CloseStatus, Observation};
use fractal_schema::harness_questions::{Status, subject};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    agent: String,
    runtime_incarnation: String,
    event: Observation,
}

fn refused(message: &str) -> St3Error {
    St3Error::new("foreign-harness-question", message)
}

fn authorize(request: &Request, peer: Option<&NativeDeliveryPeer>) -> Result<(), St3Error> {
    if !peer.is_some_and(|peer| peer.transport == "omp-channel" && peer.agent == request.agent) {
        return Err(refused("question observations require this managed OMP native driver"));
    }
    request.event.validate().map_err(|_| refused("invalid text-free question observation"))
}

pub(super) async fn ingest(
    State(state): State<AppState>,
    peer: Option<Extension<NativeDeliveryPeer>>,
    local: Option<Extension<crate::api::LocalPeer>>,
    Json(request): Json<Request>,
) -> Result<Json<Value>, ApiError> {
    authorize(&request, peer.as_ref().map(|peer| &peer.0)).map_err(ApiError::bad)?;
    if !fractal_drivers::omp_questions::enabled() {
        return Err(ApiError::bad(St3Error::new("unsupported-capability", "OMP question ingestion is disabled")));
    }
    let peer = peer.expect("authorize requires the native driver").0;
    let connector = local.and_then(|local| {
        local.0.peer_process.filter(|connector| connector.pid() == local.0.pid)
    }).ok_or_else(|| ApiError::bad(refused(
        "question observations require a kernel connector generation witness",
    )))?;
    validate_connector(&connector, &peer).map_err(ApiError::bad)?;
    let fence = Fence::new(&request.agent, &request.runtime_incarnation, "questions");
    let admission = fractal_drivers::harness_state::record_lock_admission(
        &authority::source_agent_dir(&state, &fence),
    ).await.map_err(ApiError::internal)?;
    let checked = state.clone();
    let (id, changed) = authority_action(move || {
        let _admission = admission;
        authority::with_observation_authority(&checked, &fence, &peer, |owner, witness, validate| {
            let live = live_session(&checked, &request.agent, Some(&request.runtime_incarnation))
                .map_err(|error| {
                    let code = match error.code.as_str() {
                        "stale-incarnation" => "stale-incarnation",
                        "not-found" => "not-found",
                        "runtime-not-local" => "runtime-not-local",
                        "runtime-authority-indeterminate" => "runtime-authority-indeterminate",
                        _ => "internal",
                    };
                    St3Error::new(code, error.message)
                })?;
            let dir = authority::source_agent_dir(&checked, &fence);
            let session_dir = dir.parent().ok_or_else(|| refused("invalid provider source"))?
                .join("sessions").join("omp");
            let identity = request.agent.strip_prefix("agent/").unwrap_or(&request.agent);
            let bound = fractal_drivers::omp_session::bound_native_session(
                &session_dir, identity, &live.runtime_id, &owner.session,
            ).map_err(|_| refused("native session binding is unavailable"))?;
            if bound.as_deref() != Some(request.event.session()) {
                return Err(refused("question observation belongs to another native session"));
            }
            validate()?;
            validate_connector(&connector, &peer)?;
            let generation = QuestionGeneration {
                question: subject(&request.agent, request.event.session(), request.event.native_id()),
                agent: request.agent.clone(), runtime_incarnation: request.runtime_incarnation.clone(),
                session: request.event.session().to_owned(), owner_session: owner.session.clone(),
                owner_sequence: owner.sequence, witness: witness.clone(),
            };
            apply(&checked.store, &request, &generation)
        })
    }).await?;
    if changed { signal_local_change(&state); }
    Ok(Json(json!({"question": id})))
}

fn validate_connector(
    connector: &fractal_runtime::PeerProcessIdentity,
    peer: &NativeDeliveryPeer,
) -> Result<(), St3Error> {
    if connector.pid() != peer.pid {
        return Err(refused("question caller differs from the kernel connector"));
    }
    connector.validate().map_err(|_| refused("question connector generation is no longer live"))
}

fn apply(store: &Store, request: &Request, generation: &QuestionGeneration) -> Result<(String, bool), St3Error> {
    let id = subject(&request.agent, request.event.session(), request.event.native_id());
    if generation.question != id || generation.agent != request.agent
        || generation.runtime_incarnation != request.runtime_incarnation
        || generation.session != request.event.session() {
        return Err(refused("question generation does not match its observation"));
    }
    let current = store.harness_question_held(&id)?;
    if current.as_ref().is_some_and(|question| question.agent != request.agent
        || question.runtime_incarnation != request.runtime_incarnation
        || question.session != request.event.session()) {
        return Err(refused("question belongs to another runtime binding"));
    }
    match &request.event {
        Observation::Open { session, native_id, questions } => {
            if let Some(current) = current {
                if current.questions != *questions {
                    return Err(refused("native call identity was reused with different descriptors"));
                }
                if current.resolution.is_none() {
                    store.retain_harness_question_generation(generation)?;
                }
                return Ok((id, false));
            }
            // Durable generation FIRST, under the provider record lock. A daemon crash
            // between these writes leaves at worst an orphan lease, never an immortal open.
            store.retain_harness_question_generation(generation)?;
            store.observe_harness_question_digest(&QuestionObservation {
                agent: request.agent.clone(), session: session.clone(), native_id: native_id.clone(),
                runtime_incarnation: request.runtime_incarnation.clone(), questions: questions.clone(),
            })?;
            Ok((id, true))
        }
        Observation::Closed { status, .. } => {
            let current = current.ok_or_else(|| refused("cannot close an unobserved native question"))?;
            if current.resolution.is_none() {
                store.retain_harness_question_generation(generation)?;
            }
            let status = match status {
                CloseStatus::AnsweredElsewhere => Status::AnsweredElsewhere,
                CloseStatus::Expired => Status::Expired,
                CloseStatus::Cancelled => Status::Cancelled,
            };
            let (_, changed) = store.resolve_harness_question(
                &id, status, &current.revision, &BTreeMap::new(), None,
            )?;
            store.forget_harness_question_generation(generation).map_err(smallclaims::error::internal)?;
            Ok((id, changed))
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use fractal_schema::harness_questions::{Question, native_id_digest};

    fn request() -> Request {
        Request { agent: "agent/example/question".into(), runtime_incarnation: "runtime-one".into(),
            event: Observation::Open { native_id: native_id_digest("PRIVATE_NATIVE_CALL"),
                session: "native-session".into(), questions: vec![Question {
                    id: "q0".into(), options: 2, multi: false, required: true,
                }] } }
    }

    fn generation(request: &Request) -> QuestionGeneration {
        QuestionGeneration {
            question: subject(&request.agent, request.event.session(), request.event.native_id()),
            agent: request.agent.clone(), runtime_incarnation: request.runtime_incarnation.clone(),
            session: request.event.session().to_owned(), owner_session: "owner".into(), owner_sequence: 1,
            witness: QuestionWitness { runtime_name: "runtime".into(), created_at: "created".into(),
                daemon: (1, 1), terminal: (2, 2), wrapper: (2, 2), bound: false },
        }
    }

    fn apply(store: &Store, request: &Request) -> Result<(String, bool), St3Error> {
        super::apply(store, request, &generation(request))
    }

    pub(in crate::api::mailbox) fn assert_forged_binding_cannot_write(
        peer: &NativeDeliveryPeer,
        admit: impl Fn() -> Result<(), St3Error>,
    ) {
        let store = Store::open_memory("forged-question-driver").unwrap();
        let mut frame = request();
        apply(&store, &frame).unwrap();
        // All request binding fields are real. Only the kernel process generation
        // distinguishes this attacker from the managed wrapper it impersonates.
        authorize(&frame, Some(peer)).unwrap();
        let index = store.index().unwrap();
        for close in [false, true] {
            frame.event = if close {
                Observation::Closed { session: "native-session".into(),
                    native_id: native_id_digest("PRIVATE_NATIVE_CALL"),
                    status: CloseStatus::AnsweredElsewhere }
            } else {
                Observation::Open { session: "native-session".into(),
                    native_id: native_id_digest("another-call"),
                    questions: vec![Question { id: "q0".into(), options: 2,
                        multi: false, required: true }] }
            };
            assert!(admit().and_then(|()| apply(&store, &frame)).is_err());
            assert_eq!(store.index().unwrap(), index, "forged open/close wrote a claim");
        }
    }

    #[test]
    fn native_binding_is_mandatory_before_ingestion() {
        let request = request();
        assert!(authorize(&request, None).is_err());
        for (agent, transport) in [("person/example", "omp-channel"),
            ("agent/example/another", "omp-channel"), (request.agent.as_str(), "pi-channel")] {
            let peer = NativeDeliveryPeer { agent: agent.into(), transport, pid: 1, archives_inbox: false };
            assert!(authorize(&request, Some(&peer)).is_err());
        }
    }

    #[tokio::test]
    async fn non_driver_http_and_fabric_requests_write_nothing() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests_shared::state(root.path());
        let index = state.store.index().unwrap();
        let value = request();
        let body = json!({ "agent": value.agent, "runtime_incarnation": value.runtime_incarnation,
            "event": value.event });
        let local = Router::new().route("/v1/harness-questions/native", post(ingest))
            .with_state(state.clone());
        let forged = Router::new().route("/v1/harness-questions/native", post(ingest))
            .layer(Extension(NativeDeliveryPeer {
                agent: "agent/example/another".into(), transport: "omp-channel",
                pid: std::process::id(), archives_inbox: false,
            })).with_state(state.clone());
        let fabric = router_for_transport(state.clone(), ClientTransportBoundary::FabricLoopback,
            &[super::super::routes]);
        for app in [local, forged, fabric] {
            let response = app.oneshot(axum::http::Request::builder()
                .method("POST").uri("/v1/harness-questions/native")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
            assert!(!response.status().is_success());
            assert_eq!(state.store.index().unwrap(), index);
        }
    }

    #[test]
    fn replay_and_foreign_or_unknown_closures_do_not_write() {
        let store = Store::open_memory("question-ingest").unwrap();
        let mut request = request();
        let (id, changed) = apply(&store, &request).unwrap();
        assert!(changed);
        let index = store.index().unwrap();
        assert!(!apply(&store, &request).unwrap().1);
        assert_eq!(store.index().unwrap(), index);
        request.runtime_incarnation = "forged-runtime".into();
        assert!(apply(&store, &request).is_err());
        assert_eq!(store.index().unwrap(), index);
        request.runtime_incarnation = "runtime-one".into();
        request.event = Observation::Closed { session: "native-session".into(),
            native_id: native_id_digest("PRIVATE_NATIVE_CALL"), status: CloseStatus::AnsweredElsewhere };
        assert!(apply(&store, &request).unwrap().1);
        let index = store.index().unwrap();
        assert!(!apply(&store, &request).unwrap().1);
        let held = store.harness_question_held(&id).unwrap().unwrap();
        assert_eq!(held.resolution.unwrap().status, Status::AnsweredElsewhere);
        request.event = Observation::Closed { session: "native-session".into(),
            native_id: native_id_digest("unknown"), status: CloseStatus::Cancelled };
        assert!(apply(&store, &request).is_err());
        request.agent = "agent/example/forged".into();
        assert!(apply(&store, &request).is_err());
        assert_eq!(store.index().unwrap(), index);
    }
}

#[cfg(test)]
#[path = "questions_lifecycle_tests.rs"]
mod lifecycle_tests;
