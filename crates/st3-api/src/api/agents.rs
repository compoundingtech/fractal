use super::*;

#[derive(Deserialize)]
pub struct AgentRolloutRequest {
    subject: String,
    actor: String,
    expected_desired: String,
    expected_incarnation: String,
    policy: crate::rollout::Policy,
    idempotency_key: String,
}

pub async fn rollout_agent(
    State(state): State<AppState>,
    Json(request): Json<AgentRolloutRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    let actor = person_or_agent_actor(&request.actor, "invalid-rollout-actor")?;
    let subject = agent_subject(request.subject);
    let old =
        crate::rollout::launched_member(&state.store, &subject, &request.expected_incarnation)
            .map_err(ApiError::internal)?
            .map(|(_, member)| member)
            .or_else(|| {
                state
                    .store
                    .rollout(&subject)
                    .ok()
                    .flatten()
                    .filter(|o| o.old_incarnation == request.expected_incarnation)
                    .map(|o| o.old_member)
            })
            .ok_or_else(|| {
                ApiError::bad(St3Error::new(
                    "rollout-launch-unknown",
                    "the named incarnation has no original launch receipt",
                ))
            })?;
    let response = state
        .store
        .request_rollout(
            &subject,
            &request.expected_desired,
            &old,
            &request.expected_incarnation,
            &actor,
            &request.policy,
            &format!("seat-rollout-retry:{subject}:{}", request.idempotency_key),
        )
        .map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct AgentRestartRequest {
    pub(super) subject: String,
    pub(super) actor: String,
    pub(super) idempotency_key: String,
    #[serde(default)]
    pub(super) incarnation_id: Option<String>,
    #[serde(default)]
    pub(super) fresh_context: bool,
    #[serde(default)]
    pub(super) desired_revision: Option<String>,
    /// Restart without first asking the seat for its handoff.
    #[serde(default)]
    pub(super) now: bool,
}

pub async fn restart_agent(
    State(state): State<AppState>,
    Json(request): Json<AgentRestartRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    request_agent_restart(&state, request).map(Json)
}

pub(super) fn request_agent_restart(
    state: &AppState,
    request: AgentRestartRequest,
) -> Result<ClaimRecord, ApiError> {
    let actor = person_or_agent_actor(&request.actor, "invalid-restart-actor")?;
    let subject = if request.subject.starts_with("agent/") {
        request.subject
    } else {
        format!("agent/{}", request.subject)
    };
    state.store.owned_member_guard(&subject).map_err(ApiError::bad)?;
    let key = format!("agent-restart:{subject}:{}", request.idempotency_key);
    if let Some(prior) = state
        .store
        .operation_claim(&key)
        .map_err(ApiError::internal)?
    {
        return Ok(prior);
    }
    if state
        .store
        .rollout(&subject)
        .map_err(ApiError::bad)?
        .is_some_and(|o| o.holds_seat())
        || crate::rollout::status(&state.store, &subject)
            .map_err(ApiError::internal)?
            .is_some_and(|s| s["mode"] == "manual" && s["phase"] == "pending")
    {
        return Err(ApiError::bad(St3Error::new(
            "rollout-in-progress",
            "the seat has a rollout; inspect it or retry with st agents rollout",
        )));
    }
    if crate::suspension::current(&state.store, &subject)
        .map_err(ApiError::internal)?
        .is_some_and(|suspension| suspension.holds_seat())
    {
        return Err(ApiError::bad(St3Error::new(
            "restart-suspended",
            "the seat is suspended; resume it with `st agents resume`. If resume refuses an authored session selection, use `st agents stop` then `st agents start`; this ends the suspension and launches the declaration without preserving the suspended session",
        )));
    }
    let status = state
        .store
        .status(Some(&subject))
        .map_err(ApiError::internal)?;
    let current = status
        .subjects
        .iter()
        .find(|item| item.subject == subject)
        .ok_or_else(|| {
            ApiError::bad(St3Error::new(
                "missing-agent",
                format!("no seat `{subject}`"),
            ))
        })?;
    if !current.conflicts.is_empty() {
        return Err(ApiError::bad(St3Error::new(
            "restart-conflict",
            "resolve the seat's conflicting declarations before restarting",
        )));
    }
    let desired = state
        .store
        .desired_subject_with_writer(&subject)
        .map_err(ApiError::internal)?
        .map(|(desired, _)| desired)
        .filter(|desired| desired.kind == "agent")
        .ok_or_else(|| {
            ApiError::bad(St3Error::new(
                "restart-not-declared",
                "restart needs an active seat declaration; start a stopped seat first",
            ))
        })?;
    let member = desired
        .member
        .as_ref()
        .filter(|member| member.lifecycle == crate::model::MemberLifecycle::Service)
        .ok_or_else(|| {
            ApiError::bad(St3Error::new(
                "restart-no-launch",
                "the seat has no readable service launch declaration",
            ))
        })?;
    let token = current.desired_token.clone().ok_or_else(|| {
        ApiError::bad(St3Error::new(
            "restart-not-declared",
            "the seat has no selected declaration",
        ))
    })?;
    let incarnation = current
        .actual
        .as_ref()
        .map(|actual| actual.get("fields").unwrap_or(actual))
        .and_then(|fields| fields.get("incarnation_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if request.incarnation_id.as_deref().is_some_and(|expected| expected != incarnation) {
        return Err(ApiError::bad(St3Error::new(
            "stale-incarnation",
            "the seat's runtime incarnation changed",
        )));
    }
    if request.desired_revision.as_deref().is_some_and(|expected| expected != token) {
        return Err(ApiError::bad(St3Error::new(
            "stale-revision",
            "the seat's desired revision changed",
        )));
    }
    let native_session = if request.fresh_context {
        None
    } else {
        restart_native_session(
            &state.store, &subject, incarnation, member.driver.as_deref(), None,
        ).map_err(ApiError::internal)?
    };
    let claim = state
        .store
        .append_claim(&ClaimInput {
            subject,
            kind: "runtime.action.requested".into(),
            actor: Some(actor),
            fields: BTreeMap::from([
                ("action".into(), Value::String("restart".into())),
                (
                    "runtime_id".into(),
                    Value::String(member.runtime_id.clone()),
                ),
                ("incarnation_id".into(), Value::String(incarnation.into())),
                ("native_session_id".into(), json!(native_session.as_ref().map(|(id, _)| id))),
                ("native_session_path".into(), json!(native_session.as_ref().and_then(|(_, path)| path.as_deref()))),
                ("fresh_context".into(), Value::Bool(request.fresh_context)),
            ])
            .into_iter()
            .chain(request.now.then(|| {
                (
                    "operation".to_owned(),
                    Value::String(crate::handoff::RESTART_NOW_OPERATION.into()),
                )
            }))
            .collect(),
            evidence: vec![token],
            expected_subject: None,
            idempotency_key: Some(key),
        })
        .map_err(ApiError::bad)?;
    signal_changed(state);
    Ok(claim)
}

/// Only evidence bound to this exact incarnation and supported launch driver can resume.
pub(super) fn restart_native_session(
    store: &Store,
    subject: &str,
    incarnation: &str,
    driver: Option<&str>,
    snapshot_index: Option<u64>,
) -> anyhow::Result<Option<(String, Option<String>)>> {
    let Some(driver @ ("claude" | "codex" | "omp" | "pi" | "opencode")) = driver else {
        return Ok(None);
    };
    let mut before = snapshot_index.and_then(|index| index.checked_add(1));
    loop {
        let page = store.claims_for_subject_kind_at(
            subject, "harness.session-file", before, true, 64,
        )?;
        for claim in page.claims {
            let fields = claim.body.get("fields").unwrap_or(&claim.body);
            if fields.get("incarnation_id").and_then(Value::as_str) == Some(incarnation)
                && fields.get("harness").and_then(Value::as_str) == Some(driver)
            {
                if fields.get("resume_available").and_then(Value::as_bool) != Some(true) {
                    return Ok(None);
                }
                let Some(id) = fields.get("session_id").and_then(Value::as_str)
                    .filter(|id| !id.is_empty()) else { return Ok(None) };
                let path = fields.get("path").and_then(Value::as_str)
                    .filter(|path| !path.is_empty()).map(str::to_owned);
                if driver == "pi" && path.is_none() {
                    return Ok(None);
                }
                return Ok(Some((id.to_owned(), path)));
            }
        }
        let Some(cursor) = page.next_cursor else { return Ok(None) };
        before = Some(cursor);
    }
}

#[derive(Deserialize)]
pub struct MissionSeatStartRequest {
    subject: String,
    actor: String,
    /// The selected stop the caller read, when it read one.
    #[serde(default)]
    expected: Option<String>,
    idempotency_key: String,
}

/// Record an operator's source-offline exception for one exact placement.
pub async fn override_placement_source(
    State(state): State<AppState>,
    Json(mut request): Json<crate::placement::SourceOfflineRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    request.actor = person_or_agent_actor(&request.actor, "invalid-source-offline-actor")?;
    request.subject = agent_subject(request.subject);
    let store = state.store.clone();
    let (claim, appended) = blocking_action(move || {
        let input = crate::placement::source_offline_input(&store, request)?;
        store.append_claim_outcome(&input)
    }).await?;
    if appended { signal_claim_changed(&state, crate::placement::SOURCE_OFFLINE_KIND); }
    Ok(Json(claim))
}

/// Start a stopped mission seat on the declaration its run gave it, as `st agents start` does.
pub async fn start_mission_seat(
    State(state): State<AppState>,
    Json(request): Json<MissionSeatStartRequest>,
) -> Result<Json<ApplyResponse>, ApiError> {
    let actor = person_or_agent_actor(&request.actor, "invalid-start-actor")?;
    let subject = agent_subject(request.subject);
    let response = state
        .store
        .start_mission_seat(
            &subject,
            request.expected.as_deref(),
            &actor,
            &format!("agent-start:{subject}:{}", request.idempotency_key),
        )
        .map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(response))
}

/// Ask the seat's owner to suspend it. The seat must be running and quiet now: its driver reports
/// the harness quiescent, it holds no claimed step and runs no subagent, and its driver has bound
/// a native session to resume. The owner checks again before it stops anything.
pub async fn suspend_agent(
    State(state): State<AppState>,
    Json(request): Json<AgentSuspensionRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    request_suspend(&state, request, None).map(Json)
}

#[derive(Deserialize, Serialize)]
pub struct SnapshotReadRequest {
    pub subject: String,
    pub suspend_operation: String,
    pub resume_operation: String,
    pub actor: String,
    pub offset: u64,
}

pub async fn seat_snapshot_chunk(
    State(state): State<AppState>,
    Json(request): Json<SnapshotReadRequest>,
) -> Result<Json<Value>, ApiError> {
    person_or_agent_actor(&request.actor, "invalid-resume-actor")?;
    let suspension = crate::suspension::current(&state.store, &request.subject)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("no suspension"))?;
    if suspension.operation_id != request.resume_operation
        || suspension.suspend_operation_id.as_deref() != Some(&request.suspend_operation)
        || suspension.source_host.as_deref() != Some(&state.node)
        || suspension.requested_by.as_deref() != Some(&request.actor)
        || !matches!(suspension.phase.as_str(), "fencing-source" | "transferring")
    {
        return Err(ApiError::bad(St3Error::new(
            "stale-fence",
            "snapshot read does not name the current move",
        )));
    }
    let completed = state
        .store
        .operation_claim(&crate::suspension::suspend_completed_key(
            &request.suspend_operation,
        ))
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("source is not suspended"))?;
    if completed.origin != state.node {
        return Err(ApiError::bad(St3Error::new(
            "stale-fence",
            "snapshot belongs to another source",
        )));
    }
    let sources = crate::placement::latest_by_origin(&state.store, &request.subject, u64::MAX)
        .map_err(ApiError::internal)?;
    if !sources
        .get(&state.node)
        .is_some_and(|claim| crate::placement::field(claim, "status") == Some("stopped"))
    {
        return Err(ApiError::bad(St3Error::new(
            "stale-fence",
            "source runtime has not acknowledged its stop",
        )));
    }
    crate::seat_snapshot::chunk(&state.state_dir, &request.suspend_operation, request.offset)
        .map(Json)
        .map_err(ApiError::internal)
}

