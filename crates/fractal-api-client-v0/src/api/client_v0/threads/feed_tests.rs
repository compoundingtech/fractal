use super::*;
use axum::body::{Body, to_bytes};
use fractal_client::{Client, Fence, Resource, TargetParameters};
use futures_util::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::ServiceExt as _;

type Socket = WebSocketStream<UnixStream>;

async fn connect(path: &std::path::Path, person: &str) -> Socket {
    let stream = UnixStream::connect(path).await.unwrap();
    let mut request = "ws://localhost/v1/client/collections/stream"
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "fractal.client.collections.v0".parse().unwrap(),
    );
    request
        .headers_mut()
        .insert(LOCAL_PERSON_HEADER, person.parse().unwrap());
    tokio_tungstenite::client_async(request, stream)
        .await
        .unwrap()
        .0
}

async fn subscribe(socket: &mut Socket, id: &str, collection: &str, selector: (&str, Value)) {
    socket
        .send(Message::Text(
            json!({
                "kind":"subscribe", "id":id, "collection":collection, (selector.0):selector.1,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
}

async fn frame(socket: &mut Socket) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                return serde_json::from_str(&text).unwrap();
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn thread_collections_follow_linked_replies_and_shared_read_receipts() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::test_state_named(root.path(), "thread-feed");
    state.store.configure_thread_write_rollout([]);
    let person = "person/operator";
    let agent = "agent/garden/reviewer";
    let thread = "thread/fixture";
    let intent = crate::graph::parse_intent(
        r#"version 2
agent "garden/reviewer" { workspace "/work/garden"; harness "omp" {} }
"#,
        state.store.origin(),
    )
    .unwrap();
    state
        .store
        .apply_internal(&intent, "declare-agent")
        .unwrap();
    let (created, _) = thread_metadata::apply(
        &state.store,
        person,
        thread,
        "create",
        thread_metadata::Edit::Create {
            agent,
            title: "Review",
        },
    )
    .unwrap();
    thread_metadata::apply(
        &state.store,
        person,
        thread,
        "close",
        thread_metadata::Edit::Close {
            revision: &created.id,
        },
    )
    .unwrap();

    let path = root.path().join("thread.sock");
    let listener = path.clone();
    let app = crate::api::router(state.clone());
    let writer = app.clone();
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        crate::api::serve_unix_with_ready(&listener, app, || {
            let _ = ready.send(());
        })
        .await
        .unwrap();
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), waiting)
        .await
        .unwrap()
        .unwrap();

    let mut owner = connect(&path, person).await;
    subscribe(
        &mut owner,
        "list",
        "threads",
        ("thread_context", json!({"agent_id":agent})),
    )
    .await;
    let summary = frame(&mut owner).await;
    assert_eq!(summary["items"][0]["id"], thread, "{summary}");
    assert_eq!(summary["items"][0]["state"], "closed");
    assert_eq!(summary["items"][0]["message_count"], 0);
    assert_eq!(summary["items"][0]["unread"], false);
    subscribe(
        &mut owner,
        "history",
        "thread-timeline",
        ("subject", json!(thread)),
    )
    .await;
    let timeline = frame(&mut owner).await;
    assert_eq!(timeline["items"][0]["kind"], "thread.closed", "{timeline}");

    let mut outsider = connect(&path, "person/other").await;
    subscribe(
        &mut outsider,
        "history",
        "thread-timeline",
        ("subject", json!(thread)),
    )
    .await;
    let refusal = frame(&mut outsider).await;
    assert_eq!(refusal["kind"], "error", "{refusal}");
    assert_eq!(refusal["code"], "not-found");
    subscribe(
        &mut outsider,
        "list",
        "threads",
        ("thread_context", json!({"agent_id":agent})),
    )
    .await;
    assert_eq!(frame(&mut outsider).await["items"], json!([]));

    let action = json!({
        "api_version": CLIENT_API_VERSION, "id": "action/feed-agent-reply",
        "type": "thread.reply", "idempotency_key": "feed-agent-reply",
        "fence": {"snapshot_id": new_client_snapshot(&state).id},
        "parameters": {"target_id": thread, "content":"Worker tests pass."},
    });
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/client/actions")
        .header(LOCAL_PERSON_HEADER, agent)
        .header("Content-Type", "application/json")
        .body(Body::from(action.to_string()))
        .unwrap();
    request
        .extensions_mut()
        .insert(crate::api::BoundAgent(agent.into()));
    let response = writer.oneshot(request).await.unwrap();
    let status = response.status();
    let reply: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(status, StatusCode::OK, "{reply}");
    let reply = reply["value"]["affected_ids"][1].as_str().unwrap();
    let one = frame(&mut owner).await;
    let two = frame(&mut owner).await;
    let (list, history) = if one["id"] == "list" {
        (one, two)
    } else {
        (two, one)
    };
    assert_eq!(list["upserts"][0]["unread"], true, "{list}");
    assert_eq!(list["upserts"][0]["state"], "closed");
    assert_eq!(list["upserts"][0]["message_count"], 1);
    assert_eq!(history["upserts"][0]["id"], reply, "{history}");

    let client = Client::unix_as(&path, person);
    let observed = client.messages_get(reply).await.unwrap();
    let Resource::Message(message) = observed.value else {
        panic!("expected the ordinary message")
    };
    client
        .message_read(
            "action/thread-read",
            "thread-read-receipt",
            Fence {
                snapshot_id: observed.snapshot.id,
                subject_revisions: [(reply.into(), message.header.revision)].into(),
                ..Fence::default()
            },
            TargetParameters {
                target_id: reply.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let cleared = frame(&mut owner).await;
    assert_eq!(cleared["id"], "list", "{cleared}");
    assert_eq!(cleared["upserts"][0]["unread"], false);
    assert_eq!(cleared["upserts"][0]["message_count"], 1);
    assert_eq!(cleared["upserts"][0]["state"], "closed");
    server.abort();
}
