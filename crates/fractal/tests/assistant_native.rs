//! Isolated real router/store coverage; no daemon, subprocess, ambient profile, or managed agent.
use fractal_daemon::AttentionListStore;
use fractal_daemon::PersonWorkStore;
use fractal_daemon::MissionsStore as _;
use fractal_daemon::{SeatsStore as _, SeatsStoreCore};
use std::{path::PathBuf, sync::Arc};

use axum::{Router, body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt as _;
use serde::Serialize;
use serde_json::{Value, json};
use fractal_daemon::{EvidenceStore as _, api::AppState, model::{ClaimInput, PersonAskRequest}, store::Store};
use fractal_client::{ActionRequest, AssistantGrant, AssistantGrantPolicy, AssistantMinimalReadMode,
    AssistantPairedSession, AssistantPairingBegin, AssistantPrincipal, AssistantScope,
    Fence, MessageKind, MessageSendParameters, PairingComplete, PersonAnswerInput,
    PersonStepParameters};
use tower::ServiceExt as _;

const OWNER: &str = "person/robin";
const AGENT: &str = "agent/example-worker.local";
const POLICY: &str = "custom.client.assistant.v1.policy";


async fn mcp_call(app: &Router, name: &str, arguments: Value) -> (bool, Value) {
    let response = app.clone().oneshot(Request::builder().method("POST").uri("/mcp")
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-03-26")
        .body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":name,"arguments":arguments}}).to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::OK, "MCP {name}: {}", String::from_utf8_lossy(&bytes));
    let result: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("MCP {name} returned invalid JSON: {error}: {}", String::from_utf8_lossy(&bytes)));
    assert!(result.get("error").is_none(), "{result}");
    let tool = &result["result"];
    let output = serde_json::from_str(tool["content"][0]["text"].as_str().unwrap()).unwrap();
    (tool["isError"].as_bool().unwrap_or(false), output)
}
struct Harness {
    root: tempfile::TempDir,
    state: AppState,
    local: Router,
    fabric: Router,
}

impl Harness {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&root.path().join("store.sqlite"), "assistant-native").unwrap());
        store.bind_fleet("3c9a1f2e-8b7d-4e6c-a5f4-1d2e3c4b5a69").unwrap();
        store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
        Self::with_store(root, store)
    }

    fn with_store(root: tempfile::TempDir, store: Arc<Store>) -> Self {
        let source = format!("version 2\nagent \"example-worker.local\" {{ workspace {:?}; command \"true\"; restart always; }}\nmission \"native-mission\" state=\"ready\" {{ goal \"Native integration mission\"; }}", root.path());
        store.apply_internal(&fractal_daemon::graph::parse_intent(&source, "assistant-native").unwrap(), "native-seed").unwrap();
        // There is no daemon task in this harness. Register its production publication
        // surfaces; pair() explicitly refreshes them before reading the summary.
        store.start_attention_list_refresher().unwrap();
        store.start_agent_roster_refresher().unwrap();
        let state = AppState {
            seat_admission: Default::default(),
            owning_person: Some(OWNER.into()), store,
            notify: Arc::new(tokio::sync::Notify::new()),
            event_notify: tokio::sync::watch::channel(0).0,
            node: "assistant-native".into(), state_dir: root.path().into(),
            pty_root: root.path().join("pty"), pty_binary: PathBuf::from("pty"),
            fleet_id: None, configured_peers: vec![], client_relay: None,
            native_session_home: None, harness_question_sink: None, assistant_receipts: Default::default(), private_notes: Default::default(), planner_default: Default::default(),
        };
        Self { local: fractal_daemon::api::router(state.clone()), fabric: fractal_daemon::api::fabric_router(state.clone()), root, state }
    }

    async fn owner(&self, method: &str, path: &str, body: &impl Serialize) -> Value {
        let (status, value) = request(&self.local, method, path, None, Some(OWNER), body).await;
        assert_eq!(status, StatusCode::OK, "{path}: {value}");
        value
    }

    async fn assistant(&self, paired: &AssistantPairedSession, method: &str, path: &str, body: &impl Serialize) -> (StatusCode, Value) {
        request(&self.fabric, method, path, Some(&paired.credential), None, body).await
    }

    async fn pair(&self) -> AssistantPairedSession {
        self.pair_principal(AssistantPrincipal::Chatgpt).await
    }

    async fn pair_principal(&self, principal: AssistantPrincipal) -> AssistantPairedSession {
        let begin = AssistantPairingBegin {
            api_version: fractal_client::API_VERSION.into(), device_name: "Native assistant test".into(),
            assistant_id: principal.clone(), scopes: scopes(), expires_in_seconds: Some(3600),
            disclosure_consent: true, consent_evidence: "Isolated integration disclosure consent".into(),
            minimal_read_mode: Some(AssistantMinimalReadMode::CallerBoundAllow),
        };
        let challenge = self.owner("POST", "/v1/client/pairings", &begin).await;
        let path = format!("/v1/client/pairings/{}/complete", challenge["value"]["pairing_id"].as_str().unwrap().trim_start_matches("pairing/"));
        let complete = PairingComplete {
            api_version: fractal_client::API_VERSION.into(), code: challenge["value"]["code"].as_str().unwrap().into(),
            device_public_key: "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc".into(),
            key_storage: Some("software".into()),
            fabric_node_id: None,
        };
        let (status, value) = request(&self.fabric, "POST", &path, None, None, &complete).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let paired: AssistantPairedSession = serde_json::from_value(value["value"].clone()).unwrap();
        assert_eq!(paired.grant.issuer, OWNER);
        assert_eq!(paired.principal, principal);
        assert_eq!(paired.grant.minimal_read_mode, AssistantMinimalReadMode::CallerBoundAllow);
        if self.state.store.published_attention_list().is_none() {
            let (status, unavailable) = self.assistant(&paired, "GET", "/v1/client/assistant/summary", &()).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{unavailable}");
            assert_eq!(unavailable["message"], "the attention view is still being prepared; retry shortly");
        }
        // The unavailable summary read can leave receipts queued after its response.
        // Drain them before publishing and comparing the fixture's exact claim cut.
        self.state.assistant_receipts.flush().await.unwrap();
        // Complete the same publications the daemon performs at startup, including
        // an empty owner view. Pairing alone does not make these views ready.
        fractal_daemon::api::refresh_agent_roster_for_test(&self.state.store).unwrap();
        fractal_daemon::api::refresh_attention_list(&self.state.store).unwrap();
        let publication = self.state.store.published_attention_list()
            .expect("the fixture must publish attention before assistant reads");
        assert_eq!(publication.cut, self.state.store.index().unwrap());
        let (cut, _, _) = self.state.store.published_agent_roster(
            self.state.store.index().unwrap(), false,
        ).expect("the fixture must publish agents before assistant reads");
        assert_eq!(cut, publication.cut);
        let (status, summary) = self.assistant(&paired, "GET", "/v1/client/assistant/summary", &()).await;
        assert_eq!(status, StatusCode::OK, "fresh pairing must read without a policy call: {summary}");
        for id in [paired.grant.id.as_str(), paired.grant.id.trim_start_matches("assistant-grant/")] {
            let inspected = self.owner("GET", &format!("/v1/client/assistant/grants/{}", urlencoding::encode(id)), &()).await;
            assert_eq!(inspected["value"]["id"], paired.grant.id);
            let raw = self.owner("GET", &format!("/v1/client/assistant/grants/{id}"), &()).await;
            assert_eq!(raw["value"]["id"], paired.grant.id);
        }
        let (replay, _) = request(&self.fabric, "POST", &path, None, None, &complete).await;
        assert_eq!(replay, StatusCode::FORBIDDEN);
        self.policy(&paired, scopes()).await;
        self.state.assistant_receipts.flush().await.unwrap();
        paired
    }

    async fn policy(&self, paired: &AssistantPairedSession, scopes: Vec<AssistantScope>) -> AssistantGrant {
        let policy = AssistantGrantPolicy {
            api_version: fractal_client::API_VERSION.into(), scopes, expires_in_seconds: Some(3600),
            disclosure_consent: true, consent_evidence: "Native owner explicit policy".into(),
            minimal_read_mode: AssistantMinimalReadMode::CallerBoundAllow,
            recipient_allowlist: Some(vec![]),
        };
        let value = self.owner("POST", &grant_path(paired, "/policy"), &policy).await;
        serde_json::from_value(value["value"].clone()).unwrap()
    }

    fn ask(&self, key: &str, title: &str) -> String {
        self.state.store.ask_person(&PersonAskRequest {
            deadline_ms: None,
            default_answer: None,
            legacy_request: None, person: OWNER.into(), title: title.into(), reason: "Owner must choose".into(),
            actor: AGENT.into(), step: None, new_run: Some(key.into()), incarnation: None,
            request: Some(json!({"version":1,"type":"choice","question":"Which option?","why_person":"Owner chooses.","answers":[{"id":"yes","label":"Yes","consequence":"Record yes."},{"id":"no","label":"No","consequence":"Record no."}]})),
            idempotency_key: key.into(),
        }).unwrap().subject
    }

    async fn fence(&self) -> Fence {
        let value = self.owner("GET", "/v1/client/capabilities", &()).await;
        Fence { snapshot_id: value["snapshot"]["id"].as_str().unwrap().into(), ..Default::default() }
    }

    async fn answer(&self, target: &str, answer: &str, key: &str) -> ActionRequest {
        let item = self.state.store.attention_items(Some(OWNER)).unwrap().into_iter().find(|item| item.subject == target).unwrap();
        ActionRequest::work_done(format!("action/{key}"), format!("native-test-{key}"), self.fence().await,
            PersonStepParameters { target_id: target.into(), summary: "Native test answer".into(), episode: item.episode,
                answer: Some(PersonAnswerInput { id: Some(answer.into()), ..Default::default() }), ..Default::default() }).unwrap()
    }

    async fn message(&self, to: &str, key: &str) -> ActionRequest {
        ActionRequest::message_send(format!("action/{key}"), format!("native-test-{key}"), self.fence().await,
            MessageSendParameters { to: to.into(), content: "Native assistant message".into(), title: None,
                in_reply_to: None, session_id: None, tags: vec![], kind: MessageKind::Wake,
                attachments: vec![], signature: None }).unwrap()
    }

    async fn mcp(&self, paired: &AssistantPairedSession) -> (Router, tokio::task::JoinHandle<()>) {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = self.root.path().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&directory).unwrap();
        let socket = directory.join("fractal-client.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let gateway = self.fabric.clone();
        let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
        let credential = self.root.path().join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&credential, &paired.credential).unwrap();
        std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
        (fractal_chatgpt::http_router(fractal_chatgpt::Adapter::new(&socket, &credential).unwrap()), server)
    }

    async fn reply(&self, to: &str, key: &str, text: &str, reply_to: Option<&str>) -> fractal_daemon::model::ClaimRecord {
        let mut body = json!({"from":AGENT,"to":to,"content":text,"idempotency_key":key});
        if let Some(reply_to) = reply_to { body["in_reply_to"] = json!(reply_to); }
        let receipt = self.owner("POST", "/v1/messages", &body).await;
        self.state.store.latest_claim(receipt["value"]["subject"].as_str().unwrap(),
            Some("message.sent")).unwrap().unwrap()
    }

    fn expire(&self, paired: &AssistantPairedSession) {
        let subject = grant_subject(paired);
        let previous = self.state.store.claims_for_subject_kind_at(&subject, POLICY, None, true, 1).unwrap().claims.pop().unwrap();
        let mut fields = previous.body["fields"].clone();
        fields["expires_at_unix_ms"] = json!(1);
        self.state.store.append_claim(&ClaimInput { subject, kind: POLICY.into(), actor: Some(OWNER.into()),
            fields: serde_json::from_value(fields).unwrap(), evidence: vec![previous.id.clone()],
            expected_subject: Some(Some(previous.id)), idempotency_key: None }).unwrap();
    }
}

