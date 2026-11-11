use super::*;
use fractal_jobs::stop_intent::{self, StopDecision, StopInput, StopOrigin, StopRefusal};

/// Axum catch-all captures the full slash-bearing job ID and the `/cancel` suffix.
/// The only stop origin exposed here is the daemon-authenticated session actor.
pub async fn cancel(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    config: Option<Extension<JobsConfig>>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    enabled(config.as_ref().map(|config| &config.0))?;
    client_v0::require_scope(&session, "control.work")?;
    let id = path.strip_suffix("/cancel")
        .ok_or_else(|| refused("not-found", StatusCode::NOT_FOUND))?;
    let (row, head) = read(&state, |tx| {
        Ok((projection::job_by_id(tx, id)?, smallclaims::store::latest_claim_id_tx(tx, id)?))
    })?;
    let row = row.ok_or_else(|| refused("not-found", StatusCode::NOT_FOUND))?;
    cancel_snapshot(&state, &session, id, row, head)
}

/// The authorization decision and CAS must refer to the same captured job head.
pub(super) fn cancel_snapshot(
    state: &AppState,
    session: &client_v0::ClientSession,
    id: &str,
    row: JobRow,
    head: Option<String>,
) -> Result<Json<Value>, ApiError> {
    let decision = stop_intent::decide(
        row.state,
        &row.requester,
        StopOrigin::Requester { actor: Some(&session.actor) },
        StopInput::Cancel,
    ).map_err(|reason| {
        let status = match reason {
            StopRefusal::RequesterUnresolved | StopRefusal::NotRequester => StatusCode::FORBIDDEN,
            StopRefusal::JobTerminal | StopRefusal::JobStateUncertain => StatusCode::CONFLICT,
        };
        refused(reason.code(), status)
    })?;
    if let StopDecision::Record { event, reason } = decision {
        let record = JobRecord::Transition { binding: row.binding, event };
        state.store.append_claim(&ClaimInput {
            subject: id.into(),
            kind: projection::KIND.into(),
            actor: Some(session.actor.clone()),
            fields: BTreeMap::from([
                ("record".into(), serde_json::to_value(record).map_err(ApiError::internal)?),
                ("reason".into(), serde_json::to_value(reason).map_err(ApiError::internal)?),
            ]),
            evidence: vec![],
            expected_subject: Some(head),
            idempotency_key: None,
        }).map_err(|error| {
            if error.code == "stale-subject" {
                refused("job-state-changed", StatusCode::CONFLICT)
            } else {
                ApiError::bad(error)
            }
        })?;
    }
    Ok(response(own_job(state, &session.actor, id)?))
}
