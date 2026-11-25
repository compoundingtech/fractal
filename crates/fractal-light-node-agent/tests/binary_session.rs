//! Separate agent process, real authenticated Home/Store, and canonical native runtimes.
//! Exec retirement requires the owning Linux systemd-user VM lane. Darwin lifecycle
//! acceptance requires real launchd and the real pinned PTY; no capability skips.
#![cfg(any(target_os = "linux", target_os = "macos"))]

#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

mod common;

use std::{collections::{BTreeMap, BTreeSet}, fs::{self, OpenOptions}, io::Write as _, net::SocketAddr,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _}, path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio}, sync::Arc, time::Duration};

use fractal_auth::{FleetAuth, MemberKey};
use fractal_light_node_agent::{config::Config, performance::Work};
use fractal_light_node_core::{authority::ExecutionBinding, config::{LIGHT_NODE_SESSION_PATH, LightNodeSessionConfig},
    custody::{self, LocalSession, MemberLifecycle, SessionCustody},
    journal::{Admission, Command, CommandDraft, Journal, Limits, Operation, OperationKind, Outcome, ReceiptState, Scope, Selection},
    session::Home};
use fractal_runtime::{ExecObservation, ExecRuntime, Launch, LaunchOperation, PtyRuntime};
use fractal_store::{store::Store, light_node::StoreRunEffects};
use notify::{RecursiveMode, Watcher as _};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const DEADLINE: Duration = Duration::from_secs(45);
const RUNTIME: &str = "binary-worker";

/// A kernel filesystem subscription bridges processes without polling native liveness
/// or SQLite. Install before launching the child, then check state before each wait.
struct Changes {
    _watcher: notify::RecommendedWatcher,
    events: tokio::sync::mpsc::UnboundedReceiver<notify::Result<notify::Event>>,
}
impl Changes {
    fn new(root: &Path) -> Self {
        let (send, events) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |event| { let _ = send.send(event); }).unwrap();
        watcher.watch(root, RecursiveMode::Recursive).unwrap();
        Self { _watcher: watcher, events }
    }
    async fn next(&mut self) {
        self.events.recv().await.expect("filesystem watcher ended").expect("filesystem watch failed");
    }
}

