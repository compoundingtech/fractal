use crate::AdhocWorkStore;
use crate::MissionsStore;
use crate::MissionsStoreCore;
use crate::PersonWorkStore;
use crate::SeatsStoreCore;
use super::*;
use super::client_v0::{ClientSession, require_scope};

/// Only a locally bound harness may recover a missing work fence from its own runtime.
/// Explicit fences are retained so the action's validation can refuse stale callers.
fn bind_work_incarnation(
    state: &AppState,
    bound: Option<&Extension<BoundAgent>>,
    actor: Option<&str>,
    incarnation: &mut Option<String>,
) -> Result<Option<String>, ApiError> {
    let Some(bound) = bound else { return Ok(None) };
    let seat = &bound.0.0;
    if actor.and_then(normalized_agent_actor).as_deref() != Some(seat.as_str()) {
        return Err(ApiError::bad(FractalError::new(
            "foreign-agent-actor",
            format!(
                "this harness is `{seat}` and cannot act as `{}`",
                actor.unwrap_or("<none>")
            ),
        )));
    }
    let current = state
        .store
        .latest_claim(seat, Some("runtime.observed"))
        .map_err(ApiError::internal)?
        .filter(|runtime| runtime.body["fields"]["status"] == "running")
        .and_then(|runtime| {
            runtime.body["fields"]["incarnation_id"]
                .as_str()
                .map(str::to_owned)
        });
    if incarnation.is_none() {
        *incarnation = current.clone();
    }
    Ok(current)
}

/// Only the daemon's program engine asks, or cancels an ask, as the program asker.
fn refuse_program_asker(actor: &str) -> Result<(), ApiError> {
    if actor == crate::model::PROGRAM_ASKER {
        return Err(ApiError::bad(FractalError::new(
            "reserved-actor",
            format!("`{actor}` is the daemon's program engine; a request cannot act as it"),
        )));
    }
    Ok(())
}

pub async fn ask_person(
    State(state): State<AppState>,
    bound: Option<Extension<BoundAgent>>,
    Json(mut request): Json<PersonAskRequest>,
) -> Result<Json<StepRunView>, ApiError> {
    refuse_program_asker(&request.actor)?;
    let expected = bind_work_incarnation(
        &state,
        bound.as_ref(),
        Some(&request.actor),
        &mut request.incarnation,
    )?;
    if bound.is_some()
        && request.step.is_some()
        && request.new_run.is_none()
        && request.incarnation != expected
    {
        return Err(ApiError::bad(FractalError::new(
            "stale-work-ask",
            format!(
                "cannot ask from step `{}` with stale incarnation `{}` (current harness incarnation `{}`); refresh the claim with `work ls` and use the current incarnation, or omit --step for an independent ask",
                request.step.as_deref().unwrap(),
                request.incarnation.as_deref().unwrap_or("<none>"),
                expected.as_deref().unwrap_or("<none>")
            ),
        )));
    }
    if let Some(structured) = request.request.as_mut() {
        complete_request_images(&state, &request.actor, structured)?;
    }
    let result = state.store.ask_person(&request).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(result))
}

/// A request names its images by the asker's uploads, as a send does; the ask claim keeps each
/// completed with its size and the member holding the bytes (`docs/st3/attachments.md`).
fn complete_request_images(
    state: &AppState,
    asker: &str,
    request: &mut Value,
) -> Result<(), ApiError> {
    let Some(inputs) = request.get("attachments").filter(|inputs| !inputs.is_null()) else {
        return Ok(());
    };
    let inputs: Vec<crate::model::AttachmentInput> = serde_json::from_value(inputs.clone()).map_err(|error| {
        ApiError::bad(FractalError::new(
            "invalid-person-request",
            format!(
                "each image names an upload, {{\"blob\":\"blob/<sha256>\",\"media_type\":\"image/png\",\"name\":\"sketch.png\"}}: {error}"
            ),
        ))
    })?;
    let completed = client_blobs::resolve_attachments(state, asker, &inputs)?;
    request["attachments"] = serde_json::to_value(completed).map_err(ApiError::internal)?;
    Ok(())
}

