//! Private notes over the Unix API, with full daemon state/store teardown between runs.
#![cfg(unix)]

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fractal_client::{Client, ClientError, ErrorCode, Fence, PrivateNotesWriteParameters};
use fractal_daemon::api::AppState;
use fractal_daemon::private_notes::Authority;
use fractal_daemon::store::Store;
use tokio::sync::{Notify, watch};

const NODE: &str = "notes-restart";
const ACTOR: &str = "person/operator";
const URI: &str = "dev.schickling.agent-private-notes://notes-restart/worker";

struct Daemon {
    root: tempfile::TempDir,
    state: Option<AppState>,
    server: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

impl Daemon {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let subject = root.path().join("catalog/agents/notes-restart/worker");
        std::fs::create_dir_all(subject.join("resources")).unwrap();
        std::fs::write(
            subject.join("agent.kdl"),
            format!("agent \"worker\" {{\n host \"{NODE}\"\n resource \"notes\" uri=\"{URI}\"\n}}\n"),
        )
        .unwrap();
        let mut daemon = Self { root, state: None, server: None };
        daemon.start().await;
        daemon
    }

    fn state(root: &Path) -> AppState {
        AppState {
            owning_person: None,
            store: Arc::new(Store::open(&root.join("graph.db"), NODE).unwrap()),
            notify: Arc::new(Notify::new()),
            event_notify: watch::channel(0_u64).0,
            node: NODE.into(),
            state_dir: root.into(),
            pty_root: root.join("pty"),
            pty_binary: "pty".into(),
            fleet_id: None,
            configured_peers: Vec::new(),
            client_relay: None,
            native_session_home: Some(root.join("native")),
            planner_default: Default::default(),
            assistant_receipts: Default::default(),
            private_notes: Arc::new(Authority { catalogs: vec![root.join("catalog")] }),
            harness_question_sink: None,
        }
    }

    fn socket(&self) -> PathBuf {
        self.root.path().join("st3.sock")
    }

    fn carrier(&self) -> PathBuf {
        self.root.path().join("catalog/agents/notes-restart/worker/resources/private-notes.md")
    }

    fn client(&self) -> Client {
        Client::unix_as(self.socket(), ACTOR)
    }

    async fn start(&mut self) {
        assert!(self.state.is_none());
        let state = Self::state(self.root.path());
        let router = fractal_daemon::api::router(state.clone());
        self.state = Some(state);
        let socket = self.socket();
        let (listening, ready) = tokio::sync::oneshot::channel();
        self.server = Some(tokio::spawn(async move {
            fractal_daemon::api::serve_unix_with_ready(&socket, router, move || {
                let _ = listening.send(());
            })
            .await
        }));
        tokio::time::timeout(Duration::from_secs(120), ready).await.unwrap().unwrap();
        self.client().capabilities().await.unwrap();
    }

