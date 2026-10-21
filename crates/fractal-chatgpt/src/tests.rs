use super::*;
use axum::{body::{Body, to_bytes}, http::Request};
use std::os::unix::fs::{PermissionsExt, symlink};
use tower::ServiceExt;

fn disconnected_adapter() -> Adapter {
    Adapter {
        client: reqwest::Client::builder().unix_socket("/nonexistent/st-chatgpt-tests/st3-client.sock").build().unwrap(),
        bearer: "test-only-not-a-daemon-credential".into(),
        profile: Profile::ChatGpt,
    }
}

#[test]
fn discovery_has_exactly_eleven_closed_tools_and_correct_hints() {
    let expected = [
        ("st_alerts_list", true, false, false),
        ("st_alert_get", true, false, false),
        ("st_alert_answer", false, true, false),
        ("st_missions_list", true, false, false),
        ("st_agents_list", true, false, false),
        ("st_conversations_list", true, false, false),
        ("st_conversation_detail", true, false, false),
        ("st_conversation_timeline", true, false, false),
        ("st_conversation_search", true, false, false),
        ("st_conversation_read", true, false, false),
        ("st_message_send", false, false, true),
    ];
    let tools = tool_definitions(Profile::ChatGpt);
    assert_eq!(tools.len(), expected.len());
    for (tool, (name, read, destructive, open_world)) in tools.iter().zip(expected) {
        let published = serde_json::to_value(tool).unwrap();
        assert_eq!(published["name"], name);
        assert_eq!(published["annotations"], json!({
            "readOnlyHint": read, "destructiveHint": destructive,
            "idempotentHint": true, "openWorldHint": open_world,
        }));
        assert_eq!(published["inputSchema"]["type"], "object");
        assert_eq!(published["inputSchema"]["additionalProperties"], false);
    }
}

#[test]
fn writes_publish_typed_closed_fences() {
    for name in ["st_alert_answer", "st_message_send"] {
        let contract = schema(name);
        let fence = &contract["properties"]["fence"];
        assert_eq!(fence["type"], "object");
        assert_eq!(fence["required"], json!(["snapshot_id"]));
        assert_eq!(fence["additionalProperties"], false);
        assert_eq!(fence["properties"]["snapshot_id"]["type"], "string");
        assert_eq!(fence["properties"]["subject_revisions"]["type"], "object");
        assert_eq!(fence["properties"]["subject_revisions"]["additionalProperties"], json!({"type":"string"}));
        assert!(fence["description"].as_str().unwrap().contains("snapshot"));
    }
}

#[test]
fn writes_publish_action_identity_constraints() {
    for tool in tool_definitions(Profile::ChatGpt).iter().filter(|tool| matches!(tool.name.as_ref(), "st_alert_answer" | "st_message_send")) {
        let published = serde_json::to_value(tool).unwrap();
        let properties = &published["inputSchema"]["properties"];
        assert_eq!(properties["action_id"]["pattern"], "^action/");
        assert_eq!(properties["action_id"]["description"], "Unique per intended action, e.g. action/<short-slug>; reuse the same value on a retry");
        assert_eq!(properties["idempotency_key"]["pattern"], "^[ -~]{16,256}$");
        assert!(properties["idempotency_key"].get("minLength").is_none());
        assert!(properties["idempotency_key"].get("maxLength").is_none());
        assert_eq!(properties["idempotency_key"]["description"], "16-256 printable ASCII chars only (stricter than daemon byte-length bounds); reuse the exact value on retries of the same action");
        assert!(properties["fence"]["properties"]["snapshot_id"]["description"].as_str().unwrap().contains("immediately preceding read result"));
    }
}

fn send_arguments() -> Value {
    json!({"action_id":"action/test", "idempotency_key":"test-key-12345678",
        "fence":{"snapshot_id":"snapshot/test", "subject_revisions":{"agent/example-recipient":"claim/test"}},
        "to":"agent/example-recipient", "title":"Hello", "content":"Exact content"})
}

#[test]
fn validation_accepts_every_published_tool_contract() {
    for name in ["st_alerts_list", "st_missions_list", "st_agents_list", "st_conversations_list"] {
        assert!(validate_arguments(name, &json!({})).is_ok());
        assert!(validate_arguments(name, &json!({"limit":1,"cursor":"opaque","max_response_bytes":16384})).is_ok());
    }
    assert!(validate_arguments("st_alert_get", &json!({"alert_id":"attention/test"})).is_ok());
    assert!(validate_arguments("st_conversation_read", &json!({"agent_id":"agent/example-reader","session_id":"session/a",
        "reference":"opaque","revision":"claim/a","start":0,"end":1,"max_response_bytes":1})).is_ok());
    assert!(validate_arguments("st_message_send", &send_arguments()).is_ok());
    assert!(validate_arguments("st_alert_answer", &json!({"action_id":"action/test","idempotency_key":"test-key-12345678",
        "fence":{"snapshot_id":"snapshot/test"},"alert_id":"attention/test","episode":"episode/test",
        "option":"done","summary":"Done"})).is_ok());
}