pub async fn done_person_step(
    State(state): State<AppState>,
    Json(request): Json<PersonStepResponse>,
) -> Result<Json<StepRunView>, ApiError> {
    let result = state
        .store
        .finish_person_step(&request, false)
        .map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(result))
}

pub async fn cancel_person_ask(
    State(state): State<AppState>,
    Json(request): Json<PersonStepResponse>,
) -> Result<Json<StepRunView>, ApiError> {
    refuse_program_asker(&request.actor)?;
    let result = state
        .store
        .finish_person_step(&request, true)
        .map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(result))
}

pub async fn list_work(
    State(state): State<AppState>,
    Query(query): Query<WorkQuery>,
) -> Result<Json<Vec<StepRunView>>, ApiError> {
    let store = state.store.clone();
    if matches!(query.view, WorkView::Renewal) {
        let actor = query.actor.ok_or_else(|| ApiError::bad(FractalError::new(
            "missing-work-actor", "the renewal work view requires an actor",
        )))?;
        return blocking_store(move || {
            crate::performance::task("work/renewal", || store.work_for_renewal(&actor))
        }).await.map(Json);
    }
    let (mut work, desired) = blocking_store(move || {
        let work = crate::performance::task(query.caller.label(), || {
            store.work(query.actor.as_deref(), query.include_terminal)
        })?;
        // Only the agents these steps name, not every subject the fleet has ever declared.
        let actors = work
            .iter()
            .flat_map(|step| {
                step.claimant
                    .iter()
                    .chain(step.assigned_to.iter())
                    .chain(step.available_to.iter())
                    .cloned()
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let desired = store.desired_subjects_named(&actors)?;
        Ok((work, desired))
    })
    .await?;
    let agents = desired
        .into_iter()
        .filter(|subject| subject.kind == "agent")
        .map(|subject| (subject.subject, crate::graph::agent_under(&subject.desired)))
        .collect::<BTreeMap<_, _>>();
    for step in &mut work {
        if let Some(actor) = step
            .claimant
            .as_ref()
            .or(step.assigned_to.as_ref())
            .or_else(|| (step.available_to.len() == 1).then(|| &step.available_to[0]))
        {
            step.under = agents.get(actor).cloned().unwrap_or_default();
        }
    }
    // A seat whose host keeps its receipts acknowledges a wake on that host.
    super::message_reads::read_work_wakes(&state, work.iter_mut())
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(work))
}

/// Deliberately independent of the rich native work view: adding a native field
/// cannot grant service disclosure rights.
#[derive(serde::Serialize)]
struct ServiceWork<'a> {
    subject: &'a str,
    run: &'a str,
    generation: &'a str,
    step: &'a str,
    attempt: u32,
    status: &'a str,
    claimant: &'a Option<String>,
    claim_incarnation: &'a Option<String>,
    submitted_incarnation: &'a Option<String>,
    claim_expires_at_unix_ms: Option<u128>,
    readiness_epoch: u32,
    worker_reported: bool,
    created_at_unix_ms: u128,
    updated_at_unix_ms: u128,
    clock: ServiceWorkClock,
}

#[derive(serde::Serialize)]
struct ServiceWorkClock {
    started_at_unix_ms: Option<u128>,
    paused_until_unix_ms: Option<u128>,
    waited_ms: u128,
    elapsed_ms: u128,
}

