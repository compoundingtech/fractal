//! Process composition only. Wire authentication, replay and epochs live in core.
use std::{future::{Future, IntoFuture as _}, io::Read as _, path::Path, pin::Pin, sync::Arc};

use anyhow::{Context as _, Result, ensure};
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use fractal_auth::{FleetAuth, MemberKey};
use fractal_light_node_core::{
    config::LightNodeSessionConfig,
    journal::{Command, EpochInstall, Journal, Limits, Operation, Outcome, Recovery, Selection},
    session::{Executor, SeatClient, SessionEvent, SessionObserver},
};
use fractal_runtime::{ExecRuntime, PtyRuntime};
use tokio::sync::watch;

use crate::{config::{self, Config}, native::NativeExecutor, performance::{Meter, Work}};

struct InstrumentedExecutor { native: Arc<NativeExecutor>, meter: Arc<Meter> }
impl Executor for InstrumentedExecutor {
    fn enter(&self, command: &Command, authority: &rusqlite::Connection) -> Result<String> {
        let work = match command.operation {
            Operation::Observe => Work::NativeObserve,
            Operation::Stop => Work::NativeStop,
            Operation::Start => return self.native.enter(command, authority),
        };
        let (result, accounting) = self.meter.measure_sync(work, || self.native.enter(command, authority));
        if let Err(error) = accounting { tracing::warn!(%error, "native CPU attribution unavailable"); }
        result
    }
    fn cancel(&self, command: &Command, authority: &rusqlite::Connection, reason: &str) -> Result<()> {
        let (result, accounting) = self.meter.measure_sync(Work::NativeCancel, || self.native.cancel(command, authority, reason));
        if let Err(error) = accounting { tracing::warn!(%error, "native cancellation CPU attribution unavailable"); }
        result
    }
    fn recover<'a>(&'a self, recovery: Recovery) -> Pin<Box<dyn Future<Output = Result<Outcome>> + Send + 'a>> {
        self.meter.record(Work::NativeRecover);
        self.native.recover(recovery)
    }
}

struct SessionAccounting(Arc<Meter>);
impl SessionObserver for SessionAccounting {
    fn record(&self, event: SessionEvent) {
        self.0.record(match event {
            SessionEvent::Connect => Work::SessionConnect,
            SessionEvent::Disconnect => Work::SessionDisconnect,
            SessionEvent::ReconnectWait => Work::SessionReconnectWait,
            SessionEvent::Receive => Work::SessionReceive,
            SessionEvent::Send => Work::SessionSend,
            SessionEvent::Wake => Work::SessionWake,
        });
    }
}

async fn performance(State(meter): State<Arc<Meter>>) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    meter.record(Work::LocalRequest);
    meter.snapshot().map(Json).map_err(|error| (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({"error": {"code": "light-node-self-accounting-unavailable", "message": error.to_string()}})),
    ))
}