#[test]
fn attention_selector_is_additive_discoverable_and_exact() {
    let contract = schema("st_alerts_list");
    assert_eq!(contract["required"], json!([]));
    assert_eq!(contract["properties"]["agent_id"]["maxLength"], 256);
    assert!(validate_arguments("st_alerts_list", &json!({"agent_id":"agent/chief-of-staff"})).is_ok());
    for invalid in ["chief-of-staff", "agent/", "person/operator", "agent/private prose"] {
        assert_eq!(validate_arguments("st_alerts_list", &json!({"agent_id":invalid}))
            .unwrap_err()["code"], "invalid-arguments");
    }
    assert!(validate_arguments("st_alerts_list", &json!({"agent_id":false})).is_err());
    assert!(validate_arguments("st_missions_list", &json!({"agent_id":"agent/chief-of-staff"})).is_err());
    // The two selectors are independent: each tool accepts only its own.
    assert!(validate_arguments("st_agents_list", &json!({"agent_id":"agent/chief-of-staff"})).is_ok());
}

#[test]
fn agent_selector_is_additive_discoverable_and_exact() {
    let contract = schema("st_agents_list");
    assert_eq!(contract["required"], json!([]));
    assert_eq!(contract["properties"]["agent_id"]["maxLength"], 256);
    assert!(validate_arguments("st_agents_list", &json!({"agent_id":"agent/example-reader"})).is_ok());
    for invalid in ["example-reader", "agent/", "person/robin", "agent/example-invalid prose"] {
        assert_eq!(validate_arguments("st_agents_list", &json!({"agent_id":invalid}))
            .unwrap_err()["code"], "invalid-arguments");
    }
    assert!(validate_arguments("st_agents_list", &json!({"agent_id":false})).is_err());
    assert!(validate_arguments("st_alerts_list", &json!({"agent_id":"agent/example-reader"})).is_ok());
    assert!(validate_arguments("st_missions_list", &json!({"agent_id":"agent/chief-of-staff"})).is_err());
}

fn write_arguments(name: &str) -> Value {
    if name == "st_message_send" {
        send_arguments()
    } else {
        json!({"action_id":"action/test","idempotency_key":"test-key-12345678",
            "fence":{"snapshot_id":"snapshot/test"},"alert_id":"attention/test","episode":"episode/test",
            "option":"done","summary":"Done"})
    }
}

#[tokio::test]
async fn action_identity_violations_fail_before_contacting_gateway() {
    let adapter = disconnected_adapter();
    for name in ["st_alert_answer", "st_message_send"] {
        for (field, value, rule) in [
            ("action_id", "test".to_owned(), "^action/"),
            ("action_id", "prefix/action/test".to_owned(), "^action/"),
            ("idempotency_key", "k".repeat(15), "16-256 printable ASCII"),
            ("idempotency_key", "k".repeat(257), "16-256 printable ASCII"),
            ("idempotency_key", "\u{e9}".repeat(16), "printable ASCII"),
            ("idempotency_key", "\u{e9}".repeat(256), "printable ASCII"),
            ("idempotency_key", format!("{}\n", "k".repeat(16)), "printable ASCII"),
            ("idempotency_key", format!("{}\t", "k".repeat(16)), "printable ASCII"),
        ] {
            let mut args = write_arguments(name);
            args[field] = json!(value);
            let error = call(&adapter, name, &args).await.unwrap_err();
            assert_eq!(error["code"], "invalid-arguments", "{name}: {args}");
            let message = error["message"].as_str().unwrap();
            assert!(message.contains(field) && message.contains(rule), "{message}");
        }
    }
}

#[test]
fn action_identity_accepts_inclusive_key_length_bounds() {
    for name in ["st_alert_answer", "st_message_send"] {
        for length in [16, 256] {
            for character in ['k', ' ', '~'] {
                let mut args = write_arguments(name);
                args["idempotency_key"] = json!(character.to_string().repeat(length));
                assert!(validate_arguments(name, &args).is_ok(), "{name}: {length}: {character}");
            }
        }
    }
}

