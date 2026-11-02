//! Exercise the production feed over a controlled collection socket, using the canonical wire fixtures.
use super::*;
use axum::extract::ws::{Message, WebSocketUpgrade};
use serde_json::{Value, json};
use tokio::sync::broadcast;

struct Fixture {
    frames: broadcast::Sender<Value>,
    subscriptions: channel::UnboundedReceiver<Value>,
    updates: mpsc::Receiver<Update>,
    _commands: channel::UnboundedSender<Command>,
    feed: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) { self.feed.abort(); self.server.abort(); }
}

impl Fixture {
    async fn start() -> Self {
        let (frames, _) = broadcast::channel::<Value>(16);
        let outbound = frames.clone();
        let (subscribed, subscriptions) = channel::unbounded_channel();
        let app = axum::Router::new()
            .route("/v1/client/capabilities", axum::routing::get(|| async {
                let mut caps: Value = serde_json::from_str(include_str!("../../../docs/st3/client-v0/fixtures/capabilities.json")).unwrap();
                caps["api_version"] = json!(fractal_client::API_VERSION);
                caps["value"]["capabilities"] = json!([
                    {"id":"collections", "version":1, "state":"granted"},
                    {"id":"collections.agents.revisioned.v1", "version":1, "state":"granted"}
                ]);
                axum::Json(caps)
            }))
            .route("/v1/client/collections/stream", axum::routing::get(move |upgrade: WebSocketUpgrade| {
                let mut frames = outbound.subscribe();
                let subscribed = subscribed.clone();
                async move {
                    upgrade.protocols(["fractal.client.collections.v0"]).on_upgrade(move |mut socket| async move {
                        loop {
                            tokio::select! {
                                incoming = socket.recv() => match incoming {
                                    Some(Ok(Message::Text(text))) => {
                                        let command: Value = serde_json::from_str(&text).unwrap();
                                        if command["kind"] == "subscribe" && command["collection"] == "agents" {
                                            assert_eq!(command["agents_publication_version"], 1);
                                            assert!(command.get("limit").is_none());
                                            if subscribed.send(command).is_err() { return; }
                                        }
                                    }
                                    Some(Ok(Message::Ping(bytes))) => {
                                        if socket.send(Message::Pong(bytes)).await.is_err() { return; }
                                    }
                                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                                    _ => {}
                                },
                                frame = frames.recv() => {
                                    let Ok(frame) = frame else { return; };
                                    if socket.send(Message::Text(frame.to_string().into())).await.is_err() { return; }
                                }
                            }
                        }
                    })
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::fabric_loopback(format!("http://{}", listener.local_addr().unwrap()), "test-credential");
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let (updates_tx, updates) = mpsc::channel();
        let (commands, receiver) = channel::unbounded_channel();
        let feed = tokio::spawn(run_members(vec![client], false, false, updates_tx, receiver));
        let mut fixture = Self { frames, subscriptions, updates, _commands: commands, feed, server };
        fixture.subscribed().await;
        fixture
    }

    async fn subscribed(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), self.subscriptions.recv()).await
            .expect("the feed must subscribe after connecting").expect("the server stays alive");
    }

    fn send(&self, frame: Value) { self.frames.send(frame).unwrap(); }

    async fn update(&self) -> Update {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match self.updates.try_recv() {
                    Ok(update) => return update,
                    Err(mpsc::TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(10)).await,
                    Err(mpsc::TryRecvError::Disconnected) => panic!("the feed stopped"),
                }
            }
        }).await.expect("the feed must emit the expected update")
    }

    async fn agents(&self) -> Vec<Resource> {
        loop {
            if let Update::Window { window: Window::Agents, items, has_more, .. } = self.update().await {
                assert!(!has_more);
                return items;
            }
        }
    }
}

fn chunk(index: u64, count: u64, id: &str, revision: u64) -> Value {
    let mut frame: Value = serde_json::from_str(include_str!("../../../docs/st3/client-v0/fixtures/agents-publication-chunk.json")).unwrap();
    frame["id"] = json!("agents");
    frame["chunk_index"] = json!(index);
    frame["chunk_count"] = json!(count);
    frame["publication"]["revision"] = json!(revision);
    frame["items"][0]["id"] = json!(id);
    frame["order"] = json!([id]);
    frame
}

