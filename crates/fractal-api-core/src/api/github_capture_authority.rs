//! `POST /v1/plugins/github/capture/v1/sources/{source}/authority/{root,successor}`
//! and `POST /v1/plugins/github/capture/v1/sources/{source}/fence` are trusted-local
//! operator operations, deliberately outside client-v0. Admission creates a grant, not a
//! declaration: select the returned claim ID in the source declaration to activate it.
//! Successors require the old member's durable stopped frontier. No arbitrary claim writes.
use super::*;
use fractal_schema::github_capture::{self as capture, AuthorityGrant, CaptureCommit, FenceReceipt, Source};
use rusqlite::Connection;
use serde::de::DeserializeOwned;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootRequest {
    pub actor: String,
    pub source: Source,
    pub member: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessorRequest {
    pub actor: String,
    pub source: Source,
    pub member: String,
    pub epoch: u64,
    pub predecessor_grant: String,
    pub fencing_receipt: String,
    pub final_predecessor_commit: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceRequest {
    pub actor: String,
    pub source: Source,
    pub authority_grant: String,
}

fn refused(message: impl Into<String>) -> FractalError {
    FractalError::new("github-capture-authority-refused", message)
}

fn operator(state: &AppState, bound: Option<&BoundActor>, client_session: bool, actor: &str) -> Result<(), ApiError> {
    if client_session || bound.is_some() || state.owning_person.as_deref() != Some(actor) || !actor.starts_with("person/") {
        return Err(ApiError::bad(FractalError::new("forbidden",
            "GitHub capture authority requires the configured local owning person, never a client session, bound agent or run")));
    }
    Ok(())
}

fn source_path(id: &str, source: &Source) -> Result<String, ApiError> {
    if id != source.id {
        return Err(ApiError::bad(refused("route and source identity disagree")));
    }
    Ok(format!("github-source/{id}"))
}

pub async fn root(
    State(state): State<AppState>, bound: Option<BoundActor>,
    session: Option<Extension<client_v0::ClientSession>>,
    AxumPath(id): AxumPath<String>, Json(request): Json<RootRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    operator(&state, bound.as_ref(), session.is_some(), &request.actor)?;
    let subject = source_path(&id, &request.source)?;
    let grant = AuthorityGrant { version: capture::VERSION, source: request.source, member: request.member,
        epoch: 1, predecessor_grant: None, final_predecessor_commit: None, fencing_receipt: None };
    admit(&state, subject, request.actor, grant).await
}

pub async fn successor(
    State(state): State<AppState>, bound: Option<BoundActor>,
    session: Option<Extension<client_v0::ClientSession>>,
    AxumPath(id): AxumPath<String>, Json(request): Json<SuccessorRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    operator(&state, bound.as_ref(), session.is_some(), &request.actor)?;
    let subject = source_path(&id, &request.source)?;
    if request.epoch <= 1 {
        return Err(ApiError::bad(refused("successor epoch must exceed one")));
    }
    let grant = AuthorityGrant { version: capture::VERSION, source: request.source, member: request.member,
        epoch: request.epoch, predecessor_grant: Some(request.predecessor_grant),
        final_predecessor_commit: request.final_predecessor_commit, fencing_receipt: Some(request.fencing_receipt) };
    admit(&state, subject, request.actor, grant).await
}

pub async fn fence(
    State(state): State<AppState>, bound: Option<BoundActor>,
    session: Option<Extension<client_v0::ClientSession>>,
    AxumPath(id): AxumPath<String>, Json(request): Json<FenceRequest>,
) -> Result<Json<ClaimRecord>, ApiError> {
    operator(&state, bound.as_ref(), session.is_some(), &request.actor)?;
    source_path(&id, &request.source)?;
    let claim = crate::reconcile::github_capture::fence_source_checked(
        &state.store, &state.state_dir, &id, &request.source, &request.authority_grant,
    ).await.map_err(|error| ApiError::bad(refused(error.to_string())))?;
    signal_changed(&state);
    Ok(Json(claim))
}

async fn admit(state: &AppState, subject: String, actor: String, grant: AuthorityGrant) -> Result<Json<ClaimRecord>, ApiError> {
    let store = Arc::clone(&state.store);
    let claim = blocking_store(move || Ok(append_grant(&store, subject, actor, &grant)?)).await?;
    signal_changed(state);
    Ok(Json(claim))
}
fn append_grant(store: &Store, subject: String, actor: String, grant: &AuthorityGrant) -> Result<ClaimRecord, FractalError> {
    let fields = BTreeMap::from([("record".into(), serde_json::to_value(grant).map_err(|error| refused(error.to_string()))?)]);
    capture::validate(&subject, capture::AUTHORITY_KIND, &fields).map_err(|error| refused(error.to_string()))?;
    let key = format!("github-authority:{}:{}", capture::source_key(&grant.source).map_err(|error| refused(error.to_string()))?, grant.epoch);
    let input = ClaimInput { subject: subject.clone(), kind: capture::AUTHORITY_KIND.into(), actor: Some(actor),
        fields, evidence: grant.predecessor_grant.iter().chain(grant.fencing_receipt.iter())
            .chain(grant.final_predecessor_commit.iter()).cloned().collect(), expected_subject: None, idempotency_key: Some(key) };
    let admission = |connection: &Connection| validate_admission(connection, &subject, grant);
    let (claim, _) = crate::store::append_claim_with_commit_context(&store.graph, &input, None, None, None, None,
        crate::store::ClaimCommitContext { admission: Some(&admission), ..Default::default() })?;
    Ok(claim)
}

fn record<T: DeserializeOwned>(claim: &ClaimRecord, kind: &str, source: &Source) -> Result<T, FractalError> {
    if claim.kind != kind || claim.subject != format!("github-source/{}", source.id) && kind != capture::COMMIT_KIND {
        return Err(refused("authority dependency has the wrong kind or source subject"));
    }
    serde_json::from_value(claim.body.pointer("/fields/record").cloned().ok_or_else(|| refused("dependency has no capture record"))?)
        .map_err(|error| refused(error.to_string()))
}

fn dependency(connection: &Connection, id: &str) -> Result<ClaimRecord, FractalError> {
    crate::store::claim_by_id_tx(connection, id).map_err(|error| refused(error.to_string()))?
        .ok_or_else(|| refused(format!("authority dependency {id} is not present")))
}

fn validate_transfer(connection: &Connection, grant: &AuthorityGrant) -> Result<(), FractalError> {
    let mut current = grant.clone();
    let mut seen = BTreeSet::new();
    for _ in 0..32 {
        let Some(predecessor) = current.predecessor_grant.as_deref() else {
            if current.epoch != 1 || current.final_predecessor_commit.is_some() || current.fencing_receipt.is_some() {
                return Err(refused("root authority must be epoch one without predecessor dependencies"));
            }
            return Ok(());
        };
        if !seen.insert(predecessor.to_owned()) { return Err(refused("authority chain is cyclic")); }
        let prior_claim = dependency(connection, predecessor)?;
        let prior: AuthorityGrant = record(&prior_claim, capture::AUTHORITY_KIND, &grant.source)?;
        if prior.source != grant.source || prior.version != capture::VERSION || prior.epoch.checked_add(1) != Some(current.epoch) {
            return Err(refused("predecessor source generation or consecutive epoch does not match"));
        }
        let receipt_claim = dependency(connection, current.fencing_receipt.as_deref().ok_or_else(|| refused("successor needs a fencing receipt"))?)?;
        let receipt: FenceReceipt = record(&receipt_claim, capture::FENCED_KIND, &grant.source)?;
        if receipt.version != capture::VERSION || receipt.source != grant.source || receipt.authority_grant != predecessor
            || receipt.member != prior.member || receipt_claim.origin != prior.member || receipt.epoch != prior.epoch
            || receipt.final_commit != current.final_predecessor_commit {
            return Err(refused("fencing receipt does not bind the predecessor member, epoch and stopped frontier"));
        }
        if let Some(frontier) = receipt.final_commit.as_deref() {
            let claim = dependency(connection, frontier)?;
            let commit: CaptureCommit = record(&claim, capture::COMMIT_KIND, &grant.source)?;
            if commit.version != capture::VERSION || commit.source != grant.source || commit.epoch != prior.epoch
                || commit.authority_grant != predecessor || claim.origin != prior.member || commit.sequence == 0
                || commit.subject().map_err(|error| refused(error.to_string()))? != claim.subject {
                return Err(refused("stopped frontier is not a capture by the predecessor authority"));
            }
        }
        current = prior;
    }
    Err(refused("authority chain exceeds 32 grants"))
}

fn validate_admission(connection: &Connection, subject: &str, grant: &AuthorityGrant) -> Result<(), FractalError> {
    let membership = smallclaims::store::fleet_membership_tx(connection).map_err(|error| refused(error.to_string()))?;
    if !matches!(membership.state(&grant.member), crate::fleet::MemberState::Current(_)) {
        return Err(refused("authority member is not a current admitted fleet member"));
    }
    if let Some(desired) = crate::store::current_desired_row(connection, subject).map_err(|error| refused(error.to_string()))? {
        if desired.kind != "github-source" { return Err(refused("source subject has another declaration kind")); }
        let body: Value = serde_json::from_str(&desired.body).map_err(|error| refused(error.to_string()))?;
        let definition = capture::declarations::parse_source(&body).map_err(refused)?;
        if definition.source != grant.source { return Err(refused("declared source identity or generation differs")); }
        if grant.predecessor_grant.as_ref().is_some_and(|prior| definition.authority_grant.as_ref() != Some(prior)) {
            return Err(refused("successor must replace the selected predecessor grant"));
        }
    }
    let page = crate::store::claims_for_subject_kind_at_tx(connection, subject, capture::AUTHORITY_KIND, None, false, 128)
        .map_err(|error| refused(error.to_string()))?;
    if page.next_cursor.is_some() { return Err(refused("source authority history exceeds bounded admission")); }
    let mut predecessor_present = grant.epoch == 1;
    let mut epochs = BTreeSet::new();
    for claim in page.claims {
        let prior: AuthorityGrant = record(&claim, capture::AUTHORITY_KIND, &grant.source)?;
        if prior.source != grant.source { continue; }
        if !epochs.insert(prior.epoch) { return Err(refused("source authority history contains conflicting grants")); }
        if prior.epoch >= grant.epoch { return Err(refused("source already has this or a later authority epoch")); }
        if grant.predecessor_grant.as_deref() == Some(claim.id.as_str()) { predecessor_present = true; }
        if prior.epoch.checked_add(1) == Some(grant.epoch) && grant.predecessor_grant.as_deref() != Some(claim.id.as_str()) {
            return Err(refused("predecessor epoch has conflicting grants"));
        }
    }
    if !predecessor_present { return Err(refused("predecessor is not an admitted source grant")); }
    validate_transfer(connection, grant)
}

#[cfg(test)]
#[path = "github_capture_authority_tests.rs"]
mod tests;
