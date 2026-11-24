//! Actual SQLite journals, authenticated loopback transport and native executor custody.
//! The seat graph is unavailable throughout Start/Observe/Stop and reconnect recovery.
use super::*;
use fractal_peer::light_node::journal::{Admission, CommandDraft, Limits, OperationKind, Receipt, ReceiptState, Scope};
use fractal_peer::light_node::session::{Home, SeatClient, StoreRunEffects};
use crate::store::local_sessions::{LocalSession, SessionCustody};
use smallclaims::{fleet::MemberKey, sync::FleetAuth};
use tokio::sync::watch;

#[test]
fn existing_native_custody_schema_upgrades_without_inventing_retirement_proof() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("existing-custody.sqlite3");
    let binding = LocalSession {
        subject: "agent/example/retained".into(), runtime_id: "retained".into(),
        incarnation: "retained-native-generation".into(), native_incarnation: "retained-native-generation".into(),
        terminal: true, driver: None, lifecycle: MemberLifecycle::Service,
        custody: SessionCustody::Owned, start_operation: None, invocation: None,
    };
    let body = serde_json::to_string(&binding).unwrap();
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hc51ea53acc5a owner=fractal_reconcile::reconcile::home_session_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE local_session_custody (
                subject TEXT NOT NULL, incarnation TEXT NOT NULL, body TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1, custody_settled INTEGER NOT NULL DEFAULT 0,
                detached_by TEXT, detach_receipted INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(subject,incarnation)
            ) WITHOUT ROWID;"
        ).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h06bc04a22d85 owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_session_custody(subject,incarnation,body) VALUES(?1,?2,?3)",
            rusqlite::params![binding.subject, binding.incarnation, body]).unwrap();
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    crate::store::local_sessions::create_schema(&connection).unwrap();
    crate::store::local_sessions::create_schema(&connection).unwrap();
    let (retained, active, verified): (String, bool, bool) = connection.query_row(
        "SELECT body,active,native_retirement_verified FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
        rusqlite::params![binding.subject, binding.incarnation],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(retained, body);
    assert!(active);
    assert!(!verified, "upgrading retained authority cannot manufacture positive physical proof");
}

struct NativePair {
    root: tempfile::TempDir,
    selection: Selection,
    home: Journal,
    seat: Journal,
    store: Arc<Store>,
    graph: Arc<Store>,
    runtime: Arc<NativeRuntime>,
    executor: Arc<Reconciler>,
    driver: Arc<NativeSessionExecutor>,
    binding: ExecutionBinding,
    seat_auth: FleetAuth,
    endpoint: String,
    home_service: Home,
    server: tokio::task::JoinHandle<()>,
    client: Option<tokio::task::JoinHandle<Result<()>>>,
    stop: Option<watch::Sender<bool>>,
}
impl NativePair {
    async fn new() -> Self {
        Self::with_custody(SessionCustody::Owned).await
    }

    async fn with_custody(custody: SessionCustody) -> Self {
        Self::with_options(custody, true).await
    }

    async fn for_start() -> Self {
        Self::with_options(SessionCustody::Owned, false).await
    }

    async fn with_options(custody: SessionCustody, launch_existing: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home_key = Arc::new(MemberKey::generate().unwrap().0);
        let seat_key = Arc::new(MemberKey::generate().unwrap().0);
        let selection = Selection {
            home: "example-home".into(), home_key: home_key.public().into(),
            host: "example-seat".into(), host_key: seat_key.public().into(), epoch: 1,
            grant: Scope { subjects: vec!["agent/example/worker".into(), "agent/example/second".into()],
                operations: vec![OperationKind::Start, OperationKind::Observe, OperationKind::Stop] },
        };
        let home = Journal::open(&root.path().join("home.sqlite3"), Limits::default()).unwrap();
        let seat = Journal::open(&root.path().join("seat.sqlite3"), Limits::default()).unwrap();
        home.install_epoch(&selection).unwrap();
        seat.install_epoch(&selection).unwrap();
        let store = Arc::new(Store::open(&root.path().join("native.sqlite3"), selection.host.clone()).unwrap());
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        let binary = fractal_runtime::resolve_executable("pty", &environment).expect("native session fixtures require the real pty binary");
        let runtime = Arc::new(NativeRuntime::new(root.path(), Some(&root.path().join("pty")), &binary));
        if launch_existing {
        runtime.pty().unwrap().spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
            root.path(), &environment, None, &BTreeMap::new(), None).unwrap();
        let generation = runtime.snapshot_ptys().unwrap().into_iter().find(|entry| entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
        // Owned sessions use the exact native generation. Only adoption creates a
        // distinct logical invocation identity; exercise that contract separately.
        let borrowed = custody == SessionCustody::Borrowed;
        let incarnation = if borrowed {
            format!("{generation}:bound:019a0000-0000-7000-8000-000000000001")
        } else { generation.clone() };
        let local = LocalSession {
            subject: selection.grant.subjects[0].clone(), runtime_id: "native-worker".into(),
            invocation: borrowed.then(|| incarnation.clone()),
            incarnation, native_incarnation: generation,
            terminal: true, driver: None,
            lifecycle: if borrowed { MemberLifecycle::TerminalBound } else { MemberLifecycle::Service },
            custody, start_operation: None,
        };
        store.bind_local_session(&local).unwrap();
        }
        let executor = Arc::new(Reconciler::new(store.clone(), runtime.clone(), selection.host.clone(), Arc::new(Notify::new())));
        executor.install_home_session_journal(seat.clone()).unwrap();
        let binding = executor.session_bindings(&selection).unwrap().pop().unwrap_or_else(|| ExecutionBinding {
            version: 1, home: selection.home.clone(), host: selection.host.clone(),
            subject: selection.grant.subjects[0].clone(), runtime: "native-worker".into(),
            incarnation: "immutable-start-request".into(), authorization_revision: "1".into(),
            epoch: selection.epoch, run: None,
        });
        let driver = Arc::new(NativeSessionExecutor::new(executor.clone()));
        store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-hd36c1be68e80 owner=fractal_reconcile::reconcile::home_session_tests TODO=prepare-body-and-predecessor-values */ "ALTER TABLE claims RENAME TO unavailable_claims;
            ALTER TABLE desired RENAME TO unavailable_desired;").unwrap();
        let graph = Arc::new(Store::open(&root.path().join("home-graph.sqlite3"), selection.home.clone()).unwrap());
        let home_auth = FleetAuth::test("native-session-tests", &[7; 32]).with_member_key(Some(home_key));
        let seat_auth = FleetAuth::test("native-session-tests", &[7; 32]).with_member_key(Some(seat_key));
        let home_service = Home::new(vec![home.clone()], home_auth, selection.home.clone(), Arc::new(StoreRunEffects::new(graph.clone()))).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}{}", listener.local_addr().unwrap(), fractal_peer::config::LIGHT_NODE_SESSION_PATH);
        let app = home_service.router();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let mut pair = Self { root, selection, home, seat, store, graph, runtime, executor, driver, binding, seat_auth, endpoint,
            home_service, server, client: None, stop: None };
        // Obtain the real home admission ACK and freshly revalidate native custody
        // before the fixtures disconnect and exercise their graph-free offline paths.
        if launch_existing {
            pair.connect();
            pair.authorized().await;
            pair.disconnect().await;
        }
        pair
    }
    async fn authorized(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let changed = self.seat.authority_changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.seat.with_authority(|authority| Ok(local::require_execution(authority, &self.binding).is_ok())).unwrap() {
                    break;
                }
                changed.await;
            }
        }).await.expect("real home admission and fresh native custody must activate the execution");
    }

    fn draft(&self, id: &str, operation: Operation) -> CommandDraft {
        CommandDraft { decision_id: id.into(), binding: self.binding.clone(), operation }
    }

    fn start_authorization(&self, command: &str) -> fractal_peer::light_node::journal::StartAuthorization {
        use crate::model::{LaunchSpec, MemberKind, RestartIntensity, RestartType};
        use fractal_peer::light_node::journal::StartAuthorization;
        let member = MemberSpec {
            kind: MemberKind::Agent, host: self.selection.host.clone(), runtime_id: "native-worker".into(),
            workspace: self.root.path().display().to_string(), workspace_create: false,
            cwd: self.root.path().display().to_string(), terminal: true,
            launch: LaunchSpec::Shell(command.into()), environment: BTreeMap::new(), tags: BTreeMap::new(),
            display_name: None, lifecycle: MemberLifecycle::Service, terminal_binding: None,
            one_shot: false, restart: RestartType::Never, restart_intensity: RestartIntensity::default(),
            shutdown_timeout_ms: 5_000, driver: None, terminal_size: None, capacity: None,
            declared_priority: None, declared_lane: None, declared_budgets: None,
        };
        let mut authorization = StartAuthorization {
            revision: 1, content_sha256: String::new(), desired_token: "immutable-desired".into(),
            selected_desired_token: "immutable-declaration".into(),
            declaration_sha256: "a".repeat(64), publication_sha256: "b".repeat(64),
            helper: None, rollout: None, member: member.clone(), launch_member: member,
            workspace_admission: StartWorkspaceAdmission::NonGit, start_request: "home-start-request".into(),
            expected_incarnation: None, checkout: None, render: None, credentials: None, continuation: None,
        };
        authorization.seal().unwrap();
        authorization
    }

    fn reopen_native(&mut self) {
        let journal = Journal::open(&self.root.path().join("seat.sqlite3"), Limits::default()).unwrap();
        let mut executor = Reconciler::new(self.store.clone(), self.runtime.clone(),
            self.selection.host.clone(), Arc::new(Notify::new()))
            .with_lane_host(self.executor.seat_admission.lanes)
            .with_finite_resources(Some(self.executor.seat_admission.finite_policy.clone()));
        executor.st_binary = self.executor.st_binary.clone();
        let executor = Arc::new(executor);
        executor.install_home_session_journal(journal.clone()).unwrap();
        self.seat = journal;
        self.executor = executor;
        self.driver = Arc::new(NativeSessionExecutor::new(self.executor.clone()));
        self.driver.watch_native_custody().unwrap();
    }
    fn connect(&mut self) {
        assert!(self.client.is_none());
        let journal = self.seat.clone();
        let changes = journal.authority_changed.clone();
        let executor = self.executor.clone();
        let client = SeatClient::new(journal, self.seat_auth.clone(), self.selection.host.clone(), self.endpoint.clone(), self.driver.clone()).unwrap();
        let (stop, stopped) = watch::channel(false);
        self.stop = Some(stop);
        self.client = Some(tokio::spawn(async move {
            let authority = async move {
                loop {
                    let changed = changes.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    executor.session_authority_changed()?;
                    changed.await;
                }
            };
            tokio::select! { result = client.run(stopped) => result, result = authority => result }
        }));
    }
    async fn disconnect(&mut self) {
        self.stop.take().unwrap().send_replace(true);
        self.client.take().unwrap().await.unwrap().unwrap();
    }
    async fn settled(&self, command: &Command) {
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let changed = self.home.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                match self.home.command_receipt(&command.execution_id) {
                    Ok(Some(receipt)) if matches!(receipt.state, ReceiptState::Settled { .. }) => break,
                    Err(error) if error.to_string().contains("result-expired") => {
                        assert!(self.home.settled_prefix("home", command.binding.epoch).unwrap() >= command.sequence,
                            "expired payload must have durable terminal checkpoint custody");
                        break;
                    }
                    Err(error) => panic!("reading exact native command receipt: {error:#}"),
                    Ok(_) => {}
                }
                changed.await;
            }
        }).await.expect("real native result must reach durable home custody");
        match self.home.command_receipt(&command.execution_id) {
            Ok(Some(receipt)) => assert!(matches!(receipt.state, ReceiptState::Settled { .. })),
            Err(error) => {
                assert!(error.to_string().contains("result-expired"), "{error}");
                assert!(self.home.settled_prefix("home", command.binding.epoch).unwrap() >= command.sequence);
            }
            Ok(None) => panic!("settled command cannot lose its receipt before explicit joint retirement"),
        }
    }
    async fn stopped(&self, command: &Command) {
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let changed = self.home.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                match self.home.command_receipt(&command.execution_id) {
                    Ok(Some(receipt)) if matches!(receipt.state,
                        ReceiptState::Settled { outcome: Outcome::Stopped { .. } }) => break,
                    Err(error) if error.to_string().contains("result-expired") => {
                        assert!(self.home.settled_prefix("home", command.binding.epoch).unwrap() >= command.sequence,
                            "expired closure requires durable terminal checkpoint custody");
                        assert!(matches!(&command.operation, Operation::Start { .. }),
                            "reclaimed stopped waits require this exact Start's native closure");
                        let operation = self.store.remote_runtime_start(&command.execution_id).unwrap().unwrap();
                        assert!(operation.dispatched && operation.cleanup_verified && operation.applied);
                        let result = operation.result.unwrap();
                        assert_eq!(result["verification"], "operation-incarnation");
                        let incarnation = result["incarnation"].as_str().unwrap();
                        let binding = self.store.local_session_at(&command.binding.subject, incarnation).unwrap().unwrap();
                        assert_eq!(binding.start_operation.as_deref(), Some(command.execution_id.as_str()));
                        assert!(self.store.local_session_retirement_verified(&binding).unwrap(),
                            "checkpoint alone cannot stand in for positive exact native retirement");
                        break;
                    }
                    Err(error) => panic!("reading exact native closure receipt: {error:#}"),
                    Ok(_) => {}
                }
                changed.await;
            }
        }).await.expect("exact native closure receipt must reach home");
    }
}
impl Drop for NativePair {
    fn drop(&mut self) {
        if let Some(client) = &self.client { client.abort(); }
        self.home_service.shutdown();
        self.server.abort();
        let _ = self.runtime.pty().and_then(|pty| pty.stop("native-worker"));
        let _ = self.runtime.pty().and_then(|pty| pty.stop("native-second"));
        let _ = self.runtime.pty().and_then(|pty| pty.stop("native-launcher"));
        if let Ok(Some(observed)) = self.runtime.observe_exec("native-worker")
            && let Some(incarnation) = observed.incarnation_id {
                let _ = self.runtime.exec.kill_if("native-worker", Some(&incarnation));
            }
    }
}