async fn request(app: &Router, method: &str, path: &str, bearer: Option<&str>, person: Option<&str>, body: &impl Serialize) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path).header("content-type", "application/json");
    if let Some(bearer) = bearer { builder = builder.header("authorization", format!("Bearer {bearer}")); }
    if let Some(person) = person { builder = builder.header("x-st3-person", person); }
    let response = app.clone().oneshot(builder.body(Body::from(serde_json::to_vec(body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn scopes() -> Vec<AssistantScope> {
    vec![AssistantScope::Summary, AssistantScope::Attention, AssistantScope::Missions,
        AssistantScope::Agents, AssistantScope::Conversation, AssistantScope::Messages, AssistantScope::Answers]
}
fn grant_path(paired: &AssistantPairedSession, suffix: &str) -> String {
    format!("/v1/client/assistant/grants/{}{suffix}", paired.grant.id.trim_start_matches("assistant-grant/"))
}
fn grant_subject(paired: &AssistantPairedSession) -> String {
    format!("custom/client/assistant-v1-{}", paired.grant.id.trim_start_matches("assistant-grant/"))
}

#[tokio::test]
async fn fresh_pairing_reads_an_explicitly_published_empty_attention_view() {
    let h = Harness::new();
    assert!(h.state.store.attention_list_refresher_running());
    assert!(h.state.store.published_attention_list().is_none());
    let paired = h.pair().await;
    let publication = h.state.store.published_attention_list().unwrap();
    assert!(publication.owners.get(OWNER).is_none_or(|rows| rows.is_empty()));
    let (status, attention) = h.assistant(&paired, "GET", "/v1/client/assistant/attention", &()).await;
    assert_eq!(status, StatusCode::OK, "{attention}");
    assert!(attention["value"]["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn pairing_caller_bound_rich_reads_and_budget_do_not_silently_truncate() {
    let h = Harness::new();
    let ask = h.ask("rich-ask", "Owner-visible rich question");
    let paired = h.pair().await;
    for projection in ["summary", "attention-rich", "missions-rich", "agents-rich"] {
        let (status, value) = h.assistant(&paired, "GET", &format!("/v1/client/assistant/{projection}?max_response_bytes=16384"), &()).await;
        assert_eq!(status, StatusCode::OK, "{projection}: {value}");
        assert!(value["value"].is_object());
        if projection == "attention-rich" { assert!(value.to_string().contains("Owner-visible rich question")); }
        if projection == "missions-rich" { assert!(value["value"]["items"].as_array().unwrap().iter().any(|item| item["title"] == "native-mission")); }
        if projection == "agents-rich" { assert!(value.to_string().contains(AGENT)); }
    }
    let path = format!("/v1/client/assistant/attention-content?alert_id={}&max_response_bytes=16384", urlencoding::encode(&ask));
    let (status, value) = h.assistant(&paired, "GET", &path, &()).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["value"]["request"]["question"], "Which option?");
    let (status, value) = h.assistant(&paired, "GET", "/v1/client/assistant/attention-rich?max_response_bytes=1", &()).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{value}");
    assert_eq!(value["code"], "response-budget-exceeded");
    assert!(value.get("value").is_none(), "A failed bounded read must not masquerade as a complete page");
}

#[tokio::test]
async fn assistant_message_and_agent_reply_decode_from_real_session_timeline() {
    let h = Harness::new();
    h.state.store.append_claim(&ClaimInput {
        subject: AGENT.into(), kind: "runtime.observed".into(), actor: Some(AGENT.into()),
        fields: serde_json::from_value(json!({
            "status": "running", "runtime_id": "assistant-thread-runtime",
            "incarnation_id": "assistant-thread-runtime:i1", "terminal": false
        })).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: Some("assistant-thread-runtime".into()),
    }).unwrap();
    let paired = h.pair().await;
    let send = h.message(AGENT, "timeline-assistant-send").await;
    let (status, sent) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &send).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let original = sent["value"]["affected_ids"].as_array().unwrap().iter()
        .filter_map(Value::as_str).find(|id| id.starts_with("message/")).unwrap();
    // Native agent replies preserve the assistant principal as their recipient.
    let reply = h.owner("POST", "/v1/messages", &json!({
        "idempotency_key": "timeline-agent-reply", "from": AGENT, "to": "assistant/chatgpt",
        "in_reply_to": original, "content": "Acknowledged.", "tags": [], "attachments": []
    })).await;
    let reply_id = reply["value"]["subject"].as_str().unwrap_or_else(|| panic!("reply receipt: {reply}"));
    assert_eq!(reply["value"]["to"], "assistant/chatgpt");
    // The read-side representation grants no new assistant write authority.
    let forbidden = h.message("assistant/chatgpt", "timeline-assistant-recipient-denied").await;
    let (status, refusal) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &forbidden).await;
    assert!(!status.is_success(), "{refusal}");
    assert_eq!(refusal["code"], "forbidden", "{refusal}");
    assert_eq!(h.state.store.messages(Some("assistant/chatgpt"), true).unwrap().len(), 1);

    let sessions = h.owner("GET", "/v1/client/sessions", &()).await;
    let session_id = sessions["value"]["items"].as_array().unwrap().iter()
        .find(|session| session["owner_id"] == AGENT).unwrap()["id"].as_str().unwrap();
    let timeline = h.owner("GET", &format!("/v1/client/sessions/{session_id}/timeline"), &()).await;
    let items = timeline["value"]["items"].as_array().unwrap();
    let original_entry = items.iter().find(|entry| entry["body"]["message_id"] == original).unwrap();
    let reply_entry = items.iter().find(|entry| entry["body"]["message_id"] == reply_id).unwrap();
    assert_eq!(original_entry["body"]["from"], "assistant/chatgpt");
    assert_eq!(original_entry["body"]["to"], AGENT);
    assert_eq!(reply_entry["body"]["from"], AGENT);
    assert_eq!(reply_entry["body"]["to"], "assistant/chatgpt");
    assert_eq!(reply_entry["body"]["reply_to"], original);

    // Validate the actual API serializer output against the source of the generated codecs,
    // then decode both entries with the generated Rust SDK. No timeline fixture is fabricated.
    // This test also runs as its own binary, without sibling integration-test modules.
    let mut schema: Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/client-v0.schema.json"
    )).unwrap();
    schema.as_object_mut().unwrap().remove("oneOf");
    schema["$ref"] = json!("#/$defs/TimelineEntry");
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .build(&schema).unwrap();
    for entry in [original_entry, reply_entry] {
        validator.validate(entry).unwrap_or_else(|error| panic!("timeline wire contract: {error}"));
        let decoded: fractal_client::TimelineEntry = serde_json::from_value(entry.clone()).unwrap();
        let encoded = serde_json::to_value(decoded).unwrap();
        assert_eq!(encoded["body"]["from"], entry["body"]["from"]);
        assert_eq!(encoded["body"]["to"], entry["body"]["to"]);
    }
}

#[tokio::test]
async fn mcp_conversation_discovery_reads_agent_ack_and_rejects_another_grants_handle() {
    use std::os::unix::fs::PermissionsExt as _;

    let h = Harness::new();
    let paired = h.pair().await;
    let other = h.pair().await;
    assert_ne!(paired.grant.id, other.grant.id);
    let incarnation = "native-pty:conversation-readback";
    h.state.store.append_claim(&ClaimInput {
        subject: AGENT.into(), kind: "runtime.observed".into(), actor: Some(AGENT.into()),
        fields: serde_json::from_value(json!({"status":"running",
            "runtime_id":"conversation-readback", "incarnation_id":incarnation,"terminal":true})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let message = h.message(AGENT, "conversation-readback").await;
    let (status, sent) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let incoming = h.state.store.messages(Some(AGENT), true).unwrap().pop().unwrap();
    assert_eq!(incoming.from, "assistant/chatgpt");
    assert_eq!(incoming.assistant_metadata().unwrap().assistant_grant_id, Some(paired.grant.id.as_str()));
    let ack = format!("ACK-conversation-readback-{}", uuid::Uuid::new_v4());
    let reply_text = format!("{ack}\n{}", "Agent reply body. ".repeat(700));
    let (status, reply) = request(&h.local, "POST", "/v1/messages", None, None, &json!({
        "idempotency_key":"native-test-conversation-ack", "from":AGENT,
        "to":incoming.from, "in_reply_to":incoming.subject, "content":reply_text,
    })).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let reply = h.state.store.messages(Some("assistant/chatgpt"), true).unwrap().pop().unwrap();
    assert_eq!(reply.from, AGENT);
    assert_eq!(reply.to, "assistant/chatgpt");
    assert!(reply.content.starts_with(&ack));
    let reply_claim = h.state.store.latest_claim(&reply.subject, Some("message.sent")).unwrap().unwrap();
    assert_eq!(reply_claim.body["fields"]["content"], reply.content);
    assert_eq!(reply_claim.body["fields"]["in_reply_to"], incoming.subject);


    let socket = h.root.path().join("st3-client.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let gateway = h.fabric.clone();
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let credential = h.root.path().join("assistant-credential");
    std::fs::write(&credential, &paired.credential).unwrap();
    std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
    let app = fractal_chatgpt::http_router(fractal_chatgpt::Adapter::new(&socket, &credential).unwrap());
    let other_credential = h.root.path().join("other-assistant-credential");
    std::fs::write(&other_credential, &other.credential).unwrap();
    std::fs::set_permissions(&other_credential, std::fs::Permissions::from_mode(0o600)).unwrap();
    let other_app = fractal_chatgpt::http_router(fractal_chatgpt::Adapter::new(&socket, &other_credential).unwrap());

    let (error, listed) = mcp_call(&app, "fractal_conversations_list", json!({})).await;
    assert!(!error, "{listed}");
    let conversation = listed["value"]["items"].as_array().unwrap().iter()
        .find(|item| item["agent_id"] == AGENT).expect("the managed agent is discovered");
    let (error, timeline) = mcp_call(&app, "fractal_conversation_timeline", json!({
        "agent_id":conversation["agent_id"],"session_id":conversation["session_id"],
    })).await;
    assert!(!error, "{timeline}");
    let handle = timeline["value"]["items"].as_array().unwrap().iter()
        .find(|item| item["text"].as_str().is_some_and(|text| text.contains(&ack)))
        .expect("the durable agent ACK is present in the actual timeline")["read"].clone();
    assert!(handle.is_object(), "{timeline}");
    let (error, content) = mcp_call(&app, "fractal_conversation_read", handle.clone()).await;
    assert!(!error, "{content}");
    assert!(content["value"]["text"].as_str().unwrap().contains(&ack), "{content}");
    for key in ["agent_id", "session_id", "reference", "revision", "start", "end"] {
        assert_eq!(content["value"][key], handle[key], "{key}");
    }
    let (error, denied) = mcp_call(&other_app, "fractal_conversation_read", handle).await;
    assert!(error, "another valid grant must not redeem the first grant's content handle: {denied}");
    assert_eq!(denied["code"], "daemon-denied");
    assert!(!denied.to_string().contains(&ack));
    server.abort();
}

#[tokio::test]
async fn direct_answer_executes_without_approval_with_provenance_and_rejects_stale_episode() {
    let h = Harness::new();
    let ask = h.ask("answer-ask", "Answer directly");
    let paired = h.pair().await;
    h.policy(&paired, scopes().into_iter().filter(|scope| *scope != AssistantScope::Messages && *scope != AssistantScope::Conversation).collect()).await;
    let answer = h.answer(&ask, "yes", "assistant-answer").await;
    let (status, result) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &answer).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert!(result["value"]["affected_ids"].as_array().unwrap().iter().any(|id| id.as_str() == Some(ask.as_str())));
    assert!(result["value"].get("approval_id").is_none(), "{result}");
    let durable = h.state.store.step_run(&ask).unwrap().unwrap();
    assert_eq!(durable.person_answers.len(), 1);
    assert_eq!(durable.person_answers[0].answer.as_ref().unwrap()["id"], "yes");
    assert_eq!(durable.person_answers[0].respondent, "assistant/chatgpt");
    assert_eq!(durable.person_answers[0].assistant_owner.as_deref(), Some(OWNER));
    assert_eq!(durable.person_answers[0].assistant_session_actor.as_deref(), Some(paired.session_actor.as_str()));
    assert_eq!(durable.person_answers[0].sender_class.as_deref(), Some("external-assistant"));
    assert_eq!(durable.person_answers[0].assistant_grant_id.as_deref(), Some(paired.grant.id.as_str()));
    let shown = h.owner("GET", &format!("/v1/client/work/{ask}"), &()).await;
    let projected = &shown["value"]["person_answers"][0];
    assert_eq!(projected["respondent"], "assistant/chatgpt");
    assert_eq!(projected["assistant_owner"], OWNER);
    assert_eq!(projected["assistant_session_actor"], paired.session_actor);
    assert_eq!(projected["sender_class"], "external-assistant");
    assert_eq!(projected["assistant_grant_id"], paired.grant.id);
    let decoded: fractal_client::Work = serde_json::from_value(shown["value"].clone()).unwrap();
    assert_eq!(decoded.person_answers[0].respondent, "assistant/chatgpt");
    assert_eq!(decoded.person_answers[0].assistant_owner.as_deref(), Some(OWNER));
    assert_eq!(decoded.person_answers[0].assistant_session_actor.as_deref(), Some(paired.session_actor.as_str()));
    assert_eq!(decoded.person_answers[0].sender_class.as_deref(), Some("external-assistant"));
    assert_eq!(decoded.person_answers[0].assistant_grant_id.as_deref(), Some(paired.grant.id.as_str()));
    let claim = h.state.store.latest_claim(&ask, Some("work.person-done")).unwrap().unwrap();
    assert_eq!(claim.actor.as_deref(), Some("assistant/chatgpt"));
    assert_eq!(claim.body["fields"]["assistant_owner"], OWNER);
    assert_eq!(claim.body["fields"]["assistant_session_actor"], paired.session_actor);
    assert_eq!(claim.body["fields"]["sender_class"], "external-assistant");
    assert_eq!(claim.body["fields"]["assistant_grant_id"], paired.grant.id);
    let notification = h.state.store.messages(Some(AGENT), true).unwrap().pop().unwrap();
    let notification_claim = h.state.store.latest_claim(&notification.subject, Some("message.sent")).unwrap().unwrap();
    assert_eq!(notification_claim.actor.as_deref(), Some("assistant/chatgpt"));
    assert_eq!(notification_claim.body["fields"]["from"], "assistant/chatgpt");
    assert_eq!(notification_claim.body["fields"]["assistant_owner"], OWNER);
    assert_eq!(notification_claim.body["fields"]["assistant_session_actor"], paired.session_actor);
    let metadata = notification.assistant_metadata().unwrap();
    let ding = fractal_drivers::ding::fractal_notification_with_metadata(
        &notification.subject, &notification.from, &notification.to, notification.title.as_deref(),
        &notification.content, "test-body-digest",
        fractal_drivers::ding::NotificationMetadata { attachments: &[], assistant: Some(&metadata) },
    );
    assert!(ding.contains(fractal_drivers::ding::EXTERNAL_ASSISTANT_WARNING));
    assert!(ding.contains(&format!("assistant_grant_id=\"{}\"", paired.grant.id)));
    let notification = serde_json::to_value(notification).unwrap();
    assert_eq!(notification["sender_class"], "external-assistant");
    assert_eq!(notification["assistant_grant_id"], paired.grant.id);
    assert!(h.state.store.attention_items(Some(OWNER)).unwrap().is_empty(), "Answering must not create an approval ask");
    let (status, replay) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &answer).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(result["value"]["affected_ids"], replay["value"]["affected_ids"]);
    assert_eq!(h.state.store.step_run(&ask).unwrap().unwrap().person_answers.len(), 1);
    assert_eq!(h.state.store.messages(Some(AGENT), true).unwrap().len(), 1);

    let stale_ask = h.ask("stale-ask", "Stale answer");
    let mut stale = serde_json::to_value(h.answer(&stale_ask, "yes", "stale-answer").await).unwrap();
    stale["parameters"]["episode"] = json!("not-the-current-episode");
    let stale: ActionRequest = serde_json::from_value(stale).unwrap();
    let (status, denied) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &stale).await;
    assert!(!status.is_success(), "{denied}");
    assert_eq!(denied["code"], "stale-fence");
    assert!(h.state.store.step_run(&stale_ask).unwrap().unwrap().person_answers.is_empty());
}

#[tokio::test]
async fn direct_answers_require_write_scope_and_current_unexpired_unrevoked_grant() {
    let h = Harness::new();
    for refusal in ["missing-scope", "revoked", "expired"] {
        let ask = h.ask(&format!("answer-{refusal}"), "Answer must be refused");
        let paired = h.pair().await;
        let answer = h.answer(&ask, "yes", &format!("answer-{refusal}")).await;
        match refusal {
            "missing-scope" => {
                h.policy(&paired, scopes().into_iter().filter(|scope| *scope != AssistantScope::Answers).collect()).await;
            }
            "revoked" => {
                h.owner("POST", &grant_path(&paired, "/revoke"), &json!({})).await;
            }
            "expired" => h.expire(&paired),
            _ => unreachable!(),
        }
        let (status, denied) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &answer).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refusal}: {denied}");
        assert!(h.state.store.step_run(&ask).unwrap().unwrap().person_answers.is_empty());
        assert!(h.state.store.latest_claim(&ask, Some("work.person-done")).unwrap().is_none());
    }
    assert!(h.state.store.messages(Some(AGENT), true).unwrap().is_empty());
}