impl<'a> From<&'a StepRunView> for ServiceWork<'a> {
    fn from(step: &'a StepRunView) -> Self {
        Self {
            subject: &step.subject,
            run: &step.run,
            generation: &step.generation,
            step: &step.step,
            attempt: step.attempt,
            status: &step.status,
            claimant: &step.claimant,
            claim_incarnation: &step.claim_incarnation,
            submitted_incarnation: &step.submitted_incarnation,
            claim_expires_at_unix_ms: step.claim_expires_at_unix_ms,
            readiness_epoch: step.readiness_epoch,
            worker_reported: step.worker_reported,
            created_at_unix_ms: step.created_at_unix_ms,
            updated_at_unix_ms: step.updated_at_unix_ms,
            clock: ServiceWorkClock {
                started_at_unix_ms: step.execution_started_at_unix_ms,
                paused_until_unix_ms: step.execution_paused_until_unix_ms,
                waited_ms: step.execution_waited_ms,
                elapsed_ms: step.execution_elapsed_ms,
            },
        }
    }
}

pub async fn get_work(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
    session: Option<Extension<ClientSession>>,
) -> Result<Response, ApiError> {
    let service = session.as_ref().is_some_and(|session| client_v0::external_grants::is_service(&session.0));
    if let Some(session) = &session
        && service {
        require_scope(&session.0, "read.work")?;
    }
    let subject = if subject.starts_with("step-run/") {
        subject
    } else {
        format!("step-run/{subject}")
    };
    let mut step = state
        .store
        .step_run(&subject)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("step run `{subject}` does not exist")))?;
    // Services observe work without acknowledging wakes or changing delivery state.
    if !service {
        super::message_reads::read_work_wakes(&state, std::iter::once(&mut step))
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(if service {
        Json(ServiceWork::from(&step)).into_response()
    } else {
        Json(step).into_response()
    })
}

pub async fn client_get_work(
    state: State<AppState>,
    subject: AxumPath<String>,
    session: Extension<ClientSession>,
) -> Result<Response, ApiError> {
    if !client_v0::external_grants::is_service(&session.0) {
        return Err(client_v0::forbidden("client work reads require a service principal"));
    }
    get_work(state, subject, Some(session)).await
}

pub async fn wake_work(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
    Json(request): Json<WorkWakeRequest>,
) -> Result<Json<MessageView>, ApiError> {
    if request.reason.trim().is_empty() {
        return Err(ApiError::bad(FractalError::new(
            "missing-wake-reason",
            "a manual work wake needs a reason",
        )));
    }
    let step = state
        .store
        .step_run_fenced(&subject)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("step run `{subject}` does not exist")))?;
    let agent = step.assigned_to.as_deref().ok_or_else(|| {
        ApiError::bad(FractalError::new(
            "work-has-no-assignee",
            format!("step run `{}` has no exact assignee to wake", step.subject),
        ))
    })?;
    let allowed = request
        .actor
        .strip_prefix("person/")
        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
        || normalized_agent_actor(&request.actor).is_some();
    if !allowed {
        return Err(ApiError::bad(FractalError::new(
            "invalid-wake-actor",
            format!("`{}` is neither a person nor an agent", request.actor),
        )));
    }
    if let Some(existing) = state
        .store
        .operation_claim(&request.idempotency_key)
        .map_err(ApiError::internal)?
    {
        let message = state
            .store
            .message(&existing.subject)
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal("the wake operation message is unavailable"))?;
        return Ok(Json(message));
    }
    if step.status != "ready" {
        return Err(ApiError::bad(FractalError::new(
            "work-not-ready",
            format!(
                "step run `{}` is `{}`, not ready",
                step.subject, step.status
            ),
        )));
    }
    let harness = state
        .store
        .current_harness(agent)
        .map_err(ApiError::internal)?
        .filter(crate::model::CurrentHarnessView::is_ready)
        .ok_or_else(|| {
            ApiError::bad(FractalError::new(
                "assignee-harness-not-ready",
                format!("assignee `{agent}` has no ready current harness"),
            ))
        })?;
    if !state
        .store
        .fresh_context_ready(&step, agent, &harness.incarnation_id)
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::bad(FractalError::new(
            "fresh-context-pending",
            format!(
                "`{agent}` must start a fresh harness session before waking `{}`",
                step.subject
            ),
        )));
    }
    let attempt = step
        .wake
        .as_ref()
        .map(|wake| wake.attempts)
        .unwrap_or_default()
        .saturating_add(1);
    let message = crate::reconcile::append_work_wake_message(
        &state.store,
        &step,
        agent,
        &harness.incarnation_id,
        attempt,
        "manual",
        &request.actor,
        &request.reason,
        request.idempotency_key,
    )
    .map_err(ApiError::internal)?;
    signal_changed(&state);
    Ok(Json(message))
}

