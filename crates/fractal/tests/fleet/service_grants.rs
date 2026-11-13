//! These claims are fixtures, not a service-issuance path. Production issuance stays disabled.
use super::*;
use sha2::{Digest as _, Sha256};

const GRANTED: &str = "custom.client.assistant.v1.granted";
const POLICY: &str = "custom.client.assistant.v1.policy";
const REVOKED: &str = "custom.client.assistant.v1.revoked";
const PLUGIN: &str = "plugin/issuer/downgrade-test";

struct Grant {
    id: String,
    credential: String,
    claims: BTreeSet<String>,
}

fn append(node: &Node, id: &str, kind: &str, fields: Value, previous: Option<&str>) -> String {
    assert!(node.daemon.is_none() && node.worker.is_none(), "seed only a stopped node");
    let store = fractal_daemon::store::Store::open(
        &node.state_dir().join("claims.sqlite3"), &node.name,
    ).unwrap();
    // Legacy-config nodes have no membership member key: use their existing standalone
    // node key and principal key directory, exactly as daemon startup does.
    store.set_node_key(std::sync::Arc::new(
        fractal_daemon::fleet::join::standalone_node_key(&node.state_dir()).unwrap(),
    )).unwrap();
    store.use_key_directory(&fractal_daemon::fleet::join::key_directory(&node.state_dir())).unwrap();
    store.ensure_principal_key(PERSON).unwrap();
    let claim = store.append_claim(&ClaimInput {
        subject: format!("custom/client/assistant-v1-{id}"),
        kind: kind.into(), actor: Some(PERSON.into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: previous.into_iter().map(str::to_owned).collect(),
        expected_subject: Some(previous.map(str::to_owned)), idempotency_key: None,
    }).unwrap();
    store.seal_local_batches().unwrap();
    assert_eq!(claim.origin, node.name);
    assert!(store.claim_signature(&claim.id).unwrap().is_some(), "fixture must be signed");
    claim.id
}

fn seed(node: &Node, number: u8, profile: &str, principal: &str, scope: &str,
        expired: bool, revoked: bool) -> Grant {
    let id = format!("{number:024x}");
    let credential = format!("downgrade-fixture-{}-{id}", node.name);
    let mut fields = json!({
        "version": 1, "profile": profile, "grant_id": id,
        "principal": principal, "issuer": PERSON,
        "session_actor": format!("{principal}/session/{id}"),
        "credential_hash": hex::encode(Sha256::digest(credential.as_bytes())),
        "device_id": format!("client/downgrade-{id}"), "device_name": "downgrade fixture",
        "key_id": "downgrade-fixture", "scopes": [scope], "policy_revision": 1,
        "expires_at_unix_ms": if expired { 1_u64 } else { 4_102_444_800_000_u64 },
        "minimal_read_mode": "approval-once", "disclosure_consent": true,
        "recipient_allowlist": [], "consent_evidence": []
    });
    let granted = append(node, &id, GRANTED, fields.clone(), None);
    fields["policy_revision"] = json!(2);
    let policy = append(node, &id, POLICY, fields, Some(&granted));
    let mut claims = BTreeSet::from([granted, policy.clone()]);
    if revoked {
        claims.insert(append(node, &id, REVOKED, json!({}), Some(&policy)));
    }
    Grant { id, credential, claims }
}

// Malformed JSON reaches the proposal extractor only after credential authentication and
// route admission. A 400 is therefore an auth-positive control without reading private data,
// requiring a signed per-read approval, or creating a proposal. A refusal must be exactly 403,
// not an unavailable daemon, missing route, bad request, or transport failure.
async fn auth_response(node: &Node, grant: &Grant) -> (u16, String) {
    let response = reqwest::Client::builder().unix_socket(node.socket()).build().unwrap()
        .post("http://localhost/v1/client/assistant/propose")
        .bearer_auth(&grant.credential).header("content-type", "application/json")
        .body("{").send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    let _: Value = node.client().get("/v1/health").await.unwrap();
    (status.as_u16(), body)
}

async fn assert_auth(node: &Node, grant: &Grant, accepted: bool) {
    let (status, body) = auth_response(node, grant).await;
    assert_eq!(status, if accepted { 400 } else { 403 },
        "{} grant {}: {body}\n{}", node.name, grant.id, node.logs());
}

async fn assert_retained(node: &Node, grants: &[Grant]) {
    let ids: BTreeSet<String> = node.claims().await.into_iter()
        .map(|claim| claim["id"].as_str().unwrap().to_owned()).collect();
    for grant in grants {
        assert!(grant.claims.is_subset(&ids), "{} lost grant {}", node.name, grant.id);
    }
}

async fn wait_retained(node: &Node, grants: &[Grant]) {
    wait_until("the peer retains every grant and policy claim", 90, || async {
        let ids: BTreeSet<String> = node.claims().await.into_iter()
            .map(|claim| claim["id"].as_str().unwrap().to_owned()).collect();
        grants.iter().all(|grant| grant.claims.is_subset(&ids))
    }).await;
}

async fn fixtures(node: &mut Node, counterexample: bool) -> Vec<Grant> {
    node.stop();
    // Public /v1/claims deliberately forbids these authority kinds and subjects.
    // Fixture-only offline writes preserve that production restriction. Even the
    // baseline-origin case is seeded by the candidate store, never old-binary issuance.
    let mut grants = vec![
        seed(node, 1, "service", PLUGIN, "read.service.status", false, false),
        seed(node, 2, "service", PLUGIN, "read.assistant.summary", false, false),
        seed(node, 3, "service", "assistant/chatgpt", "read.service.status", false, false),
        seed(node, 4, "assistant", "assistant/chatgpt", "read.assistant.summary", false, false),
        seed(node, 5, "assistant", "assistant/chatgpt", "read.assistant.summary", true, false),
        seed(node, 6, "assistant", "assistant/chatgpt", "read.assistant.summary", false, true),
    ];
    if counterexample {
        grants.push(seed(node, 7, "service", "assistant/chatgpt", "read.assistant.summary", false, false));
    }
    node.start().await;
    grants
}

async fn mixed_pair(old: Option<PathBuf>, old_issuer: bool) {
    let root = tempfile::tempdir().unwrap();
    let secret = root.path().join("fleet.secret");
    fs::write(&secret, hex::encode([7_u8; 32])).unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    let mut issuer = Node::new(root.path(), "issuer");
    let mut peer = Node::new(root.path(), "peer");
    if let Some(old) = old {
        if old_issuer { issuer.binary = old; } else { peer.binary = old; }
    }
    let fleet = "542c372d-0a13-4217-9817-4e868f45a734";
    issuer.legacy_config(fleet, &secret, &[("peer", peer.port)]);
    peer.legacy_config(fleet, &secret, &[("issuer", issuer.port)]);
    issuer.start().await;
    peer.start().await;
    let grants = fixtures(&mut issuer, false).await;
    wait_retained(&peer, &grants).await;
    for (index, grant) in grants.iter().enumerate() {
        assert_auth(&issuer, grant, index == 3).await;
        // Even the assistant control is refused off-origin: replication is not authority.
        assert_auth(&peer, grant, false).await;
    }
    assert_retained(&issuer, &grants).await;
    assert_retained(&peer, &grants).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: the pinned deployed source"]
async fn previous_release_refuses_service_grants_from_a_candidate_issuer() {
    if fractal_daemon::test_support::supervise_test() { return; }
    mixed_pair(Some(compat_binary()), false).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: the pinned deployed source"]
async fn previous_release_refuses_offline_seeded_service_grants_with_its_own_origin() {
    if fractal_daemon::test_support::supervise_test() { return; }
    mixed_pair(Some(compat_binary()), true).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: the pinned deployed source"]
async fn previous_release_refuses_plugin_service_grants_on_same_database_rollback() {
    if fractal_daemon::test_support::supervise_test() { return; }
    let old = PathBuf::from(std::env::var("ST3_COMPAT_BIN").expect("ST3_COMPAT_BIN names the pinned baseline"));
    let root = tempfile::tempdir().unwrap();
    let mut node = Node::new(root.path(), "issuer");
    node.start().await;
    assert_candidate_issuance_grammar(&node).await;
    // Includes the unissuable profile-blind counterexample; candidate must refuse it.
    let grants = fixtures(&mut node, true).await;
    for (index, grant) in grants.iter().enumerate() {
        assert_auth(&node, grant, index == 3).await;
    }
    let state = node.state_dir();
    node.stop();
    node.binary = old;
    node.start().await;
    assert_eq!(node.state_dir(), state);
    for (index, grant) in grants.iter().take(6).enumerate() {
        assert_auth(&node, grant, index == 3).await;
    }
    // The deployed baseline understands service profiles. The counterexample is
    // refused by its principal grammar, not by treating the profile as assistant.
    let (status, body) = auth_response(&node, &grants[6]).await;
    assert_eq!(status, 403, "{body}");
    let error: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(error["code"], "forbidden", "{body}");
    assert_eq!(error["message"], "invalid service principal", "{body}");
    // Owner inspection must preserve the plugin principal rather than laundering
    // a retained service record into an assistant identity.
    let owner = Client::unix_as(node.socket(), PERSON).unwrap();
    let inspected: Value = owner
        .get(&format!("/v1/client/assistant/grants/{}", grants[0].id)).await.unwrap();
    assert_eq!(inspected["kind"], "assistant-grant", "{inspected}");
    assert_eq!(inspected["principal"], PLUGIN, "{inspected}");
    assert_eq!(inspected["scopes"], json!(["read.service.status"]), "{inspected}");
    // Keep a positive owner-read control with the assistant principal and profile
    // accepted by both binaries; refusal above must not conceal a broken inspection route.
    let inspected: Value = owner
        .get(&format!("/v1/client/assistant/grants/{}", grants[3].id)).await.unwrap();
    assert_eq!(inspected["kind"], "assistant-grant", "{inspected}");
    assert_eq!(inspected["principal"], "assistant/chatgpt", "{inspected}");
    assert_eq!(inspected["scopes"], json!(["read.assistant.summary"]), "{inspected}");
    assert_retained(&node, &grants).await;
    node.stop();
    node.binary = PathBuf::from(st3());
    node.start().await;
    assert_eq!(node.state_dir(), state);
    for (index, grant) in grants.iter().enumerate() {
        assert_auth(&node, grant, index == 3).await;
    }
    assert_retained(&node, &grants).await;
}

fn compat_binary() -> PathBuf {
    PathBuf::from(std::env::var("ST3_COMPAT_BIN").expect("ST3_COMPAT_BIN names the pinned baseline"))
}

#[tokio::test(flavor = "multi_thread")]
async fn candidate_offline_service_fixtures_replicate_without_authorizing_credentials() {
    if fractal_daemon::test_support::supervise_test() { return; }
    mixed_pair(None, false).await;
}

async fn assert_candidate_issuance_grammar(node: &Node) {
    let client = reqwest::Client::builder().unix_socket(node.socket()).build().unwrap();
    let request = json!({"api_version":"st3.client.v0", "device_name":"fixture",
        "assistant_id":"assistant/chatgpt",
        "scopes":["read.assistant.summary", "read.assistant.attention",
            "read.assistant.missions", "read.assistant.agents"],
        "disclosure_consent":true, "consent_evidence":"fixture consent"});
    let control = client.post("http://localhost/v1/client/pairings")
        .header("x-st3-person", PERSON).json(&request).send().await.unwrap();
    let status = control.status();
    let body = control.text().await.unwrap();
    assert_eq!(status.as_u16(), 200, "assistant issuance control: {body}");
    let mut service = request.clone();
    service["profile"] = json!("service");
    let mut plugin = request;
    plugin["assistant_id"] = json!(PLUGIN);
    for refused in [service, plugin] {
        let response = client.post("http://localhost/v1/client/pairings")
            .header("x-st3-person", PERSON).json(&refused).send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status.as_u16(), 422, "service issuance grammar: {body}");
    }
    assert!(!node.claims().await.iter().any(|claim| claim["kind"] == GRANTED));
}

#[test]
fn candidate_issuance_grammar_cannot_encode_a_service_as_an_assistant() {
    // No profile field and no plugin principal are expressible in the public issuance DTO.
    let request = json!({"api_version":"st3.client.v0", "device_name":"fixture",
        "assistant_id":"assistant/chatgpt", "scopes":["read.assistant.summary"],
        "disclosure_consent":true, "consent_evidence":"fixture consent"});
    assert!(serde_json::from_value::<fractal_client::AssistantPairingBegin>(request.clone()).is_ok());
    let mut service = request.clone();
    service["profile"] = json!("service");
    assert!(serde_json::from_value::<fractal_client::AssistantPairingBegin>(service).is_err());
    let mut plugin = request;
    plugin["assistant_id"] = json!(PLUGIN);
    assert!(serde_json::from_value::<fractal_client::AssistantPairingBegin>(plugin).is_err());
}