struct Agent(Child);
impl Agent {
    fn launch(config: &Path) -> Self {
        Self(ProcessCommand::new(common::agent_binary())
            .arg("--config").arg(config).stdin(Stdio::null()).stdout(Stdio::null())
            .stderr(Stdio::inherit()).spawn().unwrap())
    }
    fn kill(&mut self) {
        self.0.kill().unwrap();
        assert!(!self.0.wait().unwrap().success(), "crash must not become graceful shutdown");
    }
}
impl Drop for Agent {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

struct Pair {
    root: tempfile::TempDir,
    config_path: PathBuf,
    config: Config,
    selection: Selection,
    binding: Option<ExecutionBinding>,
    local: LocalSession,
    journal: Journal,
    graph: Arc<Store>,
    auth: FleetAuth,
    address: SocketAddr,
    home: Home,
    server: tokio::task::JoinHandle<()>,
    pty: PtyRuntime,
    exec: ExecRuntime,
}
impl Pair {
    async fn new(terminal: bool) -> Self {
        // Darwin's conventional temporary root can contain a /var symlink.
        // Fixtures obey the same physical-root rule as production provisioning.
        let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let (home_key, _) = MemberKey::generate().unwrap();
        let (agent_key, agent_pkcs8) = MemberKey::generate().unwrap();
        let selection = Selection { home: "example-home".into(), home_key: home_key.public().into(),
            host: "example-agent".into(), host_key: agent_key.public().into(), epoch: 1,
            grant: Scope { subjects: vec!["agent/example/worker".into()],
                operations: vec![OperationKind::Observe, OperationKind::Stop] } };
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        if !terminal {
            assert!(fractal_runtime::systemd_user_available(), "exec acceptance belongs in the systemd-user VM lane");
            assert_eq!(fractal_runtime::initialize_isolation(&environment), fractal_runtime::Isolation::Scope,
                "exec acceptance requires whole-scope containment before any native process is launched");
        }
        let binary = fractal_runtime::resolve_executable("pty", &environment)
            .expect("binary acceptance requires the real pinned PTY binary; missing binaries are not skips");
        let binary = fs::canonicalize(binary).unwrap();
        let pty_root = root.path().join("pty");
        let native_state_dir = root.path().join("native");
        fs::create_dir(&pty_root).unwrap();
        fs::create_dir(&native_state_dir).unwrap();
        let pty = PtyRuntime::new(pty_root.clone()).with_binary(binary.to_string_lossy());
        let exec = ExecRuntime::new(native_state_dir.join("exec"), native_state_dir.join("logs"));
        let generation = if terminal {
            pty.spawn(RUNTIME, &Launch::Shell("exec sleep 600".into()), root.path(), &environment,
                None, &BTreeMap::new(), None).unwrap();
            let entry = pty.snapshot().unwrap().into_iter().find(|entry| entry.name == RUNTIME).unwrap();
            assert_eq!(entry.status, "running");
            format!("{}:{}", entry.pid.unwrap(), entry.created_at.unwrap())
        } else {
            let generation = exec.clone().with_launch_operation(Some(LaunchOperation("binary-exec-launch".into())))
                .spawn(RUNTIME, &Launch::Shell("exec sleep 600".into()), root.path(), &environment).unwrap();
            assert!(generation.scope_unit.is_some(), "exec positive closure requires real systemd containment, not a logical-only fallback");
            generation.generation_id
        };
        let local = LocalSession { subject: selection.grant.subjects[0].clone(), runtime_id: RUNTIME.into(),
            incarnation: generation.clone(), native_incarnation: generation, terminal, driver: None,
            lifecycle: MemberLifecycle::Service, custody: SessionCustody::Owned,
            start_operation: (!terminal).then(|| "binary-exec-launch".into()), invocation: None };
        let journal = Journal::open(&root.path().join("home.sqlite3"), Limits::default()).unwrap();
        journal.install_epoch(&selection).unwrap();
        let graph = Arc::new(Store::open(&root.path().join("home-graph.sqlite3"), selection.home.clone()).unwrap());
        let secret = vec![7; 32];
        let auth = FleetAuth::from_secret_bytes("binary-acceptance", secret.clone()).unwrap()
            .with_member_key(Some(Arc::new(home_key)));
        let home = Home::new(vec![journal.clone()], auth.clone(), selection.home.clone(),
            Arc::new(StoreRunEffects::new(graph.clone()))).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = home.router();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let secret_path = root.path().join("fleet-secret");
        let key_path = root.path().join("member-key.pkcs8");
        private_write(&secret_path, &secret);
        private_write(&key_path, &agent_pkcs8);
        let config = Config { version: 1, state_dir: root.path().join("agent"), native_state_dir, pty_root,
            pty_binary: Some(binary), fleet_id: "binary-acceptance".into(), fleet_secret_file: secret_path,
            member_key_file: key_path, session: LightNodeSessionConfig::Agent {
                home_endpoint: format!("ws://{address}{LIGHT_NODE_SESSION_PATH}"), grant: selection.clone() },
            native_sessions: vec![local.clone()] };
        let config_path = root.path().join("agent.json");
        private_write(&config_path, &serde_json::to_vec(&config).unwrap());
        Self { root, config_path, config, selection, binding: None, local, journal, graph, auth, address, home, server, pty, exec }
    }
    async fn offered(&self) -> ExecutionBinding {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let changed = self.journal.authority_changed.notified();
                tokio::pin!(changed); changed.as_mut().enable();
                let bindings = self.journal.reported_executions().unwrap();
                if let Some(binding) = bindings.into_iter().find(|binding|
                    binding.subject == self.local.subject && binding.incarnation == self.local.incarnation) {
                    assert_eq!(binding.runtime, self.local.runtime_id);
                    assert!(binding.run.is_none());
                    return binding;
                }
                changed.await;
            }
        }).await.expect("real Home must retain the exact native authority offer")
    }
    fn draft(&self, decision: &str, operation: Operation) -> CommandDraft {
        CommandDraft { decision_id: decision.into(), binding: self.binding.as_ref().unwrap().clone(), operation }
    }
    fn running(&self) {
        if self.local.terminal {
            let entry = self.pty.snapshot().unwrap().into_iter().find(|entry| entry.name == RUNTIME).unwrap();
            assert_eq!(entry.status, "running");
            assert_eq!(format!("{}:{}", entry.pid.unwrap(), entry.created_at.unwrap()), self.local.native_incarnation);
        } else {
            let Some(ExecObservation::Running(entry)) = self.exec.observe(RUNTIME).unwrap() else { panic!("native exec must remain healthy") };
            assert_eq!(entry.generation_id, self.local.native_incarnation);
        }
    }
    async fn settled(&self, command: &Command) -> Option<Outcome> {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let changed = self.journal.changed.notified();
                tokio::pin!(changed); changed.as_mut().enable();
                if self.journal.settled_prefix("home", command.binding.epoch).unwrap() >= command.sequence {
                    match self.journal.command_receipt(&command.execution_id) {
                        Ok(Some(receipt)) => {
                            let ReceiptState::Settled { outcome } = receipt.state else { panic!("settled prefix requires terminal receipt") };
                            return Some(outcome);
                        }
                        // Canonical peers jointly checkpoint completed prefixes. An
                        // expired body is acceptable only after durable settlement;
                        // native proof and work accounting remain asserted below.
                        Err(error) if error.to_string().contains("result-expired") => return None,
                        result => panic!("settled receipt disappeared without joint retirement: {result:?}"),
                    }
                }
                changed.await;
            }
        }).await.expect("native result must reach durable home custody within the event-driven deadline")
    }
    async fn offline(&mut self) {
        self.home.shutdown();
        self.server.abort();
        let _ = (&mut self.server).await;
    }
    async fn online(&mut self) {
        self.home = Home::new(vec![self.journal.clone()], self.auth.clone(), self.selection.home.clone(),
            Arc::new(StoreRunEffects::new(self.graph.clone()))).unwrap();
        let listener = tokio::net::TcpListener::bind(self.address).await.unwrap();
        let app = self.home.router();
        self.server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    }
    fn agent_journal(&self) -> Journal {
        Journal::open(&self.config.state_dir.join("session.sqlite3"), Limits::default()).unwrap()
    }
}
impl Drop for Pair {
    fn drop(&mut self) {
        self.home.shutdown(); self.server.abort();
        if self.local.terminal { let _ = self.pty.stop(RUNTIME); }
        else {
            let _ = self.exec.kill_if(RUNTIME, Some(&self.local.native_incarnation));
            let _ = self.exec.wait_for_exit(RUNTIME, &self.local.native_incarnation);
        }
    }
}
fn private_write(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).unwrap();
    file.write_all(bytes).unwrap();
}

