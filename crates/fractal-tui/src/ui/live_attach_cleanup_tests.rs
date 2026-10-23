//! Exercise the real feed against a server that retains failed-subscription bookkeeping.
use crate::feed::{self, Command, TerminalUpdate, Update};
use axum::{Json, Router, extract::{State, WebSocketUpgrade, ws::Message}, response::Response, routing::{get, post}};
use fractal_client::Client;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::{Arc, mpsc}, time::Duration};
use tokio::sync::Mutex;

#[derive(Default)]
struct Bookkeeping {
    follows: BTreeSet<String>,
    presence: BTreeSet<String>,
    terminals_opened: BTreeSet<String>,
    subscriptions: usize,
    cleanups: usize,
    connected: bool,
}

#[derive(Clone)]
struct FakeServer {
    bookkeeping: Arc<Mutex<Bookkeeping>>,
    cleaned: mpsc::Sender<String>,
}

fn envelope() -> Value {
    serde_json::from_str(include_str!("../../../../docs/st3/client-v0/fixtures/terminal-screen.json")).unwrap()
}

async fn capabilities() -> Json<Value> {
    Json(serde_json::from_str(include_str!("../../../../docs/st3/client-v0/fixtures/capabilities.json")).unwrap())
}

async fn runtime() -> Json<Value> {
    let mut response = envelope();
    response["value"] = json!({
        "kind": "runtime", "id": "runtime/demo", "revision": "r1",
        "updated_at": "2026-10-23T00:00:00Z", "runtime_kind": "agent",
        "owner_id": "agent/demo", "owner_host_id": "host/demo", "state": "running",
        "runtime_id": "demo", "incarnation_id": "demo:i1",
        "terminal_id": "terminal/demo", "terminal_sequence": 1
    });
    Json(response)
}

async fn action(Json(request): Json<Value>) -> Json<Value> {
    let mut response = envelope();
    response["value"] = json!({
        "kind": "action-result", "action_id": request["id"],
        "operation_id": "operation/test", "status": "completed",
        "affected_ids": [], "snapshot_id": response["snapshot"]["id"]
    });
    if request["type"] == "terminal.attach" {
        response["value"]["terminal_attachment"] = json!({
            "attachment_id": "terminal-attachment/test", "terminal_id": "terminal/demo",
            "runtime_incarnation": "demo:i1", "owner_host_id": "host/demo",
            "stream_url": "/v1/client/terminals/demo/stream", "stream_capability": "test-capability",
            "state": "attached", "expires_at": "2099-01-01T00:00:00Z"
        });
    }
    Json(response)
}

async fn collections(State(server): State<FakeServer>, ws: WebSocketUpgrade) -> Response {
    ws.protocols(["st3.client.collections.v0"]).on_upgrade(move |mut socket| async move {
        server.bookkeeping.lock().await.connected = true;
        while let Some(message) = socket.recv().await {
            let Ok(Message::Text(text)) = message else { break; };
            let command: Value = serde_json::from_str(text.as_str()).unwrap();
            let id = command["id"].as_str().unwrap_or_default().to_owned();
            if command["kind"] == "subscribe" && command["collection"] == "terminal" {
                {
                    let mut state = server.bookkeeping.lock().await;
                    state.subscriptions += 1;
                    state.follows.insert(id.clone());
                    state.presence.insert(id.clone());
                    state.terminals_opened.insert(id.clone());
                    // Match the daemon error path: its receiver goes away, but the socket's
                    // presence and pre-first-screen opened entries survive until Unsubscribe.
                    state.follows.remove(&id);
                }
                let error = json!({
                    "kind": "error", "id": id, "code": fractal_client::ErrorCode::Forbidden,
                    "message": "fixture refuses before the first screen"
                });
                if socket.send(Message::Text(error.to_string().into())).await.is_err() { break; }
            } else if command["kind"] == "unsubscribe" {
                let removed = {
                    let mut state = server.bookkeeping.lock().await;
                    let follow = state.follows.remove(&id);
                    let presence = state.presence.remove(&id);
                    let opened = state.terminals_opened.remove(&id);
                    let removed = follow || presence || opened;
                    if removed { state.cleanups += 1; }
                    removed
                };
                if removed { let _ = server.cleaned.send(id); }
            }
        }
        server.bookkeeping.lock().await.connected = false;
    })
}

#[tokio::test]
async fn failed_generations_leave_no_follow_presence_or_opened_entries_before_disconnect() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("feed.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (cleaned_tx, cleaned_rx) = mpsc::channel();
    let bookkeeping = Arc::new(Mutex::new(Bookkeeping::default()));
    let app = Router::new()
        .route("/v1/client/collections/stream", get(collections))
        .route("/v1/client/capabilities", get(capabilities))
        .route("/v1/client/runtimes/{id}", get(runtime))
        .route("/v1/client/actions", post(action))
        .with_state(FakeServer { bookkeeping: bookkeeping.clone(), cleaned: cleaned_tx });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let (updates_tx, updates_rx) = mpsc::channel();
    let (commands_tx, commands_rx) = tokio::sync::mpsc::unbounded_channel();
    let feed = tokio::spawn(feed::run(Client::unix_as(&socket, "person/test"), updates_tx, commands_rx));
    tokio::task::spawn_blocking(move || {
        const GENERATIONS: u64 = 8;
        for generation in 1..=GENERATIONS {
            commands_tx.send(Command::Follow { runtime_ids: vec!["runtime/demo".into()], generation }).unwrap();
            loop {
                let update = updates_rx.recv_timeout(Duration::from_secs(5)).expect("feed must report the failed generation");
                if let Update::Terminal { generation: result, update: TerminalUpdate::Ended { .. } } = update {
                    assert_eq!(result, generation);
                    break;
                }
            }
            let id = cleaned_rx.recv_timeout(Duration::from_secs(5)).expect("failed subscription must be explicitly cleaned up");
            assert_eq!(id, format!("terminal/{generation}"));
            let state = bookkeeping.blocking_lock();
            assert!(state.connected, "cleanup must not depend on a client disconnect");
            assert!(state.follows.is_empty());
            assert!(state.presence.is_empty());
            assert!(state.terminals_opened.is_empty());
            assert_eq!(state.subscriptions, generation as usize);
            assert_eq!(state.cleanups, generation as usize);
        }
        drop(commands_tx);
    }).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), feed).await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
}