#[test]
fn validation_rejects_unknown_tools_nonobjects_extra_missing_and_wrong_types() {
    assert_eq!(validate_arguments("shell", &json!({})).unwrap_err()["code"], "unknown-tool");
    let negatives = [
        ("st_alerts_list", json!(null)),
        ("st_alerts_list", json!([])),
        ("st_alerts_list", json!({"command":"ignored?"})),
        ("st_alerts_list", json!({"limit":"1"})),
        ("st_alerts_list", json!({"limit":0})),
        ("st_alerts_list", json!({"limit":-1})),
        ("st_alerts_list", json!({"limit":1.5})),
        ("st_alerts_list", json!({"cursor":true})),
        ("st_alerts_list", json!({"max_response_bytes":0})),
        ("st_alerts_list", json!({"max_response_bytes":16385})),
        ("st_alert_get", json!({})),
        ("st_alert_get", json!({"alert_id":null})),
        ("st_alert_get", json!({"alert_id":3})),
        ("st_alert_answer", json!({"action_id":"a"})),
        ("st_conversation_read", json!({"agent_id":"a","session_id":"s","reference":"r","revision":"v","start":-1,"end":3})),
        ("st_conversation_read", json!({"agent_id":"a","session_id":"s","reference":"r","revision":"v","start":0,"end":"3"})),
    ];
    for (name, args) in negatives {
        assert_eq!(validate_arguments(name, &args).unwrap_err()["code"], "invalid-arguments", "{name}: {args}");
    }
}

#[test]
fn validation_rejects_open_or_untyped_nested_fences() {
    for fence in [json!(null), json!({}), json!({"snapshot_id":7}),
        json!({"snapshot_id":"s","shell":"ignored?"}),
        json!({"snapshot_id":"s","subject_revisions":[]}),
        json!({"snapshot_id":"s","subject_revisions":{"agent/example-reader":42}})] {
        let mut args = send_arguments();
        args["fence"] = fence;
        assert_eq!(validate_arguments("st_message_send", &args).unwrap_err()["code"], "invalid-arguments");
    }
    for key in ["action_id", "idempotency_key", "to", "title", "content"] {
        let mut args = send_arguments();
        args[key] = json!(false);
        assert!(validate_arguments("st_message_send", &args).is_err(), "{key}");
        args.as_object_mut().unwrap().remove(key);
        assert!(validate_arguments("st_message_send", &args).is_err(), "missing {key}");
    }
}

fn bounded_read_cases() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("st_conversations_list", "conversations", json!({"cursor":"page/a +&?","limit":2,"max_response_bytes":16384})),
        ("st_conversation_detail", "conversation-detail", json!({"agent_id":"agent/example-reader","max_response_bytes":16384})),
        ("st_conversation_timeline", "conversation-timeline", json!({"agent_id":"agent/example-reader","session_id":"session/a","cursor":"page/a +&?","limit":2,"max_response_bytes":16384})),
        ("st_conversation_search", "conversation-search", json!({"text":"hello +&? café","agent_id":"agent/example-reader","cursor":"page/a +&?","limit":2,"max_response_bytes":16384})),
        ("st_conversation_read", "conversation-content", json!({"agent_id":"agent/example-reader","session_id":"session/a","reference":"content/a +&?","revision":"revision/a","start":0,"end":12,"max_response_bytes":16384})),
    ]
}

#[test]
fn bounded_reads_validate_exact_fields_required_selectors_and_budgets() {
    for (name, _, args) in bounded_read_cases() {
        assert!(validate_arguments(name, &args).is_ok(), "{name}");
        let contract = schema(name);
        for required in contract["required"].as_array().unwrap() {
            let mut missing = args.clone();
            missing.as_object_mut().unwrap().remove(required.as_str().unwrap());
            assert!(validate_arguments(name, &missing).is_err(), "{name}: missing {required}");
        }
        for (key, value) in [
            ("max_response_bytes", json!(0)),
            ("max_response_bytes", json!(16385)),
            ("max_response_bytes", json!("4096")),
            ("person", json!("person/robin")),
            ("environment", json!(true)),
            ("sender", json!("assistant/chatgpt")),
            ("recipient", json!("agent/example-recipient")),
        ] {
            let mut invalid = args.clone();
            invalid[key] = value;
            assert!(validate_arguments(name, &invalid).is_err(), "{name}: {key}");
        }
        for (key, property) in contract["properties"].as_object().unwrap() {
            let mut invalid = args.clone();
            invalid[key] = if property["type"] == "string" { json!(7) } else { json!("7") };
            assert!(validate_arguments(name, &invalid).is_err(), "{name}: {key}");
        }
    }
    assert!(validate_arguments("st_conversations_list", &json!({})).is_ok());
    assert!(validate_arguments("st_conversation_timeline", &json!({"agent_id":"agent/example-reader"})).is_ok());
    assert!(validate_arguments("st_conversation_search", &json!({"text":"hello"})).is_ok());
}