/// Retry one failed step as a person or an agent.
pub async fn retry_work(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
    Json(request): Json<WorkRetryRequest>,
) -> Result<Json<MissionRunView>, ApiError> {
    if request.reason.trim().is_empty() {
        return Err(ApiError::bad(FractalError::new(
            "missing-retry-reason",
            "a work retry needs a reason",
        )));
    }
    if let Some(cached) = state
        .store
        .cached_idempotency_response::<MissionRunView>(&request.idempotency_key)
        .map_err(ApiError::internal)?
    {
        let mut cached = cached;
        runs::present_mission_run(&state.store, &mut cached).map_err(ApiError::bad)?;
        return Ok(Json(cached));
    }
    let actor = match request.actor.as_str() {
        actor if actor.starts_with("person/") => actor.to_owned(),
        actor => normalized_agent_actor(actor).ok_or_else(|| {
            ApiError::bad(FractalError::new(
                "retry-authority-denied",
                "a work retry needs a person or agent actor",
            ))
        })?,
    };
    state
        .store
        .step_run_fenced(&subject)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("step run `{subject}` does not exist")))?;
    let store = state.store.clone();
    let retried = blocking_action(move || {
        store.retry_failed_step(&subject, &actor, &request.reason, &request.idempotency_key)
    })
    .await?;
    let mut retried = retried;
    runs::present_mission_run(&state.store, &mut retried).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(retried))
}

pub async fn publish_work_mission(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
    bound: Option<Extension<BoundAgent>>,
    Json(mut request): Json<MissionProductionRequest>,
) -> Result<Json<MissionOutputView>, ApiError> {
    bind_work_incarnation(
        &state,
        bound.as_ref(),
        Some(&request.actor),
        &mut request.incarnation,
    )?;
    let step = state
        .store
        .step_run_fenced(&subject)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("step run `{subject}` does not exist")))?;
    let actor = if request.actor.contains('/') {
        request.actor.clone()
    } else {
        format!("agent/{}", request.actor)
    };
    if !state
        .store
        .mission_output_authorized(&step.subject, &actor, request.incarnation.as_deref())
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::bad(FractalError::new(
            "work-not-claimed",
            format!(
                "`{actor}` does not hold the mission-producing step `{}` or its nested work",
                step.subject
            ),
        )));
    }
    let run = state
        .store
        .mission_run(&step.run)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("mission run `{}` does not exist", step.run)))?;
    let root = state
        .store
        .mission_spec(
            run.mission.strip_prefix("mission/").unwrap_or(&run.mission),
            Some(&run.revision),
        )
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::bad(FractalError::new(
                "missing-mission-revision",
                "the running mission revision is unavailable",
            ))
        })?;
    let definition = crate::mission::find_step(&root, &step.step).ok_or_else(|| {
        ApiError::bad(FractalError::new(
            "missing-step-definition",
            format!("step `{}` is absent from its mission revision", step.step),
        ))
    })?;
    let expected_mission = definition.produces_mission.as_deref().ok_or_else(|| {
        ApiError::bad(FractalError::new(
            "step-does-not-produce-mission",
            format!("step `{}` does not declare produces-mission", step.step),
        ))
    })?;

    let initial = parse_intent(&request.intent.kdl, &state.node).map_err(ApiError::bad)?;
    if initial.missions.len() != 1 {
        return Err(ApiError::bad(FractalError::new(
            "invalid-mission-output-intent",
            "a mission output must contain exactly one mission",
        )));
    }
    if !mission_only_subjects(&initial) {
        return Err(ApiError::bad(FractalError::new(
            "invalid-mission-output-intent",
            "a mission output can contain only its mission",
        )));
    }
    let bindings = state
        .store
        .document_bindings_at(&initial.document_refs, None)
        .map_err(ApiError::internal)?;
    let resolved_kdl =
        resolve_document_references(&request.intent.kdl, &bindings).map_err(ApiError::bad)?;
    let intent = parse_intent(&resolved_kdl, &state.node).map_err(ApiError::bad)?;
    if !mission_only_subjects(&intent) {
        return Err(ApiError::bad(FractalError::new(
            "invalid-mission-output-intent",
            "a mission output can contain only its mission",
        )));
    }
    let mission = intent
        .missions
        .values()
        .next()
        .expect("one mission was checked");
    if mission.id != expected_mission || mission.state != crate::model::MissionState::Ready {
        return Err(ApiError::bad(FractalError::new(
            "wrong-mission-output",
            format!(
                "step `{}` must publish ready mission `{expected_mission}`",
                step.step
            ),
        )));
    }
    let publication = &intent;
    let mut planned = state
        .store
        .mission(
            publication,
            crate::model::IntentInput {
                kdl: resolved_kdl,
                source_name: request.intent.source_name,
            },
        )
        .map_err(ApiError::bad)?;
    publication_refusals(&state, publication)
        .await?
        .block(&mut planned);
    if !planned.blockers.is_empty() {
        return Err(ApiError::bad(FractalError::new(
            "mission-output-blocked",
            planned.blockers.join("; "),
        )));
    }
    state
        .store
        .apply(
            publication,
            &planned.subject_tokens,
            &format!("{}:publish", request.idempotency_key),
        )
        .map_err(ApiError::bad)?;
    let output = state
        .store
        .record_mission_output(
            &step.subject,
            &actor,
            request.incarnation.as_deref(),
            expected_mission,
            mission,
            &format!("{}:bind", request.idempotency_key),
        )
        .map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(output))
}

