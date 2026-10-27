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

const OWNER: &str = "person/ada";
const AGENT: &str = "agent/native.worker";
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
        let source = format!("version 2\nagent \"native.worker\" {{ workspace {:?}; command \"true\"; restart always; }}\nmission \"native-mission\" state=\"ready\" {{ goal \"Native integration mission\"; }}", root.path());
        store.apply_internal(&fractal_daemon::graph::parse_intent(&source, "assistant-native").unwrap(), "native-seed").unwrap();
        // There is no daemon task in this harness. Register its production publication
        // surfaces; pair() explicitly refreshes them before reading the summary.
        store.start_attention_list_refresher().unwrap();
        store.start_agent_roster_refresher().unwrap();
        let state = AppState {
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
        let begin = AssistantPairingBegin {
            api_version: fractal_client::API_VERSION.into(), device_name: "Native ChatGPT test".into(),
            assistant_id: AssistantPrincipal::Chatgpt, scopes: scopes(), expires_in_seconds: Some(3600),
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
        assert_eq!(paired.principal, AssistantPrincipal::Chatgpt);
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
    use fractal_daemon::store::assistant_write::{AssistantWriteAuthorization, authorize};
    let h = Harness::new();
    let paired = h.pair().await;
    let grant = h.policy(&paired, scopes()).await;
    let authorization = AssistantWriteAuthorization {
        grant_subject: grant_subject(&paired), actor: paired.session_actor.clone(), owner: OWNER.into(),
        policy_revision: grant.policy_revision, scope: "write.assistant.messages".into(),
        recipient: Some(AGENT.into()),
    };
    let mut connection = rusqlite::Connection::open(h.root.path().join("store.sqlite")).unwrap();
    let transaction = connection.transaction().unwrap();
    authorize(&transaction, "assistant-native", &authorization).unwrap();
    let mut wrong_actor = authorization.clone();
    wrong_actor.actor = "assistant/chatgpt/session/another-key".into();
    assert!(authorize(&transaction, "assistant-native", &wrong_actor).is_err());
    assert!(authorize(&transaction, "another-member", &authorization).is_err());
    let mut missing_scope = authorization.clone();
    missing_scope.scope = "write.assistant.unknown".into();
    assert!(authorize(&transaction, "assistant-native", &missing_scope).is_err());
    for recipient in ["agent/does-not-exist", "daemon/runtime", OWNER] {
        let invalid_recipient = AssistantWriteAuthorization { recipient: Some(recipient.into()), ..authorization.clone() };
        assert!(authorize(&transaction, "assistant-native", &invalid_recipient).is_err(), "{recipient}");
    }
    transaction.rollback().unwrap();
    h.policy(&paired, scopes()).await;
    let transaction = connection.transaction().unwrap();
    assert!(authorize(&transaction, "assistant-native", &authorization).is_err(), "Authentication captured before a policy change cannot authorize the writer transaction");
    transaction.rollback().unwrap();
    let current = h.policy(&paired, scopes()).await;
    let current = AssistantWriteAuthorization { policy_revision: current.policy_revision, ..authorization };
    h.owner("POST", &grant_path(&paired, "/revoke"), &json!({})).await;
    let transaction = connection.transaction().unwrap();
    assert!(authorize(&transaction, "assistant-native", &current).is_err(), "Revocation must be visible at the mutation transaction, not just HTTP ingress");
    transaction.rollback().unwrap();
    let expired = h.pair().await;
    let grant = h.policy(&expired, scopes()).await;
    let expired_authorization = AssistantWriteAuthorization {
        grant_subject: grant_subject(&expired), actor: expired.session_actor.clone(), owner: OWNER.into(),
        policy_revision: grant.policy_revision, scope: "write.assistant.messages".into(),
        recipient: Some(AGENT.into()),
    };
    h.expire(&expired);
    let transaction = connection.transaction().unwrap();
    assert!(authorize(&transaction, "assistant-native", &expired_authorization).is_err());
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
    let source = format!("version 2\nagent \"native.worker\" {{ workspace {:?}; command \"true\"; restart always; }}\nmission \"authored-review\" state=\"ready\" {{ goal \"Review\"; step \"review\" {{ assigned-to \"person/ada\"; goal \"Review release\"; }} }}", h.root.path());
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
    for (index, recipient) in ["agent/does-not-exist", "daemon/runtime", "person/ada"].iter().enumerate() {
        let message = h.message(recipient, &format!("invalid-recipient-{index}")).await;
        let (status, denied) = h.assistant(&paired, "POST", "/v1/client/assistant/propose", &message).await;
        assert!(!status.is_success(), "{recipient}: {denied}");
        assert!(h.state.store.messages(Some(recipient), true).unwrap().is_empty());
    }
}