#[tokio::test]
async fn bounded_reads_dispatch_exact_get_routes_queries_and_bearer_and_preserve_envelopes() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let expected = json!({
        "api_version":"st3.client.v0",
        "value":{"items":[],"page":{"next_cursor":"page/next"}},
        "snapshot":{"id":"snapshot/read"}
    });
    let response = expected.clone();
    let (sent, mut received) = tokio::sync::mpsc::channel(1);
    let gateway = Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri,
              headers: axum::http::HeaderMap,
              axum::extract::Query(query): axum::extract::Query<std::collections::BTreeMap<String, String>>| {
            let sent = sent.clone();
            let response = response.clone();
            async move {
                sent.send((method, uri.path().to_owned(), headers, query)).await.unwrap();
                axum::Json(response)
            }
        },
    );
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let adapter = Adapter {
        client: reqwest::Client::builder().unix_socket(socket).build().unwrap(),
        bearer: "test-only-read-credential".into(),
        profile: Profile::ChatGpt,
    };
    for (name, route, args) in bounded_read_cases() {
        assert_eq!(call(&adapter, name, &args).await.unwrap(), expected, "{name}");
        let (method, path, headers, query) = received.recv().await.unwrap();
        assert_eq!(method, axum::http::Method::GET, "{name}");
        assert_eq!(path, format!("/v1/client/assistant/{route}"), "{name}");
        assert_eq!(headers["authorization"], "Bearer test-only-read-credential");
        let expected_query: std::collections::BTreeMap<_, _> = args.as_object().unwrap()
            .iter().map(|(key, value)| (key.clone(), value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())))
            .collect();
        assert_eq!(query, expected_query, "{name}");
    }
    server.abort();
}

#[tokio::test]
async fn bounded_read_budget_denials_remain_incomplete_errors() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let gateway = Router::new().fallback(|| async {
        (StatusCode::PAYLOAD_TOO_LARGE, axum::Json(json!({"code":"response-budget-exceeded"})))
    });
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let adapter = Adapter {
        client: reqwest::Client::builder().unix_socket(socket).build().unwrap(),
        bearer: "test-only-read-credential".into(),
        profile: Profile::ChatGpt,
    };
    let result = call(&adapter, "st_conversations_list", &json!({"max_response_bytes":1})).await;
    server.abort();
    assert_eq!(result.unwrap_err(), json!({
        "code":"incomplete","http_status":413,"daemon":{"code":"response-budget-exceeded"}
    }));
}

#[tokio::test]
async fn invalid_calls_fail_before_contacting_gateway() {
    let error = call(&disconnected_adapter(), "st_message_send", &json!({"command":"no"})).await.unwrap_err();
    assert_eq!(error["code"], "invalid-arguments");
}

#[tokio::test]
async fn alert_answer_maps_to_work_done_and_returns_direct_action_result_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let expected = json!({
        "api_version":"st3.client.v0",
        "value":{"kind":"action-result", "action_id":"action/answer",
            "operation_id":"operation/client-answer", "status":"completed",
            "affected_ids":["step-run/test/ask"], "snapshot_id":"snapshot/after-answer"},
        "snapshot":{"id":"snapshot/after-answer"}
    });
    let response = expected.clone();
    let (sent, mut received) = tokio::sync::mpsc::channel(1);
    let gateway = Router::new().route("/v1/client/assistant/propose", axum::routing::post(
        move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| async move {
            sent.send((headers, body)).await.unwrap();
            axum::Json(response)
        },
    ));
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let adapter = Adapter {
        client: reqwest::Client::builder().unix_socket(socket).build().unwrap(),
        bearer: "test-only-answer-credential".into(),
        profile: Profile::ChatGpt,
    };
    let arguments = json!({
        "action_id":"action/answer", "idempotency_key":"answer-key-0000001",
        "fence":{"snapshot_id":"snapshot/before-answer"},
        "alert_id":"step-run/test/ask", "episode":"claim/ask", "option":"yes",
        "summary":"Proceed with the selected option"
    });
    let result = call(&adapter, "st_alert_answer", &arguments).await;
    server.abort();
    assert_eq!(result.unwrap(), expected);
    let (headers, body) = received.recv().await.unwrap();
    assert_eq!(headers["authorization"], "Bearer test-only-answer-credential");
    assert_eq!(body, json!({
        "api_version":"st3.client.v0", "id":"action/answer", "type":"work.done",
        "idempotency_key":"answer-key-0000001", "fence":{"snapshot_id":"snapshot/before-answer"},
        "parameters":{"target_id":"step-run/test/ask", "episode":"claim/ask",
            "summary":"Proceed with the selected option", "answer":{"id":"yes"}}
    }));
    assert!(expected["value"].get("approval_id").is_none());
}

#[tokio::test]
async fn http_router_rejects_get_mcp_and_unknown_paths_without_content() {
    let app = http_router(disconnected_adapter());
    for (method, path, status) in [
        ("GET", "/mcp", StatusCode::METHOD_NOT_ALLOWED),
        ("GET", "/unknown", StatusCode::NOT_FOUND),
        ("POST", "/unknown", StatusCode::NOT_FOUND),
    ] {
        let response = app.clone().oneshot(Request::builder().method(method).uri(path).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), status);
        assert!(to_bytes(response.into_body(), 1024).await.unwrap().is_empty());
    }
}