/// Move one run in a seat's queue as a person or an agent.
pub async fn move_agent_queue(
    State(state): State<AppState>,
    Json(mut request): Json<crate::model::SeatQueueMoveRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    request.agent = if request.agent.starts_with("agent/") {
        request.agent
    } else {
        format!("agent/{}", request.agent)
    };
    request.actor = match request.actor.as_str() {
        actor if actor.starts_with("person/") => actor.to_owned(),
        actor => normalized_agent_actor(actor).ok_or_else(|| {
            ApiError::bad(FractalError::new(
                "invalid-queue-move-actor",
                "a queue move needs a person or agent actor",
            ))
        })?,
    };
    let store = state.store.clone();
    let claim = blocking_action(move || store.move_seat_queue_run(&request)).await?;
    signal_changed(&state);
    Ok(Json(claim))
}

pub async fn start_work(
    State(state): State<AppState>,
    Json(request): Json<crate::model::WorkStartRequest>,
) -> Result<Json<StepRunView>, ApiError> {
    let store = state.store.clone();
    let result = blocking_action(move || store.start_work(&request)).await?;
    signal_changed(&state);
    Ok(Json(result))
}

pub async fn handoff_work(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
    bound: Option<Extension<BoundAgent>>,
    Json(mut input): Json<crate::model::WorkHandoffRequest>,
) -> Result<Json<StepRunView>, ApiError> {
    bind_work_incarnation(
        &state,
        bound.as_ref(),
        Some(&input.actor),
        &mut input.incarnation,
    )?;
    let request = WorkRequest {
        wait_ms: None,
        actor: Some(input.actor.clone()),
        incarnation: input.incarnation.clone(),
        summary: Some(input.note.clone()),
        reason: None,
        evidence: input.evidence.clone(),
        idempotency_key: input.idempotency_key.clone(),
    };
    work_action_response(state, "release".into(), subject, request, None, Some(input)).await
}