#[tokio::test]
async fn existing_agent_outside_recipient_allowlist_succeeds_and_person_scope_revocation_expiry_fail_closed() {
    let h = Harness::new();
    let paired = h.pair().await;
    let message = h.message(AGENT, "broad-agent-message").await;
    let (status, first) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (status, replay) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(first["value"]["affected_ids"], replay["value"]["affected_ids"]);
    assert!(first["value"]["affected_ids"].as_array().unwrap().iter().any(|id| id.as_str().is_some_and(|id| id.starts_with("message/"))));
    assert_eq!(h.state.store.messages(Some(AGENT), true).unwrap().len(), 1);
    let subject = first["value"]["affected_ids"].as_array().unwrap().iter()
        .filter_map(Value::as_str).find(|id| id.starts_with("message/")).unwrap();
    let claim = h.state.store.latest_claim(subject, Some("message.sent")).unwrap().unwrap();
    assert_eq!(claim.actor.as_deref(), Some("assistant/chatgpt"));
    assert_eq!(claim.body["fields"]["from"], "assistant/chatgpt");
    assert_eq!(claim.body["fields"]["assistant_owner"], OWNER);
    assert_eq!(claim.body["fields"]["assistant_session_actor"], paired.session_actor);
    assert_eq!(claim.body["fields"]["sender_class"], "external-assistant");
    assert_eq!(claim.body["fields"]["assistant_grant_id"], paired.grant.id);
    let projected = h.state.store.messages(Some(AGENT), true).unwrap().pop().unwrap();
    assert_eq!(projected.from, "assistant/chatgpt");
    let metadata = projected.assistant_metadata().unwrap();
    let notification = fractal_drivers::ding::fractal_notification_with_metadata(
        &projected.subject, &projected.from, &projected.to, projected.title.as_deref(),
        &projected.content, &"0".repeat(64),
        fractal_drivers::ding::NotificationMetadata { attachments: &[], assistant: Some(&metadata) },
    );
    assert!(notification.contains("from=\"assistant/chatgpt\""));
    assert!(notification.contains(&format!("assistant_grant_id=\"{}\"", paired.grant.id)));
    assert!(notification.contains(fractal_drivers::ding::EXTERNAL_ASSISTANT_WARNING));
    assert!(!notification.contains(fractal_drivers::ding::PERSON_REPLY_INSTRUCTION));
    let audit = h.state.store.claims_page(None, None, 0, None, false, 1000).unwrap().claims
        .into_iter().find(|claim| claim.kind == "custom.client.assistant.v1.audit"
            && claim.body["fields"]["operation"] == "grant-authorized-write").unwrap();
    assert_eq!(audit.actor.as_deref(), Some(paired.session_actor.as_str()));
    assert_eq!(audit.body["fields"]["actor"], paired.session_actor);
    assert_eq!(audit.body["fields"]["principal"], "assistant/chatgpt");
    assert_eq!(audit.body["fields"]["issuer"], OWNER);
    assert_eq!(audit.body["fields"]["grant_id"], paired.grant.id);
    let projected = serde_json::to_value(projected).unwrap();
    assert_eq!(projected["from"], "assistant/chatgpt");
    assert_eq!(projected["sender_class"], "external-assistant");
    assert_eq!(projected["assistant_grant_id"], paired.grant.id);
    let forbidden = h.message(OWNER, "person-recipient-denied").await;
    let (status, _) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &forbidden).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(h.state.store.messages(Some(OWNER), true).unwrap().is_empty());
    h.policy(&paired, scopes().into_iter().filter(|scope| *scope != AssistantScope::Messages).collect()).await;
    let (status, _) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Even a cached write must require the current scope");
    h.policy(&paired, scopes()).await;
    h.owner("POST", &grant_path(&paired, "/revoke"), &json!({})).await;
    let (status, _) = h.assistant(&paired, "GET", "/v1/client/assistant/agents-rich", &()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let expired = h.pair().await;
    h.expire(&expired);
    let (status, _) = h.assistant(&expired, "GET", "/v1/client/assistant/summary", &()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(h.state.store.messages(Some(AGENT), true).unwrap().len(), 1);
}

#[tokio::test]
async fn writer_transaction_rechecks_policy_revision_actor_scope_and_revocation() {
    use fractal_daemon::store::assistant_write::{AssistantWriteAuthorization, authorize_grant};
    for principal in [AssistantPrincipal::Chatgpt, AssistantPrincipal::Mcp] {
    let h = Harness::new();
    let paired = h.pair_principal(principal.clone()).await;
    let grant = h.policy(&paired, scopes()).await;
    let authorization = AssistantWriteAuthorization {
        grant_subject: grant_subject(&paired), actor: paired.session_actor.clone(), owner: OWNER.into(),
        policy_revision: grant.policy_revision, scope: "write.assistant.messages".into(),
        recipient: Some(AGENT.into()),
    };
    let mut connection = rusqlite::Connection::open(h.root.path().join("store.sqlite")).unwrap();
    let transaction = connection.transaction().unwrap();
    authorize_grant(&transaction, "assistant-native", &authorization).unwrap();
    let mut wrong_actor = authorization.clone();
    wrong_actor.actor = "assistant/chatgpt/session/another-key".into();
    assert!(authorize_grant(&transaction, "assistant-native", &wrong_actor).is_err());
    assert!(authorize_grant(&transaction, "another-member", &authorization).is_err());
    let mut wrong_principal = authorization.clone();
    let other = if principal == AssistantPrincipal::Mcp { "assistant/chatgpt" } else { "assistant/mcp" };
    wrong_principal.actor = format!("{other}/session/{}", paired.grant.id);
    assert!(authorize_grant(&transaction, "assistant-native", &wrong_principal).is_err());
    let mut missing_scope = authorization.clone();
    missing_scope.scope = "write.assistant.unknown".into();
    assert!(authorize_grant(&transaction, "assistant-native", &missing_scope).is_err());
    for recipient in ["agent/example-missing", "daemon/runtime", OWNER] {
        let invalid_recipient = AssistantWriteAuthorization { recipient: Some(recipient.into()), ..authorization.clone() };
        assert!(authorize_grant(&transaction, "assistant-native", &invalid_recipient).is_err(), "{recipient}");
    }
    transaction.rollback().unwrap();
    h.policy(&paired, scopes()).await;
    let transaction = connection.transaction().unwrap();
    assert!(authorize_grant(&transaction, "assistant-native", &authorization).is_err(), "Authentication captured before a policy change cannot authorize the writer transaction");
    transaction.rollback().unwrap();
    let current = h.policy(&paired, scopes()).await;
    let current = AssistantWriteAuthorization { policy_revision: current.policy_revision, ..authorization };
    h.owner("POST", &grant_path(&paired, "/revoke"), &json!({})).await;
    let transaction = connection.transaction().unwrap();
    assert!(authorize_grant(&transaction, "assistant-native", &current).is_err(), "Revocation must be visible at the mutation transaction, not just HTTP ingress");
    transaction.rollback().unwrap();
    let expired = h.pair_principal(principal).await;
    let grant = h.policy(&expired, scopes()).await;
    let expired_authorization = AssistantWriteAuthorization {
        grant_subject: grant_subject(&expired), actor: expired.session_actor.clone(), owner: OWNER.into(),
        policy_revision: grant.policy_revision, scope: "write.assistant.messages".into(),
        recipient: Some(AGENT.into()),
    };
    h.expire(&expired);
    let transaction = connection.transaction().unwrap();
    assert!(authorize_grant(&transaction, "assistant-native", &expired_authorization).is_err());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronous_send_rechecks_revocation_and_scope_removal_before_commit() {
    let h = Arc::new(Harness::new());
    let paired = h.pair().await;
    let gate = h.root.path().join("mutation-gate");
    fractal_daemon::store::assistant_write_gate::install(&gate, Some(grant_subject(&paired))).unwrap();
    let connection = rusqlite::Connection::open(h.root.path().join("store.sqlite")).unwrap();
    let count_messages = || connection.query_row(
        "SELECT count(*) FROM claims WHERE kind='message.sent'", [], |row| row.get::<_, u64>(0),
    ).unwrap();
    for revoke in [false, true] {
        h.policy(&paired, scopes()).await;
        let before = count_messages();
        let request = h.message(AGENT, if revoke { "held-revoke" } else { "held-scope" }).await;
        for name in ["hold", "entered", "release", "done"] {
            let path = gate.join(name);
            if path.exists() { std::fs::remove_file(path).unwrap(); }
        }
        std::fs::write(gate.join("hold"), b"hold").unwrap();
        let send_h = h.clone();
        let send_paired = paired.clone();
        let send = tokio::spawn(async move {
            send_h.assistant(&send_paired, "POST", "/v1/client/assistant/propose", &request).await
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !gate.join("entered").exists() {
            assert!(std::time::Instant::now() < deadline, "send did not reach the pre-commit gate");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!send.is_finished(), "synchronous send must still await its mutation");
        if revoke {
            h.owner("POST", &grant_path(&paired, "/revoke"), &json!({})).await;
        } else {
            h.owner("POST", &grant_path(&paired, "/policy"), &AssistantGrantPolicy {
                api_version: fractal_client::API_VERSION.into(),
                scopes: scopes().into_iter().filter(|scope| *scope != AssistantScope::Messages).collect(),
                expires_in_seconds: Some(3600), disclosure_consent: true,
                consent_evidence: "Owner removes message scope during held send".into(),
                minimal_read_mode: AssistantMinimalReadMode::CallerBoundAllow,
                recipient_allowlist: Some(vec![]),
            }).await;
        }
        std::fs::remove_file(gate.join("hold")).unwrap();
        std::fs::write(gate.join("release"), b"release").unwrap();
        let (status, denial) = send.await.unwrap();
        assert!(!status.is_success(), "{denial}");
        assert_eq!(denial["code"], "forbidden", "{denial}");
        assert_eq!(count_messages(), before, "denied send must append no message claim");
    }
}

#[tokio::test]
async fn assistant_provenance_generic_and_native_forgery_is_refused() {
    let h = Harness::new();
    let paired = h.pair().await;
    for (index, (actor, extra)) in [
        ("assistant/chatgpt", json!({})),
        (OWNER, json!({"from":"assistant/chatgpt"})),
        (OWNER, json!({"sender_class":"external-assistant"})),
        (OWNER, json!({"assistant_grant_id":paired.grant.id})),
        (OWNER, json!({"assistant_owner":OWNER})),
        (OWNER, json!({"assistant_session_actor":paired.session_actor})),
    ].into_iter().enumerate() {
        let mut fields = json!({"from":OWNER,"to":AGENT,"content":"forged","status":"sent"});
        fields.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let (status, error) = request(&h.local, "POST", "/v1/claims", None, Some(OWNER),
            &json!({"subject":format!("message/assistant-forged-{index}"),"kind":"message.sent","actor":actor,"fields":fields})).await;
        assert!(!status.is_success(), "{error}");
        assert_eq!(error["code"], "assistant-send-refused", "{error}");
    }
    let (status, _) = request(&h.local, "POST", "/v1/messages", None, Some(OWNER),
        &json!({"idempotency_key":"assistant-native-forged","from":"assistant/chatgpt","to":AGENT,"content":"forged"})).await;
    assert!(!status.is_success());
    for field in ["from", "actor", "sender_class", "assistant_grant_id", "assistant_owner", "assistant_session_actor"] {
        let mut message = serde_json::to_value(h.message(AGENT, field).await).unwrap();
        message["parameters"][field] = json!("forged");
        let (status, _) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert!(h.state.store.messages(Some(AGENT), true).unwrap().is_empty());
}

#[tokio::test]
async fn assistant_cannot_complete_authored_owner_step_but_owner_can() {
    let h = Harness::new();
    let source = format!("version 2\nagent \"example-worker.local\" {{ workspace {:?}; command \"true\"; restart always; }}\nmission \"authored-review\" state=\"ready\" {{ goal \"Review\"; step \"review\" {{ assigned-to \"person/robin\"; goal \"Review release\"; }} }}", h.root.path());
    h.state.store.apply_internal(&fractal_daemon::graph::parse_intent(&source, "assistant-native").unwrap(), "authored-review").unwrap();
    let run = h.state.store.create_mission_run(&fractal_daemon::model::MissionRunRequest {
        mission: "authored-review".into(), revision: None, workspace: h.root.path().to_string_lossy().into_owned(),
        requester: Some(OWNER.into()), mode: Some("run".into()), inputs: Default::default(), idempotency_key: "authored-review".into(),
    }).unwrap();
    h.state.store.set_step_state(&run.steps[0].subject, "ready", Some("test activation")).unwrap();
    let step = h.state.store.step_run(&run.steps[0].subject).unwrap().unwrap();
    assert_eq!(step.status, "ready");
    let paired = h.pair().await;
    let action = ActionRequest::work_done("action/authored", "native-test-authored-answer", h.fence().await,
        PersonStepParameters { target_id: step.subject.clone(), summary: "Reviewed".into(),
            episode: format!("{}:{}:{}", step.generation, step.attempt, step.readiness_epoch),
            ..Default::default() }).unwrap();
    let (status, denied) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &action).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert!(h.state.store.step_run(&step.subject).unwrap().unwrap().person_answers.is_empty());
    h.owner("POST", "/v1/client/actions", &action).await;
    assert_eq!(h.state.store.step_run(&step.subject).unwrap().unwrap().status, "completed");
}

#[tokio::test]
async fn assistant_message_recipient_must_be_a_currently_declared_agent() {
    let h = Harness::new();
    let paired = h.pair().await;
    for (index, recipient) in ["agent/example-missing", "daemon/runtime", "person/robin"].iter().enumerate() {
        let message = h.message(recipient, &format!("invalid-recipient-{index}")).await;
        let (status, denied) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
        assert!(!status.is_success(), "{recipient}: {denied}");
        assert!(h.state.store.messages(Some(recipient), true).unwrap().is_empty());
    }
}

#[tokio::test]
async fn mcp_grant_reads_sends_and_answers_with_its_own_principal_then_revokes() {
    let h = Harness::new();
    let ask = h.ask("mcp-answer", "Choose through MCP");
    let paired = h.pair_principal(AssistantPrincipal::Mcp).await;
    assert_eq!(paired.grant.principal, AssistantPrincipal::Mcp);
    assert!(paired.session_actor.starts_with("assistant/mcp/session/"));
    for projection in ["summary", "attention", "agents", "missions"] {
        let (status, value) = h.assistant(&paired, "GET",
            &format!("/v1/client/assistant/{projection}"), &()).await;
        assert_eq!(status, StatusCode::OK, "{projection}: {value}");
    }
    let message = h.message(AGENT, "mcp-send").await;
    let (status, result) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let subject = result["value"]["affected_ids"].as_array().unwrap().iter()
        .filter_map(Value::as_str).find(|id| id.starts_with("message/")).unwrap();
    let claim = h.state.store.latest_claim(subject, Some("message.sent")).unwrap().unwrap();
    assert_eq!(claim.actor.as_deref(), Some("assistant/mcp"));
    assert_eq!(claim.body["fields"]["from"], "assistant/mcp");
    assert_eq!(claim.body["fields"]["sender_class"], "external-assistant");
    assert_eq!(claim.body["fields"]["assistant_grant_id"], paired.grant.id);
    assert_eq!(claim.body["fields"]["assistant_owner"], OWNER);
    assert_eq!(claim.body["fields"]["assistant_session_actor"], paired.session_actor);
    let stored = h.state.store.message(subject).unwrap().unwrap();
    let projected = serde_json::to_value(&stored).unwrap();
    assert_eq!(projected["from"], "assistant/mcp");
    assert_eq!(projected["sender_class"], "external-assistant");
    assert_eq!(projected["assistant_grant_id"], paired.grant.id);
    let metadata = stored.assistant_metadata().unwrap();
    let notification = fractal_drivers::ding::fractal_notification_with_metadata(
        &stored.subject, &stored.from, &stored.to, stored.title.as_deref(), &stored.content,
        &"0".repeat(64), fractal_drivers::ding::NotificationMetadata {
            attachments: &[], assistant: Some(&metadata),
        },
    );
    assert!(notification.contains("from=\"assistant/mcp\""));
    assert!(notification.contains(fractal_drivers::ding::EXTERNAL_ASSISTANT_WARNING));
    assert!(!notification.contains(fractal_drivers::ding::PERSON_REPLY_INSTRUCTION));
    let answer = h.answer(&ask, "yes", "mcp-answer").await;
    let (status, result) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &answer).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let durable = h.state.store.step_run(&ask).unwrap().unwrap();
    assert_eq!(durable.person_answers[0].respondent, "assistant/mcp");
    assert_eq!(durable.person_answers[0].sender_class.as_deref(), Some("external-assistant"));
    assert_eq!(durable.person_answers[0].assistant_grant_id.as_deref(), Some(paired.grant.id.as_str()));
    let claim = h.state.store.latest_claim(&ask, Some("work.person-done")).unwrap().unwrap();
    assert_eq!(claim.actor.as_deref(), Some("assistant/mcp"));
    h.owner("POST", &grant_path(&paired, "/revoke"), &json!({})).await;
    for projection in ["summary", "attention", "agents", "missions"] {
        let (status, denial) = h.assistant(&paired, "GET",
            &format!("/v1/client/assistant/{projection}"), &()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{denial}");
    }
    let message = h.message(AGENT, "mcp-revoked-send").await;
    let (status, denial) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denial}");
}

#[tokio::test]
async fn mcp_principal_cannot_be_forged_or_override_a_chatgpt_grant() {
    let h = Harness::new();
    for principal in [AssistantPrincipal::Chatgpt, AssistantPrincipal::Mcp] {
        let paired = h.pair_principal(principal.clone()).await;
        let expected = match principal {
            AssistantPrincipal::Chatgpt => "assistant/chatgpt",
            AssistantPrincipal::Mcp => "assistant/mcp",
        };
        assert!(paired.session_actor.starts_with(&format!("{expected}/session/")));
        for field in ["principal", "from", "actor", "assistant_session_actor"] {
            let mut message = serde_json::to_value(h.message(AGENT,
                &format!("principal-forgery-{principal:?}-{field}")).await).unwrap();
            message["parameters"][field] = json!(if principal == AssistantPrincipal::Mcp {
                "assistant/chatgpt"
            } else {
                "assistant/mcp"
            });
            let (status, denial) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{denial}");
        }
    }
    let (status, denial) = request(&h.local, "POST", "/v1/messages", None, Some(OWNER),
        &json!({"idempotency_key":"mcp-native-forged","from":"assistant/mcp","to":AGENT,"content":"forged"})).await;
    assert!(!status.is_success(), "{denial}");
    let (status, denial) = request(&h.local, "POST", "/v1/claims", None, Some(OWNER),
        &json!({"subject":"message/mcp-forged","kind":"message.sent","actor":"assistant/mcp",
            "fields":{"from":"assistant/mcp","to":AGENT,"content":"forged","status":"sent"}})).await;
    assert!(!status.is_success(), "{denial}");
    assert_eq!(denial["code"], "assistant-send-refused");
    assert!(h.state.store.messages(Some(AGENT), true).unwrap().is_empty());
}

#[tokio::test]
async fn read_only_mcp_grant_cannot_send_or_answer_and_appends_no_mutation() {
    let h = Harness::new();
    let ask = h.ask("mcp-read-only-answer", "Read only must not answer");
    let paired = h.pair_principal(AssistantPrincipal::Mcp).await;
    h.policy(&paired, scopes().into_iter().filter(|scope|
        *scope != AssistantScope::Messages && *scope != AssistantScope::Answers).collect()).await;
    let message = h.message(AGENT, "mcp-read-only-send").await;
    let answer = h.answer(&ask, "yes", "mcp-read-only-answer").await;
    h.state.assistant_receipts.flush().await.unwrap();
    let before = h.state.store.index().unwrap();
    for action in [message, answer] {
        let (status, denial) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &action).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{denial}");
        assert_eq!(h.state.store.index().unwrap(), before, "Denied read-only writes must not append claims");
    }
    assert!(h.state.store.messages(Some(AGENT), true).unwrap().is_empty());
    assert!(h.state.store.step_run(&ask).unwrap().unwrap().person_answers.is_empty());
    assert!(h.state.store.latest_claim(&ask, Some("work.person-done")).unwrap().is_none());
}

#[tokio::test]
async fn mcp_canonical_replies_are_recipient_scoped_bounded_and_survive_sender_restart() {
    let h = Harness::new();
    let paired = h.pair().await;
    let other = h.pair().await;
    let foreign = h.pair_principal(AssistantPrincipal::Mcp).await;
    h.state.store.append_claim(&ClaimInput {
        subject: AGENT.into(), kind: "runtime.observed".into(), actor: Some(AGENT.into()),
        fields: serde_json::from_value(json!({"status":"running","runtime_id":"original-reply-runtime",
            "incarnation_id":"original-reply-runtime:i1","terminal":false})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    // Empty, system-only gate histories occupy the first native conversation page.
    // Recipient reply discovery must not scan only that page to find canonical ACKs.
    for index in 0..12 {
        let agent = format!("agent/aaa-gate-{index:02}");
        h.state.store.append_claim(&ClaimInput {
            subject: agent.clone(), kind: "runtime.observed".into(), actor: Some(agent),
            fields: serde_json::from_value(json!({"status":"waiting",
                "runtime_id":format!("gate-{index:02}"),
                "incarnation_id":format!("gate-{index:02}:i1"),"terminal":false})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    let send = h.message(AGENT, "canonical-reply-send").await;
    let (status, sent) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &send).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let original = sent["value"]["affected_ids"].as_array().unwrap().iter()
        .filter_map(Value::as_str).find(|id| id.starts_with("message/")).unwrap();
    let ack = format!("ACK-canonical-{}", uuid::Uuid::new_v4());
    let text = format!("{ack}\n{}", "Canonical agent reply. é🚀 ".repeat(900));
    assert!(text.len() > 16 * 1024);
    let reply = h.reply("assistant/chatgpt", "canonical-reply", &text, Some(original)).await;
    let foreign_reply = h.reply("assistant/mcp", "foreign-canonical-reply", "FOREIGN-REPLY-SECRET", None).await;
    let (app, server) = h.mcp(&paired).await;
    let (other_app, other_server) = h.mcp(&other).await;
    let (foreign_app, foreign_server) = h.mcp(&foreign).await;
    let (error, listed) = mcp_call(&app, "fractal_conversations_list", json!({"replies":true})).await;
    assert!(!error, "{listed}");
    for secret in [ack.as_str(), "Canonical agent reply.", "FOREIGN-REPLY-SECRET"] {
        assert!(!listed.to_string().contains(secret));
    }
    let items = listed["value"]["items"].as_array().unwrap();
    assert!(items.iter().all(|item| item["to"] == "assistant/chatgpt"
        && item.get("text").is_none() && item.get("content").is_none()));
    assert!(!items.iter().any(|item| item["message_id"] == foreign_reply.subject));
    let item = items.iter().find(|item| item["message_id"] == reply.subject).unwrap();
    assert_eq!(item["agent_id"], AGENT);
    assert_eq!(item["from"], AGENT);
    assert_eq!(item["reply_to"], original);
    assert!(!item["sent_at"].is_null());
    let handle = item["read"].clone();
    assert_eq!(handle["session_id"], "session/assistant-replies");
    assert_eq!(handle["revision"], reply.id);
    assert_eq!(handle["start"], 0);
    assert!(handle["end"].as_u64().unwrap() <= 512);
    let (error, content) = mcp_call(&app, "fractal_conversation_read", handle.clone()).await;
    assert!(!error, "{content}");
    assert!(content["value"]["text"].as_str().unwrap().starts_with(&ack), "{content}");
    for key in ["agent_id", "session_id", "reference", "revision", "start", "end"] {
        assert_eq!(content["value"][key], handle[key], "{key}");
    }
    let mut complete = content["value"]["text"].as_str().unwrap().to_owned();
    let mut chunk = content;
    while !chunk["value"]["next_range"].is_null() {
        let next = &chunk["value"]["next_range"];
        assert_eq!(next["start"], chunk["value"]["end"]);
        assert!(next["end"].as_u64().unwrap() > next["start"].as_u64().unwrap());
        let mut continuation = handle.clone();
        continuation["start"] = next["start"].clone();
        continuation["end"] = next["end"].clone();
        let (error, continued) = mcp_call(&app, "fractal_conversation_read", continuation).await;
        assert!(!error, "{continued}");
        assert_eq!(continued["value"]["reference"], handle["reference"]);
        assert_eq!(continued["value"]["revision"], reply.id);
        complete.push_str(continued["value"]["text"].as_str().unwrap());
        assert!(complete.len() <= text.len(), "continuation must make bounded progress");
        chunk = continued;
    }
    assert_eq!(complete, text);
    for denied_app in [&other_app, &foreign_app] {
        let (error, denied) = mcp_call(denied_app, "fractal_conversation_read", handle.clone()).await;
        assert!(error, "{denied}");
        assert!(!denied.to_string().contains(&ack));
    }
    let (error, foreign_list) = mcp_call(&foreign_app, "fractal_conversations_list", json!({"replies":true})).await;
    assert!(!error, "{foreign_list}");
    assert!(foreign_list["value"]["items"].as_array().unwrap().iter()
        .all(|item| item["to"] == "assistant/mcp" && item["message_id"] != reply.subject));
    for (field, value) in [
        ("reference", json!(foreign_reply.subject)),
        ("reference", json!(format!("{}x", handle["reference"].as_str().unwrap()))),
        ("revision", json!(foreign_reply.id)),
        ("agent_id", json!("agent/foreign-worker.local")),
    ] {
        let mut tampered = handle.clone();
        tampered[field] = value;
        let (error, denied) = mcp_call(&app, "fractal_conversation_read", tampered).await;
        assert!(error, "{field}: {denied}");
        assert!(!denied.to_string().contains(&ack));
        assert!(!denied.to_string().contains("FOREIGN-REPLY-SECRET"));
    }
    for selector in ["recipient", "person", "principal"] {
        let mut arguments = json!({"replies":true});
        arguments[selector] = json!("assistant/mcp");
        let (error, denied) = mcp_call(&app, "fractal_conversations_list", arguments).await;
        assert!(error, "{selector}: {denied}");
    }
    let (error, tiny) = mcp_call(&app, "fractal_conversations_list",
        json!({"replies":true,"max_response_bytes":1})).await;
    assert!(error, "a nonempty discovery page cannot fit a one-byte budget: {tiny}");
    assert!(!tiny.to_string().contains(&ack));
    let (error, bounded) = mcp_call(&app, "fractal_conversations_list",
        json!({"replies":true,"limit":1000000})).await;
    assert!(error, "oversized reply pages must be refused: {bounded}");
    assert!(!bounded.to_string().contains(&ack));
    h.state.store.append_claim(&ClaimInput {
        subject: AGENT.into(), kind: "runtime.observed".into(), actor: Some(AGENT.into()),
        fields: serde_json::from_value(json!({"status":"running","runtime_id":"restarted-reply-runtime",
            "incarnation_id":"restarted-reply-runtime:i2","terminal":false})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let (error, restarted) = mcp_call(&app, "fractal_conversation_read", handle.clone()).await;
    assert!(!error, "{restarted}");
    assert!(restarted["value"]["text"].as_str().unwrap().starts_with(&ack));
    assert_eq!(restarted["value"]["revision"], reply.id);
    h.policy(&paired, scopes()).await;
    let (error, denied) = mcp_call(&app, "fractal_conversation_read", handle.clone()).await;
    assert!(error, "old policy handle: {denied}");
    let (error, refreshed) = mcp_call(&app, "fractal_conversations_list", json!({"replies":true})).await;
    assert!(!error, "{refreshed}");
    let refreshed_handle = refreshed["value"]["items"].as_array().unwrap().iter()
        .find(|item| item["message_id"] == reply.subject).unwrap()["read"].clone();
    let (error, other_list) = mcp_call(&other_app, "fractal_conversations_list", json!({"replies":true})).await;
    assert!(!error, "{other_list}");
    let other_handle = other_list["value"]["items"].as_array().unwrap().iter()
        .find(|item| item["message_id"] == reply.subject).unwrap()["read"].clone();
    let (error, other_content) = mcp_call(&other_app, "fractal_conversation_read", other_handle.clone()).await;
    assert!(!error, "{other_content}");
    h.owner("POST", &grant_path(&paired, "/revoke"), &json!({})).await;
    h.expire(&other);
    for (denied_app, replay) in [(&app, refreshed_handle), (&other_app, other_handle)] {
        for (tool, arguments) in [
            ("fractal_conversations_list", json!({"replies":true})),
            ("fractal_conversation_read", replay),
        ] {
            let (error, denied) = mcp_call(denied_app, tool, arguments).await;
            assert!(error, "{tool}: {denied}");
            assert!(!denied.to_string().contains(&ack));
        }
    }
    server.abort();
    other_server.abort();
    foreign_server.abort();
}

#[tokio::test]
async fn signed_old_reader_ack_upgrades_through_normal_admission_into_mcp_readback() {
    use fractal_daemon::model::ReplicationInventory;
    const FLEET: &str = "3c9a1f2e-8b7d-4e6c-a5f4-1d2e3c4b5a69";
    let source = Store::open_memory("reply-source").unwrap();
    source.bind_fleet(FLEET).unwrap();
    let key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
    source.set_member_key(Some(key.clone())).unwrap();
    let ack = format!("ACK-signed-upgrade-{}", uuid::Uuid::new_v4());
    let claim = source.append_claim(&ClaimInput {
        subject: "message/signed-upgrade-ack".into(), kind: "message.sent".into(), actor: Some(AGENT.into()),
        fields: serde_json::from_value(json!({"from":AGENT,"to":"assistant/chatgpt",
            "content":ack,"status":"sent"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let mut exchange = source.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap();
    let position = exchange.envelopes.iter().position(|envelope| {
        let payload: smallclaims::claim::ReplicaEnvelopePayload =
            ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
        payload.batch.claims.iter().any(|candidate| candidate.id == claim.id)
    }).unwrap();
    let original = exchange.envelopes.remove(position);
    let mut ack_exchange = exchange.clone();
    ack_exchange.envelopes = vec![original.clone()];
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("store.sqlite");
    let older = Store::open(&path, "assistant-native").unwrap();
    older.bind_fleet(FLEET).unwrap();
    let mut registry = fractal_schema::registry().clone();
    registry.subjects.remove("assistant");
    older.set_claim_registry(registry);
    older.set_member_key(Some(key.clone())).unwrap();
    older.pin_fleet_anchor(key.public()).unwrap();
    older.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    older.append_claim(&ClaimInput {
        subject: "host/reply-source".into(), kind: "fleet.member-admitted".into(), actor: None,
        fields: serde_json::from_value(json!({"fleet_id":FLEET,"member_key":key.public(),
            "via":"invite","mode":"listening"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    older.receive_replication_exchange("reply-source", FLEET, &exchange).unwrap();
    let bootstrap = older.validate_replication_backlog().unwrap();
    assert_eq!((bootstrap.invalid, bootstrap.held), (0, 0));
    assert!(matches!(older.fleet_membership().unwrap().window("reply-source", original.sequence),
        smallclaims::fleet::Window::Keyed(_)));
    older.receive_replication_exchange("reply-source", FLEET, &ack_exchange).unwrap();
    let rejected = older.validate_replication_backlog().unwrap();
    assert_eq!((rejected.valid, rejected.invalid, rejected.held), (0, 1, 0));
    let records = older.replica_records(true).unwrap();
    assert_eq!(records.len(), 1);
    assert!(records[0].error_message.as_deref().unwrap().contains("violates invalid-subject-reference:"));
    assert!(older.message(&claim.subject).unwrap().is_none());
    drop(older);
    // Reopening with the current registry is the upgrade: no repair, resend, DB edit or migration.
    let upgraded = Arc::new(Store::open(&path, "assistant-native").unwrap());
    let admitted = upgraded.validate_replication_backlog().unwrap();
    assert_eq!((admitted.valid, admitted.invalid, admitted.held), (1, 0, 0));
    upgraded.project_replication_backlog().unwrap();
    let recovered = upgraded.latest_claim(&claim.subject, Some("message.sent")).unwrap().unwrap();
    assert_eq!(recovered.id, claim.id);
    assert_eq!(recovered.batch_id, claim.batch_id);
    assert_eq!(recovered.origin, claim.origin);
    assert_eq!(recovered.actor, claim.actor);
    assert_eq!(recovered.body, claim.body);
    assert_eq!(recovered.predecessors, claim.predecessors);
    assert_eq!(recovered.accepted_at_unix_ms, claim.accepted_at_unix_ms);
    let relay = upgraded.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap();
    let retained = relay.envelopes.iter().find(|envelope| envelope.writer == original.writer
        && envelope.sequence == original.sequence && envelope.hash == original.hash).unwrap();
    assert_eq!(retained.payload.bytes().unwrap(), original.payload.bytes().unwrap());
    assert_eq!(retained.previous_hash, original.previous_hash);
    assert_eq!(retained.member_key, original.member_key);
    assert_eq!(retained.signature, original.signature);
    assert!(retained.signature.is_some());
    assert_eq!(retained.accepted_at_unix_ms, original.accepted_at_unix_ms);
    let h = Harness::with_store(root, upgraded);
    let paired = h.pair().await;
    let (app, server) = h.mcp(&paired).await;
    let (error, listed) = mcp_call(&app, "fractal_conversations_list", json!({"replies":true})).await;
    assert!(!error, "{listed}");
    assert!(!listed.to_string().contains(&ack));
    let item = listed["value"]["items"].as_array().unwrap().iter()
        .find(|item| item["message_id"] == claim.subject).unwrap();
    assert_eq!(item["from"], AGENT);
    assert_eq!(item["to"], "assistant/chatgpt");
    assert_eq!(item["read"]["revision"], claim.id);
    let (error, content) = mcp_call(&app, "fractal_conversation_read", item["read"].clone()).await;
    assert!(!error, "{content}");
    assert_eq!(content["value"]["text"], ack);
    assert_eq!(content["value"]["revision"], claim.id);
    let converged = h.state.store.validate_replication_backlog().unwrap();
    assert_eq!((converged.valid, converged.invalid, converged.held), (0, 0, 0));
    server.abort();
}