#[test]
fn credential_reader_accepts_only_bounded_owner_only_regular_token_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential");
    std::fs::write(&path, "test-token\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(read_credential(&path).unwrap(), "test-token");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(read_credential(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(read_credential(&link).is_err());
    assert!(read_credential(dir.path()).is_err());
    for bad in ["", " \n", "two tokens", "token\ninjected", "nonascii-\u{e9}"] {
        std::fs::write(&path, bad).unwrap();
        assert!(read_credential(&path).is_err());
    }
    std::fs::write(&path, "a".repeat(4097)).unwrap();
    assert!(read_credential(&path).is_err());
    std::fs::write(&path, [0xff]).unwrap();
    assert!(read_credential(&path).is_err());
}

#[derive(Clone, Default)]
struct CapturedLogs(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);
impl std::io::Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer { self.clone() }
}
impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().clone()).unwrap()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_telemetry_is_correlated_and_content_free() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt().without_time().with_ansi(false)
        .with_writer(logs.clone()).finish();
    let _capture = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("st3-client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let id = "request/12345678-1234-1234-1234-123456789abc";
    let body = json!({"request_id":id,"value":{"content":"RESPONSE_CONTENT_SENTINEL"}}).to_string();
    let expected_bytes = body.len();
    let server_body = body.clone();
    let router = Router::new()
        .route("/v1/client/assistant/attention-rich", get(move || {
            let body = server_body.clone();
            async move { body }
        }))
        .route("/v1/client/assistant/propose", axum::routing::post(|| async {
            (StatusCode::FORBIDDEN, r#"{"request_id":"REQUEST_SECRET_SENTINEL","code":"ERROR_SECRET_SENTINEL","message":"ERROR_CONTENT_SENTINEL"}"#)
        }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let adapter = Adapter { client: reqwest::Client::builder().unix_socket(socket.as_path()).build().unwrap(),
        bearer: "BEARER_SECRET_SENTINEL".into(), profile: Profile::ChatGpt };
    assert_eq!(call(&adapter, "st_alerts_list", &json!({"cursor":"CURSOR_SECRET_SENTINEL"})).await.unwrap()["request_id"], id);
    let mut args = send_arguments();
    args["content"] = json!("ARGUMENT_CONTENT_SENTINEL");
    assert_eq!(call(&adapter, "st_message_send", &args).await.unwrap_err()["code"], "daemon-denied");
    assert_eq!(call(&adapter, "UNKNOWN_TOOL_SECRET_SENTINEL", &json!({})).await.unwrap_err()["code"], "unknown-tool");
    assert_eq!(call(&adapter, "st_alert_get", &json!({})).await.unwrap_err()["code"], "invalid-arguments");
    let text = logs.text();
    assert_eq!(text.matches("ChatGPT tool call finished").count(), 4, "{text}");
    for field in ["tool=\"st_alerts_list\"", "outcome=\"success\"", "code=\"ok\"",
        "outcome=\"error\"", "code=\"daemon-denied\"", "code=\"invalid-arguments\"",
        "tool=\"unknown\"", "duration_ms=", id, &format!("response_bytes={expected_bytes}")] {
        assert!(text.contains(field), "{field}: {text}");
    }
    assert!(!text.contains("SENTINEL"), "{text}");
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_telemetry_records_decode_transport_and_cancelled_outcomes() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt().without_time().with_ansi(false)
        .with_writer(logs.clone()).finish();
    let _capture = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("st3-client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify = entered.clone();
    let router = Router::new()
        .route("/v1/client/assistant/attention-rich", get(|| async { "INVALID_BODY_SECRET_SENTINEL" }))
        .route("/v1/client/assistant/agents-rich", get(move || {
            let notify = notify.clone();
            async move {
                notify.notify_one();
                std::future::pending::<String>().await
            }
        }))
        .route("/v1/client/assistant/missions-rich", get(|| async {
            (StatusCode::PAYLOAD_TOO_LARGE, r#"{"code":"response-budget-exceeded","message":"BUDGET_CONTENT_SENTINEL"}"#)
        }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let adapter = Adapter { client: reqwest::Client::builder().unix_socket(socket.as_path()).build().unwrap(), bearer: "test".into(), profile: Profile::ChatGpt };
    assert_eq!(call(&adapter, "st_alerts_list", &json!({})).await.unwrap_err()["code"], "invalid-daemon-response");
    assert_eq!(call(&adapter, "st_missions_list", &json!({})).await.unwrap_err()["code"], "incomplete");
    assert_eq!(call(&disconnected_adapter(), "st_alerts_list", &json!({})).await.unwrap_err()["code"], "unavailable");
    {
        let args = json!({});
        let pending = call(&adapter, "st_agents_list", &args);
        tokio::pin!(pending);
        tokio::select! {
            () = entered.notified() => {},
            result = &mut pending => panic!("dispatch completed before cancellation: {result:?}"),
        }
    }
    let text = logs.text();
    assert_eq!(text.matches("ChatGPT tool call finished").count(), 4, "{text}");
    for code in ["invalid-daemon-response", "incomplete", "unavailable", "cancelled"] {
        assert!(text.contains(&format!("code=\"{code}\"")), "{text}");
    }
    assert!(text.contains("outcome=\"cancelled\""), "{text}");
    assert!(!text.contains("SENTINEL"), "{text}");
    server.abort();
}

#[test]
fn chatgpt_discovery_matches_main_golden() {
    // Captured by serializing Adapter::list_tools's complete rmcp ListToolsResult
    // on origin/main bed66fbea69fae546ce2a5f7d6df942ae335ce47, not regenerated
    // from this branch's TOOLS/schema(). Keep this file immutable. Compared as JSON
    // values because object key order depends on serde_json's preserve_order
    // feature, which smallclaims enables under workspace feature unification only.
    // Regenerate only in a scratch worktree at that commit: temporarily add a test
    // in crates/st-chatgpt/src/tests.rs writing serde_json::to_vec(&ListToolsResult {
    // tools: tool_definitions(), ..Default::default() }) to
    // crates/st-chatgpt/src/testdata/chatgpt-tools-list.json; run
    // `cargo test -p st-chatgpt <name>`, then remove the temporary test.
    // main 787744ffea's crates/fractal-chatgpt/src/lib.rs is byte-identical to
    // bed66fbea6's crates/st-chatgpt/src/lib.rs.
    let result = ListToolsResult {
        tools: tool_definitions(Profile::ChatGpt),
        ..Default::default()
    };
    let golden: Value =
        serde_json::from_slice(include_bytes!("testdata/chatgpt-tools-list.json")).unwrap();
    assert_eq!(serde_json::to_value(&result).unwrap(), golden);
}

#[test]
fn profiles_pin_server_identity_and_cross_profile_annotations() {
    assert_eq!(Profile::ChatGpt.server_name(), "st-chatgpt");
    assert_eq!(Profile::ChatGpt.assistant_principal(), "assistant/chatgpt");
    assert_eq!(Profile::Mcp.server_name(), "st");
    assert_eq!(Profile::Mcp.assistant_principal(), "assistant/mcp");
    let actual = serde_json::to_value(tool_definitions(Profile::ChatGpt)).unwrap();
    let mcp = serde_json::to_value(tool_definitions(Profile::Mcp)).unwrap();
    for (old, new) in actual.as_array().unwrap().iter().zip(mcp.as_array().unwrap()) {
        assert!(old.get("outputSchema").is_none());
        assert_eq!(old["name"], new["name"]);
        assert_eq!(old["inputSchema"], new["inputSchema"]);
        assert_eq!(old["annotations"], new["annotations"]);
        assert!(!new["description"].as_str().unwrap().contains("ChatGPT"));
    }
}

#[test]
fn all_eleven_mcp_output_schemas_pin_envelopes_and_value_contracts() {
    let expected = [
        ("st_alerts_list", Some("page"), vec!["kind","items","page"]),
        ("st_alert_get", None, vec!["id","episode","kind","title","detail","request"]),
        ("st_alert_answer", Some("action-result"), vec!["kind","action_id","status"]),
        ("st_missions_list", Some("page"), vec!["kind","items","page"]),
        ("st_agents_list", Some("page"), vec!["kind","items","page"]),
        ("st_conversations_list", Some("page"), vec!["kind","items","page"]),
        ("st_conversation_detail", Some("conversation-detail"), vec!["kind","items","page","agent_id","session_id","current_session_id"]),
        ("st_conversation_timeline", Some("conversation-timeline"), vec!["kind","items","page","agent_id","session_id","current_session_id"]),
        ("st_conversation_search", Some("conversation-search"), vec!["kind","items","page"]),
        ("st_conversation_read", Some("assistant-conversation-content"), vec!["kind","agent_id","session_id","reference","revision","start","end","text","returned_bytes","truncated","next_range"]),
        ("st_message_send", Some("action-result"), vec!["kind","action_id","status"]),
    ];
    let tools = tool_definitions(Profile::Mcp);
    assert_eq!(tools.len(), expected.len());
    for (tool, (name, kind, required)) in tools.into_iter().zip(expected) {
        let tool = serde_json::to_value(tool).unwrap();
        assert_eq!(tool["name"], name);
        let output = &tool["outputSchema"];
        assert_eq!(output["type"], "object");
        assert_eq!(output["title"], format!("{name} result"));
        let branches = output["oneOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0]["required"], json!(["api_version","request_id","snapshot","value"]));
        assert_eq!(branches[0]["properties"]["api_version"], json!({"const":"st3.client.v0"}));
        assert_eq!(branches[0]["properties"]["request_id"], json!({"type":"string"}));
        let value = &branches[0]["properties"]["value"];
        assert_eq!(value["type"], "object");
        assert_eq!(value["required"], json!(required));
        if let Some(kind) = kind { assert_eq!(value["properties"]["kind"], json!({"const":kind})); }
        assert_eq!(branches[1], json!({"type":"object","properties":{
            "code":{"enum":["unknown-tool","invalid-arguments","unavailable","invalid-daemon-response","incomplete","daemon-denied"]},
            "message":{"type":"string"},"http_status":{"type":"integer"},"daemon":{}
        },"required":["code"],"additionalProperties":false}));
        if value["properties"].get("page").is_some() {
            assert_eq!(value["properties"]["items"], json!({"type":"array","items":{"type":"object"}}));
            assert_eq!(value["properties"]["page"]["required"], json!(["limit","has_more","next_cursor"]));
        }
    }
}

#[test]
fn legacy_call_bytes_and_mcp_structured_text_mirrors_are_pinned() {
    for (value, error, text) in [
        (json!({"api_version":"st3.client.v0","request_id":"request/example",
            "snapshot":{"id":"snapshot/example","host_id":"host/example","store_index":1},
            "value":{"kind":"page","items":[],"page":{"limit":1,"has_more":false,"next_cursor":null}}}), false,
            r#"{"api_version":"st3.client.v0","request_id":"request/example","snapshot":{"host_id":"host/example","id":"snapshot/example","store_index":1},"value":{"items":[],"kind":"page","page":{"has_more":false,"limit":1,"next_cursor":null}}}"#),
        (json!({"code":"invalid-arguments","message":"Use the exact published tool schema."}), true,
            r#"{"code":"invalid-arguments","message":"Use the exact published tool schema."}"#),
        (json!({"code":"incomplete","http_status":413,"daemon":{"code":"response-budget-exceeded"}}), true,
            r#"{"code":"incomplete","daemon":{"code":"response-budget-exceeded"},"http_status":413}"#),
        (json!({"code":"daemon-denied","http_status":403,"daemon":{"code":"forbidden"}}), true,
            r#"{"code":"daemon-denied","daemon":{"code":"forbidden"},"http_status":403}"#),
        (json!({"code":"unavailable","message":"The paired st daemon is unavailable."}), true,
            r#"{"code":"unavailable","message":"The paired st daemon is unavailable."}"#),
        (json!({"code":"invalid-daemon-response"}), true, r#"{"code":"invalid-daemon-response"}"#),
        (json!({"code":"unknown-tool"}), true, r#"{"code":"unknown-tool"}"#),
    ] {
        let legacy = serde_json::to_value(tool_result(Profile::ChatGpt, value.clone(), error)).unwrap();
        // Pin the wire fields independently of rmcp's result constructors.
        let mut expected = json!({"resultType":"complete","content":[{"type":"text","text":text}],"isError":error});
        assert_eq!(serde_json::to_vec(&legacy).unwrap(), serde_json::to_vec(&expected).unwrap());
        assert!(legacy.get("structuredContent").is_none());
        let mcp = serde_json::to_value(tool_result(Profile::Mcp, value.clone(), error)).unwrap();
        expected["structuredContent"] = value.clone();
        assert_eq!(mcp, expected);
        assert_eq!(mcp["isError"], error);
        assert_eq!(mcp["content"][0]["text"], text);
        assert_eq!(mcp["structuredContent"], value);
    }
}

#[tokio::test]
async fn mcp_dispatches_all_eleven_tools_over_only_the_existing_gateway_routes() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("st3-client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let envelope = json!({"api_version":"st3.client.v0","request_id":"request/example",
        "snapshot":{"id":"snapshot/example","host_id":"host/example","store_index":1},
        "value":{"kind":"page","items":[],"page":{"limit":1,"has_more":false,"next_cursor":null}}});
    let response = envelope.clone();
    let (sent, mut received) = tokio::sync::mpsc::channel(1);
    let gateway = Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri,
              headers: axum::http::HeaderMap, body: axum::body::Bytes| {
            let sent = sent.clone();
            let response = response.clone();
            async move {
                sent.send((method, uri, headers, body)).await.unwrap();
                axum::Json(response)
            }
        });
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let adapter = Adapter { client:reqwest::Client::builder().unix_socket(socket).build().unwrap(),
        bearer:"test-only-mcp-credential".into(), profile:Profile::Mcp };
    let cases = [
        ("st_alerts_list","attention-rich",json!({})),
        ("st_alert_get","attention-content",json!({"alert_id":"alert/example"})),
        ("st_alert_answer","propose",json!({"action_id":"action/example-answer","idempotency_key":"example-answer-key",
            "fence":{"snapshot_id":"snapshot/example"},"alert_id":"alert/example","episode":"claim/example","option":"yes","summary":"Approved"})),
        ("st_missions_list","missions-rich",json!({})),
        ("st_agents_list","agents-rich",json!({"agent_id":"agent/example-reader"})),
        ("st_conversations_list","conversations",json!({})),
        ("st_conversation_detail","conversation-detail",json!({"agent_id":"agent/example-reader"})),
        ("st_conversation_timeline","conversation-timeline",json!({"agent_id":"agent/example-reader","session_id":"session/example"})),
        ("st_conversation_search","conversation-search",json!({"text":"example"})),
        ("st_conversation_read","conversation-content",json!({"agent_id":"agent/example-reader","session_id":"session/example",
            "reference":"content/example","revision":"revision/example","start":0,"end":1})),
        ("st_message_send","propose",json!({"action_id":"action/example-message","idempotency_key":"example-message-key",
            "fence":{"snapshot_id":"snapshot/example"},"to":"agent/example-recipient","title":"Example","content":"Exact example"})),
    ];
    for (name, route, args) in cases {
        let result = call(&adapter, name, &args).await.unwrap();
        assert_eq!(result, envelope);
        let published = serde_json::to_value(tool_result(Profile::Mcp, result, false)).unwrap();
        assert_eq!(published["structuredContent"], envelope);
        assert_eq!(published["content"][0]["text"], envelope.to_string());
        let (method, uri, headers, body) = received.recv().await.unwrap();
        assert_eq!(uri.path(), format!("/v1/client/assistant/{route}"));
        assert_eq!(headers["authorization"], "Bearer test-only-mcp-credential");
        if route == "propose" {
            assert_eq!(method, axum::http::Method::POST);
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["type"], if name == "st_alert_answer" {"work.done"} else {"message.send"});
            assert_eq!(body["id"], args["action_id"]);
            assert_eq!(body["idempotency_key"], args["idempotency_key"]);
            assert_eq!(body["fence"], args["fence"]);
            assert!(body.get("principal").is_none());
        } else {
            assert_eq!(method, axum::http::Method::GET);
            assert!(body.is_empty());
        }
    }
    server.abort();
}

#[test]
fn mcp_uses_the_same_bounded_raw_token_file_and_gateway_socket_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let credential = dir.path().join("credential");
    std::fs::write(&credential, "test-only-mcp-token\n").unwrap();
    std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
    let adapter = Adapter::with_profile(&dir.path().join("st3-client.sock"), &credential, Profile::Mcp).unwrap();
    assert_eq!(adapter.profile, Profile::Mcp);
    assert_eq!(adapter.bearer, "test-only-mcp-token");
    assert!(Adapter::with_profile(&dir.path().join("st3.sock"), &credential, Profile::Mcp).is_err());
    std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Adapter::with_profile(&dir.path().join("st3-client.sock"), &credential, Profile::Mcp).is_err());
}

#[tokio::test]
async fn both_profiles_preserve_gateway_write_denials_without_claiming_authority() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("st3-client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    // Scope authorization belongs to the daemon, not the adapter. This gateway
    // response exercises pass-through, not issuance or real grant admission.
    let denial = json!({"code":"forbidden","message":"missing required scope"});
    let response = denial.clone();
    let (sent, mut received) = tokio::sync::mpsc::channel(1);
    let gateway = Router::new().route("/v1/client/assistant/propose", axum::routing::post(
        move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
            let sent = sent.clone();
            let response = response.clone();
            async move {
                sent.send((headers, body)).await.unwrap();
                (StatusCode::FORBIDDEN, axum::Json(response))
            }
        },
    ));
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    for profile in [Profile::ChatGpt, Profile::Mcp] {
        let adapter = Adapter { client: reqwest::Client::builder().unix_socket(socket.as_path()).build().unwrap(),
            bearer: "test-only-read-credential".into(), profile };
        for name in ["st_alert_answer", "st_message_send"] {
            let arguments = write_arguments(name);
            let error = call(&adapter, name, &arguments).await.unwrap_err();
            assert_eq!(error, json!({"code":"daemon-denied","http_status":403,"daemon":denial}));
            let published = serde_json::to_value(tool_result(profile, error.clone(), true)).unwrap();
            assert_eq!(published["isError"], true);
            assert_eq!(published["content"][0]["text"], error.to_string());
            if profile == Profile::Mcp { assert_eq!(published["structuredContent"], error); }
            else { assert!(published.get("structuredContent").is_none()); }
            let (headers, body) = received.recv().await.unwrap();
            assert_eq!(headers["authorization"], "Bearer test-only-read-credential");
            assert!(body.get("principal").is_none());
            assert!(body.get("scopes").is_none());
        }
    }
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn mcp_telemetry_has_no_chatgpt_wording() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt().without_time().with_ansi(false)
        .with_writer(logs.clone()).finish();
    let _capture = tracing::subscriber::set_default(subscriber);
    let mut adapter = disconnected_adapter();
    adapter.profile = Profile::Mcp;
    assert_eq!(adapter.get_info().server_info.name, "st");
    assert_eq!(call(&adapter, "unknown", &json!({})).await.unwrap_err()["code"], "unknown-tool");
    let text = logs.text();
    assert!(text.contains("MCP tool call finished"), "{text}");
    assert!(!text.contains("ChatGPT"), "{text}");
}