#[tokio::test]
async fn native_receipt_replay_reconnect_and_offline_stop_need_no_seat_graph() {
    let mut pair = NativePair::new().await;
    let observe = pair.home.enqueue(pair.draft("observe-before-ack-loss", Operation::Observe)).unwrap();
    assert!(matches!(pair.seat.admit(&observe).unwrap(), Admission::Accepted(_)));
    pair.seat.enter(&observe.execution_id, |command, authority| pair.driver.enter(command, authority)).unwrap();
    let recovery = pair.seat.recoverable().unwrap().into_iter().find(|entry| entry.command.execution_id == observe.execution_id).unwrap();
    let outcome = pair.driver.recover(recovery).await.unwrap();
    assert!(matches!(&outcome, Outcome::Observed { incarnation, .. } if incarnation == &pair.binding.incarnation));
    pair.seat.complete(&observe.execution_id, outcome).unwrap();
    // Crash window: no frame/ACK was sent. Reopening and connecting must replay the native
    // receipt rather than re-enter the executor or interpret transport as process liveness.
    pair.connect();
    pair.settled(&observe).await;
    assert_eq!(local_session(&pair.store, pair.runtime.as_ref(), &pair.binding.subject, None).unwrap().incarnation, pair.binding.incarnation);
    pair.disconnect().await;
    let stop = pair.home.enqueue(pair.draft("stop-while-offline", Operation::Stop)).unwrap();
    assert!(pair.home.command_receipt(&stop.execution_id).unwrap().is_none());
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &pair.binding.subject, None).is_ok());
    pair.connect();
    pair.settled(&stop).await;
    pair.disconnect().await;
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &pair.binding.subject, Some(&pair.binding.incarnation)).is_err());
    // Even after a runtime ID is reused, a replay cannot enter native stop again.
    let original = pair.store.local_session_at(&pair.binding.subject, &pair.binding.incarnation).unwrap().unwrap();
    assert!(pair.store.local_session_retirement_verified(&original).unwrap());
    let original_native = original.native_incarnation.clone();
    pair.runtime.remove("native-worker", true).unwrap();
    pair.runtime.pty().unwrap().spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
        pair.root.path(), &std::env::vars().collect(), None, &BTreeMap::new(), None).unwrap();
    let replacement = pair.runtime.snapshot_ptys().unwrap().into_iter().find(|entry| entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
    assert_ne!(replacement, original_native);
    assert!(matches!(pair.seat.admit(&stop).unwrap(), Admission::Duplicate(_) | Admission::ResultExpired));
    let running = pair.runtime.snapshot_ptys().unwrap().into_iter().find(|entry| entry.runtime_id == "native-worker").unwrap();
    assert_eq!(running.status, "running");
    assert_eq!(running.incarnation_id.as_deref(), Some(replacement.as_str()));
    assert!(stop_local_session(&pair.store, pair.runtime.as_ref(), &pair.binding.subject, &pair.binding.incarnation, "stale-offline-stop").is_err());
    let binding = LocalSession {
        subject: pair.binding.subject.clone(), runtime_id: "native-worker".into(),
        incarnation: replacement.clone(), native_incarnation: replacement,
        terminal: true, driver: None, lifecycle: MemberLifecycle::Service, custody: SessionCustody::Owned,
        start_operation: None, invocation: None,
    };
    pair.store.bind_local_session(&binding).unwrap();
    assert!(!pair.store.local_session_retirement_verified(&binding).unwrap(),
        "replacement custody must not inherit its predecessor's native retirement proof");
    stop_local_session(&pair.store, pair.runtime.as_ref(), &binding.subject, &binding.incarnation, "fresh-offline-stop").unwrap();
    assert!(pair.store.local_session_retirement_verified(&binding).unwrap());
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &binding.subject, None).is_err());
}