/// Exposes no TCP listener and does not open a Store, replication worker or graph.
#[cfg(unix)]
pub async fn run(config: Config, meter: Arc<Meter>) -> Result<()> {
    use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
    config.validate()?;
    config::prepare_state(&config.state_dir)?;
    let custody_lock = config::private_state_file(&config.state_dir.join("agent.lock"))?;
    custody_lock.try_lock().context("another agent already owns this private custody directory")?;
    let journal_path = config.state_dir.join("session.sqlite3");
    drop(config::private_state_file(&journal_path)?);
    let journal = Journal::open(&journal_path, Limits::default())?;
    let LightNodeSessionConfig::Agent { home_endpoint, grant } = &config.session else { unreachable!("validated agent configuration") };
    journal.install_epoch(grant)?;
    let mut secret = Vec::new();
    let secret_file = config::private_file(&config.fleet_secret_file)?;
    ensure!(secret_file.metadata()?.len() <= 4096, "fleet secret file exceeds capacity");
    secret_file.take(4097).read_to_end(&mut secret)?;
    let mut key_bytes = Vec::new();
    let key_file = config::private_file(&config.member_key_file)?;
    ensure!(key_file.metadata()?.len() <= 4096, "member key file exceeds capacity");
    key_file.take(4097).read_to_end(&mut key_bytes)?;
    let key = Arc::new(MemberKey::from_pkcs8(&key_bytes)?);
    ensure!(key.public() == grant.host_key, "member signing key differs from the explicit admitted host grant");
    let auth = FleetAuth::from_secret_bytes(&config.fleet_id, secret)?.with_member_key(Some(key));
    let pty_binary = config.pty_executable()?;
    let native = Arc::new(NativeExecutor::new(grant.host.clone(), journal.clone(),
        PtyRuntime::new(config.pty_root.clone()).with_binary(pty_binary.to_string_lossy()),
        ExecRuntime::new(config.native_state_dir.join("exec"), config.native_state_dir.join("logs")))?);
    native.bootstrap(grant, &config.native_sessions)?;
    let executor: Arc<dyn Executor> = Arc::new(InstrumentedExecutor { native: native.clone(), meter: meter.clone() });
    let observer: Arc<dyn SessionObserver> = Arc::new(SessionAccounting(meter.clone()));

    let socket = config.socket_path();
    match std::fs::symlink_metadata(&socket) {
        Ok(metadata) => {
            ensure!(metadata.file_type().is_socket(), "refusing to replace a non-socket local API path");
            // The exclusive directory lock, not a connection probe, proves the old listener is gone.
            std::fs::remove_file(&socket)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = tokio::net::UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let _socket_cleanup = SocketCleanup::new(&socket)?;
    let app = Router::new().route("/v1/performance", get(performance)).with_state(meter);
    let (stop, stopped) = watch::channel(false);
    let api_stop = stopped.clone();
    let api = axum::serve(listener, app).with_graceful_shutdown(wait_for_shutdown(api_stop)).into_future();
    let client = run_client(journal.clone(), auth, grant.clone(), home_endpoint.clone(), executor, observer, stopped.clone());
    let refresh = refresh_authority(journal, native, grant, &config.native_sessions, stopped);
    tokio::pin!(api, client, refresh);
    let result = tokio::select! {
        result = &mut api => result.context("private self-accounting listener ended"),
        result = &mut client => result.context("home session ended"),
        result = &mut refresh => result.context("native authority refresh ended"),
        result = termination() => result,
    };
    stop.send_replace(true);
    // Dropping transport futures preserves all entered native custody in the journal;
    // it never turns an agent/service stop into Stop for an owned native process.
    result
}

async fn refresh_authority(journal: Journal, native: Arc<NativeExecutor>, target: &Selection,
    bindings: &[fractal_light_node_core::custody::LocalSession], mut stopped: watch::Receiver<bool>) -> Result<()> {
    loop {
        let changed = journal.authority_changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        native.bootstrap(target, bindings)?;
        tokio::select! {
            _ = changed => {},
            _ = stopped.changed() => return Ok(()),
        }
    }
}

/// Same paired durable barrier as the full-node client, without its graph adapter.
async fn run_client(journal: Journal, auth: FleetAuth, target: Selection, endpoint: String,
    executor: Arc<dyn Executor>, observer: Arc<dyn SessionObserver>, shutdown: watch::Receiver<bool>) -> Result<()> {
    loop {
        if *shutdown.borrow() { return Ok(()); }
        if journal.selection()? == target {
            journal.remember_endpoint(&target, &endpoint)?;
            let client = SeatClient::new(journal.clone(), auth.clone(), target.host.clone(), endpoint.clone(), executor.clone())?.with_observer(observer.clone());
            client.run(shutdown.clone()).await?;
            ensure!(*shutdown.borrow() || journal.closing()?, "active home session ended without shutdown or an epoch fence");
            wait_for_shutdown(shutdown).await;
            return Ok(());
        }
        if matches!(journal.install_epoch(&target)?, EpochInstall::Installed) { continue; }
        let retained = journal.selection()?;
        let prior_endpoint = journal.endpoint(retained.epoch)?.context("retained home endpoint is required to drain its epoch")?;
        let client = SeatClient::new(journal.clone(), auth.clone(), target.host.clone(), prior_endpoint, executor.clone())?.with_observer(observer.clone());
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
            result = client.run(shutdown.clone()) => result?,
            result = transition => result?,
        }
    }
}

async fn wait_for_shutdown(mut stopped: watch::Receiver<bool>) {
    while !*stopped.borrow_and_update() {
        if stopped.changed().await.is_err() { break; }
    }
}

#[cfg(unix)]
async fn termination() -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("listen for interrupt"),
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(unix)]
struct SocketCleanup { path: std::path::PathBuf, identity: (u64, u64) }
#[cfg(unix)]
impl SocketCleanup {
    fn new(path: &Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = std::fs::symlink_metadata(path)?;
        Ok(Self { path: path.into(), identity: (metadata.dev(), metadata.ino()) })
    }
}
#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt as _;
        if std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == self.identity) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