async fn performance(socket: &Path) -> std::io::Result<Value> {
    let mut stream = tokio::net::UnixStream::connect(socket).await?;
    stream.write_all(b"GET /v1/performance HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let split = response.windows(4).position(|bytes| bytes == b"\r\n\r\n").expect("HTTP header terminator");
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"), "private performance request failed: {}", String::from_utf8_lossy(&response));
    Ok(serde_json::from_slice(&response[split + 4..]).expect("performance response must be JSON"))
}
async fn ready(agent: &mut Agent, socket: &Path, changes: &mut Changes) -> Value {
    tokio::time::timeout(DEADLINE, async {
        loop {
            assert!(agent.0.try_wait().unwrap().is_none(), "standalone agent exited before readiness");
            match performance(socket).await {
                Ok(snapshot) => return snapshot,
                Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => changes.next().await,
                Err(error) => panic!("private API readiness failed: {error}"),
            }
        }
    }).await.expect("agent must publish its private API within the filesystem-event deadline")
}
fn integer(value: &Value, field: &str) -> u64 { value[field].as_u64().unwrap_or_else(|| panic!("missing integer {field}")) }
fn accounting(snapshot: &Value, pid: u32) {
    assert_eq!(snapshot["schema_version"], 1);
    assert_eq!(snapshot["pid"], pid);
    assert_ne!(pid, std::process::id(), "home and agent must be separate processes");
    assert_eq!(uuid::Uuid::parse_str(snapshot["process_instance_id"].as_str().unwrap()).unwrap().get_version_num(), 4);
    assert_eq!(snapshot["process_cpu_clock"], "CLOCK_PROCESS_CPUTIME_ID");
    assert_eq!(snapshot["process_cpu_scope"], "all threads of this process; excludes children and home process");
    assert!(integer(snapshot, "accounting_started_unix_ns") > 0);
    assert!(integer(snapshot, "process_cpu_ns") >= integer(snapshot, "process_cpu_baseline_ns"));
    assert!(integer(snapshot, "cpu_baseline_uptime_before_ns") <= integer(snapshot, "cpu_baseline_uptime_after_ns"));
    assert!(integer(snapshot, "cpu_baseline_uptime_after_ns") <= integer(snapshot, "cpu_sample_uptime_before_ns"));
    assert!(integer(snapshot, "cpu_sample_uptime_before_ns") <= integer(snapshot, "cpu_sample_uptime_after_ns"));
    assert_eq!(snapshot["accounting_uptime_ns"], snapshot["cpu_sample_uptime_after_ns"]);
    let rows = snapshot["work"].as_array().unwrap();
    assert_eq!(rows.len(), Work::ALL.len());
    for (row, work) in rows.iter().zip(Work::ALL) {
        assert_eq!(row["kind"], work.name());
        for field in ["count", "thread_cpu_ns", "thread_cpu_samples", "thread_cpu_errors"] { integer(row, field); }
    }
}
fn work(snapshot: &Value, kind: Work) -> u64 { integer(&snapshot["work"][kind as usize], "count") }
fn lean_state(pair: &Pair) {
    assert_eq!(fs::metadata(&pair.config.state_dir).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(pair.config.socket_path()).unwrap().permissions().mode() & 0o777, 0o600);
    let names = fs::read_dir(&pair.config.state_dir).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<BTreeSet<_>>();
    for required in ["agent.lock", "agent.sock", "session.sqlite3"] { assert!(names.contains(required), "missing {required}"); }
    let allowed = BTreeSet::from(["agent.lock", "agent.sock", "session.sqlite3", "session.sqlite3-wal", "session.sqlite3-shm"]);
    assert!(names.iter().all(|name| allowed.contains(name.as_str())), "standalone state must contain only the journal, lock and socket; no graph, Store or replication files: {names:?}");
}

async fn exercise(terminal: bool) {
    let mut pair = Pair::new(terminal).await;
    let mut changes = Changes::new(pair.root.path());
    let mut agent = Agent::launch(&pair.config_path);
    let first = ready(&mut agent, &pair.config.socket_path(), &mut changes).await;
    accounting(&first, agent.0.id());
    lean_state(&pair);

    // The real Home accepts the offer and its ACK activates native authority. No
    // direct journal authority edits or synthetic protocol acknowledgments occur.
    pair.binding = Some(pair.offered().await);
    let observe = pair.journal.enqueue(pair.draft("observe-before-crash", Operation::Observe)).unwrap();
    if let Some(outcome) = pair.settled(&observe).await {
        assert!(matches!(outcome, Outcome::Observed { incarnation, .. } if incarnation == pair.local.incarnation));
    }
    pair.running();
    let before_crash = performance(&pair.config.socket_path()).await.unwrap();
    accounting(&before_crash, agent.0.id());
    assert_eq!(before_crash["process_instance_id"], first["process_instance_id"]);
    assert_eq!(before_crash["process_cpu_baseline_ns"], first["process_cpu_baseline_ns"]);
    assert!(integer(&before_crash, "process_cpu_ns") >= integer(&first, "process_cpu_ns"));
    assert!(work(&before_crash, Work::NativeObserve) > 0);

    // A second actual process must fail before unlinking the live listener.
    let identity = fs::metadata(pair.config.socket_path()).unwrap();
    let duplicate = tokio::time::timeout(DEADLINE, tokio::process::Command::new(common::agent_binary())
        .arg("--config").arg(&pair.config_path).kill_on_drop(true).output()).await.unwrap().unwrap();
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("another agent already owns"));
    let retained = fs::metadata(pair.config.socket_path()).unwrap();
    assert_eq!((identity.dev(), identity.ino()), (retained.dev(), retained.ino()));
    assert_eq!(performance(&pair.config.socket_path()).await.unwrap()["process_instance_id"], first["process_instance_id"]);

    agent.kill();
    pair.running();
    // Reopen actual retained custody after SIGKILL. Duplicate admission resolves
    // to the original durable receipt, never a new native execution identity.
    {
        let retained = pair.agent_journal();
        assert!(matches!(retained.admit(&observe).unwrap(), Admission::Duplicate(_) | Admission::ResultExpired));
        assert!(!retained.with_authority(|authority| custody::retirement_verified(authority, &pair.local)).unwrap());
        assert_eq!(retained.with_authority(|authority| custody::local_session(authority, &pair.local.subject)).unwrap(), Some(pair.local.clone()));
    }
    match pair.journal.enqueue(pair.draft("observe-before-crash", Operation::Observe)) {
        Ok(duplicate) => {
            assert_eq!(duplicate.execution_id, observe.execution_id);
            assert_eq!(duplicate.sequence, observe.sequence);
        }
        Err(error) => assert!(error.to_string().contains("result-expired"), "{error}"),
    }
    let replay = pair.journal.enqueue(pair.draft("observe-after-crash", Operation::Observe)).unwrap();
    assert!(pair.journal.command_receipt(&replay.execution_id).unwrap().is_none());
    pair.offline().await;
    agent = Agent::launch(&pair.config_path);
    let restarted = ready(&mut agent, &pair.config.socket_path(), &mut changes).await;
    accounting(&restarted, agent.0.id());
    assert_ne!(restarted["process_instance_id"], first["process_instance_id"]);
    assert_eq!(work(&restarted, Work::NativeObserve), 0, "restart does not replay already settled native entry or adopt unrequested work");
    pair.running();
    pair.online().await;
    if let Some(outcome) = pair.settled(&replay).await {
        assert!(matches!(outcome, Outcome::Observed { incarnation, .. } if incarnation == pair.local.incarnation));
    }
    assert!(work(&performance(&pair.config.socket_path()).await.unwrap(), Work::NativeObserve) > 0);
    pair.running();

    // Disconnect while the process is healthy. An offline queued Stop is not a
    // signal, absence proof, retirement receipt, or authority to invent a result.
    pair.offline().await;
    let stop = pair.journal.enqueue(pair.draft("stop-after-home-offline", Operation::Stop)).unwrap();
    assert!(pair.journal.command_receipt(&stop.execution_id).unwrap().is_none());
    pair.running();
    assert!(!pair.agent_journal().with_authority(|authority| custody::retirement_verified(authority, &pair.local)).unwrap());
    pair.online().await;
    if let Some(outcome) = pair.settled(&stop).await {
        assert_eq!(outcome, Outcome::Stopped { incarnation: pair.local.incarnation.clone() });
    }
    let retained = pair.agent_journal();
    assert!(retained.with_authority(|authority| custody::retirement_verified(authority, &pair.local)).unwrap());
    assert!(retained.with_authority(|authority| Ok(custody::local_session(authority, &pair.local.subject)?.is_none())).unwrap());
    if terminal {
        assert!(!pair.pty.snapshot().unwrap().iter().any(|entry| entry.name == RUNTIME && entry.status == "running"));
    } else {
        assert!(pair.exec.retired_if(RUNTIME, &pair.local.native_incarnation).unwrap(), "positive whole-scope proof must survive durable receipt publication");
    }
    let final_snapshot = performance(&pair.config.socket_path()).await.unwrap();
    accounting(&final_snapshot, agent.0.id());
    assert_eq!(final_snapshot["process_instance_id"], restarted["process_instance_id"]);
    assert_eq!(final_snapshot["process_cpu_baseline_ns"], restarted["process_cpu_baseline_ns"]);
    assert!(integer(&final_snapshot, "process_cpu_ns") >= integer(&restarted, "process_cpu_ns"));
    assert!(work(&final_snapshot, Work::NativeStop) > 0);
    lean_state(&pair);
    // Own-process busy/idle sensor discrimination is covered by Meter's isolated
    // positive control. This test asserts no timing budget or child-CPU estimate.
}