#[tokio::test]
async fn native_retirement_proof_is_fenced_to_the_exact_current_custody_body() {
    let pair = NativePair::new().await;
    let original = pair.store.local_session(&pair.binding.subject).unwrap().unwrap();
    let mut updated = original.clone();
    updated.lifecycle = MemberLifecycle::AdoptOnly;
    pair.store.bind_local_session(&updated).unwrap();
    assert!(pair.store.verify_local_session_retirement(&original).is_err(),
        "a stale lifecycle/custody snapshot cannot record physical retirement");
    assert!(!pair.store.local_session_retirement_verified(&updated).unwrap());
    assert!(stop_local_session_with_retirement_proof(&pair.store, pair.runtime.as_ref(),
        &updated.subject, &updated.incarnation, "exact-custody-stop").unwrap());
    assert!(pair.store.local_session_retirement_verified(&updated).unwrap());
    assert!(!pair.store.local_session_retirement_verified(&original).unwrap());
    assert!(pair.store.has_current_local_session(&updated.subject).unwrap(),
        "positive native proof is not the durable completion receipt");
    pair.store.bind_local_session(&original).unwrap();
    assert!(!pair.store.local_session_retirement_verified(&original).unwrap(),
        "changing the custody body invalidates the previous body's native proof");
    assert!(!pair.store.local_session_retirement_verified(&updated).unwrap());
}

#[tokio::test]
async fn borrowed_invocation_keeps_distinct_logical_identity_and_detaches_without_native_exit() {
    let mut pair = NativePair::with_custody(SessionCustody::Borrowed).await;
    let local = pair.store.local_session(&pair.binding.subject).unwrap().unwrap();
    assert_eq!(pair.binding.incarnation, local.incarnation);
    assert_ne!(local.incarnation, local.native_incarnation,
        "adopted invocation identity must not collapse into the person's native PTY identity");
    assert_eq!(local.invocation.as_deref(), Some(local.incarnation.as_str()));
    let observe = pair.home.enqueue(pair.draft("borrowed-observe", Operation::Observe)).unwrap();
    assert!(matches!(pair.seat.admit(&observe).unwrap(), Admission::Accepted(_)));
    let recovery = pair.seat.enter(&observe.execution_id, |command, authority| pair.driver.enter(command, authority)).unwrap();
    let outcome = pair.driver.recover(recovery).await.unwrap();
    assert!(matches!(&outcome, Outcome::Observed { incarnation, .. } if incarnation == &local.incarnation));
    pair.seat.complete(&observe.execution_id, outcome).unwrap();
    pair.connect();
    pair.settled(&observe).await;
    pair.disconnect().await;
    let stop = pair.home.enqueue(pair.draft("borrowed-detach", Operation::Stop)).unwrap();
    assert!(matches!(pair.seat.admit(&stop).unwrap(), Admission::Accepted(_)));
    let recovery = pair.seat.enter(&stop.execution_id, |command, authority| pair.driver.enter(command, authority)).unwrap();
    let outcome = pair.driver.recover(recovery).await.unwrap();
    assert!(matches!(&outcome, Outcome::Detached { incarnation } if incarnation == &local.incarnation));
    pair.seat.complete(&stop.execution_id, outcome).unwrap();
    pair.connect();
    pair.settled(&stop).await;
    pair.disconnect().await;
    assert!(pair.store.local_session_detached(&local.subject, &local.incarnation).unwrap());
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &local.subject, Some(&local.incarnation)).is_err());
    assert_eq!(pair.store.pending_local_session_detaches().unwrap(), vec![(local.clone(), stop.execution_id.clone())]);
    assert!(matches!(pair.seat.admit(&stop).unwrap(), Admission::Duplicate(_) | Admission::ResultExpired));
    let running = pair.runtime.snapshot_ptys().unwrap().into_iter()
        .find(|entry| entry.runtime_id == local.runtime_id).unwrap();
    assert_eq!(running.status, "running", "borrowed detach must not stop the person's shell");
    assert_eq!(running.incarnation_id.as_deref(), Some(local.native_incarnation.as_str()));
    assert_ne!(running.incarnation_id.as_deref(), Some(local.incarnation.as_str()));
}

#[tokio::test]
async fn paired_epoch_replacement_preserves_native_process_and_fences_old_home_stop() {
    use fractal_peer::light_node::journal::EpochInstall;
    let mut pair = NativePair::new().await;
    pair.connect();
    let observe = pair.home.enqueue(pair.draft("old-epoch-observe", Operation::Observe)).unwrap();
    pair.settled(&observe).await;
    let old = pair.binding.clone();
    let mut successor = pair.selection.clone();
    successor.epoch += 1;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let home_changed = pair.home.changed.notified();
            let seat_changed = pair.seat.changed.notified();
            tokio::pin!(home_changed, seat_changed);
            home_changed.as_mut().enable();
            seat_changed.as_mut().enable();
            let home = pair.home.install_epoch(&successor).unwrap();
            let seat = pair.seat.install_epoch(&successor).unwrap();
            if matches!(home, EpochInstall::Installed) && matches!(seat, EpochInstall::Installed) { break; }
            tokio::select! { _ = home_changed => {}, _ = seat_changed => {} }
        }
    }).await.expect("real old-home connection must drain receipts, revocations and paired epoch proof");
    pair.disconnect().await;
    pair.graph.install_run_epoch(&successor.host, successor.epoch).unwrap();
    pair.executor.refresh_session_bindings().unwrap();
    pair.selection = successor;
    pair.binding = pair.executor.session_bindings(&pair.selection).unwrap().pop().unwrap();
    assert_ne!(pair.binding.authorization_revision, old.authorization_revision);
    assert_eq!(pair.binding.incarnation, old.incarnation, "control handoff must not restart native custody");
    assert!(pair.home.enqueue(CommandDraft { decision_id: "old-home-return".into(), binding: old, operation: Operation::Stop }).is_err());
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &pair.binding.subject, Some(&pair.binding.incarnation)).is_ok());
    pair.connect();
    pair.authorized().await;
    let stop = pair.home.enqueue(pair.draft("new-epoch-stop", Operation::Stop)).unwrap();
    pair.settled(&stop).await;
    pair.disconnect().await;
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &pair.binding.subject, None).is_err());
}