pub async fn acknowledge_work(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
    Json(request): Json<crate::model::WorkAcknowledgeRequest>,
) -> Result<Json<StepRunView>, ApiError> {
    let store = state.store.clone();
    let result = blocking_action(move || store.acknowledge_work(&subject, &request)).await?;
    signal_changed(&state);
    Ok(Json(result))
}

/// `?view=lease` asks a renewal to answer with the step's row view alone. The native driver
/// renews each held step every minute and reads nothing from the answer, so folding the step's
/// timing, summaries and wake for it only costs reads.
#[derive(Debug, Default, Deserialize)]
pub struct WorkActionQuery {
    #[serde(default)]
    pub view: Option<String>,
}

impl WorkActionQuery {
    pub(crate) fn lease_view(&self, action: &str) -> Result<bool, FractalError> {
        match self.view.as_deref() {
            None => Ok(false),
            Some("lease") if action == "renew" => Ok(true),
            Some(view) => Err(FractalError::new(
                "invalid-work-view",
                format!("`{view}` is not a view of `{action}`; only a renewal answers with `lease`"),
            )),
        }
    }
}

pub async fn post_work_action(
    State(state): State<AppState>,
    AxumPath((action, subject)): AxumPath<(String, String)>,
    Query(query): Query<WorkActionQuery>,
    bound: Option<Extension<BoundAgent>>,
    Json(mut request): Json<WorkRequest>,
) -> Result<Json<StepRunView>, ApiError> {
    let lease_view = query.lease_view(&action).map_err(ApiError::bad)?;
    bind_work_incarnation(
        &state,
        bound.as_ref(),
        request.actor.as_deref(),
        &mut request.incarnation,
    )?;
    if request.actor.as_deref().is_some_and(|actor| actor.starts_with("exec/")) {
        return exec_clock_response(state, action, subject, request).await;
    }
    work_action_response_viewing(state, action, subject, request, None, None, lease_view).await
}

/// `work wait` and `work resume` from a step's own exec, which holds no claim. The exec's launch
/// is its fence, as an incarnation is a seat's: the request names the start operation its runtime
/// put in its environment (`ST3_START_OPERATION`) or its generation, and is refused unless that is
/// the exec's live generation on this host. The fence refuses earlier launches, other attempts and
/// other hosts; like a seat's incarnation it is not a secret that authenticates the caller.
async fn exec_clock_response(
    state: AppState,
    action: String,
    subject: String,
    request: WorkRequest,
) -> Result<Json<StepRunView>, ApiError> {
    let Some(launch) = request.incarnation.clone().filter(|launch| !launch.is_empty()) else {
        return Err(ApiError::bad(FractalError::new(
            "missing-work-incarnation",
            "an exec's work action names its launch: the ST3_START_OPERATION its runtime gave it",
        )));
    };
    let store = state.store.clone();
    let runtime = fractal_runtime::ExecRuntime::new(state.state_dir.join("exec"), state.state_dir.join("logs"));
    let node = state.node.clone();
    let response = blocking_action(move || {
        let read = |error: anyhow::Error| FractalError::new("store-read-failed", format!("{error:#}"));
        let exec = request.actor.as_deref().unwrap_or_default();
        // An exact retry answers from its record, even after the exec has exited.
        if store.cached_idempotency_response::<StepRunView>(&request.idempotency_key).map_err(read)?.is_none() {
            let generation = store
                .desired_subject_with_writer(exec)
                .map_err(read)?
                .and_then(|(desired, _)| desired.member)
                .filter(|member| !member.terminal && member.host == node)
                .map(|member| runtime.observe(&member.runtime_id))
                .transpose()
                .map_err(read)?
                .flatten();
            let live = matches!(&generation, Some(fractal_runtime::ExecObservation::Running(generation))
                if generation.start_operation.as_deref() == Some(launch.as_str()) || generation.generation_id == launch);
            if !live {
                return Err(FractalError::new(
                    "inactive-work-incarnation",
                    format!("`{launch}` is not the live launch of `{exec}` on this host"),
                ));
            }
        }
        store.work_action_answering(&subject, &action, &request, None, None, true)
    })
    .await?;
    signal_changed(&state);
    Ok(Json(response))
}

