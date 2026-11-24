//! Opt-in Phase 2 sessions. Full-node planning, replication and default startup stay unchanged.
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use axum::{Json, Router, extract::{Path as AxumPath, Query, State}, http::StatusCode, routing::{get, post}};
use fractal_daemon::api::run_requests::LocalRunAuthority;
use fractal_daemon::api::ApiError;
use fractal_daemon::config::{Config, LightNodeSessionConfig};
use fractal_daemon::fleet::MemberKey;
use fractal_daemon::light_node::journal::{CommandDraft, EpochInstall, Journal, Limits, Operation, RunRequest, RunResult, Selection};
use fractal_daemon::light_node::session::{Home, Requester, RunEffects as _, SeatClient, StoreRunEffects};
use fractal_daemon::model::{ClaimInput, FractalError};
use fractal_daemon::reconcile::{NativeSessionExecutor, Reconciler, program::{ProgramExecution, RunAuthority}};
use fractal_daemon::store::run_credentials::{ExecutionBinding, assertions::{RequestAssertion, RequestIdentity}, local};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use smallclaims::sync::FleetAuth;
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use tokio::sync::watch;

fn now_ms() -> Result<u64> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).context("session clock predates Unix epoch")?;
    Ok(u64::try_from(elapsed.as_millis()).context("session clock overflow")?)
}
fn authority_error(error: anyhow::Error) -> FractalError {
    match error.downcast::<FractalError>() {
        Ok(error) => error,
        Err(error) => FractalError::new("light-node-authority-unavailable", error.to_string()),
    }
}
fn open_journal(root: &Path, name: &str) -> Result<Journal> {
    let path = root.join(name);
    match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
        Ok(file) => file.sync_all()?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    Journal::open(&path, Limits::default())
}

struct NativeRunAuthority {
    journal: Journal,
    selection: Selection,
    key: Arc<MemberKey>,
    requester: Requester,
}
impl NativeRunAuthority {
    fn binding(&self, execution: &ProgramExecution) -> Result<ExecutionBinding> {
        let revision = hex::encode(Sha256::digest(serde_json::to_vec(&(
            &self.selection, &execution.run, &execution.runtime, &execution.incarnation,
        ))?));
        Ok(ExecutionBinding {
            version: 1, home: self.selection.home.clone(), host: self.selection.host.clone(), epoch: self.selection.epoch,
            subject: execution.run.step_run.clone(), runtime: execution.runtime.clone(), incarnation: execution.incarnation.clone(),
            authorization_revision: revision, run: Some(execution.run.clone()),
        })
    }
}
impl RunAuthority for NativeRunAuthority {
    fn issue(&self, execution: &ProgramExecution, lifetime_ms: u64) -> Result<String> {
        if self.journal.closing()? || self.journal.selection()? != self.selection {
            return Err(FractalError::new("local-authority-backpressure", "selected home is still draining its predecessor").into());
        }
        self.journal.activate_and_issue(&self.binding(execution)?, lifetime_ms, now_ms()?)
    }
    fn revoke(&self, execution: &ProgramExecution) -> Result<()> {
        self.journal.revoke_execution(&self.binding(execution)?)
    }
}
impl LocalRunAuthority for NativeRunAuthority {
    fn resolve(&self, secret: &str) -> Result<ExecutionBinding, FractalError> {
        let now = now_ms().map_err(authority_error)?;
        self.journal.with_authority(|connection| Ok(local::local_binding(connection, secret, now)?))
            .map_err(authority_error)
    }
    fn sign(&self, secret: &str, request: RequestIdentity, lifetime_ms: u64) -> Result<RequestAssertion, FractalError> {
        let now = now_ms().map_err(authority_error)?;
        self.journal.with_authority(|connection| Ok(RequestAssertion::sign_local(
            connection, secret, request, &self.key, now, lifetime_ms,
        )?)).map_err(authority_error)
    }
    fn execute_claim<'a>(&'a self, secret: &'a str, request_id: &'a str, claim: ClaimInput)
        -> Pin<Box<dyn Future<Output = Result<Value, ApiError>> + Send + 'a>>
    {
        Box::pin(async move {
            let payload = serde_json::to_vec(&claim).map_err(|error|
                ApiError::bad(FractalError::new("invalid-run-request", error.to_string())))?;
            let identity = RequestIdentity::new(request_id.into(), "POST".into(), "/v1/claims".into(), &payload);
            let assertion = self.sign(secret, identity, 30_000).map_err(ApiError::bad)?;
            let result = self.requester.submit(RunRequest { assertion, claim }).await
                .map_err(|error| ApiError::bad(authority_error(error)))?;
            // Wire refusal codes are owned data, not the static codes of a local
            // FractalError. Preserve the exact home outcome without leaking strings.
            result.result.map_err(|error| ApiError {
                status: StatusCode::BAD_REQUEST, code: error.code,
                message: error.message, details: Box::default(),
            })
        })
    }
    fn acknowledge_claim(
        &self, secret: &str, request_id: &str, claim: ClaimInput, accepted: RunResult,
    ) -> Result<(), FractalError> {
        let payload = serde_json::to_vec(&claim)
            .map_err(|error| FractalError::new("invalid-run-request", error.to_string()))?;
        let identity = RequestIdentity::new(request_id.into(), "POST".into(), "/v1/claims".into(), &payload);
        // Resolve the credential again, then compare the original immutable logical
        // request digest. A run cannot retire another execution's retained result.
        let assertion = self.sign(secret, identity, 30_000)?;
        self.requester.acknowledge(&RunRequest { assertion, claim }, &accepted).map_err(authority_error)
    }
}

