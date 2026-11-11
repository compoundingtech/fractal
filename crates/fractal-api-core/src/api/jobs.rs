//! Recording only: classes and priorities declare intent, never authority or preemption.
//! Even explicitly enabled routes cannot launch work: admission is not wired.
use super::*;
use crate::config::JobsConfig;
use fractal_jobs::{JobPriority, host_authority::JobHostBinding, identity::*};
use fractal_store::jobs::{self as projection, JobRecord, JobRow, JobsProjectionStale};
use smallclaims::fleet::MemberState;

/// Every jobs method is requester-private, including reads and future intent routes.
pub fn requester_scoped_path(path: &str) -> bool {
    path == "/v1/client/jobs" || path.starts_with("/v1/client/jobs/")
}

fn refused(code: &'static str, status: StatusCode) -> ApiError {
    ApiError { status, code: code.into(), message: code.into(), details: Box::default() }
}

fn enabled(config: Option<&JobsConfig>) -> Result<&JobsConfig, ApiError> {
    config.filter(|config| config.enabled && !config.allowed_classes.is_empty())
        .ok_or_else(|| refused("jobs-disabled", StatusCode::SERVICE_UNAVAILABLE))
}

fn read_error(error: anyhow::Error) -> ApiError {
    if error.downcast_ref::<JobsProjectionStale>().is_some() {
        refused("jobs-projection-stale", StatusCode::SERVICE_UNAVAILABLE)
    } else {
        ApiError::internal(error)
    }
}

