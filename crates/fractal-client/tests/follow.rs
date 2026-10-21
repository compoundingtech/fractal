use fractal_client::{
    Client, ClientError,
    follow::{
        AttentionDescriptor, Descriptor, FollowClient, FollowError, FollowReceiver, FollowState,
        Freshness, decode_row,
    },
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

type Socket = WebSocketStream<tokio::net::UnixStream>;
#[derive(Debug, Deserialize)]
struct Row {
    id: String,
    title: String,
}
struct Agents;
impl Descriptor for Agents {
    type Row = Row;
    fn selector(&self) -> Value {
        json!({"collection":"agents"})
    }
    fn decode(&self, value: Value) -> Result<Row, ClientError> {
        decode_row(value)
    }
}

// Tungstenite requires an HTTP response as this callback's error type.
#[allow(clippy::result_large_err)]
async fn accept(stream: tokio::net::UnixStream) -> Socket {
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    tokio_tungstenite::accept_hdr_async(stream, |request: &Request, mut response: Response| {
        assert_eq!(request.uri().path(), "/v1/client/collections/stream");
        response.headers_mut().insert(
            "sec-websocket-protocol",
            "st3.client.collections.v0".parse().unwrap(),
        );
        Ok(response)
    })
    .await
    .unwrap()
}

// Same scripted Unix WebSocket transport as the crate's raw-terminal tests.
async fn connection() -> (tempfile::TempDir, FollowClient, Socket) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("follow.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        (listener, accept(stream).await)
    });
    let client = FollowClient::connect(Client::unix(&path)).await.unwrap();
    let (_, socket) = server.await.unwrap();
    (root, client, socket)
}
async fn command(socket: &mut Socket) -> Value {
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_slice(&message.into_data()).unwrap()
}
async fn send(socket: &mut Socket, frame: Value) {
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
}
fn snapshot(id: &str, rows: Value, order: Value, index: u64) -> Value {
    json!({"kind":"snapshot", "id":id, "items":rows, "order":order, "has_more":false,
        "snapshot":{"id":format!("s{index}"), "host_id":"host/test", "store_index":index,"projection_version":"1","created_at":"2026-10-21T00:00:00Z"}})
}
async fn state<T: Send + Sync + 'static>(
    receiver: &mut FollowReceiver<T>,
    predicate: impl Fn(&FollowState<T>) -> bool,
) -> Arc<FollowState<T>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = receiver.next().await.unwrap();
            if predicate(&value) {
                return value;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn ordered_snapshot_changes_removes_and_missing_row() {
    let (_root, client, mut socket) = connection().await;
    let mut rows = client.watch(Agents, 10).await.unwrap();
    let id = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    send(
        &mut socket,
        snapshot(
            &id,
            json!([{"id":"a","title":"A"},{"id":"b","title":"B"}]),
            json!(["b", "a"]),
            1,
        ),
    )
    .await;
    let initial = state(&mut rows, |s| s.freshness == Freshness::Fresh).await;
    assert_eq!(
        initial
            .value
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
    let mut changes = snapshot(&id, json!([]), json!(["c", "b"]), 2);
    changes["kind"] = json!("changes");
    changes["upserts"] = json!([{"id":"c","title":"C"},{"id":"b","title":"updated"}]);
    changes["removes"] = json!(["a"]);
    send(&mut socket, changes.clone()).await;
    let updated = state(&mut rows, |s| {
        s.value
            .as_ref()
            .is_some_and(|v| v.snapshot.store_index == 2)
    })
    .await;
    assert_eq!(updated.value.as_ref().unwrap().items[1].title, "updated");
    assert_eq!(initial.value.as_ref().unwrap().items[0].title, "B");
    changes["order"] = json!(["missing"]);
    send(&mut socket, changes).await;
    let failed = state(&mut rows, |s| s.error.is_some()).await;
    assert!(
        matches!(&failed.error, Some(FollowError::Protocol(message)) if message.contains("missing row"))
    );
    assert_eq!(failed.freshness, Freshness::Stale);
    assert_eq!(failed.value.as_ref().unwrap().snapshot.store_index, 2);
}

#[tokio::test]
async fn two_models_share_socket_and_last_clone_unsubscribes() {
    let (_root, client, mut socket) = connection().await;
    let mut agents = client.watch(Agents, 10).await.unwrap();
    let agents_id = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut attention = client
        .clone()
        .watch(AttentionDescriptor::default(), 10)
        .await
        .unwrap();
    let attention_command = command(&mut socket).await;
    assert_eq!(attention_command["collection"], "attention");
    let attention_id = attention_command["id"].as_str().unwrap();
    send(
        &mut socket,
        snapshot(
            &agents_id,
            json!([{"id":"a","title":"agent"}]),
            json!(["a"]),
            1,
        ),
    )
    .await;
    send(&mut socket, snapshot(attention_id, json!([]), json!([]), 1)).await;
    state(&mut agents, |s| s.freshness == Freshness::Fresh).await;
    state(&mut attention, |s| s.freshness == Freshness::Fresh).await;
    let clone = agents.clone();
    drop(agents);
    // A second subscription still works after dropping a non-last receiver.
    send(&mut socket, snapshot(attention_id, json!([]), json!([]), 2)).await;
    state(&mut attention, |s| {
        s.value
            .as_ref()
            .is_some_and(|v| v.snapshot.store_index == 2)
    })
    .await;
    drop(clone);
    let unsubscribe = command(&mut socket).await;
    assert_eq!(unsubscribe, json!({"kind":"unsubscribe", "id":agents_id}));
    drop(attention);
    assert_eq!(
        command(&mut socket).await,
        json!({"kind":"unsubscribe", "id":attention_id})
    );
}

#[tokio::test]
async fn resync_fences_old_generation_and_errors_differ_from_stale() {
    let (_root, client, mut socket) = connection().await;
    let mut rows = client.watch(Agents, 10).await.unwrap();
    let old = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    send(&mut socket, snapshot(&old, json!([]), json!([]), 1)).await;
    state(&mut rows, |s| s.freshness == Freshness::Fresh).await;
    send(&mut socket, json!({"kind":"resync","id":old})).await;
    let stale = state(&mut rows, |s| s.freshness == Freshness::Stale).await;
    assert!(stale.error.is_none());
    assert_eq!(
        command(&mut socket).await,
        json!({"kind":"unsubscribe", "id":old})
    );
    let new = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(old, new);
    send(
        &mut socket,
        snapshot(
            &old,
            json!([{"id":"bad","title":"old generation"}]),
            json!(["bad"]),
            99,
        ),
    )
    .await;
    send(
        &mut socket,
        snapshot("wrong-subscription", json!([]), json!([]), 100),
    )
    .await;
    send(&mut socket, snapshot(&new, json!([]), json!([]), 2)).await;
    state(&mut rows, |s| s.freshness == Freshness::Fresh).await;
    assert_eq!(
        rows.current().value.as_ref().unwrap().snapshot.store_index,
        2
    );
    send(
        &mut socket,
        json!({"kind":"error","id":new,"code":"forbidden","message":"authority revoked"}),
    )
    .await;
    let failed = state(&mut rows, |s| s.error.is_some()).await;
    assert!(
        matches!(&failed.error, Some(FollowError::Subscription {message,..}) if message == "authority revoked")
    );
}

#[tokio::test]
async fn reconnect_retains_value_until_authoritative_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("reconnect.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        (listener, accept(stream).await)
    });
    let client = FollowClient::connect(Client::unix(&path)).await.unwrap();
    let (listener, mut socket) = server.await.unwrap();
    let mut rows = client.watch(Agents, 10).await.unwrap();
    let old = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    send(&mut socket, snapshot(&old, json!([]), json!([]), 1)).await;
    state(&mut rows, |s| s.freshness == Freshness::Fresh).await;
    socket.close(None).await.unwrap();
    let stale = state(&mut rows, |s| s.freshness == Freshness::Stale).await;
    assert!(stale.error.is_none());
    assert_eq!(stale.value.as_ref().unwrap().snapshot.store_index, 1);
    let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut socket = accept(stream).await;
    let new = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(old, new);
    send(&mut socket, snapshot(&new, json!([]), json!([]), 2)).await;
    state(&mut rows, |s| s.freshness == Freshness::Fresh).await;
    assert_eq!(
        rows.current().value.as_ref().unwrap().snapshot.store_index,
        2
    );
}