#[derive(Clone)]
struct Controls { journals: BTreeMap<String, Journal> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enqueue {
    host: String,
    decision_id: String,
    binding: ExecutionBinding,
    operation: Operation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel { host: String, reason: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedHost { host: String }

type ControlError = (StatusCode, Json<Value>);
fn control_error(error: anyhow::Error) -> ControlError {
    let error = error.downcast::<FractalError>().unwrap_or_else(|error| FractalError::new("light-node-command-refused", error.to_string()));
    (StatusCode::CONFLICT, Json(json!({"error":{"code":error.code,"message":error.message,"details":error.details}})))
}
async fn enqueue(State(controls): State<Controls>, Json(request): Json<Enqueue>) -> Result<Json<Value>, ControlError> {
    let journal = controls.journals.get(&request.host).ok_or_else(|| control_error(anyhow::anyhow!("host has no explicit session grant")))?;
    let command = journal.enqueue(CommandDraft {
        decision_id: request.decision_id, binding: request.binding, operation: request.operation,
    }).map_err(control_error)?;
    Ok(Json(json!(command)))
}
async fn executions(State(controls): State<Controls>) -> Result<Json<Value>, ControlError> {
    let mut bindings = BTreeMap::new();
    for (host, journal) in &controls.journals {
        bindings.insert(host, journal.reported_executions().map_err(control_error)?);
    }
    Ok(Json(json!({"executions":bindings})))
}
async fn receipt(State(controls): State<Controls>, AxumPath(execution): AxumPath<String>, Query(request): Query<SelectedHost>) -> Result<Json<Value>, ControlError> {
    let journal = controls.journals.get(&request.host).ok_or_else(|| control_error(anyhow::anyhow!("host has no explicit session grant")))?;
    let receipt = journal.command_receipt(&execution).map_err(control_error)?;
    Ok(Json(json!({"execution_id":execution,"receipt":receipt})))
}
async fn cancel(State(controls): State<Controls>, AxumPath(execution): AxumPath<String>, Json(request): Json<Cancel>) -> Result<Json<Value>, ControlError> {
    let journal = controls.journals.get(&request.host).ok_or_else(|| control_error(anyhow::anyhow!("host has no explicit session grant")))?;
    journal.request_cancel(&execution, &request.reason).map_err(control_error)?;
    Ok(Json(json!({"execution_id":execution,"cancellation":"durably-requested"})))
}

enum Role {
    Home { listener: tokio::net::TcpListener, home: Home, pending: Vec<(Journal, Selection)>, effects: Arc<StoreRunEffects> },
    Agent { client: SeatClient, journal: Journal, reconciler: Arc<Reconciler>, auth: FleetAuth, node: String, target: Selection, endpoint: String },
}
pub struct Service {
    role: Role,
    authority: Option<Arc<dyn LocalRunAuthority>>,
    controls: Option<Controls>,
}
impl Service {
    pub async fn prepare(config: &Config, reconciler: Arc<Reconciler>) -> Result<Option<Self>> {
        let Some(session) = &config.light_node_session else { return Ok(None); };
        session.validate_for(&config.node)?;
        let root = config.state_dir.join("light-node-session/v1");
        fs::create_dir_all(&root)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let file = config.fleet.as_ref().context("session needs the admitted fleet identity")?;
        let key = Arc::new(MemberKey::load(&file.node_key_path(&config.state_dir))?);
        let auth = FleetAuth::load(
            config.fleet_id.as_deref().context("session needs fleet ID")?,
            config.shared_secret_file.as_deref().context("session needs fleet secret file")?,
        )?.with_member_key(Some(key.clone()));
        match session {
            LightNodeSessionConfig::Home { listen, grants } => {
                let mut journals = BTreeMap::new();
                let mut pending = Vec::new();
                for grant in grants {
                    let file = format!("host-{}.sqlite3", hex::encode(Sha256::digest(grant.host.as_bytes())));
                    let journal = open_journal(&root, &file)?;
                    if matches!(journal.install_epoch(grant)?, EpochInstall::Draining) {
                        pending.push((journal.clone(), grant.clone()));
                    }
                    journals.insert(grant.host.clone(), journal);
                }
                let effects = Arc::new(StoreRunEffects::new(reconciler.store.clone()));
                let home = Home::new(journals.values().cloned().collect(), auth, config.node.clone(), effects.clone())?;
                let listener = tokio::net::TcpListener::bind(listen).await.context("bind authenticated home session listener")?;
                Ok(Some(Self { role: Role::Home { listener, home, pending, effects }, authority: None, controls: Some(Controls { journals }) }))
            }
            LightNodeSessionConfig::Agent { home_endpoint, grant } => {
                let journal = open_journal(&root, "agent.sqlite3")?;
                if matches!(journal.install_epoch(grant)?, EpochInstall::Installed) {
                    journal.remember_endpoint(grant, home_endpoint)?;
                }
                reconciler.install_home_session_journal(journal.clone())?;
                let selected = journal.selection()?;
                let endpoint = journal.endpoint(selected.epoch)?.context("retained home endpoint is required to drain its epoch")?;
                let client = SeatClient::new(journal.clone(), auth.clone(), config.node.clone(), endpoint, Arc::new(NativeSessionExecutor::new(reconciler.clone())))?;
                let authority = Arc::new(NativeRunAuthority {
                    journal: journal.clone(), selection: grant.clone(), key, requester: client.requester(),
                });
                reconciler.install_program_run_authority(authority.clone())?;
                Ok(Some(Self { role: Role::Agent { client, journal, reconciler, auth, node: config.node.clone(),
                    target: grant.clone(), endpoint: home_endpoint.clone() }, authority: Some(authority), controls: None }))
            }
        }
    }

    pub fn authority(&self) -> Option<Arc<dyn LocalRunAuthority>> { self.authority.clone() }

    /// These routes are merged only into the guarded owner-local Unix listener, never TCP.
    pub fn control_router(&self) -> Router {
        match &self.controls {
            Some(controls) => Router::new()
                .route("/v1/internal/light-node/commands", post(enqueue))
                .route("/v1/internal/light-node/commands/{execution}", get(receipt))
                .route("/v1/internal/light-node/executions", get(executions))
                .route("/v1/internal/light-node/commands/{execution}/cancel", post(cancel))
                .with_state(controls.clone()),
            None => Router::new(),
        }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<()> {
        match self.role {
            Role::Home { listener, home, pending, effects } => {
                struct StopHome(Home);
                impl Drop for StopHome { fn drop(&mut self) { self.0.shutdown(); } }
                let _stop = StopHome(home.clone());
                let shutting_down = home.clone();
                let transition_shutdown = shutdown.clone();
                let serving = async move { axum::serve(listener, home.router()).with_graceful_shutdown(async move {
                    while !*shutdown.borrow_and_update() {
                        if shutdown.changed().await.is_err() { break; }
                    }
                    shutting_down.shutdown();
                }).await.context("serve authenticated home sessions") };
                tokio::try_join!(serving, advance_home(pending, effects, transition_shutdown))?;
                Ok(())
            }
            Role::Agent { client, journal, reconciler, auth, node, target, endpoint } => {
                let mut authority_shutdown = shutdown.clone();
                tokio::try_join!(run_agent(client, journal.clone(), reconciler.clone(), auth, node, target, endpoint, shutdown), async move {
                    loop {
                        if *authority_shutdown.borrow() { return Ok::<(), anyhow::Error>(()); }
                        let changed = journal.authority_changed.notified();
                        tokio::pin!(changed);
                        changed.as_mut().enable();
                        reconciler.session_authority_changed()?;
                        tokio::select! {
                            _ = changed => {}
                            changed = authority_shutdown.changed() => {
                                if changed.is_err() || *authority_shutdown.borrow() { return Ok::<(), anyhow::Error>(()); }
                            }
                        }
                    }
                })?;
                Ok(())
            }
        }
    }
}

async fn advance_home(pending: Vec<(Journal, Selection)>, effects: Arc<StoreRunEffects>, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    let mut transitions = FuturesUnordered::new();
    for (journal, target) in pending {
        let effects = effects.clone();
        let mut stopped = shutdown.clone();
        transitions.push(async move {
            loop {
                if *stopped.borrow() { return Ok::<(), anyhow::Error>(()); }
                let changed = journal.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if matches!(journal.install_epoch(&target)?, EpochInstall::Installed) {
                    effects.install_selection(&target)?;
                    return Ok(());
                }
                tokio::select! {
                    _ = changed => {}
                    _ = stopped.changed() => return Ok(()),
                }
            }
        });
    }
    while let Some(result) = transitions.next().await { result?; }
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() { break; }
    }
    Ok(())
}

async fn run_agent(first: SeatClient, journal: Journal, reconciler: Arc<Reconciler>, auth: FleetAuth, node: String,
    target: Selection, endpoint: String, shutdown: watch::Receiver<bool>) -> Result<()> {
    let mut first = Some(first);
    loop {
        if *shutdown.borrow() { return Ok(()); }
        if journal.selection()? == target {
            journal.remember_endpoint(&target, &endpoint)?;
            let client = match first.take() {
                Some(client) => client,
                None => SeatClient::new(journal.clone(), auth, node, endpoint, Arc::new(NativeSessionExecutor::new(reconciler)))?,
            };
            client.run(shutdown.clone()).await?;
            if !*shutdown.borrow() && !journal.closing()? {
                anyhow::bail!("active home session ended without shutdown or an epoch fence");
            }
            let mut stopped = shutdown;
            while !*stopped.borrow_and_update() {
                if stopped.changed().await.is_err() { break; }
            }
            return Ok(());
        }
        if matches!(journal.install_epoch(&target)?, EpochInstall::Installed) {
            journal.remember_endpoint(&target, &endpoint)?;
            reconciler.refresh_session_bindings()?;
            continue;
        }
        let retained = journal.selection()?;
        let prior_endpoint = journal.endpoint(retained.epoch)?.context("retained home endpoint is required to drain its epoch")?;
        let client = match first.take() {
            Some(client) => client,
            None => SeatClient::new(journal.clone(), auth.clone(), node.clone(), prior_endpoint, Arc::new(NativeSessionExecutor::new(reconciler.clone())))?,
        };
        // Keep the old authenticated connection alive until its durable, paired barrier
        // permits replacement. Dropping this scope closes it before the new client starts.
        let transition = async {
            loop {
                let changed = journal.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if matches!(journal.install_epoch(&target)?, EpochInstall::Installed) { return Ok::<(), anyhow::Error>(()); }
                changed.await;
            }
        };
        tokio::select! {
            result = client.run(shutdown.clone()) => {
                result?;
                if *shutdown.borrow() { return Ok(()); }
            }
            result = transition => result?,
        }
        if journal.selection()? == target && !journal.closing()? {
            journal.remember_endpoint(&target, &endpoint)?;
            reconciler.refresh_session_bindings()?;
        }
    }
}

#[cfg(test)]
#[path = "light_node_tests.rs"]
mod tests;