/// Ask the seat's owner to resume a suspended seat on the native session it suspended.
pub async fn resume_agent(
    State(state): State<AppState>,
    Json(request): Json<AgentSuspensionRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    request_resume(&state, request, None).map(Json)
}

#[derive(Deserialize)]
pub struct NativeSessionReport {
    #[serde(default)]
    account_ref: Option<String>,
    subject: String,
    actor: String,
    incarnation_id: String,
    harness: String,
    session_id: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    desired_token: Option<String>,
    #[serde(default)]
    resume_available: Option<bool>,
    #[serde(default)]
    idempotency_key: Option<String>,
}

/// A seat's driver reports the native session its harness bound for one incarnation. Only the
/// seat itself reports its own session.
pub async fn report_native_session(
    State(state): State<AppState>,
    Json(request): Json<NativeSessionReport>,
) -> Result<Json<ClaimRecord>, ApiError> {
    let subject = agent_subject(request.subject);
    if request.actor != subject {
        return Err(ApiError::bad(St3Error::new(
            "foreign-agent-actor",
            "a seat reports only its own native session",
        )));
    }
    if request.session_id.is_empty() || request.incarnation_id.is_empty()
        || !matches!(request.harness.as_str(), "claude" | "codex" | "omp" | "pi" | "opencode")
    {
        return Err(ApiError::bad(St3Error::new(
            "invalid-native-session",
            "a native session report needs a supported harness, session ID and an incarnation",
        )));
    }
    let store = state.store.clone();
    let (claim, appended) = blocking_action(move || {
        let internal = |error: anyhow::Error| St3Error::new("internal", error.to_string());
        let status = store.status(Some(&subject)).map_err(internal)?;
        let current = status.subjects.iter().find(|item| item.subject == subject)
            .ok_or_else(|| St3Error::new("missing-agent", "the native report has no current seat"))?;
        let incarnation = current.actual.as_ref()
            .map(|actual| actual.get("fields").unwrap_or(actual))
            .and_then(|fields| fields.get("incarnation_id")).and_then(Value::as_str);
        if incarnation != Some(request.incarnation_id.as_str())
            || current.actual_origin.as_deref() != Some(store.origin())
        {
            return Err(St3Error::new(
                "stale-incarnation", "the native report does not name this host's current incarnation",
            ));
        }
        let desired = store.desired_subject_with_writer(&subject).map_err(internal)?
            .map(|(desired, _)| desired)
            .filter(|desired| desired.kind == "agent")
            .ok_or_else(|| St3Error::new("binding-not-declared", "the seat has no active native declaration"))?;
        if !current.conflicts.is_empty()
            || desired.member.as_ref().and_then(|member| member.driver.as_deref())
                != Some(request.harness.as_str())
        {
            return Err(St3Error::new(
                "binding-driver-mismatch", "the native report must match the seat's declared harness",
            ));
        }
        let mut fields = BTreeMap::from([
            ("harness".into(), json!(request.harness)),
            ("session_id".into(), json!(request.session_id)),
            ("agent".into(), json!(subject)),
            ("incarnation_id".into(), json!(request.incarnation_id)),
            ("status".into(), json!("active")),
            // Existing callers report a bound saved session without an availability field.
            ("resume_available".into(), json!(request.resume_available.unwrap_or(true))),
        ]);
        if let Some(path) = request.path {
            fields.insert("path".into(), json!(path));
        }
        if let Some(account) = request.account_ref.filter(|name| !name.is_empty()) {
            fields.insert("account_ref".into(), json!(account));
        }
        if let Some(token) = request.desired_token.filter(|token| !token.is_empty()) {
            fields.insert("desired_token".into(), json!(token));
        }
        // Availability and selected-session changes are new observations, not per-session replays.
        let key = match request.idempotency_key {
            Some(key) => key,
            None => {
                let latest = store.latest_claim(&subject, Some("harness.session-file"))
                    .map_err(internal)?;
                let previous_id = match latest {
                    Some(claim) => {
                        if claim.body.get("fields").and_then(Value::as_object).is_some_and(|previous| {
                            previous.len() == fields.len()
                                && fields.iter().all(|(key, value)| previous.get(key) == Some(value))
                        }) {
                            return Ok((claim, false));
                        }
                        claim.id
                    },
                    None => String::new(),
                };
                let digest = hex::encode(Sha256::digest(
                    serde_json::to_vec(&fields).map_err(|error| internal(error.into()))?,
                ));
                format!("native-session:{subject}:{previous_id}:{digest}")
            },
        };
        store.append_claim_outcome(&ClaimInput {
            subject: subject.clone(),
            kind: "harness.session-file".into(),
            actor: Some(subject),
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(key),
        })
    }).await?;
    if appended {
        signal_changed(&state);
    }
    Ok(Json(claim))
}

#[derive(Deserialize)]
pub struct AgentRenameRequest {
    subject: String,
    name: Option<String>,
    actor: String,
    idempotency_key: String,
}

pub async fn rename_agent(
    State(state): State<AppState>,
    Json(request): Json<AgentRenameRequest>,
) -> Result<Json<ApplyResponse>, ApiError> {
    let subject = if request.subject.starts_with("agent/") {
        request.subject
    } else {
        format!("agent/{}", request.subject)
    };
    if !request.actor.starts_with("person/") {
        normalized_agent_actor(&request.actor).ok_or_else(|| {
            ApiError::bad(St3Error::new("invalid-rename-actor", "rename needs a person or agent actor"))
        })?;
    }
    let response = state.store.rename_agent(
        &subject,
        request.name.as_deref(),
        &request.idempotency_key,
    ).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(response))
}
