//! Prototype: daemon-owned, durable approval asks; adapter holds only an assistant bearer.
use super::*;

pub(in crate::api) async fn propose(State(state): State<AppState>, Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<ClientSession>, Json(request): Json<ActionRequest>) -> Result<Json<Value>, ApiError> {
    if !matches!(request.action_type.as_str(), "work.done" | "message.send" | "custom.reply") {
        return Err(forbidden("only typed person answers, custom replies and message sends may be proposed"));
    }
    let required_scope = match request.action_type.as_str() {
        "message.send" => "write.assistant.messages", "custom.reply" => "write.assistant.custom-reply",
        _ => "read.assistant.attention",
    };
    if !session.allows(required_scope) {
        return Err(forbidden("proposal scope is absent"));
    }
    let grant_subject = session.pairing_grant.as_deref().ok_or_else(|| forbidden("assistant grant absent"))?;
    let _guard = TRANSITIONS.lock().await;
    let granted = latest(&state,grant_subject,GRANTED)?.ok_or_else(|| forbidden("assistant grant absent"))?;
    let current_session = super::session(&state,&granted,session.transport)?;
    if !current_session.allows(required_scope) {
        return Err(forbidden("proposal scope is absent"));
    }
    let owner = owning_person(&state)?.to_owned();
    if matches!(request.action_type.as_str(), "message.send" | "custom.reply") {
        let policy = effective(&state,&granted)?;
        if request.action_type == "message.send" {
            let to = parameter_string(&request.parameters,"to")?;
            if !fields(&policy)?["recipient_allowlist"].as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(&to))) {
                return Err(forbidden("message recipient is not owner-allowlisted"));
            }
        }
        let mut owner_session = ClientSession::local(Some(&owner))?;
        owner_session.actor = session.actor.clone();
        let result = super::super::action(State(state.clone()),Extension(snapshot),Extension(owner_session),Json(request)).await;
        audit(&state,"grant-authorized-write",&session.actor,session.transport,fields(&granted)?["grant_id"].as_str(),&result)?;
        return result;
    }
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(&json!({"grant":grant_subject,"request":request})).map_err(ApiError::internal)?));
    let subject = format!("step-run/chatgpt-{digest}/ask");
    state.store.ask_daemon_approval(&owner,&digest,serde_json::to_value(&request).map_err(ApiError::internal)?)
        .map_err(ApiError::bad)?;
    signal_changed(&state);
    let ask = state.store.step_run(&subject).map_err(ApiError::internal)?.ok_or_else(|| ApiError::internal("approval ask absent"))?;
    let approved = ask.person_answers.iter().any(|answer| answer.respondent == owner && answer.acted_for.is_none()
        && answer.status == "completed" && answer.answer.as_ref().is_some_and(|value| value["id"] == "approve"));
    if !approved {
        return Ok(Json(json!({"status":if ask.person_answers.is_empty() {"approval-required"} else {"approval-denied"},"approval_id":subject,"request_digest":digest})));
    }
    // Execute via the same fenced, idempotent typed action path as the owner's CLI/UI.
    let mut owner_session = ClientSession::local(Some(&owner))?;
    owner_session.actor = session.actor.clone();
    let result = super::super::action(State(state.clone()),Extension(snapshot),Extension(owner_session),Json(request)).await;
    audit(&state,"approved-write",&session.actor,session.transport,fields(&granted)?["grant_id"].as_str(),&result)?;
    result
}