#[tokio::test]
async fn restart_finishes_native_custody_from_unacknowledged_stop_receipt_before_replay() {
    let mut pair = NativePair::new().await;
    let stop = pair.home.enqueue(pair.draft("stop-before-native-metadata-finalization", Operation::Stop)).unwrap();
    assert!(matches!(pair.seat.admit(&stop).unwrap(), Admission::Accepted(_)));
    let recovery = pair.seat.enter(&stop.execution_id, |command, authority| pair.driver.enter(command, authority)).unwrap();
    assert!(pair.runtime.snapshot_ptys().unwrap().iter().all(|entry| entry.runtime_id != "native-worker" || entry.status != "running"));
    // The physical proof is durable, then the command receipt commits. Model process death
    // at precisely the next instruction, before the local metadata finalization.
    pair.seat.complete_requiring_local_finalization(&stop.execution_id, Outcome::Stopped { incarnation: pair.binding.incarnation.clone() },
        recovery.custody_proof.as_deref().unwrap()).unwrap();
    assert!(pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    assert_eq!(pair.seat.pending_terminal_results().unwrap().len(), 1);
    let reopened = Journal::open(&pair.root.path().join("seat.sqlite3"), Limits::default()).unwrap();
    let restarted = Arc::new(Reconciler::new(pair.store.clone(), pair.runtime.clone(), pair.selection.host.clone(), Arc::new(Notify::new())));
    restarted.install_home_session_journal(reopened.clone()).unwrap();
    assert!(!pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    pair.seat = reopened;
    pair.executor = restarted;
    pair.driver = Arc::new(NativeSessionExecutor::new(pair.executor.clone()));
    pair.connect();
    pair.settled(&stop).await;
    pair.disconnect().await;
    assert!(!pair.store.has_current_local_session(&pair.binding.subject).unwrap(),
        "home acknowledgement and payload retirement cannot resurrect local custody");
}

#[tokio::test]
async fn entered_stop_before_native_signal_recovers_the_exact_running_generation() {
    let mut pair = NativePair::new().await;
    let stop = pair.home.enqueue(pair.draft("entry-fence-before-native-signal", Operation::Stop)).unwrap();
    assert!(matches!(pair.seat.admit(&stop).unwrap(), Admission::Accepted(_)));
    let error = pair.seat.enter(&stop.execution_id, |_, _| anyhow::bail!("process died before the native callback")).unwrap_err();
    assert!(error.to_string().contains("before the native callback"));
    assert!(pair.seat.recoverable().unwrap().iter().any(|entry|
        entry.command.execution_id == stop.execution_id && entry.custody_proof.is_none()));
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &pair.binding.subject, Some(&pair.binding.incarnation)).is_ok());

    let reopened = Journal::open(&pair.root.path().join("seat.sqlite3"), Limits::default()).unwrap();
    let restarted = Arc::new(Reconciler::new(pair.store.clone(), pair.runtime.clone(), pair.selection.host.clone(), Arc::new(Notify::new())));
    restarted.install_home_session_journal(reopened.clone()).unwrap();
    pair.seat = reopened;
    pair.executor = restarted;
    pair.driver = Arc::new(NativeSessionExecutor::new(pair.executor.clone()));
    pair.connect();
    pair.settled(&stop).await;
    pair.disconnect().await;
    assert!(!pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    assert!(pair.runtime.snapshot_ptys().unwrap().iter().all(|entry|
        entry.runtime_id != "native-worker" || entry.status != "running"));
}

#[tokio::test]
async fn a_live_store_finalization_error_cannot_send_or_reclaim_the_native_stop_receipt() {
    let mut pair = NativePair::new().await;
    let stop = pair.home.enqueue(pair.draft("native-finalization-write-failure", Operation::Stop)).unwrap();
    assert!(matches!(pair.seat.admit(&stop).unwrap(), Admission::Accepted(_)));
    let recovery = pair.seat.enter(&stop.execution_id, |command, authority| pair.driver.enter(command, authority)).unwrap();
    pair.store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h49ca84a271f1 owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER fail_native_finalization BEFORE UPDATE OF custody_settled ON local_session_custody
         WHEN NEW.custody_settled=1 BEGIN SELECT RAISE(ABORT,'injected native finalization failure'); END;"
    ).unwrap();
    let error = pair.driver.recover(recovery).await.unwrap_err();
    assert!(format!("{error:#}").contains("injected native finalization failure"));
    assert_eq!(pair.seat.pending_terminal_results().unwrap().len(), 1);
    assert!(pair.seat.pending_receipts().unwrap().iter().all(|receipt| matches!(&receipt.state, ReceiptState::Accepted)));
    assert!(pair.store.has_current_local_session(&pair.binding.subject).unwrap());

    // Keep the real connection live: the home can ACK the Accepted receipt, but cannot
    // observe a terminal result or retire its native witness while Store still fails.
    pair.connect();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let changed = pair.home.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if pair.home.command_receipt(&stop.execution_id).unwrap()
                .is_some_and(|receipt| matches!(receipt.state, ReceiptState::Accepted)) {
                break;
            }
            changed.await;
        }
    }).await.expect("the live home must receive the nonterminal admission receipt");
    assert!(pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    assert_eq!(pair.home.settled_prefix("home", pair.selection.epoch).unwrap(), 0);
    assert!(pair.seat.checkpoint("seat", pair.selection.epoch, stop.sequence).is_err());
    assert_eq!(pair.seat.pending_terminal_results().unwrap().len(), 1);

    pair.store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h40e04f0e7f4e owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER fail_native_finalization;").unwrap();
    let followup = pair.home.enqueue(pair.draft("observe-after-native-finalization-repair", Operation::Observe)).unwrap();
    pair.settled(&stop).await;
    pair.settled(&followup).await;
    pair.disconnect().await;
    assert!(!pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    assert!(pair.seat.pending_terminal_results().unwrap().is_empty());
}

#[tokio::test]
async fn an_entered_stop_does_not_signal_a_replacement_native_generation() {
    let pair = NativePair::new().await;
    let stop = pair.home.enqueue(pair.draft("unknown-entry-before-generation-reuse", Operation::Stop)).unwrap();
    pair.seat.admit(&stop).unwrap();
    pair.seat.enter(&stop.execution_id, |_, _| anyhow::bail!("interrupted native entry")).unwrap_err();
    let recovery = pair.seat.recoverable().unwrap().into_iter().find(|entry|
        entry.command.execution_id == stop.execution_id).unwrap();
    pair.runtime.pty().unwrap().stop("native-worker").unwrap();
    let error = pair.driver.recover(recovery).await.unwrap_err();
    assert!(error.to_string().contains("native terminal scope retirement is unproven"), "{error:#}");
    assert!(pair.seat.pending_terminal_results().unwrap().is_empty(),
        "a direct native stop without retained whole-scope proof must remain unknown");
    assert!(pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    let recovery = pair.seat.recoverable().unwrap().into_iter().find(|entry|
        entry.command.execution_id == stop.execution_id).unwrap();
    pair.runtime.remove("native-worker", true).unwrap();
    pair.runtime.pty().unwrap().spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
        pair.root.path(), &std::env::vars().collect(), None, &BTreeMap::new(), None).unwrap();
    let replacement = pair.runtime.snapshot_ptys().unwrap().into_iter().find(|entry|
        entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
    let error = pair.driver.recover(recovery).await.unwrap_err();
    assert!(error.to_string().contains("native terminal scope retirement is unproven"), "{error:#}");
    let actual = pair.runtime.snapshot_ptys().unwrap().into_iter().find(|entry|
        entry.runtime_id == "native-worker").unwrap();
    assert_eq!(actual.status, "running");
    assert_eq!(actual.incarnation_id.as_deref(), Some(replacement.as_str()));
    assert!(pair.seat.recoverable().unwrap().iter().any(|entry|
        entry.command.execution_id == stop.execution_id && entry.custody_proof.is_none()));
}

#[tokio::test]
async fn positively_retired_unknown_entry_keeps_its_native_snapshot_through_a_store_failure() {
    let mut pair = NativePair::for_start().await;
    let authorization = pair.start_authorization("printf ready > unknown-stop-ready; exec sleep 600");
    let start = pair.home.enqueue(pair.draft("unknown-stop-associated-start",
        Operation::Start { authorization: Box::new(authorization) })).unwrap();
    pair.connect();
    pair.settled(&start).await;
    assert_eq!(native_file(&pair.root.path().join("unknown-stop-ready")).await, b"ready");
    pair.binding = pair.executor.session_bindings(&pair.selection).unwrap().pop().unwrap();
    pair.authorized().await;
    pair.disconnect().await;
    let stop = pair.home.enqueue(pair.draft("unknown-entry-positive-native-retirement", Operation::Stop)).unwrap();
    pair.seat.admit(&stop).unwrap();
    pair.seat.enter(&stop.execution_id, |_, _| anyhow::bail!("interrupted native entry")).unwrap_err();
    assert!(stop_local_session_with_retirement_proof(&pair.store, pair.runtime.as_ref(),
        &pair.binding.subject, &pair.binding.incarnation, "independent-local-retirement").unwrap());
    let binding = pair.store.local_session_at(&pair.binding.subject, &pair.binding.incarnation).unwrap().unwrap();
    assert!(pair.store.local_session_retirement_verified(&binding).unwrap());
    // Reopen only native SQLite custody, without reinstalling the deliberately unavailable
    // graph schema. The positive proof must already be committed, not an in-memory waiter.
    let reopened = rusqlite::Connection::open(pair.root.path().join("native.sqlite3")).unwrap();
    let verified: bool = reopened.query_row(
        "SELECT native_retirement_verified FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
        rusqlite::params![binding.subject, binding.incarnation], |row| row.get(0)).unwrap();
    assert!(verified);
    drop(reopened);
    pair.store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h9089b8a75c90 owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER fail_native_finalization BEFORE UPDATE OF custody_settled ON local_session_custody
         WHEN NEW.custody_settled=1 BEGIN SELECT RAISE(ABORT,'injected native finalization failure'); END;"
    ).unwrap();
    let recovery = pair.seat.recoverable().unwrap().into_iter().find(|entry|
        entry.command.execution_id == stop.execution_id).unwrap();
    let error = pair.driver.recover(recovery).await.unwrap_err();
    assert!(format!("{error:#}").contains("injected native finalization failure"), "{error:#}");
    let pending = pair.seat.pending_terminal_results().unwrap();
    assert_eq!(pending.len(), 1);
    let proof = pending[0].0.custody_proof.as_deref().unwrap();
    assert!(matches!(serde_json::from_str::<NativeProof>(proof).unwrap(), NativeProof::Stop { retired: true, .. }));
    assert!(pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    assert!(matches!(pair.seat.admit(&start).unwrap(),
        Admission::Duplicate(Receipt { state: ReceiptState::Settled { outcome: Outcome::Started { .. } }, .. })),
        "associated Start must not publish closure before Stop local finalization");
    assert!(matches!(pair.home.command_receipt(&start.execution_id).unwrap().unwrap().state,
        ReceiptState::Settled { outcome: Outcome::Started { .. } }));
    assert!(pair.seat.checkpoint("seat", pair.selection.epoch, start.sequence).is_err());
    assert!(!pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
    pair.store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-he41d028a3d0c owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER fail_native_finalization;").unwrap();

    let reopened = Journal::open(&pair.root.path().join("seat.sqlite3"), Limits::default()).unwrap();
    let restarted = Arc::new(Reconciler::new(pair.store.clone(), pair.runtime.clone(), pair.selection.host.clone(), Arc::new(Notify::new())));
    restarted.install_home_session_journal(reopened.clone()).unwrap();
    assert!(!pair.store.has_current_local_session(&pair.binding.subject).unwrap());
    pair.seat = reopened;
    pair.executor = restarted;
    pair.driver = Arc::new(NativeSessionExecutor::new(pair.executor.clone()));
    pair.connect();
    pair.settled(&stop).await;
    pair.stopped(&start).await;
    pair.disconnect().await;
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
}

async fn native_file(path: &Path) -> Vec<u8> {
    let changed = Arc::new(Notify::new());
    let wake = changed.clone();
    let mut watcher = notify::recommended_watcher(move |_: notify::Result<notify::Event>| { wake.notify_one(); }).unwrap();
    watcher.watch(path.parent().unwrap(), notify::RecursiveMode::NonRecursive).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let notified = changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match std::fs::read(path) {
                Ok(bytes) => return bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => notified.await,
                Err(error) => panic!("native fixture file: {error}"),
            }
        }
    }).await.expect("actual native child must publish its file")
}

