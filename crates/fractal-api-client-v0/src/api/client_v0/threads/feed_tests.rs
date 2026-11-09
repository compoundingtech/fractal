use super::*;
use axum::body::{Body, to_bytes};
use fractal_client::{Client, Fence, Resource, ThreadReadParameters};
use futures_util::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::ServiceExt as _;

type Socket = WebSocketStream<UnixStream>;

async fn connect(path: &std::path::Path, person: &str) -> Socket {
    connect_with_header(path, LOCAL_PERSON_HEADER, person).await
}

async fn connect_with_header(path: &std::path::Path, header: &'static str, value: &str) -> Socket {
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
        .insert(header, value.parse().unwrap());
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
    thread_metadata::apply(
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
    let observed_revision = thread_metadata::summary(&state.store, person, thread).unwrap().revision;
    thread_metadata::apply(
        &state.store,
        person,
        thread,
        "close",
        thread_metadata::Edit::Close {
            revision: &observed_revision,
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
    subscribe(&mut outsider, "exact", "threads", ("subject", json!(thread))).await;
    let refusal = frame(&mut outsider).await;
    assert_eq!(refusal["kind"], "error", "{refusal}");
    assert_eq!(refusal["code"], "not-found");
    assert!(refusal.get("items").is_none());


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

    let pairing_http = reqwest::Client::builder().unix_socket(path.clone()).build().unwrap();
    let owner_post = |route: String, body: Value| {
        let request = pairing_http
            .post(format!("http://localhost/v1/client/{route}"))
            .header(LOCAL_PERSON_HEADER, person)
            .json(&body);
        async move {
            let response = request.send().await.unwrap();
            let status = response.status();
            let payload: Value = response.json().await.unwrap();
            assert_eq!(status, StatusCode::OK, "{payload}");
            payload["value"].clone()
        }
    };
    let challenge = owner_post("pairings".into(), json!({
        "api_version": CLIENT_API_VERSION, "person_id": person,
        "device_name": "Thread summary reader", "scopes": ["read.projections"],
    })).await;
    let pairing = challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap();
    let paired = owner_post(format!("pairings/{pairing}/complete"), json!({
        "api_version": CLIENT_API_VERSION, "code": challenge["code"],
        "device_public_key": "opaque-thread-summary-reader-without-signing-key",
    })).await;
    let credential = paired["credential"].as_str().unwrap();
    let mut exact = connect_with_header(&path, "authorization", &format!("Bearer {credential}")).await;
    subscribe(&mut exact, "summary", "threads", ("subject", json!(thread))).await;
    let initial = frame(&mut exact).await;
    assert_eq!(initial["items"][0]["unread"], true, "{initial}");
    assert_eq!(initial["has_more"], false);
    assert_eq!(initial["items"].as_array().unwrap().len(), 1);

    let client = Client::unix_as(&path, person);
    let observed = client.messages_get(reply).await.unwrap();
    assert!(matches!(observed.value, Resource::Message(_)));
    let receipt = client.thread_read(
        "action/thread-read",
        "thread-read-receipt",
        Fence { snapshot_id: observed.snapshot.id, ..Fence::default() },
        ThreadReadParameters {
            target_id: thread.into(),
            through_message_id: Some(reply.into()),
            seen_message_ids: vec![reply.into()],
        },
    ).await.unwrap();
    assert_eq!(receipt.value.affected_ids, [reply]);
    let cleared = frame(&mut owner).await;
    assert_eq!(cleared["id"], "list", "{cleared}");
    assert_eq!(cleared["upserts"][0]["unread"], false);
    assert_eq!(cleared["upserts"][0]["message_count"], 1);
    assert_eq!(cleared["upserts"][0]["state"], "closed");
    let exact_clear = frame(&mut exact).await;
    assert_eq!(exact_clear["id"], "summary", "{exact_clear}");
    assert_eq!(exact_clear["upserts"][0]["id"], thread);
    assert_eq!(exact_clear["upserts"][0]["unread"], false);
    assert_eq!(exact_clear["upserts"][0]["revision"], initial["items"][0]["revision"]);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(120), frame(&mut owner)).await.is_err(),
        "a read-only change must not produce a timeline patch");

    owner_post("actions".into(), json!({
        "api_version": CLIENT_API_VERSION, "id": "action/revoke-thread-summary-reader",
        "type": "pairing.revoke", "idempotency_key": "revoke-thread-summary-reader",
        "fence": {"snapshot_id": new_client_snapshot(&state).id},
        "parameters": {"target_id": paired["device_id"]},
    })).await;
    let refusal = frame(&mut exact).await;
    assert_eq!(refusal["kind"], "error", "{refusal}");
    assert_eq!(refusal["id"], "summary");
    assert_eq!(refusal["code"], "forbidden");
    assert!(refusal.get("upserts").is_none());
    let current = thread_metadata::summary(&state.store, person, thread).unwrap();
    thread_metadata::apply(&state.store, person, thread, "rename-after-revocation", thread_metadata::Edit::Rename {
        title: "Still visible locally", title_revision: &current.title_revision, revision: &current.revision,
    }).unwrap();
    crate::api::signal_visible_change(&state);
    let one = frame(&mut owner).await;
    let two = frame(&mut owner).await;
    let (list, history) = if one["id"] == "list" { (one, two) } else { (two, one) };
    assert_eq!(list["upserts"][0]["title"], "Still visible locally");
    assert_eq!(history["upserts"][0]["kind"], "thread.renamed");
    server.abort();
}