fn read<T>(state: &AppState, query: impl FnOnce(&rusqlite::Transaction<'_>) -> anyhow::Result<T>) -> Result<T, ApiError> {
    let connection = state.store.readers.get();
    let tx = connection.unchecked_transaction().map_err(ApiError::internal)?;
    query(&tx).map_err(read_error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    key: String,
    execution_host: String,
    request: ExecutionRequest,
}

fn decode(raw: Value, config: &JobsConfig) -> Result<Submission, ApiError> {
    serde_json::from_value::<JobPriority>(raw.pointer("/request/resources/priority").cloned().unwrap_or(Value::Null))
        .map_err(|_| refused("job-priority-unknown", StatusCode::UNPROCESSABLE_ENTITY))?;
    let submission: Submission = serde_json::from_value(raw)
        .map_err(|_| refused("invalid-request", StatusCode::UNPROCESSABLE_ENTITY))?;
    if !config.allowed_classes.contains(&submission.request.resources.requested_class) {
        return Err(refused("job-class-not-allowed", StatusCode::UNPROCESSABLE_ENTITY));
    }
    if submission.key.is_empty() || submission.request.resources.validate().is_err()
        || submission.request.command.validate().is_err() {
        return Err(refused("invalid-request", StatusCode::UNPROCESSABLE_ENTITY));
    }
    Ok(submission)
}

fn receipt(state: &AppState, identity: &IdempotencyKey) -> Result<IdempotencyDecision<String>, ApiError> {
    let existing = read(state, |tx| projection::receipt_for_key(tx, &identity.requester, &identity.key))?;
    decide_idempotency(identity, existing)
        .map_err(|_| refused("idempotency-digest-mismatch", StatusCode::CONFLICT))
}

fn response(job: JobRow) -> Json<Value> {
    Json(json!({"job": job, "admission": "not wired"}))
}

fn own_job(state: &AppState, requester: &str, id: &str) -> Result<JobRow, ApiError> {
    read(state, |tx| projection::job_by_id(tx, id))?
        .filter(|row| row.requester == requester)
        .ok_or_else(|| refused("not-found", StatusCode::NOT_FOUND))
}

pub async fn submit(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    config: Option<Extension<JobsConfig>>,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let config = enabled(config.as_ref().map(|config| &config.0))?;
    client_v0::require_scope(&session, "control.work")?;
    let raw = body.map_err(|_| refused("invalid-request", StatusCode::UNPROCESSABLE_ENTITY))?.0;
    let submission = decode(raw, config)?;
    let identity = IdempotencyKey { requester: session.actor.clone(), key: submission.key,
        request_digest: submission.request.digest() };
    if let IdempotencyDecision::ExistingJob(id) = receipt(&state, &identity)? {
        return Ok(response(own_job(&state, &session.actor, &id)?));
    }
    let id = projection::job_id(&identity.requester, &identity.key);
    let binding = read(&state, |tx| {
        let membership = smallclaims::store::fleet_membership_tx(tx)?;
        let member = match membership.state(&submission.execution_host) {
            MemberState::Current(member) => member,
            MemberState::Conflicted(_) => return Ok(Err(refused("conflicting-host", StatusCode::CONFLICT))),
            MemberState::Ended(_) | MemberState::LegacyRemoved(_) => return Ok(Err(refused("stale-authority", StatusCode::CONFLICT))),
            MemberState::NotMember => return Ok(Err(refused("unknown-host", StatusCode::UNPROCESSABLE_ENTITY))),
        };
        if submission.execution_host != state.node {
            return Ok(Err(refused("origin-mismatch", StatusCode::CONFLICT)));
        }
        Ok(Ok(JobHostBinding {
            job_id: id.clone(),
            execution_host: submission.execution_host.clone(),
            authority_declaration: member.admitted_claim.clone(),
        }))
    })??;
    let record = JobRecord::Submitted { binding, identity: identity.clone(), request: Box::new(submission.request) };
    record_submission(&state, &identity, &id, record)?;
    Ok(response(own_job(&state, &session.actor, &id)?))
}

/// Returns whether this call appended, or resolved the winner of an absent-subject CAS.
fn record_submission(
    state: &AppState,
    identity: &IdempotencyKey,
    id: &str,
    record: JobRecord,
) -> Result<bool, ApiError> {
    let append = state.store.append_claim(&ClaimInput {
        subject: id.into(),
        kind: projection::KIND.into(),
        actor: Some(identity.requester.clone()),
        fields: BTreeMap::from([
            ("record".into(), serde_json::to_value(record).map_err(ApiError::internal)?),
        ]),
        evidence: vec![],
        expected_subject: Some(None),
        idempotency_key: None,
    });
    match append {
        Ok(_) => Ok(true),
        Err(error) if error.code == "stale-subject" => {
            // Resolve the durable winner, never mint another key or job ID.
            match receipt(state, identity)? {
                IdempotencyDecision::ExistingJob(_) => Ok(false),
                IdempotencyDecision::NewJob => Err(ApiError::bad(error)),
            }
        }
        Err(error) => Err(ApiError::bad(error)),
    }
}

pub async fn show(
    State(state): State<AppState>, Extension(session): Extension<client_v0::ClientSession>,
    config: Option<Extension<JobsConfig>>, AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    enabled(config.as_ref().map(|config| &config.0))?;
    client_v0::require_scope(&session, "read.projections")?;
    Ok(response(own_job(&state, &session.actor, &id)?))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    host: Option<String>,
    requester: Option<String>,
}

pub async fn list(
    State(state): State<AppState>, Extension(session): Extension<client_v0::ClientSession>,
    config: Option<Extension<JobsConfig>>, Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    enabled(config.as_ref().map(|config| &config.0))?;
    client_v0::require_scope(&session, "read.projections")?;
    if query.requester.as_deref().is_some_and(|requester| requester != session.actor)
        || query.host.is_some() && query.requester.is_some() {
        return Err(refused("forbidden", StatusCode::FORBIDDEN));
    }
    let rows = read(&state, |tx| match query.host.as_deref() {
        Some(host) => projection::queue_for_host(tx, host),
        None => projection::jobs_for_requester(tx, &session.actor),
    })?;
    let rows: Vec<_> = rows.into_iter().filter(|row| row.requester == session.actor).collect();
    Ok(Json(json!({"jobs": rows, "admission": "not wired"})))
}

#[cfg(test)]
mod tests;
