//! Production feed delivery with all six inventories and an independent invalidation socket.
use super::*;
use axum::extract::ws::{Message, WebSocketUpgrade};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use tokio::sync::broadcast;

struct Fixture {
    frames: broadcast::Sender<Value>,
    subscriptions: channel::UnboundedReceiver<(usize, Value)>,
    updates: mpsc::Receiver<Update>,
    commands: channel::UnboundedSender<Command>,
    feed: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    http_reads: Arc<AtomicUsize>,
}

impl Drop for Fixture {
    fn drop(&mut self) { self.feed.abort(); self.server.abort(); }
}

impl Fixture {
    async fn start() -> Self {
        let (frames, _) = broadcast::channel::<Value>(32);
        let outbound = frames.clone();
        let (subscribed, subscriptions) = channel::unbounded_channel();
        let sockets = Arc::new(AtomicUsize::new(0));
        let http_reads = Arc::new(AtomicUsize::new(0));
        let counted = http_reads.clone();
        let app = axum::Router::new()
            .route("/v1/client/capabilities", axum::routing::get(|| async {
                let mut caps: Value = serde_json::from_str(include_str!("../../../docs/st3/client-v0/fixtures/capabilities.json")).unwrap();
                caps["api_version"] = json!(fractal_client::API_VERSION);
                caps["value"]["capabilities"] = json!([
                    {"id":"collections", "version":1, "state":"granted"},
                    {"id":"inventory-collections", "version":1, "state":"granted"},
                    {"id":"read-positions", "version":1, "state":"granted"}
                ]);
                axum::Json(caps)
            }))
            .route("/v1/client/collections/stream", axum::routing::get(move |upgrade: WebSocketUpgrade| {
                let mut frames = outbound.subscribe();
                let subscribed = subscribed.clone();
                let socket_id = sockets.fetch_add(1, Ordering::SeqCst);
                async move {
                    upgrade.protocols(["fractal.client.collections.v0"]).on_upgrade(move |mut socket| async move {
                        let mut held = BTreeSet::new();
                        loop {
                            tokio::select! {
                                incoming = socket.recv() => match incoming {
                                    Some(Ok(Message::Text(text))) => {
                                        let command: Value = serde_json::from_str(&text).unwrap();
                                        let id = command["id"].as_str().unwrap().to_owned();
                                        if command["kind"] == "subscribe" {
                                            held.insert(id);
                                            assert!(held.len() <= 16, "server admission is unchanged");
                                        } else if command["kind"] == "unsubscribe" {
                                            held.remove(&id);
                                        }
                                        if subscribed.send((socket_id, command)).is_err() { return; }
                                    }
                                    Some(Ok(Message::Ping(bytes))) => {
                                        if socket.send(Message::Pong(bytes)).await.is_err() { return; }
                                    }
                                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                                    _ => {}
                                },
                                frame = frames.recv() => {
                                    let Ok(frame) = frame else { return; };
                                    if !held.contains(frame["id"].as_str().unwrap()) { continue; }
                                    if socket.send(Message::Text(frame.to_string().into())).await.is_err() { return; }
                                }
                            }
                        }
                    })
                }
            }))
            .fallback(move || {
                let counted = counted.clone();
                async move { counted.fetch_add(1, Ordering::SeqCst); axum::http::StatusCode::NOT_FOUND }
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::fabric_loopback(format!("http://{}", listener.local_addr().unwrap()), "test-credential");
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let (tx, updates) = mpsc::channel();
        let (commands, rx) = channel::unbounded_channel();
        let feed = tokio::spawn(run_members_with_sidebar(vec![client], false, false, true, Some("person/robin".into()), tx, rx));
        Self { frames, subscriptions, updates, commands, feed, server, http_reads }
    }

    async fn subscription(&mut self) -> (usize, Value) {
        tokio::time::timeout(Duration::from_secs(5), self.subscriptions.recv()).await.unwrap().unwrap()
    }

    async fn update(&self) -> Update {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match self.updates.try_recv() {
                    Ok(update) => return update,
                    Err(mpsc::TryRecvError::Empty) => tokio::task::yield_now().await,
                    Err(mpsc::TryRecvError::Disconnected) => panic!("feed stopped"),
                }
            }
        }).await.unwrap()
    }

    fn snapshot(&self, id: &str) {
        let fixture: Value = serde_json::from_str(include_str!("../../../docs/st3/client-v0/fixtures/agents-publication-chunk.json")).unwrap();
        self.frames.send(json!({"kind":"snapshot", "id":id, "collection":if id == "usage-24" { "usage" } else { id },
            "snapshot":fixture["snapshot"], "items":[], "order":[], "has_more":false})).unwrap();
    }
}

#[tokio::test]
async fn six_inventories_keep_delivering_beside_read_position_invalidations() {
    let mut fixture = Fixture::start().await;
    fixture.commands.send(Command::Inventories(Inventories {
        clients: true, usage_hours: Some(24), machines: true, devices: true, sessions: true, terminals: true,
    })).unwrap();
    let mut seen = BTreeMap::new();
    while seen.len() < 10 {
        let (socket, command) = fixture.subscription().await;
        assert_eq!(command["kind"], "subscribe");
        seen.insert(command["id"].as_str().unwrap().to_owned(), socket);
    }
    let positions_socket = seen["read-positions"];
    assert_eq!(seen.values().filter(|socket| **socket == positions_socket).count(), 1,
        "the extra socket subscribes only to read positions");
    let ids = ["clients", "usage-24", "machines", "devices", "sessions", "terminals", "read-positions"];
    for id in ids { fixture.snapshot(id); }
    let mut delivered = BTreeSet::new();
    while delivered.len() < ids.len() {
        match fixture.update().await {
            Update::Window { window, .. } => { delivered.insert(window.id()); }
            Update::WindowFailed(window, reason) => panic!("{window:?}: {reason}"),
            _ => {}
        }
    }
    assert!(delivered.contains("read-positions"));
    // A second invalidation must still arrive after every inventory has delivered.
    let wire: Value = serde_json::from_str(include_str!("../../../docs/st3/client-v0/fixtures/agents-publication-chunk.json")).unwrap();
    fixture.frames.send(json!({"kind":"changes", "id":"read-positions", "collection":"read-positions",
        "snapshot":wire["snapshot"], "upserts":[], "removes":[], "order":[], "has_more":false})).unwrap();
    loop {
        if let Update::Window { window: Window::ReadPositions, .. } = fixture.update().await { break; }
    }
    assert_eq!(fixture.http_reads.load(Ordering::SeqCst), 0, "delivery never substitutes HTTP listing");
}

#[tokio::test]
async fn refused_read_position_subscription_surfaces_without_retrying_or_listing() {
    let mut fixture = Fixture::start().await;
    loop {
        let (_, command) = fixture.subscription().await;
        if command["id"] == "read-positions" { break; }
    }
    fixture.frames.send(json!({"kind":"error", "id":"read-positions", "message":"not allowed", "retryable":false})).unwrap();
    loop {
        if let Update::WindowFailed(Window::ReadPositions, reason) = fixture.update().await {
            assert!(reason.contains("not allowed"));
            break;
        }
    }
    // The main stream remains usable after the dedicated subscription is refused.
    fixture.snapshot("attention");
    loop {
        if let Update::Window { window: Window::Attention, .. } = fixture.update().await { break; }
    }
    assert_eq!(fixture.http_reads.load(Ordering::SeqCst), 0);
    while let Ok((_, command)) = fixture.subscriptions.try_recv() {
        assert_ne!(command["id"], "read-positions", "a refusal must not resubscribe");
    }
}