#[tokio::test]
async fn revisioned_agents_feed_installs_chunks_atomically_then_applies_whole_row_delta() {
    let fixture = Fixture::start().await;
    let first = chunk(0, 2, "agent/a", 42);
    fixture.send(first.clone());
    // A different window is an ordered processing barrier: once it arrives, chunk zero was
    // consumed, but no agents replacement may have escaped.
    fixture.send(json!({"kind":"snapshot", "id":"attention", "collection":"attention",
        "snapshot":first["snapshot"], "items":[], "order":[], "has_more":false}));
    loop {
        match fixture.update().await {
            Update::Window { window: Window::Agents, .. } => panic!("a partial roster escaped"),
            Update::Window { window: Window::Attention, .. } => break,
            _ => {}
        }
    }
    fixture.send(chunk(1, 2, "agent/b", 42));
    let items = fixture.agents().await;
    assert_eq!(items.iter().map(|item| item.header().id.as_str()).collect::<Vec<_>>(), ["agent/a", "agent/b"]);
    let mut changed = chunk(0, 1, "agent/c", 43);
    changed["kind"] = json!("changes");
    changed["base_revision"] = json!(42);
    changed["upserts"] = changed["items"].take();
    changed["removes"] = json!(["agent/a"]);
    changed["order"] = json!(["agent/b", "agent/c"]);
    changed.as_object_mut().unwrap().remove("chunk_index");
    changed.as_object_mut().unwrap().remove("chunk_count");
    changed.as_object_mut().unwrap().remove("items");
    fixture.send(changed);
    let items = fixture.agents().await;
    assert_eq!(items.iter().map(|item| item.header().id.as_str()).collect::<Vec<_>>(), ["agent/b", "agent/c"]);
}

#[tokio::test]
async fn revisioned_agents_feed_fences_wrong_base_and_reconnects_with_a_complete_snapshot() {
    let mut fixture = Fixture::start().await;
    fixture.send(chunk(0, 1, "agent/held", 42));
    assert_eq!(fixture.agents().await[0].header().id, "agent/held");
    let initial = chunk(0, 1, "agent/invalid", 44);
    fixture.send(json!({"kind":"changes", "id":"agents", "collection":"agents",
        "snapshot":initial["snapshot"], "publication":initial["publication"],
        "base_revision":41, "upserts":initial["items"], "removes":["agent/held"],
        "order":["agent/invalid"], "has_more":false}));
    loop {
        match fixture.update().await {
            Update::Window { window: Window::Agents, .. } => panic!("a delta with the wrong base escaped"),
            Update::Degraded(reason) | Update::Offline(reason) => {
                assert!(reason.contains("held publication"));
                break;
            }
            _ => {}
        }
    }
    fixture.subscribed().await;
    fixture.send(chunk(0, 1, "agent/current", 45));
    let items = fixture.agents().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].header().id, "agent/current");
}

#[tokio::test]
async fn revisioned_agents_permanent_error_surfaces_without_resubscribing() {
    let mut fixture = Fixture::start().await;
    fixture.send(json!({"kind":"error", "id":"agents", "collection":"agents",
        "retryable":false, "message":"one agents row exceeds the publication frame limit"}));
    loop {
        match fixture.update().await {
            Update::WindowFailed(Window::Agents, message) => {
                assert_eq!(message, "one agents row exceeds the publication frame limit");
                break;
            }
            Update::Window { window: Window::Agents, .. } => panic!("a refused roster escaped"),
            _ => {}
        }
    }
    // The first retry, including its maximum jitter, would arrive before this deadline.
    assert!(tokio::time::timeout(RETRY_DELAYS[0] * 2, fixture.subscriptions.recv()).await.is_err(),
        "a permanent refusal must not prepare the full publication again");
}

#[tokio::test]
async fn revisioned_agents_permanent_resync_cancels_an_already_scheduled_retry() {
    let mut fixture = Fixture::start().await;
    fixture.send(chunk(0, 2, "agent/partial", 42));
    fixture.send(json!({"kind":"error", "id":"agents", "collection":"agents",
        "retryable":true, "message":"the source is catching up"}));
    fixture.send(json!({"kind":"resync", "id":"agents", "collection":"agents",
        "retryable":false, "message":"the agents publication cannot be served"}));
    loop {
        match fixture.update().await {
            Update::WindowFailed(Window::Agents, message) => {
                assert_eq!(message, "the agents publication cannot be served");
                break;
            }
            Update::Window { window: Window::Agents, .. } => panic!("a partial roster escaped"),
            _ => {}
        }
    }
    assert!(tokio::time::timeout(RETRY_DELAYS[0] * 2, fixture.subscriptions.recv()).await.is_err(),
        "a permanent resync must cancel the previous retry");
}

#[tokio::test]
async fn revisioned_agents_retryable_error_resubscribes_and_recovers() {
    let mut fixture = Fixture::start().await;
    fixture.send(chunk(0, 2, "agent/partial", 42));
    fixture.send(json!({"kind":"error", "id":"agents", "collection":"agents",
        "retryable":true, "message":"the source is catching up"}));
    fixture.subscribed().await;
    fixture.send(chunk(0, 1, "agent/recovered", 43));
    let items = fixture.agents().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].header().id, "agent/recovered");
}
