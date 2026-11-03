use super::*;
use std::os::unix::fs::PermissionsExt as _;
use parking_lot::Mutex;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OldRequest {
    authority_actor: String,
    request: OldMessages,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
enum OldMessages {
    Messages { actor: String, limit: Option<usize>, cursor: Option<String> },
}

#[derive(Clone)]
struct Owner {
    auth: FleetAuth,
    received: Arc<Mutex<Vec<Value>>>,
    capabilities: Arc<Mutex<Value>>,
    capability_bytes: Arc<Mutex<Option<Vec<u8>>>>,
}

fn signed(owner: &Owner, body: &[u8], value: Value) -> Response {
    let bytes = serde_json::to_vec(&ApiResponse {
        api_version: "st3.v1".into(),
        request_id: "request/test".into(),
        snapshot_host: "owner-node".into(),
        store_index: 0,
        value,
    }).unwrap();
    signed_bytes(owner, body, bytes)
}

fn signed_bytes(owner: &Owner, body: &[u8], bytes: Vec<u8>) -> Response {
    let headers = owner.auth.response_headers_for(
        CLIENT_READ_PATH, "owner-node", &bytes, &FleetAuth::body_digest(body),
    ).unwrap();
    (headers, bytes).into_response()
}

async fn old_messages(State(owner): State<Owner>, headers: HeaderMap, body: Bytes) -> Response {
    owner.auth.verify(&headers, "POST", CLIENT_READ_PATH, &body, Some("gateway-node"), None).unwrap();
    assert!(!headers.contains_key("x-st3-features"));
    let old: OldRequest = serde_json::from_slice(&body).expect("old deny_unknown_fields owner accepts the exact legacy shape");
    assert_eq!(old.authority_actor, "person/alex");
    let OldMessages::Messages { actor, limit, cursor } = old.request;
    assert_eq!(actor, "agent/owner");
    assert_eq!(limit, Some(1));
    assert!(cursor.is_none());
    owner.received.lock().push(serde_json::from_slice(&body).unwrap());
    signed(&owner, &body, serde_json::json!({"kind":"messages","items":[]}))
}

async fn inspect(State(owner): State<Owner>, headers: HeaderMap, body: Bytes) -> Response {
    assert!(body.is_empty());
    assert!(!headers.contains_key("x-st3-features"));
    owner.auth.verify(&headers, "GET", CLIENT_READ_PATH, &body, Some("gateway-node"), None).unwrap();
    if let Some(bytes) = owner.capability_bytes.lock().clone() {
        return signed_bytes(&owner, &body, bytes);
    }
    let capabilities = owner.capabilities.lock().clone();
    signed(&owner, &body, capabilities)
}

async fn paged_messages(State(owner): State<Owner>, headers: HeaderMap, body: Bytes) -> Response {
    owner.auth.verify(&headers, "POST", CLIENT_READ_PATH, &body, Some("gateway-node"), None).unwrap();
    let request: Value = serde_json::from_slice(&body).unwrap();
    let session_detail = request["request"]["operation"] == "session-detail";
    if !session_detail {
        assert_eq!(request["request"]["messages_paging"], true);
    }
    owner.received.lock().push(request);
    signed(&owner, &body, if session_detail {
        serde_json::json!({"kind":"session","id":"session/external-child-test"})
    } else {
        serde_json::json!({"kind":"messages","items":[],"cut":{"kind":"head","state":"complete"}})
    })
}

async fn fixture(advertised: bool) -> (tempfile::TempDir, ClientRelay, Owner, tokio::task::JoinHandle<()>) {
    let root = tempfile::tempdir().unwrap();
    let secret = root.path().join("fleet-secret");
    fs::write(&secret, [7_u8; 32]).unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = ClientRelay::from_config(&Config {
        node: "gateway-node".into(),
        fleet_id: Some("fleet-test".into()),
        shared_secret_file: Some(secret),
        peers: vec![PeerConfig { name: "owner-node".into(), url: format!("http://{}", listener.local_addr().unwrap()) }],
        ..Default::default()
    }).unwrap().unwrap();
    let owner = Owner {
        auth: relay.auth.clone(), received: Arc::default(),
        capabilities: Arc::new(Mutex::new(serde_json::json!({"capabilities":[
            {"id":"messages-paging","version":2,"state":"granted"},
            {"id":"session-detail-child-id","version":1,"state":"granted"}
        ]}))),
        capability_bytes: Arc::default(),
    };
    let route = if advertised { post(paged_messages).get(inspect) } else { post(old_messages) };
    let router = Router::new().route(CLIENT_READ_PATH, route).with_state(owner.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    (root, relay, owner, server)
}

fn request(messages_paging: bool) -> ClientReadRequest {
    ClientReadRequest {
        authority_actor: "person/alex".into(),
        relay: None,
        request: ClientReadOperation::Messages {
            actor: "agent/owner".into(), history: false, limit: Some(1), cursor: None, messages_paging,
        },
    }
}

#[tokio::test]
async fn post_only_old_owner_never_receives_a_v2_field_and_legacy_still_works() {
    let (_root, relay, owner, server) = fixture(false).await;
    let error = relay.read("host/owner-node", &request(true)).await.unwrap_err();
    assert_eq!(error.downcast_ref::<ClientReadRejected>().unwrap().code, "unsupported-capability");
    assert!(owner.received.lock().is_empty(), "no v2 POST reaches the old decoder");
    let mut forwarded = request(true);
    forwarded.relay = Some(ClientReadRoute {
        target: "host/owner-node".into(), path: vec!["first-gateway".into()], hops_left: 3,
    });
    let error = relay.forward(&forwarded).await.unwrap_err();
    assert_eq!(error.downcast_ref::<ClientReadRejected>().unwrap().code, "unsupported-capability");
    assert!(owner.received.lock().is_empty(), "intermediate forwarding also gates the owner");
    relay.read("host/owner-node", &request(false)).await.unwrap();
    assert_eq!(*owner.received.lock(), vec![serde_json::json!({
        "authority_actor":"person/alex",
        "request":{"operation":"messages","actor":"agent/owner","limit":1,"cursor":null}
    })]);
    server.abort();
}

#[tokio::test]
async fn advertised_owner_receives_v2_only_after_authenticated_inspection() {
    let (_root, relay, owner, server) = fixture(true).await;
    let page = relay.read("host/owner-node", &request(true)).await.unwrap();
    assert_eq!(page["cut"]["state"], "complete");
    assert_eq!(owner.received.lock().len(), 1);
    server.abort();
}

#[tokio::test]
async fn only_exact_granted_version_two_allows_a_v2_post() {
    let (_root, relay, owner, server) = fixture(true).await;
    for capabilities in [
        serde_json::json!({"capabilities":[]}),
        serde_json::json!({"capabilities":[{"id":"messages-paging","version":1,"state":"granted"}]}),
        serde_json::json!({"capabilities":[{"id":"messages-paging","version":2,"state":"ungranted"}]}),
        serde_json::json!({"capabilities":[{"id":"messages-paging","version":2,"state":"unavailable"}]}),
    ] {
        *owner.capabilities.lock() = capabilities;
        let error = relay.read("host/owner-node", &request(true)).await.unwrap_err();
        assert_eq!(error.downcast_ref::<ClientReadRejected>().unwrap().code, "unsupported-capability");
        assert!(owner.received.lock().is_empty());
    }
    server.abort();
}

fn session_detail_request() -> ClientReadRequest {
    ClientReadRequest {
        authority_actor: "person/alex".into(),
        relay: None,
        request: ClientReadOperation::SessionDetail {
            session_id: "session/external-child-test".into(),
        },
    }
}

#[tokio::test]
async fn session_detail_never_reaches_a_post_only_old_peer() {
    let (_root, relay, owner, server) = fixture(false).await;
    let error = relay.read("host/owner-node", &session_detail_request()).await.unwrap_err();
    let rejected = error.downcast_ref::<ClientReadRejected>().unwrap();
    assert_eq!(rejected.code, "peer-too-old");
    assert!(rejected.message.contains("update it to open subagent conversations"));
    assert!(owner.received.lock().is_empty(), "the old closed-enum decoder is never probed");
    server.abort();
}

#[tokio::test]
async fn session_detail_requires_the_exact_granted_child_id_capability_at_every_hop() {
    let (_root, relay, owner, server) = fixture(true).await;
    for capabilities in [
        serde_json::json!({}),
        serde_json::json!({"capabilities":[]}),
        serde_json::json!({"capabilities":[{"id":"messages-paging","version":2,"state":"granted"}]}),
        serde_json::json!({"capabilities":[{"id":"session-detail-child-id","version":2,"state":"granted"}]}),
        serde_json::json!({"capabilities":[{"id":"session-detail-child-id","version":1,"state":"ungranted"}]}),
    ] {
        *owner.capabilities.lock() = capabilities;
        let mut request = session_detail_request();
        let error = relay.read("host/owner-node", &request).await.unwrap_err();
        assert_eq!(error.downcast_ref::<ClientReadRejected>().unwrap().code, "peer-too-old");
        request.relay = Some(ClientReadRoute {
            target: "host/owner-node".into(), path: vec!["first-gateway".into()], hops_left: 3,
        });
        let error = relay.forward(&request).await.unwrap_err();
        assert_eq!(error.downcast_ref::<ClientReadRejected>().unwrap().code, "peer-too-old");
        assert!(owner.received.lock().is_empty());
    }
    server.abort();
}

#[tokio::test]
async fn session_detail_succeeds_with_the_advertised_three_field_child_id_protocol() {
    let (_root, relay, owner, server) = fixture(true).await;
    let detail = relay.read("host/owner-node", &session_detail_request()).await.unwrap();
    assert_eq!(detail["kind"], "session");
    assert_eq!(owner.received.lock().len(), 1);
    server.abort();
}

#[tokio::test]
async fn authenticated_unreadable_capabilities_refuse_without_a_session_detail_post() {
    let (_root, relay, owner, server) = fixture(true).await;
    for bytes in [
        b"not JSON".to_vec(),
        serde_json::to_vec(&serde_json::json!({"value":{"capabilities":[]}})).unwrap(),
        serde_json::to_vec(&serde_json::json!({
            "api_version":"future.v9","request_id":"request/test","snapshot_host":"owner-node",
            "store_index":0,"value":{"capabilities":[
                {"id":"session-detail-child-id","version":1,"state":"granted"}
            ]}
        })).unwrap(),
    ] {
        *owner.capability_bytes.lock() = Some(bytes);
        let error = relay.read("host/owner-node", &session_detail_request()).await.unwrap_err();
        assert_eq!(error.downcast_ref::<ClientReadRejected>().unwrap().code, "peer-too-old");
        assert!(owner.received.lock().is_empty());
    }
    server.abort();
}