#[tokio::test]
async fn retained_stream_coalesces_burst_and_admission_is_bounded() {
    let (_root, client, mut socket) = connection().await;
    let mut rows = client.watch(Agents, 1).await.unwrap();
    let id = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    rows.next().await.unwrap(); // Consume Pending before deliberately pausing this consumer.
    let mut marker = client.watch(Agents, 1).await.unwrap();
    let marker_id = command(&mut socket).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for index in 1..=1000 {
        send(
            &mut socket,
            snapshot(
                &id,
                json!([{"id":"a", "title":index.to_string()}]),
                json!(["a"]),
                index,
            ),
        )
        .await;
    }
    send(
        &mut socket,
        snapshot(&marker_id, json!([]), json!([]), 1001),
    )
    .await;
    state(&mut marker, |s| s.freshness == Freshness::Fresh).await;
    assert_eq!(
        rows.current().value.as_ref().unwrap().items[0].title,
        "1000"
    );
    let latest = rows.next().await.unwrap();
    assert_eq!(latest.value.as_ref().unwrap().snapshot.store_index, 1000);
    assert!(futures_util::poll!(rows.next()).is_pending());
    assert!(client.watch(Agents, 0).await.is_err());
    assert!(client.watch(Agents, 201).await.is_err());
    let mut held = Vec::new();
    for _ in 2..16 {
        held.push(client.watch(Agents, 1).await.unwrap());
        command(&mut socket).await;
    }
    assert!(client.watch(Agents, 1).await.is_err());
}