#[tokio::test]
async fn standalone_pty_observe_stop_restart_and_private_accounting() { exercise(true).await; }

#[tokio::test]
#[cfg(target_os = "linux")]
async fn standalone_exec_observe_stop_restart_and_private_accounting_requires_systemd() { exercise(false).await; }

#[cfg(target_os = "macos")]
fn service(action: &str, config: &Path) -> std::process::Output {
    ProcessCommand::new(common::agent_binary()).args(["service", action, "--config"])
        .arg(config).output().expect("run the real minimal agent lifecycle")
}

#[cfg(target_os = "macos")]
fn service_ok(action: &str, config: &Path) -> Value {
    let output = service(action, config);
    assert!(output.status.success(), "{action} failed: {}",
        String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("lifecycle output must be structured JSON")
}

/// The hosted user is disposable, but a pre-existing service is never a fixture.
/// Cleanup removes only the exact service installed for this private config.
#[cfg(target_os = "macos")]
struct InstalledService(PathBuf);
#[cfg(target_os = "macos")]
impl Drop for InstalledService {
    fn drop(&mut self) {
        let output = service("uninstall", &self.0);
        assert!(output.status.success(), "remove only this fixture's owned launchd service: {}",
            String::from_utf8_lossy(&output.stderr));
    }
}

#[cfg(target_os = "macos")]
async fn service_ready(config: &Path, socket: &Path, changes: &mut Changes) -> Value {
    tokio::time::timeout(DEADLINE, async {
        loop {
            let next = changes.next();
            tokio::pin!(next);
            match performance(socket).await {
                Ok(_) => return service_ok("health", config),
                Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound |
                    std::io::ErrorKind::ConnectionRefused) => next.await,
                Err(error) => panic!("Darwin service private health failed: {error}"),
            }
        }
    }).await.expect("real launchd must publish the private agent API")
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn darwin_launchd_install_start_stop_restart_preserves_native_custody() {
    let mut pair = Pair::new(true).await;
    let mut changes = Changes::new(pair.root.path());
    let install = service_ok("install", &pair.config_path);
    let installed = InstalledService(pair.config_path.clone());
    assert!(!pair.config.socket_path().exists(), "installation must not start the agent");
    service_ok("start", &pair.config_path);
    let first_health = service_ready(&pair.config_path, &pair.config.socket_path(), &mut changes).await;
    let first = performance(&pair.config.socket_path()).await.unwrap();
    let first_pid = u32::try_from(integer(&first, "pid")).unwrap();
    accounting(&first, first_pid);
    assert_eq!(install["ok"], true);
    assert_eq!(install["operation"], "install");
    assert_eq!(first_health["local_status"], "healthy");
    assert_eq!(first_health["pid"], first_pid);
    assert_eq!(first_health["process_instance_id"], first["process_instance_id"]);
    assert_eq!(first_health["graph_ready"], false);
    assert_eq!(first_health["home_status"], "not_observed");
    pair.binding = Some(pair.offered().await);
    let observe = pair.journal.enqueue(pair.draft("darwin-observe", Operation::Observe)).unwrap();
    if let Some(outcome) = pair.settled(&observe).await {
        assert!(matches!(outcome, Outcome::Observed { incarnation, .. } if incarnation == pair.local.incarnation));
    }
    pair.running();

    // Unchanged install/start converges without killing or duplicating the job.
    service_ok("install", &pair.config_path);
    service_ok("start", &pair.config_path);
    assert_eq!(performance(&pair.config.socket_path()).await.unwrap()["process_instance_id"],
        first["process_instance_id"]);
    pair.offline().await;
    service_ok("health", &pair.config_path);
    service_ok("restart", &pair.config_path);
    let _ = service_ready(&pair.config_path, &pair.config.socket_path(), &mut changes).await;
    let second = performance(&pair.config.socket_path()).await.unwrap();
    accounting(&second, u32::try_from(integer(&second, "pid")).unwrap());
    assert_ne!(second["process_instance_id"], first["process_instance_id"]);
    pair.running();
    // The retained journal recovers even while the home is disconnected.
    assert_eq!(pair.agent_journal().with_authority(|authority|
        custody::local_session(authority, &pair.local.subject)).unwrap(), Some(pair.local.clone()));
    service_ok("stop", &pair.config_path);
    assert!(!service("health", &pair.config_path).status.success());
    pair.running();
    service_ok("stop", &pair.config_path);
    service_ok("start", &pair.config_path);
    let _ = service_ready(&pair.config_path, &pair.config.socket_path(), &mut changes).await;
    pair.online().await;
    let after = pair.journal.enqueue(pair.draft("darwin-after-restart", Operation::Observe)).unwrap();
    if let Some(outcome) = pair.settled(&after).await {
        assert!(matches!(outcome, Outcome::Observed { incarnation, .. } if incarnation == pair.local.incarnation));
    }
    pair.running();
    drop(installed);
    assert!(!service("start", &pair.config_path).status.success(),
        "uninstall must remove the owned login-start definition");
    assert!(pair.config.state_dir.join("session.sqlite3").is_file(),
        "uninstall must not remove durable custody");
    pair.running();
}
