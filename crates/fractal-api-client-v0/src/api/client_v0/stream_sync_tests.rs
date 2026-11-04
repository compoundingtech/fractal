//! Attention window frames state this host's sync (docs/st3/client-v0/collections.md, Sync on
//! the stream): every snapshot says it, `level` included, and a changes frame says it only when
//! its state or the peers it names changed since that subscription's previous frame.
use super::*;
use crate::PublishedViewsAttentionStore;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

type Socket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A socket whose windows each read one row that never changes, so only `sync` changes a frame.
async fn open(state: &AppState) -> (Socket, tokio::task::JoinHandle<()>) {
    let serving = state.clone();
    let app = axum::Router::new().route(
        "/stream",
        axum::routing::get(move |upgrade: WebSocketUpgrade| {
            let state = serving.clone();
            async move {
                upgrade.on_upgrade(move |socket| {
                    collection_stream_socket_with_reader(
                        socket,
                        state,
                        ClientSession::local(None).unwrap(),
                        None,
                        |state: AppState,
                         _: ClientSession,
                         request: CollectionSubscribe,
                         _: tokio::sync::OwnedSemaphorePermit| async move {
                            let row = json!({"id": format!("{}/row", request.collection)});
                            Ok((new_client_snapshot(&state), vec![row], false, None))
                        },
                    )
                })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/stream"))
        .await
        .unwrap();
    (socket, server)
}

async fn subscribe(socket: &mut Socket, id: &str, collection: &str) {
    let command = json!({"kind":"subscribe","id":id,"collection":collection,"limit":200});
    socket.send(Message::Text(command.to_string().into())).await.unwrap();
}

async fn frame(socket: &mut Socket) -> Value {
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let message = tokio::time::timeout_at(until, socket.next())
            .await
            .expect("a collection frame")
            .unwrap()
            .unwrap();
        // Protocol pings carry no frame.
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// The next frame of each subscription named, by id.
async fn frames(socket: &mut Socket, ids: &[&str]) -> BTreeMap<String, Value> {
    let mut frames = BTreeMap::new();
    while frames.len() < ids.len() {
        let frame = frame(socket).await;
        let id = frame["id"].as_str().unwrap().to_owned();
        assert!(ids.contains(&id.as_str()), "unexpected frame: {frame}");
        assert!(frames.insert(id, frame).is_none(), "a second frame before the others");
    }
    frames
}

/// No frame arrives for longer than a paced reread can take.
async fn quiet(socket: &mut Socket) {
    let until = tokio::time::Instant::now() + COLLECTION_REREAD_INTERVAL + Duration::from_millis(500);
    while let Ok(message) = tokio::time::timeout_at(until, socket.next()).await {
        if let Message::Text(text) = message.unwrap().unwrap() {
            panic!("unexpected frame: {text}");
        }
    }
}

/// This host measured `behind` envelopes it lacks from `peer`: more than one exchange's worth is
/// catching up.
fn measure(state: &AppState, peer: &str, behind: u64) {
    state
        .store
        .replication_sync
        .lock()
        .unwrap()
        .entry(peer.into())
        .or_default()
        .observe(0, Some((behind, 0)), smallclaims::store::now_ms());
}

/// Its graph has differed from `peer`'s for longer than divergence takes.
fn diverge(state: &AppState, peer: &str) {
    let now = smallclaims::store::now_ms();
    let mut sync = state.store.replication_sync.lock().unwrap();
    let progress = sync.get_mut(peer).unwrap();
    progress.compare_graphs(false, now - smallclaims::store::REPLICATION_DIVERGED_AFTER_MS);
    progress.compare_graphs(false, now);
}

/// A frame's `sync` as its state and the peers it names.
fn said(frame: &Value) -> (String, Vec<String>) {
    let sync = &frame["sync"];
    assert!(sync.is_object(), "no sync: {frame}");
    sync_said(sync)
}

fn level() -> (String, Vec<String>) {
    ("level".into(), vec![])
}

fn behind(state: &str, peers: &[&str]) -> (String, Vec<String>) {
    (state.into(), peers.iter().map(|peer| (*peer).to_owned()).collect())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attention_window_states_its_sync_on_every_snapshot_and_only_each_change() {
    let root = tempfile::tempdir().unwrap();
    let mut state = super::tests::test_state_named(root.path(), "hub");
    state.configured_peers = vec!["edge".into(), "fir".into()];
    let (mut socket, server) = open(&state).await;
    // A level host says so; a collection that does not carry sync says nothing.
    subscribe(&mut socket, "a", "attention").await;
    subscribe(&mut socket, "missions", "missions").await;
    let first = frames(&mut socket, &["a", "missions"]).await;
    assert_eq!(first["a"]["kind"], "snapshot");
    assert_eq!(said(&first["a"]), level());
    assert!(first["missions"].get("sync").is_none(), "{}", first["missions"]);

    // Starting to catch up rereads the window at once, and a changes frame with no row says so.
    measure(&state, "edge", 9_000);
    crate::api::signal_changed(&state);
    let changes = frame(&mut socket).await;
    assert_eq!((changes["id"].as_str(), changes["kind"].as_str()), (Some("a"), Some("changes")));
    assert_eq!(changes["upserts"], json!([]), "{changes}");
    assert_eq!(said(&changes), behind("catching-up", &["host/edge"]));
    assert_eq!(changes["sync"]["peers"][0]["peer_only_envelopes"], 9_000);

    // A second window subscribes while the host catches up: its snapshot says so, and it holds
    // its own last frame, so the first window's change is not sent to it again.
    subscribe(&mut socket, "b", "attention").await;
    let snapshot = frame(&mut socket).await;
    assert_eq!((snapshot["id"].as_str(), snapshot["kind"].as_str()), (Some("b"), Some("snapshot")));
    assert_eq!(said(&snapshot), behind("catching-up", &["host/edge"]));

    // A peer's counts moving is no change, even on a reread.
    measure(&state, "edge", 8_000);
    state.store.publish_collection_view("attention");
    quiet(&mut socket).await;

    // Another peer named is a change, sent on the windows' next read.
    measure(&state, "fir", 2_000);
    state.store.publish_collection_view("attention");
    for (_, changes) in frames(&mut socket, &["a", "b"]).await {
        assert_eq!(changes["kind"], "changes");
        assert_eq!(said(&changes), behind("catching-up", &["host/edge", "host/fir"]));
    }

    // Diverging moves the flags, so it is sent at once.
    diverge(&state, "edge");
    crate::api::signal_changed(&state);
    for (_, changes) in frames(&mut socket, &["a", "b"]).await {
        assert_eq!(said(&changes), behind("diverged", &["host/edge", "host/fir"]));
    }

    // A client that reconnects while the host is behind learns it from its first snapshot.
    let (mut again, again_server) = open(&state).await;
    subscribe(&mut again, "c", "attention").await;
    let snapshot = frame(&mut again).await;
    assert_eq!(snapshot["kind"], "snapshot");
    assert_eq!(said(&snapshot), behind("diverged", &["host/edge", "host/fir"]));

    // Back to level is a change too, on every window that said otherwise.
    state.store.replication_sync.lock().unwrap().clear();
    crate::api::signal_changed(&state);
    for (_, changes) in frames(&mut socket, &["a", "b"]).await {
        assert_eq!(changes["kind"], "changes");
        assert_eq!(said(&changes), level());
        assert_eq!(changes["sync"], json!({"state":"level"}));
    }
    assert_eq!(said(&frame(&mut again).await), level());
    quiet(&mut socket).await;
    quiet(&mut again).await;
    server.abort();
    again_server.abort();
}

#[test]
fn a_sync_is_compared_by_its_state_and_the_peers_it_names() {
    let peer = |host: &str, behind: u64| json!({"host_id": host, "peer_only_envelopes": behind});
    let catching = |peers: Vec<Value>| json!({"state": "catching-up", "peers": peers});
    assert_eq!(sync_said(&json!({"state":"level"})), level());
    // Counts moving and the order peers are named in change nothing.
    assert_eq!(
        sync_said(&catching(vec![peer("host/edge", 9_000), peer("host/fir", 1)])),
        sync_said(&catching(vec![peer("host/fir", 7), peer("host/edge", 8_000)])),
    );
    assert_ne!(
        sync_said(&catching(vec![peer("host/edge", 9_000)])),
        sync_said(&catching(vec![peer("host/edge", 9_000), peer("host/fir", 1)])),
    );
    assert_ne!(
        sync_said(&catching(vec![peer("host/edge", 9_000)])),
        sync_said(&json!({"state": "diverged", "peers": [peer("host/edge", 9_000)]})),
    );
}

#[test]
fn a_window_read_from_an_incremental_source_states_no_sync() {
    let subscription = |collection: &str, ivm: Option<Arc<collection_ivm::Adapter>>| {
        let request = serde_json::from_value::<CollectionSubscribe>(
            json!({"kind":"subscribe", "id":"a", "collection":collection, "limit":200}),
        )
        .unwrap();
        CollectionSubscription { generation: 0, reading: None, dirty: false, delivered: true, previous: Arc::new(BTreeMap::new()), ivm, cursor: None, order: Vec::new(), has_more: false, roster_revision: 0, view_revision: 0, opened: None, waiting_for_view: false, pinned: None, sync: None, sync_flags: (false, false), request }
    };
    assert!(window_states_sync(&subscription("attention", None)));
    assert!(!window_states_sync(&subscription("missions", None)));
    // Its reads say nothing when no row changed, so it could not say a changed sync alone: it
    // states none, which reads as unknown.
    let adapter = Arc::new(collection_ivm::Adapter {
        view: "fixture.attention",
        rows: Arc::new(|_, _, _, _, _, _, _| Ok((Vec::new(), false))),
    });
    assert!(!window_states_sync(&subscription("attention", Some(adapter))));
}