    async fn restart(&mut self) {
        let server = self.server.take().unwrap();
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        let old = self.state.take().unwrap();
        let store = Arc::downgrade(&old.store);
        let authority = Arc::downgrade(&old.private_notes);
        drop(old);
        // Accepted HTTP connections have their own tasks. Wait for their closed
        // clients to drain too, not merely for the listener's cancellation.
        tokio::time::timeout(Duration::from_secs(120), async {
            while store.strong_count() != 0 || authority.strong_count() != 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("old daemon connections did not release their state");
        // These assertions prohibit a restart that only replaces Authority while retaining
        // the old router, store, or AppState through another owner.
        assert!(store.upgrade().is_none(), "old daemon store is still alive");
        assert!(authority.upgrade().is_none(), "old notes authority is still alive");
        std::fs::remove_file(self.socket()).unwrap();
        self.start().await;
    }

    async fn fence(&self) -> Fence {
        self.client().private_notes_get(URI).await.unwrap().value.actions[0].fence.clone()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

fn file_identity(path: &Path) -> (u64, i64, i64) {
    let metadata = std::fs::metadata(path).unwrap();
    (metadata.ino(), metadata.mtime(), metadata.mtime_nsec())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_and_pending_private_notes_writes_survive_daemon_restart() {
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    let mut daemon = Daemon::new().await;
    let first_fence = daemon.fence().await;
    let first = PrivateNotesWriteParameters { uri: URI.into(), markdown: "completed private context\n".into() };
    let receipt = daemon.client().private_notes_write(
        "action/notes-completed", "notes-completed-restart-0001", first_fence.clone(), first.clone(),
    ).await.unwrap().value;
    let completed_identity = file_identity(&daemon.carrier());

    daemon.restart().await;
    assert_eq!(daemon.client().private_notes_get(URI).await.unwrap().value.data.markdown, first.markdown);
    let index = daemon.state.as_ref().unwrap().store.index().unwrap();
    let replay = daemon.client().private_notes_write(
        "action/notes-completed", "notes-completed-restart-0001", first_fence, first,
    ).await.unwrap().value;
    assert_eq!(replay.private_notes, receipt.private_notes);
    assert_eq!(replay.operation_id, receipt.operation_id);
    assert_eq!(daemon.state.as_ref().unwrap().store.index().unwrap(), index);
    assert_eq!(file_identity(&daemon.carrier()), completed_identity);

    let pending_fence = daemon.fence().await;
    let pending = PrivateNotesWriteParameters { uri: URI.into(), markdown: "pending durable private context\n".into() };
    // The real API publishes requested audit and replaces the carrier before this
    // SQLite fault refuses completion. Its private ledger must retain recovery state.
    let graph = rusqlite::Connection::open(daemon.root.path().join("graph.db")).unwrap();
    graph.execute_batch("CREATE TRIGGER refuse_notes_completion BEFORE INSERT ON claims
        WHEN NEW.kind='resource.observed' AND json_extract(NEW.body,'$.fields.stage')='completed'
        BEGIN SELECT RAISE(FAIL, 'completed audit refusal'); END;").unwrap();
    assert!(matches!(daemon.client().private_notes_write(
        "action/notes-pending", "notes-pending-restart-0001", pending_fence.clone(), pending.clone(),
    ).await, Err(ClientError::Api(ErrorCode::PrivateNotesIndeterminate, ..))));
    assert_eq!(std::fs::read_to_string(daemon.carrier()).unwrap(), pending.markdown);
    let pending_identity = file_identity(&daemon.carrier());
    drop(graph);

    daemon.restart().await;
    assert_eq!(daemon.client().private_notes_get(URI).await.unwrap().value.data.markdown, pending.markdown);
    let later_fence = daemon.fence().await;
    let later = PrivateNotesWriteParameters { uri: URI.into(), markdown: "must wait for recovery\n".into() };
    assert!(matches!(daemon.client().private_notes_write(
        "action/notes-later", "notes-later-restart-0001", later_fence.clone(), later.clone(),
    ).await, Err(ClientError::Api(ErrorCode::PrivateNotesIndeterminate, ..))));
    let graph = rusqlite::Connection::open(daemon.root.path().join("graph.db")).unwrap();
    graph.execute_batch("DROP TRIGGER refuse_notes_completion;").unwrap();
    drop(graph);
    let recovered = daemon.client().private_notes_write(
        "action/notes-pending", "notes-pending-restart-0001", pending_fence.clone(), pending.clone(),
    ).await.unwrap().value;
    let index = daemon.state.as_ref().unwrap().store.index().unwrap();
    let repeated = daemon.client().private_notes_write(
        "action/notes-pending", "notes-pending-restart-0001", pending_fence, pending,
    ).await.unwrap().value;
    assert_eq!(recovered.private_notes, repeated.private_notes);
    assert_eq!(recovered.operation_id, repeated.operation_id);
    assert_eq!(daemon.state.as_ref().unwrap().store.index().unwrap(), index);
    assert_eq!(file_identity(&daemon.carrier()), pending_identity, "recovery must not rewrite durable bytes");
    // Completion unblocks a subsequent fenced write, rather than merely returning a receipt.
    daemon.client().private_notes_write(
        "action/notes-later", "notes-later-restart-0001", later_fence, later.clone(),
    ).await.unwrap();
    assert_eq!(std::fs::read_to_string(daemon.carrier()).unwrap(), later.markdown);
}