#[tokio::test]
async fn native_start_runs_real_bun_with_sealed_render_then_reconnects_reopens_and_stops_exactly_once() {
    use fractal_peer::light_node::journal::StartRender;
    let mut pair = NativePair::for_start().await;
    std::fs::write(pair.root.path().join("local.json"), br#"{"seatOnly":true}"#).unwrap();
    std::fs::write(pair.root.path().join("local.lines"), "seat-only\n").unwrap();
    let mut authorization = pair.start_authorization("unused");
    authorization.member.launch = crate::model::LaunchSpec::Argv(vec!["bun".into(), "-e".into(), r#"
        const doc = await Bun.file("sealed.txt").text();
        const json = await Bun.file("local.json").json();
        const lines = await Bun.file("local.lines").text();
        if (doc !== "home document bytes" || !json.seatOnly || json.fromHome !== 7 ||
            lines !== "seat-only\nhome-line\n") throw new Error("seat render differs");
        const count = await Bun.file("launch-count").exists() ? Number(await Bun.file("launch-count").text()) : 0;
        await Bun.write("launch-count", String(count + 1));
        setInterval(() => {}, 60000);
    "#.into()]);
    authorization.launch_member = authorization.member.clone();
    let hash = hex::encode(Sha256::digest(b"home document bytes"));
    authorization.render = Some(StartRender {
        declaration: serde_json::json!({"name":"render","arguments":[],"properties":{},"children":[
            {"name":"copy","arguments":[format!("doc/native@{hash}"),"sealed.txt"],"properties":{},"children":[]},
            {"name":"json-upsert","arguments":["local.json","{\"fromHome\":7}"],"properties":{},"children":[]},
            {"name":"ensure-line","arguments":["local.lines","home-line"],"properties":{},"children":[]}
        ]}),
        documents: BTreeMap::from([(format!("doc/native@{hash}"), b"home document bytes".to_vec())]),
    });
    authorization.seal().unwrap();
    let start = pair.home.enqueue(pair.draft("actual-bun-start", Operation::Start { authorization: Box::new(authorization) })).unwrap();
    pair.connect();
    pair.settled(&start).await;
    let receipt = pair.home.command_receipt(&start.execution_id).unwrap().unwrap();
    let ReceiptState::Settled { outcome: Outcome::Started { incarnation } } = receipt.state else {
        panic!("native Start must return actual generation");
    };
    assert_ne!(incarnation, start.binding.incarnation);
    assert_eq!(native_file(&pair.root.path().join("launch-count")).await, b"1");
    let local = pair.store.local_session(&start.binding.subject).unwrap().unwrap();
    assert_eq!(local.native_incarnation, incarnation);
    assert_eq!(local.start_operation.as_deref(), Some(start.execution_id.as_str()));
    pair.disconnect().await;
    pair.reopen_native();
    pair.driver.watch_native_custody().unwrap();
    pair.connect();
    pair.settled(&start).await;
    assert!(matches!(pair.seat.admit(&start).unwrap(), Admission::Duplicate(_) | Admission::ResultExpired));
    assert_eq!(std::fs::read(pair.root.path().join("launch-count")).unwrap(), b"1");
    assert_eq!(pair.runtime.start_operation(&local.runtime_id, true, &incarnation).unwrap().as_deref(), Some(start.execution_id.as_str()));
    pair.binding = pair.executor.session_bindings(&pair.selection).unwrap().pop().unwrap();
    pair.authorized().await;
    let observe = pair.home.enqueue(pair.draft("actual-bun-observe", Operation::Observe)).unwrap();
    pair.settled(&observe).await;
    assert!(matches!(pair.home.command_receipt(&observe.execution_id).unwrap().unwrap().state,
        ReceiptState::Settled { outcome: Outcome::Observed { incarnation: seen, .. } } if seen == incarnation));
    let stop = pair.home.enqueue(pair.draft("actual-bun-stop", Operation::Stop)).unwrap();
    pair.settled(&stop).await;
    pair.disconnect().await;
    assert!(pair.store.local_session_retirement_verified(&local).unwrap());
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
    assert_eq!(std::fs::read(pair.root.path().join("launch-count")).unwrap(), b"1");
}

#[tokio::test]
async fn native_start_preentry_withdrawal_never_creates_native_producer() {
    let mut pair = NativePair::for_start().await;
    let mut authorization = pair.start_authorization("printf entered > forbidden; exec sleep 600");
    let bytes = b"withdrawn render must not commit";
    let hash = hex::encode(Sha256::digest(bytes));
    authorization.render = Some(fractal_peer::light_node::journal::StartRender {
        declaration: serde_json::json!({"name":"render","arguments":[],"properties":{},"children":[
            {"name":"copy","arguments":[format!("doc/withdrawn@{hash}"),"forbidden-render"],"properties":{},"children":[]}
        ]}),
        documents: BTreeMap::from([(format!("doc/withdrawn@{hash}"), bytes.to_vec())]),
    });
    authorization.seal().unwrap();
    let start = pair.home.enqueue(pair.draft("withdraw-before-entry", Operation::Start { authorization: Box::new(authorization) })).unwrap();
    pair.home.request_cancel(&start.execution_id, "home withdrew before entry").unwrap();
    pair.connect();
    pair.settled(&start).await;
    pair.disconnect().await;
    assert!(!pair.root.path().join("forbidden").exists());
    assert!(!pair.root.path().join("forbidden-render").exists());
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().is_none());
    assert!(pair.store.local_session(&start.binding.subject).unwrap().is_none());
}

#[tokio::test]
async fn connected_unknown_start_recovers_late_native_provenance_and_cancel_without_reconnect() {
    let mut pair = NativePair::for_start().await;
    let authorization = pair.start_authorization("printf x >> late-entered; exec sleep 600");
    let start = pair.home.enqueue(pair.draft("connected-late-provenance",
        Operation::Start { authorization: Box::new(authorization.clone()) })).unwrap();
    pair.seat.admit(&start).unwrap();
    pair.seat.enter(&start.execution_id, |_, _| {
        let operation = crate::store::RuntimeStartOperation {
            id: start.execution_id.clone(), subject: start.binding.subject.clone(), owner: start.execution_id.clone(),
            body: serde_json::json!({"remote_start": authorization}),
            revoked_by: None, finished: false, result: None, applied: false,
            cleanup_verified: false, dispatched: false, receipted: false,
        };
        assert!(pair.store.admit_remote_runtime_start(&operation, start_jobs::MAX_START_JOBS)?);
        pair.store.enter_remote_runtime_start(&operation.id, &operation.owner)?;
        anyhow::bail!("interrupted after backend admission before provenance publication")
    }).unwrap_err();
    pair.driver.watch_native_custody().unwrap();
    let mut failures = pair.driver.recovery_failures.subscribe();
    pair.connect();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while *failures.borrow_and_update() == 0 { failures.changed().await.unwrap(); }
    }).await.expect("connected native recovery must actually fail on missing provenance");
    assert!(!pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
    let mut progress = pair.driver.native_generation.subscribe();
    let environment = std::env::vars().collect::<BTreeMap<_, _>>();
    pair.runtime.pty().unwrap().spawn("native-worker",
        &fractal_runtime::Launch::Shell("printf x >> late-entered; exec sleep 600".into()),
        pair.root.path(), &environment, None,
        &BTreeMap::from([(start_jobs::OPERATION_TAG.into(), start.execution_id.clone())]), None).unwrap();
    assert_eq!(native_file(&pair.root.path().join("late-entered")).await, b"x");
    tokio::time::timeout(std::time::Duration::from_secs(15), progress.changed()).await
        .expect("actual native provenance must advance recovery generation").unwrap();
    let generation = pair.runtime.snapshot_ptys().unwrap().into_iter()
        .find(|entry| entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
    // A native event alone must recover this connected execution. Sending Cancel
    // first would independently reopen recovery and conceal a missing native wake.
    pair.settled(&start).await;
    assert!(matches!(pair.home.command_receipt(&start.execution_id).unwrap().unwrap().state,
        ReceiptState::Settled { outcome: Outcome::Started { incarnation } } if incarnation == generation));
    assert!(pair.client.as_ref().is_some_and(|client| !client.is_finished()));
    pair.home.request_cancel(&start.execution_id, "withdraw late native producer").unwrap();
    pair.stopped(&start).await;
    assert!(pair.client.as_ref().is_some_and(|client| !client.is_finished()));
    let local = pair.store.local_session_at(&start.binding.subject, &generation).unwrap().unwrap();
    assert!(pair.store.local_session_retirement_verified(&local).unwrap());
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
    assert_eq!(std::fs::read(pair.root.path().join("late-entered")).unwrap(), b"x");
    pair.disconnect().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn native_ordinary_agents_hold_aggregate_physical_envelopes_without_graph_reads() {
    let mut pair = NativePair::for_start().await;
    let room = fractal_runtime::host::for_lanes(fractal_runtime::host::size().unwrap());
    let outside = fractal_runtime::host::outside_lane_memory().unwrap().outside_bytes;
    let lane = fractal_runtime::Capacity { cores: 1, memory: room.memory.saturating_sub(outside) / 3 * 2 };
    assert!(lane.memory >= 512 * 1024 * 1024, "native capacity acceptance requires physical headroom");
    let lane_host = seat_admission::LaneHost { room,
        outside: fractal_runtime::host::outside_lane_memory, burst_lane: None };
    let executor = Arc::new(Reconciler::new(pair.store.clone(), pair.runtime.clone(), pair.selection.host.clone(),
        Arc::new(Notify::new())).with_lane_host(Some(lane_host)));
    executor.install_home_session_journal(pair.seat.clone()).unwrap();
    pair.executor = executor;
    pair.driver = Arc::new(NativeSessionExecutor::new(pair.executor.clone()));
    let mut first = pair.start_authorization("printf first > first-entered; exec sleep 600");
    first.member.capacity = Some(lane);
    first.launch_member = first.member.clone();
    first.seal().unwrap();
    let mut second = first.clone();
    second.member.runtime_id = "native-second".into();
    second.member.launch = crate::model::LaunchSpec::Shell("printf second > forbidden-second; exec sleep 600".into());
    second.launch_member = second.member.clone();
    second.seal().unwrap();
    assert!(pair.executor.remote_seat_lane_fits("agent/example/worker", &first.member).unwrap());
    assert!(pair.executor.remote_seat_lane_fits("agent/example/second", &second.member).unwrap());
    let start = pair.home.enqueue(pair.draft("ordinary-first", Operation::Start { authorization: Box::new(first) })).unwrap();
    pair.connect();
    pair.settled(&start).await;
    assert!(matches!(pair.home.command_receipt(&start.execution_id).unwrap().unwrap().state,
        ReceiptState::Settled { outcome: Outcome::Started { .. } }));
    assert_eq!(native_file(&pair.root.path().join("first-entered")).await, b"first");
    assert!(!pair.executor.remote_seat_lane_fits("agent/example/second", &second.member).unwrap());
    let mut binding = pair.binding.clone();
    binding.subject = "agent/example/second".into();
    binding.runtime = "native-second".into();
    let denied = pair.home.enqueue(CommandDraft { decision_id: "ordinary-second".into(), binding,
        operation: Operation::Start { authorization: Box::new(second) } }).unwrap();
    pair.settled(&denied).await;
    assert!(matches!(pair.home.command_receipt(&denied.execution_id).unwrap().unwrap().state,
        ReceiptState::Settled { outcome: Outcome::Refused { .. } }));
    assert!(!pair.root.path().join("forbidden-second").exists());
    assert!(!pair.runtime.snapshot_ptys().unwrap().iter().any(|entry| entry.runtime_id == "native-second"));
    pair.home.request_cancel(&start.execution_id, "capacity fixture cleanup").unwrap();
    pair.stopped(&start).await;
    pair.disconnect().await;
}

#[tokio::test]
async fn native_start_physical_workspace_freeze_and_preexisting_generation_are_not_graph_bypasses() {
    for boundary in ["workspace", "checkout", "freeze", "generation", "shared-producer", "capacity", "finite-root-fit"] {
        let mut pair = NativePair::for_start().await;
        let mut authorization = pair.start_authorization("printf entered > forbidden; exec sleep 600");
        match boundary {
            "workspace" => {
                authorization.member.workspace = pair.root.path().join("missing").display().to_string();
                authorization.launch_member = authorization.member.clone();
            }
            "checkout" => {
                authorization.checkout = Some(fractal_peer::light_node::journal::StartCheckout {
                    repository: pair.root.path().join("missing-repository").display().to_string(),
                    base: "main".into(), branch: "expected".into(), remove_at_run_end: false,
                });
            }
            "shared-producer" => {
                // A graph-owned launch lost its daemon after backend entry. Remote
                // authorization may not erase that same journal's producer exclusion.
                pair.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-hf72dc22c4e56 owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_runtime_starts(id,subject,owner,body,dispatched) VALUES(?1,?2,?3,?4,1)",
                    rusqlite::params!["prior-full-node-operation", pair.binding.subject, "prior-native-owner",
                        serde_json::to_string(&serde_json::json!({
                            "member": authorization.member,
                            "backend_entry": {"version":1,"operation":"prior-full-node-operation",
                                "owner":"prior-native-owner","phase":"backend-entering"}
                        })).unwrap()]).unwrap();
            }
            "freeze" => { pair.store.set_launch_freeze("person/operator", "fixture", "physical fence", 60_000, 0).unwrap(); }
            "generation" => {
                pair.runtime.pty().unwrap().spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
                    pair.root.path(), &std::env::vars().collect(), None, &BTreeMap::new(), None).unwrap();
            }
            "capacity" => {
                for index in 0..start_jobs::MAX_START_JOBS {
                    let id = format!("held-{index}");
                    let operation = crate::store::RuntimeStartOperation {
                        id: id.clone(), subject: format!("agent/example/held-{index}"), owner: id.clone(),
                        body: serde_json::json!({"remote_start": {"fixture":"capacity"}}),
                        revoked_by: None, finished: false, result: None, applied: false,
                        cleanup_verified: false, dispatched: false, receipted: false,
                    };
                    assert!(pair.store.admit_remote_runtime_start(&operation, start_jobs::MAX_START_JOBS).unwrap());
                    pair.store.enter_remote_runtime_start(&id, &id).unwrap();
                }
            }
            "finite-root-fit" => {
                use crate::store::resource_membership::{Coverage, LaneRequest, Owner};
                let request = LaneRequest {
                    id: "remote-impossible".into(), name: "remote-checks".into(), host: pair.selection.host.clone(),
                    max_members: 1, profile: None, profile_estimate: None, disk_path: None,
                    capacity: Some(fractal_runtime::Capacity { cores: 65_536, memory: 1 << 60 }),
                    class: fractal_runtime::RequestClass::Check, disk: None, io: None,
                    coverage: Coverage::Enforced, admission_timeout_ms: None,
                    owner: Owner { run: "run/remote".into(), step: "step/remote".into(), attempt: 1,
                        invocation: Some(pair.binding.subject.clone()) },
                };
                authorization.member.terminal = false;
                resource_lanes::set_request(&mut authorization.member, &request).unwrap();
                authorization.launch_member = authorization.member.clone();
            }
            _ => unreachable!(),
        }
        authorization.seal().unwrap();
        let start = pair.home.enqueue(pair.draft(&format!("physical-{boundary}"), Operation::Start { authorization: Box::new(authorization) })).unwrap();
        assert!(matches!(pair.seat.admit(&start).unwrap(), Admission::Accepted(_)));
        let _entry = pair.seat.enter(&start.execution_id, |command, authority|
            pair.driver.enter(command, authority));
        let recovery = pair.seat.recoverable().unwrap().into_iter()
            .find(|recovery| recovery.command.execution_id == start.execution_id).unwrap();
        let outcome = pair.driver.recover(recovery).await.unwrap();
        assert!(matches!(&outcome, Outcome::Refused { .. }), "{boundary}: {outcome:?}");
        pair.seat.complete(&start.execution_id, outcome).unwrap();
        pair.connect();
        pair.settled(&start).await;
        pair.disconnect().await;
        assert!(!pair.root.path().join("forbidden").exists(), "{boundary}");
        assert!(pair.store.local_session(&start.binding.subject).unwrap().is_none(), "{boundary}");
    }
}

#[tokio::test]
async fn native_start_entered_unknown_recovers_only_its_positive_operation_proof_after_reopen() {
    let mut pair = NativePair::for_start().await;
    let authorization = pair.start_authorization("printf x >> entered; exec sleep 600");
    let start = pair.home.enqueue(pair.draft("entered-with-lost-result",
        Operation::Start { authorization: Box::new(authorization) })).unwrap();
    assert!(matches!(pair.seat.admit(&start).unwrap(), Admission::Accepted(_)));
    let entered = pair.seat.enter(&start.execution_id, |command, authority| pair.driver.enter(command, authority)).unwrap();
    assert_eq!(native_file(&pair.root.path().join("entered")).await, b"x");
    // Crash at native acceptance before the operation result and session proof commit.
    // The actual PTY/process survives; no fake runtime or restored graph is involved.
    pair.store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h5221d97ee725 owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_runtime_starts SET finished=0,result=NULL WHERE id=?1", [&start.execution_id]).unwrap();
    pair.reopen_native();
    let outcome = pair.driver.recover(Recovery { custody_proof: None, ..entered }).await.unwrap();
    let Outcome::Started { incarnation } = &outcome else { panic!("positive operation provenance must recover Start"); };
    let binding = pair.store.local_session(&start.binding.subject).unwrap().unwrap();
    assert_eq!(&binding.native_incarnation, incarnation);
    pair.seat.complete(&start.execution_id, outcome).unwrap();
    pair.connect();
    pair.settled(&start).await;
    pair.disconnect().await;
    assert_eq!(std::fs::read(pair.root.path().join("entered")).unwrap(), b"x");
    pair.home.request_cancel(&start.execution_id, "withdraw recovered producer").unwrap();
    pair.seat.cancel_or_revoke(&start.execution_id, "withdraw recovered producer".into(),
        |command, authority, reason| pair.driver.cancel(command, authority, reason)).unwrap();
    let recovery = pair.seat.recoverable().unwrap().into_iter()
        .find(|recovery| recovery.command.execution_id == start.execution_id).unwrap();
    let outcome = pair.driver.recover(recovery).await.unwrap();
    assert!(matches!(outcome, Outcome::Stopped { incarnation } if incarnation == binding.native_incarnation));
    assert!(pair.store.local_session_retirement_verified(&binding).unwrap());
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
}

#[tokio::test]
async fn native_start_unknown_never_adopts_or_stops_a_reused_runtime_generation() {
    let mut pair = NativePair::for_start().await;
    let authorization = pair.start_authorization("printf x >> entered; exec sleep 600");
    let start = pair.home.enqueue(pair.draft("unknown-before-generation-reuse",
        Operation::Start { authorization: Box::new(authorization) })).unwrap();
    assert!(matches!(pair.seat.admit(&start).unwrap(), Admission::Accepted(_)));
    let entered = pair.seat.enter(&start.execution_id, |command, authority| pair.driver.enter(command, authority)).unwrap();
    assert_eq!(native_file(&pair.root.path().join("entered")).await, b"x");
    let original = pair.runtime.snapshot_ptys().unwrap().into_iter()
        .find(|entry| entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
    pair.store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h3387d8df1df9 owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_runtime_starts SET finished=0,result=NULL WHERE id=?1", [&start.execution_id]).unwrap();
    pair.runtime.pty().unwrap().stop_if("native-worker", Some(&original)).unwrap();
    pair.runtime.pty().unwrap().spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
        pair.root.path(), &std::env::vars().collect(), None, &BTreeMap::new(), None).unwrap();
    let successor = pair.runtime.snapshot_ptys().unwrap().into_iter()
        .find(|entry| entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
    assert_ne!(original, successor);
    pair.reopen_native();
    assert!(pair.driver.recover(Recovery { custody_proof: None, ..entered }).await.is_err(),
        "absence of original provenance must retain entered custody, never launch/adopt a successor");
    pair.seat.cancel_or_revoke(&start.execution_id, "withdraw unknown".into(),
        |command, authority, reason| pair.driver.cancel(command, authority, reason)).unwrap();
    let recovery = pair.seat.recoverable().unwrap().into_iter()
        .find(|recovery| recovery.command.execution_id == start.execution_id).unwrap();
    assert!(pair.driver.recover(recovery).await.is_err(), "unknown producer closure cannot be guessed");
    let running = pair.runtime.snapshot_ptys().unwrap().into_iter().find(|entry| entry.runtime_id == "native-worker").unwrap();
    assert_eq!(running.incarnation_id.as_deref(), Some(successor.as_str()));
    assert_eq!(running.status, "running", "old Start revocation must not signal a reused generation");
    assert_eq!(std::fs::read(pair.root.path().join("entered")).unwrap(), b"x");
    assert!(pair.store.local_session(&start.binding.subject).unwrap().is_none());
    assert!(!pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
}

#[tokio::test]
async fn native_helper_exit_event_withdraws_remote_producer_without_fleet_graph_or_polling() {
    use fractal_peer::light_node::journal::StartHelperScope;
    let mut pair = NativePair::for_start().await;
    pair.runtime.pty().unwrap().spawn("native-launcher", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
        pair.root.path(), &std::env::vars().collect(), None, &BTreeMap::new(), None).unwrap();
    let generation = pair.runtime.snapshot_ptys().unwrap().into_iter()
        .find(|entry| entry.runtime_id == "native-launcher").unwrap().incarnation_id.unwrap();
    let launcher = LocalSession {
        subject: "agent/example/launcher".into(), runtime_id: "native-launcher".into(),
        incarnation: generation.clone(), native_incarnation: generation.clone(), terminal: true,
        driver: None, lifecycle: MemberLifecycle::Service, custody: SessionCustody::Owned,
        start_operation: None, invocation: None,
    };
    pair.store.bind_local_session(&launcher).unwrap();
    let mut authorization = pair.start_authorization("printf ready > helper-ready; exec sleep 600");
    authorization.helper = Some(StartHelperScope { subject: launcher.subject.clone(), lifetime: generation.clone() });
    authorization.seal().unwrap();
    let start = pair.home.enqueue(pair.draft("helper-root-custody",
        Operation::Start { authorization: Box::new(authorization) })).unwrap();
    pair.connect();
    pair.settled(&start).await;
    assert_eq!(native_file(&pair.root.path().join("helper-ready")).await, b"ready");
    let child = pair.store.local_session(&start.binding.subject).unwrap().unwrap();
    pair.disconnect().await;
    pair.home.request_cancel(&start.execution_id, "operator already withdrew helper child").unwrap();
    pair.seat.cancel_or_revoke(&start.execution_id, "operator already withdrew helper child".into(),
        |command, authority, reason| pair.driver.cancel(command, authority, reason)).unwrap();
    pair.runtime.pty().unwrap().stop_if("native-launcher", Some(&generation)).unwrap();
    pair.executor.session_authority_changed().unwrap();
    pair.connect();
    pair.stopped(&start).await;
    pair.disconnect().await;
    assert!(pair.store.local_session_retirement_verified(&child).unwrap());
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn native_finite_start_uses_real_cli_bun_kernel_lane_and_graph_free_fit_and_retirement() {
    use crate::store::resource_membership::{Coverage, LaneRequest, Owner};
    let mut pair = NativePair::for_start().await;
    let environment = std::env::vars().collect::<BTreeMap<_, _>>();
    let binary = fractal_runtime::resolve_executable("st3", &environment)
        .expect("native finite session acceptance requires the built real st3 executable on PATH");
    let lane_host = seat_admission::LaneHost {
        room: fractal_runtime::host::for_lanes(fractal_runtime::host::size().expect("native finite host sizing")),
        outside: fractal_runtime::host::outside_lane_memory, burst_lane: Some(seat_admission::BURST_LANE),
    };
    let mut executor = Reconciler::new(pair.store.clone(), pair.runtime.clone(), pair.selection.host.clone(),
        Arc::new(Notify::new())).with_lane_host(Some(lane_host));
    executor.st_binary = Some(binary);
    executor.install_home_session_journal(pair.seat.clone()).unwrap();
    pair.executor = Arc::new(executor);
    pair.driver = Arc::new(NativeSessionExecutor::new(pair.executor.clone()));
    pair.driver.watch_native_custody().unwrap();
    let mut authorization = pair.start_authorization("unused");
    authorization.member.terminal = false;
    authorization.member.launch = crate::model::LaunchSpec::Argv(vec!["bun".into(), "-e".into(),
        "await Bun.write('finite-entered', 'actual-bun'); setInterval(() => {}, 60000);".into()]);
    let request = LaneRequest {
        id: "remote-native-finite".into(), name: "remote-native-checks".into(), host: pair.selection.host.clone(),
        max_members: 1, profile: None, profile_estimate: None, disk_path: None,
        capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 512 * 1024 * 1024 }),
        class: fractal_runtime::RequestClass::Check, disk: None, io: None,
        coverage: Coverage::Enforced, admission_timeout_ms: None,
        owner: Owner { run: "run/remote-native".into(), step: "step/remote-native".into(), attempt: 1,
            invocation: Some(pair.binding.subject.clone()) },
    };
    resource_lanes::set_request(&mut authorization.member, &request).unwrap();
    authorization.launch_member = authorization.member.clone();
    authorization.seal().unwrap();
    let start = pair.home.enqueue(pair.draft("real-finite-start",
        Operation::Start { authorization: Box::new(authorization) })).unwrap();
    pair.connect();
    pair.settled(&start).await;
    let ReceiptState::Settled { outcome: Outcome::Started { incarnation } } =
        pair.home.command_receipt(&start.execution_id).unwrap().unwrap().state else {
            panic!("supported Linux native lane must launch, not silently skip unsupported coverage");
        };
    assert_eq!(native_file(&pair.root.path().join("finite-entered")).await, b"actual-bun");
    let record = pair.store.resource_membership_record(&request.id, &request.owner).unwrap();
    assert_eq!(record.state, "launched");
    assert_eq!(record.roots.len(), 1);
    assert_eq!(record.roots[0].execution.as_ref().unwrap().generation_id, incarnation);
    assert!(record.roots[0].payload_released_at.is_some());
    let local = pair.store.local_session(&start.binding.subject).unwrap().unwrap();
    pair.binding = pair.executor.session_bindings(&pair.selection).unwrap().pop().unwrap();
    pair.authorized().await;
    let stop = pair.home.enqueue(pair.draft("real-finite-stop", Operation::Stop)).unwrap();
    pair.settled(&stop).await;
    pair.disconnect().await;
    assert!(pair.store.local_session_retirement_verified(&local).unwrap());
    assert_eq!(pair.store.resource_membership_record(&request.id, &request.owner).unwrap().state, "retired");
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
}

#[tokio::test]
async fn native_start_prepares_and_validates_a_real_git_checkout_under_its_activation_lease() {
    use fractal_peer::light_node::journal::StartCheckout;
    let mut pair = NativePair::for_start().await;
    let repository = pair.root.path().join("repository");
    let workspace = pair.root.path().join("remote-worktree");
    let status = std::process::Command::new("git").args(["init", "--initial-branch=main"])
        .arg(&repository).status().unwrap();
    assert!(status.success());
    std::fs::write(repository.join("seed.txt"), "tracked checkout seed\n").unwrap();
    assert!(std::process::Command::new("git").arg("-C").arg(&repository)
        .args(["add", "seed.txt"]).status().unwrap().success());
    assert!(std::process::Command::new("git").arg("-C").arg(&repository)
        .args(["-c", "user.name=Native fixture", "-c", "user.email=native@example.test",
            "commit", "-m", "Native checkout fixture"]).status().unwrap().success());
    let mut authorization = pair.start_authorization("printf checkout-ready > ready; exec sleep 600");
    authorization.member.workspace = workspace.display().to_string();
    authorization.member.cwd = workspace.display().to_string();
    authorization.launch_member = authorization.member.clone();
    authorization.checkout = Some(StartCheckout {
        repository: repository.display().to_string(), base: "main".into(),
        branch: "remote-admitted".into(), remove_at_run_end: false,
    });
    authorization.workspace_admission = StartWorkspaceAdmission::Git {
        owner: workspace.display().to_string(), repository: repository.display().to_string(),
    };
    authorization.seal().unwrap();
    let start = pair.home.enqueue(pair.draft("physical-checkout-preparation",
        Operation::Start { authorization: Box::new(authorization) })).unwrap();
    pair.connect();
    pair.settled(&start).await;
    assert!(matches!(pair.home.command_receipt(&start.execution_id).unwrap().unwrap().state,
        ReceiptState::Settled { outcome: Outcome::Started { .. } }));
    assert_eq!(native_file(&workspace.join("ready")).await, b"checkout-ready");
    Checkout { repository, base: "main".into(), branch: "remote-admitted".into(), remove_at_run_end: false }
        .validate_workspace(&workspace).unwrap();
    pair.disconnect().await;
}

#[path = "home_session_stop_tests.rs"]
mod stop_tests;

#[cfg(target_os = "linux")]
#[tokio::test]
async fn inherited_native_stop_withdrawal_and_predispatch_refusal_preserve_parent_and_sibling() {
    use crate::store::resource_membership::{Admission as ResourceAdmission, Coverage, LaneRequest, Owner};
    for boundary in ["stop", "withdraw", "refusal", "undispatched-withdrawal"] {
        let mut pair = NativePair::for_start().await;
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        let binary = fractal_runtime::resolve_executable("st3", &environment)
            .expect("inherited finite acceptance requires the real st3 executable");
        let lane_host = seat_admission::LaneHost {
            room: fractal_runtime::host::for_lanes(fractal_runtime::host::size().unwrap()),
            outside: fractal_runtime::host::outside_lane_memory,
            burst_lane: Some(seat_admission::BURST_LANE),
        };
        let mut executor = Reconciler::new(pair.store.clone(), pair.runtime.clone(), pair.selection.host.clone(),
            Arc::new(Notify::new())).with_lane_host(Some(lane_host));
        executor.st_binary = Some(binary);
        executor.install_home_session_journal(pair.seat.clone()).unwrap();
        pair.executor = Arc::new(executor);
        pair.driver = Arc::new(NativeSessionExecutor::new(pair.executor.clone()));
        pair.driver.watch_native_custody().unwrap();
        let request = LaneRequest {
            id: format!("inherited-{boundary}"), name: "inherited-native-checks".into(),
            host: pair.selection.host.clone(), max_members: 1, profile: None, profile_estimate: None,
            disk_path: None, capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 512 * 1024 * 1024 }),
            class: fractal_runtime::RequestClass::Check, disk: None, io: None,
            coverage: Coverage::Enforced, admission_timeout_ms: None,
            owner: Owner { run: "run/native-parent".into(), step: "step/native-parent".into(),
                attempt: 1, invocation: Some("exec/native-parent".into()) },
        };
        pair.executor.ensure_resource_authority().unwrap();
        let fit = pair.executor.remote_resource_lane_fit(&request).unwrap();
        assert!(matches!(pair.store.request_resource_membership(&request, fit,
            u64::try_from(now_ms()).unwrap()).unwrap(), ResourceAdmission::Admitted(_)));
        let mut authorization = pair.start_authorization("printf ready > inherited-ready; exec sleep 600");
        authorization.member.terminal = false;
        resource_lanes::set_request(&mut authorization.member, &request).unwrap();
        authorization.launch_member = authorization.member.clone();
        authorization.seal().unwrap();
        let mut survivors = Vec::new();
        for runtime_id in ["native-parent", "native-sibling"] {
            let mut member = authorization.member.clone();
            member.runtime_id = runtime_id.into();
            member.launch = crate::model::LaunchSpec::Shell("exec sleep 600".into());
            pair.runtime.start_resource_guarded(&pair.store, &member, None, &|| Ok(())).unwrap();
            let incarnation = pair.runtime.observe_exec(runtime_id).unwrap().unwrap().incarnation_id.unwrap();
            survivors.push((runtime_id, incarnation));
        }
        if matches!(boundary, "refusal" | "undispatched-withdrawal") {
            authorization.member.cwd = pair.root.path().join("unavailable-cwd").display().to_string();
            authorization.launch_member = authorization.member.clone();
            authorization.seal().unwrap();
        }
        let start = pair.home.enqueue(pair.draft(&format!("inherited-{boundary}"),
            Operation::Start { authorization: Box::new(authorization) })).unwrap();
        if boundary == "undispatched-withdrawal" {
            pair.seat.admit(&start).unwrap();
            assert!(pair.seat.enter(&start.execution_id, |command, authority|
                pair.driver.enter(command, authority)).is_err());
            pair.home.request_cancel(&start.execution_id, "withdraw admitted child before backend").unwrap();
            pair.seat.cancel_or_revoke(&start.execution_id, "withdraw admitted child before backend".into(),
                |command, authority, reason| pair.driver.cancel(command, authority, reason)).unwrap();
        }
        pair.connect();
        pair.settled(&start).await;
        if matches!(boundary, "stop" | "withdraw") {
            assert_eq!(native_file(&pair.root.path().join("inherited-ready")).await, b"ready");
            let local = pair.store.local_session(&start.binding.subject).unwrap().unwrap();
            if boundary == "stop" {
                pair.binding = pair.executor.session_bindings(&pair.selection).unwrap().pop().unwrap();
                pair.authorized().await;
                let stop = pair.home.enqueue(pair.draft("stop-only-inherited-child", Operation::Stop)).unwrap();
                pair.settled(&stop).await;
            } else {
                pair.disconnect().await;
                pair.home.request_cancel(&start.execution_id, "withdraw only inherited child").unwrap();
                pair.seat.cancel_or_revoke(&start.execution_id, "withdraw only inherited child".into(),
                    |command, authority, reason| pair.driver.cancel(command, authority, reason)).unwrap();
                pair.connect();
                pair.stopped(&start).await;
            }
            assert!(pair.store.local_session_retirement_verified(&local).unwrap());
        } else {
            assert!(!pair.root.path().join("inherited-ready").exists());
        }
        pair.disconnect().await;
        let record = pair.store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(record.state, "launched");
        assert!(record.retirement_requested_at.is_none());
        assert!(record.retired_at.is_none());
        for (runtime_id, incarnation) in &survivors {
            let observed = pair.runtime.observe_exec(runtime_id).unwrap().unwrap();
            assert_eq!(observed.incarnation_id.as_deref(), Some(incarnation.as_str()));
            assert_eq!(observed.status, "running");
            assert!(record.roots.iter().find(|root| root.runtime_id == *runtime_id).unwrap().execution_ended_at.is_none());
        }
        if matches!(boundary, "stop" | "withdraw") {
            assert!(record.roots.iter().find(|root| root.runtime_id == "native-worker").unwrap().execution_ended_at.is_some());
        } else {
            assert!(!record.roots.iter().any(|root| root.runtime_id == "native-worker"));
        }
        for (runtime_id, incarnation) in survivors {
            pair.runtime.exec.retire_if(runtime_id, &incarnation).unwrap();
        }
    }
}
