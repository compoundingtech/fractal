use super::*;
use tracing::instrument::WithSubscriber as _;

const CREDENTIAL: &str = "push-test-credential";
const GRANT: &str = "custom/client/push-test";
const PERSON: &str = "person/example";
const ACTOR: &str = "person/example/session/phone";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn state(root: &std::path::Path) -> AppState {
    AppState {
        store: Arc::new(Store::open(&root.join("graph.db"), "push-test").unwrap()),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0u64).0,
        node: "push-test".into(),
        state_dir: root.into(),
        pty_root: root.join("pty"),
        pty_binary: root.join("unused"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
    }
}

fn grant(state: &AppState, expiry: u64) {
    state
        .store
        .append_claim(&ClaimInput {
            subject: GRANT.into(),
            kind: "custom.client.pairing-completed".into(),
            actor: Some(PERSON.into()),
            fields: serde_json::from_value(json!({
                "credential_hash":hex::encode(Sha256::digest(CREDENTIAL.as_bytes())),
                "session_actor":ACTOR,"person_id":PERSON,"device_id":"device/phone",
                "scopes":["read.projections"],"expires_at_unix_ms":expiry
            }))
            .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn body() -> Value {
    json!({"token":TOKEN,"environment":"development","bundle_id":"org.example.push","app_version":"1.0"})
}

async fn send(
    app: Router,
    method: &str,
    credential: Option<&str>,
    body: Vec<u8>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri("/v1/client/push-registration")
        .header("content-type", "application/json");
    if let Some(credential) = credential {
        request = request.header("authorization", format!("Bearer {credential}"));
    }
    let response = app
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains(TOKEN),
        "HTTP responses must never echo the secret"
    );
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn assert_local_envelope(response: &Value, status: Value) {
    assert_eq!(response["api_version"], "st3.client.v0");
    assert!(response["request_id"].is_string());
    assert_eq!(response["value"], status);
    for field in ["snapshot", "claims", "receipt", "token"] {
        assert!(response.get(field).is_none());
    }
    assert_eq!(response.as_object().unwrap().len(), 3);
}

#[tokio::test]
async fn push_http_register_replace_unregister_and_capability_are_local_and_safe() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    grant(&state, client_now_ms() as u64 + 600_000);
    let app = fabric_router(state.clone());
    let before = state.store.index().unwrap();
    let (status, response) = send(
        app.clone(),
        "POST",
        Some(CREDENTIAL),
        serde_json::to_vec(&body()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_local_envelope(
        &response,
        json!({"registered":true,"environment":"development"}),
    );
    let mut replacement = body();
    replacement["token"] = json!("AABBCCDD");
    replacement["environment"] = json!("production");
    let (status, response) = send(
        app.clone(),
        "POST",
        Some(CREDENTIAL),
        serde_json::to_vec(&replacement).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_local_envelope(
        &response,
        json!({"registered":true,"environment":"production"}),
    );
    let count: u64 = state
        .store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM local_push_registrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    for _ in 0..2 {
        let (status, response) = send(app.clone(), "DELETE", Some(CREDENTIAL), vec![]).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_local_envelope(&response, json!({"registered":false}));
    }
    assert_eq!(state.store.index().unwrap(), before);
    let request = Request::builder()
        .uri("/v1/client/capabilities")
        .header("authorization", format!("Bearer {CREDENTIAL}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        response["value"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability
                == &json!({"id":"push-registration","version":1,"state":"granted"}))
    );
}

#[tokio::test]
async fn push_local_unpaired_unknown_revoked_and_expired_sessions_are_forbidden() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    grant(&state, client_now_ms() as u64 + 600_000);
    let wire = serde_json::to_vec(&body()).unwrap();
    for method in ["POST", "DELETE"] {
        let bytes = if method == "POST" {
            wire.clone()
        } else {
            vec![]
        };
        for (app, credential) in [
            (router(state.clone()), None),
            (fabric_router(state.clone()), None),
            (fabric_router(state.clone()), Some("unknown-credential")),
        ] {
            assert_eq!(
                send(app, method, credential, bytes.clone()).await.0,
                StatusCode::FORBIDDEN
            );
        }
    }
    let app = fabric_router(state.clone());
    let (status, response) = send(app.clone(), "POST", Some(CREDENTIAL), wire.clone()).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    state
        .store
        .append_claim(&ClaimInput {
            subject: GRANT.into(),
            kind: "custom.client.pairing-revoked".into(),
            actor: Some(PERSON.into()),
            fields: BTreeMap::new(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    for (method, bytes) in [("POST", wire.clone()), ("DELETE", vec![])] {
        assert_eq!(
            send(app.clone(), method, Some(CREDENTIAL), bytes).await.0,
            StatusCode::FORBIDDEN
        );
    }
    grant(&state, client_now_ms() as u64 - 1);
    for (method, bytes) in [("POST", wire), ("DELETE", vec![])] {
        assert_eq!(
            send(app.clone(), method, Some(CREDENTIAL), bytes).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let count: u64 = state
        .store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM local_push_registrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn push_invalid_and_identity_body_extras_never_mutate_or_echo_inputs() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    grant(&state, client_now_ms() as u64 + 600_000);
    let app = fabric_router(state.clone());
    let (status, response) = send(
        app.clone(),
        "POST",
        Some(CREDENTIAL),
        serde_json::to_vec(&body()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let before = state.store.index().unwrap();
    let mut invalid = vec![];
    for (field, value) in [
        ("token", json!("")),
        ("token", json!("f")),
        ("token", json!("abc")),
        ("token", json!("ff".repeat(257))),
        ("token", json!("gg")),
        ("token", json!(format!("{TOKEN}\n"))),
        ("token", json!(7)),
        ("token", json!("éé")),
        ("environment", json!(TOKEN)),
        ("environment", json!(null)),
        ("bundle_id", json!("")),
        ("bundle_id", json!("x".repeat(256))),
        ("bundle_id", json!("org.example/app")),
        ("app_version", json!("")),
        ("app_version", json!("x".repeat(65))),
        ("app_version", json!("1\n0")),
        ("app_version", json!("1é")),
        ("person", json!("person/other")),
        ("person_id", json!("person/other")),
        ("device", json!("device/other")),
        ("device_id", json!("device/other")),
        ("pairing_grant", json!("custom/client/other")),
        (TOKEN, json!(TOKEN)),
    ] {
        let mut input = body();
        input[field] = value;
        invalid.push(serde_json::to_vec(&input).unwrap());
    }
    for field in ["token", "environment", "bundle_id", "app_version"] {
        let mut input = body();
        input.as_object_mut().unwrap().remove(field);
        invalid.push(serde_json::to_vec(&input).unwrap());
    }
    invalid.extend([
        format!("{{\"token\":\"{TOKEN}\",broken").into_bytes(),
        format!("{{\"token\":\"{TOKEN}\",\"token\":\"ff\",\"environment\":\"development\",\"bundle_id\":\"org.example.push\",\"app_version\":\"1\"}}").into_bytes(),
        format!("[\"{TOKEN}\"]").into_bytes(),
    ]);
    for bytes in invalid {
        let (status, response) = send(app.clone(), "POST", Some(CREDENTIAL), bytes).await;
        assert!(status.is_client_error());
        assert_eq!(response["code"], "validation-failed", "{response}");
    }
    let mut oversized = serde_json::to_vec(&body()).unwrap();
    oversized.resize(MAX_BODY_BYTES + 1, b' ');
    assert_eq!(
        send(app.clone(), "POST", Some(CREDENTIAL), oversized)
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for bytes in [
        serde_json::to_vec(&body()).unwrap(),
        b"{}".to_vec(),
        b" ".to_vec(),
    ] {
        let (status, response) = send(app.clone(), "DELETE", Some(CREDENTIAL), bytes).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
        assert_eq!(response["code"], "validation-failed", "{response}");
    }
    assert_eq!(state.store.index().unwrap(), before);
    assert_eq!(
        state
            .store
            .push_registration_statuses(PERSON, client_now_ms() as u64)
            .unwrap()["device/phone"]
            .environment,
        Some(crate::store::push_registrations::PushEnvironment::Development)
    );
}

#[test]
fn push_validation_accepts_both_exact_bounds_and_rejects_unbounded_metadata() {
    for size in [2, 512] {
        for environment in ["development", "production"] {
            let input: PushRegistration = serde_json::from_value(json!({
                "token":"F".repeat(size),"environment":environment,"bundle_id":"x".repeat(255),"app_version":"~".repeat(64)
            })).unwrap();
            assert!(validate(&input).is_ok());
        }
    }
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn push_http_logs_and_storage_failures_are_redacted() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    grant(&state, client_now_ms() as u64 + 600_000);
    let app = fabric_router(state.clone());
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    async {
        let (status, response) = send(app.clone(), "POST", Some(CREDENTIAL), serde_json::to_vec(&body()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        let mut input = body(); input[TOKEN] = json!(TOKEN);
        assert!(send(app.clone(), "POST", Some(CREDENTIAL), serde_json::to_vec(&input).unwrap()).await.0.is_client_error());
        // Make SQLite's error text contain the secret; the HTTP boundary must discard it.
        let connection = state.store.connection.lock().expect("fixture writer");
        connection.execute_batch(&format!("CREATE TRIGGER push_failure BEFORE INSERT ON local_push_registrations BEGIN SELECT RAISE(ABORT,'{TOKEN}'); END;")).unwrap();
        drop(connection);
        let (status, error) = send(app, "POST", Some(CREDENTIAL), serde_json::to_vec(&body()).unwrap()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error["message"], "local push registration storage is unavailable", "{error}");
    }.with_subscriber(subscriber).await;
    let captured = logs
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(!String::from_utf8_lossy(&captured).contains(TOKEN));
}
