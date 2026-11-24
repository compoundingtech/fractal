//! Actual SQLite journals, authenticated loopback transport and native executor custody.
//! The seat graph is unavailable throughout observe/stop and reconnect recovery.
use super::*;
use fractal_peer::light_node::journal::{Admission, CommandDraft, Limits, OperationKind, ReceiptState, Scope};
use fractal_peer::light_node::session::{Home, SeatClient, StoreRunEffects};
use crate::store::local_sessions::{LocalSession, SessionCustody};
use smallclaims::{fleet::MemberKey, sync::FleetAuth};
use tokio::sync::watch;

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
        let root = tempfile::tempdir().unwrap();
        let home_key = Arc::new(MemberKey::generate().unwrap().0);
        let seat_key = Arc::new(MemberKey::generate().unwrap().0);
        let selection = Selection {
            home: "example-home".into(), home_key: home_key.public().into(),
            host: "example-seat".into(), host_key: seat_key.public().into(), epoch: 1,
            grant: Scope { subjects: vec!["agent/example/worker".into()],
                operations: vec![OperationKind::Observe, OperationKind::Stop] },
        };
        let home = Journal::open(&root.path().join("home.sqlite3"), Limits::default()).unwrap();
        let seat = Journal::open(&root.path().join("seat.sqlite3"), Limits::default()).unwrap();
        home.install_epoch(&selection).unwrap();
        seat.install_epoch(&selection).unwrap();
        let store = Arc::new(Store::open(&root.path().join("native.sqlite3"), selection.host.clone()).unwrap());
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        let binary = fractal_runtime::resolve_executable("pty", &environment).expect("native session fixtures require the real pty binary");
        let runtime = Arc::new(NativeRuntime::new(root.path(), Some(&root.path().join("pty")), &binary));
        runtime.pty().unwrap().spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
            root.path(), &environment, None, &BTreeMap::new(), None).unwrap();
        let generation = runtime.snapshot_ptys().unwrap().into_iter().find(|entry| entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
        let local = LocalSession {
            subject: selection.grant.subjects[0].clone(), runtime_id: "native-worker".into(),
            incarnation: format!("owned:{generation}"), native_incarnation: generation,
            terminal: true, driver: None, lifecycle: MemberLifecycle::Service, custody: SessionCustody::Owned,
            start_operation: None, invocation: None,
        };
        store.bind_local_session(&local).unwrap();
        let executor = Arc::new(Reconciler::new(store.clone(), runtime.clone(), selection.host.clone(), Arc::new(Notify::new())));
        executor.install_home_session_journal(seat.clone()).unwrap();
        let binding = executor.session_bindings(&selection).unwrap().pop().unwrap();
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
        pair.connect();
        pair.authorized().await;
        pair.disconnect().await;
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
                if self.home.settled_prefix("home", command.binding.epoch).unwrap() >= command.sequence { break; }
                changed.await;
            }
        }).await.expect("real native result must reach durable home custody");
        match self.home.command_receipt(&command.execution_id) {
            Ok(Some(receipt)) => assert!(matches!(receipt.state, ReceiptState::Settled { .. })),
            Err(error) => assert!(error.to_string().contains("result-expired"), "{error}"),
            Ok(None) => panic!("settled command cannot lose its receipt before explicit joint retirement"),
        }
    }
}
impl Drop for NativePair {
    fn drop(&mut self) {
        if let Some(client) = &self.client { client.abort(); }
        self.home_service.shutdown();
        self.server.abort();
        let _ = self.runtime.pty().and_then(|pty| pty.stop("native-worker"));
    }
}

#[tokio::test]
async fn native_receipt_replay_reconnect_and_offline_stop_need_no_seat_graph() {
    let mut pair = NativePair::new().await;
    let observe = pair.home.enqueue(pair.draft("observe-before-ack-loss", Operation::Observe)).unwrap();
    let refused = pair.home.enqueue(pair.draft("unsupported-start", Operation::Start)).unwrap_err();
    assert_eq!(refused.downcast_ref::<FractalError>().unwrap().code, "light-node-start-not-supported");
    let mut bypass = observe.clone();
    bypass.operation = Operation::Start;
    let refused = pair.seat.with_authority(|authority| pair.driver.enter(&bypass, authority)).unwrap_err();
    assert_eq!(refused.downcast_ref::<FractalError>().unwrap().code, "light-node-start-not-supported");
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
    let original_native = pair.store.local_session_at(&pair.binding.subject, &pair.binding.incarnation).unwrap().unwrap().native_incarnation;
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
    stop_local_session(&pair.store, pair.runtime.as_ref(), &binding.subject, &binding.incarnation, "fresh-offline-stop").unwrap();
    assert!(local_session(&pair.store, pair.runtime.as_ref(), &binding.subject, None).is_err());
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
    pair.runtime.remove("native-worker", true).unwrap();
    pair.runtime.pty().unwrap().spawn("native-worker", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
        pair.root.path(), &std::env::vars().collect(), None, &BTreeMap::new(), None).unwrap();
    let replacement = pair.runtime.snapshot_ptys().unwrap().into_iter().find(|entry|
        entry.runtime_id == "native-worker").unwrap().incarnation_id.unwrap();
    assert!(pair.driver.recover(recovery).await.is_err());
    let actual = pair.runtime.snapshot_ptys().unwrap().into_iter().find(|entry|
        entry.runtime_id == "native-worker").unwrap();
    assert_eq!(actual.status, "running");
    assert_eq!(actual.incarnation_id.as_deref(), Some(replacement.as_str()));
    assert!(pair.seat.recoverable().unwrap().iter().any(|entry|
        entry.command.execution_id == stop.execution_id && entry.custody_proof.is_none()));
}

#[tokio::test]
async fn positively_retired_unknown_entry_keeps_its_native_snapshot_through_a_store_failure() {
    let mut pair = NativePair::new().await;
    let stop = pair.home.enqueue(pair.draft("unknown-entry-positive-native-retirement", Operation::Stop)).unwrap();
    pair.seat.admit(&stop).unwrap();
    pair.seat.enter(&stop.execution_id, |_, _| anyhow::bail!("interrupted native entry")).unwrap_err();
    assert!(stop_local_session_with_retirement_proof(&pair.store, pair.runtime.as_ref(),
        &pair.binding.subject, &pair.binding.incarnation, "independent-local-retirement").unwrap());
    pair.store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h9089b8a75c90 owner=fractal_reconcile::reconcile::home_session_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER fail_native_finalization BEFORE UPDATE OF custody_settled ON local_session_custody
         WHEN NEW.custody_settled=1 BEGIN SELECT RAISE(ABORT,'injected native finalization failure'); END;"
    ).unwrap();
    let recovery = pair.seat.recoverable().unwrap().into_iter().find(|entry|
        entry.command.execution_id == stop.execution_id).unwrap();
    assert!(pair.driver.recover(recovery).await.is_err());
    let pending = pair.seat.pending_terminal_results().unwrap();
    assert_eq!(pending.len(), 1);
    let proof = pending[0].0.custody_proof.as_deref().unwrap();
    assert!(matches!(serde_json::from_str::<NativeProof>(proof).unwrap(), NativeProof::Stop { retired: true, .. }));
    assert!(pair.store.has_current_local_session(&pair.binding.subject).unwrap());
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
    pair.disconnect().await;
}