/// Add time to the execution budget of the step attempt this seat holds.
pub async fn extend_work(
    State(state): State<AppState>,
    AxumPath(subject): AxumPath<String>,
    bound: Option<Extension<BoundAgent>>,
    Json(mut request): Json<crate::model::WorkExtendRequest>,
) -> Result<Json<StepRunView>, ApiError> {
    bind_work_incarnation(
        &state,
        bound.as_ref(),
        request.actor.as_deref(),
        &mut request.incarnation,
    )?;
    let extend_ms = request.by_ms;
    let request = WorkRequest {
        wait_ms: None,
        actor: request.actor,
        incarnation: request.incarnation,
        summary: None,
        reason: request.reason,
        evidence: Vec::new(),
        idempotency_key: request.idempotency_key,
    };
    work_action_response(state, "extend".into(), subject, request, Some(extend_ms), None).await
}

pub async fn post_gate_result(
    State(state): State<AppState>,
    Json(request): Json<GateResultRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    if !matches!(request.verdict.as_str(), "pass" | "fail") {
        return Err(ApiError::bad(FractalError::new(
            "invalid-gate-result",
            "a gate-result verdict must be pass or fail",
        )));
    }
    let capability = state
        .store
        .capability(&request.operation_capability, "gate-result")
        .map_err(ApiError::bad)?;
    if capability.used {
        let prior = state
            .store
            .latest_claim(&capability.subject, Some("gate.result"))
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::bad(FractalError::new(
                    "used-capability",
                    "the gate-result capability was already consumed",
                ))
            })?;
        let same = prior
            .body
            .pointer("/fields/verdict")
            .and_then(Value::as_str)
            == Some(request.verdict.as_str())
            && prior.body.pointer("/fields/reason").and_then(Value::as_str)
                == Some(request.reason.as_str());
        if same {
            return Ok(Json(prior));
        }
        return Err(ApiError::bad(FractalError::new(
            "used-capability",
            "the gate-result capability was already used for another verdict",
        )));
    }
    let input = ClaimInput {
        subject: capability.subject.clone(),
        kind: "gate.result".into(),
        actor: None,
        fields: BTreeMap::from([
            ("verdict".into(), Value::String(request.verdict.clone())),
            ("reason".into(), Value::String(request.reason.clone())),
        ]),
        evidence: request.evidence.clone(),
        expected_subject: None,
        idempotency_key: Some(request.idempotency_key.clone()),
    };
    state
        .store
        .validate_claim_input(&input)
        .map_err(ApiError::bad)?;
    for evidence in &input.evidence {
        if !state
            .store
            .evidence_exists(evidence)
            .map_err(ApiError::internal)?
        {
            return Err(ApiError::bad(FractalError::new(
                "missing-evidence",
                format!("evidence claim `{evidence}` is not stored"),
            )));
        }
    }
    let capability = state
        .store
        .consume_capability(&request.operation_capability, "gate-result")
        .map_err(ApiError::bad)?;
    if capability.used {
        let prior = state
            .store
            .latest_claim(&capability.subject, Some("gate.result"))
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::bad(FractalError::new(
                    "used-capability",
                    "the gate-result capability was already consumed",
                ))
            })?;
        let same = prior
            .body
            .pointer("/fields/verdict")
            .and_then(Value::as_str)
            == Some(request.verdict.as_str())
            && prior.body.pointer("/fields/reason").and_then(Value::as_str)
                == Some(request.reason.as_str());
        if same {
            return Ok(Json(prior));
        }
        return Err(ApiError::bad(FractalError::new(
            "used-capability",
            "the gate-result capability was already used for another verdict",
        )));
    }
    let response = state.store.append_claim(&input).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(response))
}
