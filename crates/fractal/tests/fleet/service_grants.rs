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
        // Same shape as the owner revoke route: only an authoritative revocation counts.
        claims.insert(append(node, &id, REVOKED, json!({
            "version": 1, "grant_id": id, "principal": principal, "issuer": PERSON,
            "transport": "fixture", "timestamp": "2026-01-01T00:00:00Z", "outcome": "revoked"
        }), Some(&policy)));
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
    assert_status(node, grant, if accepted { 400 } else { 403 }).await;
}

async fn assert_status(node: &Node, grant: &Grant, expected: u16) {
    let (status, body) = auth_response(node, grant).await;
    assert_eq!(status, expected,
        "{} grant {}: {body}\n{}", node.name, grant.id, node.logs());
}

// With the counterexample, assistant grant 4 (index 3) and service grant 7 (index 6)
// are active, well-formed opposite-profile grants for assistant/chatgpt. The
// candidate suspends both: it refuses with the conflict (409 grant-profile-conflict)
// before the service-profile refusal (403), so grant 7 answers 409, not 403.
// Grant 3 (index 2) has a non-assistant scope,
// so it is malformed for conflicts and stays a plain 403 service refusal.
fn candidate_counterexample_status(index: usize) -> u16 {
    if index == 3 || index == 6 { 409 } else { 403 }
}

async fn assert_candidate_counterexample(node: &Node, grants: &[Grant]) {
    for (index, grant) in grants.iter().enumerate() {
        assert_status(node, grant, candidate_counterexample_status(index)).await;
    }
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
#[ignore = "needs ST3_COMPAT_BIN: the pinned previous published release"]
async fn previous_release_refuses_service_grants_from_a_candidate_issuer() {
    if fractal_daemon::test_support::supervise_test() { return; }
    mixed_pair(Some(compat_binary()), false).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: the pinned previous published release"]
async fn previous_release_refuses_offline_seeded_service_grants_with_its_own_origin() {
    if fractal_daemon::test_support::supervise_test() { return; }
    mixed_pair(Some(compat_binary()), true).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: the pinned previous published release"]
async fn previous_release_refuses_plugin_service_grants_on_same_database_rollback() {
    if fractal_daemon::test_support::supervise_test() { return; }
    let old = PathBuf::from(std::env::var("ST3_COMPAT_BIN").expect("ST3_COMPAT_BIN names the pinned baseline"));
    let root = tempfile::tempdir().unwrap();
    let mut node = Node::new(root.path(), "issuer");
    node.start().await;
    assert_candidate_issuance_grammar(&node).await;
    // Includes the unissuable profile-blind counterexample; candidate must refuse it.
    let grants = fixtures(&mut node, true).await;
    assert_candidate_counterexample(&node, &grants).await;
    let state = node.state_dir();
    node.stop();
    node.binary = old;
    node.start().await;
    assert_eq!(node.state_dir(), state);
    for (index, grant) in grants.iter().take(6).enumerate() {
        assert_auth(&node, grant, index == 3).await;
    }
    // Older baselines admitted this profile-blind counterexample. v0.3.31's
    // Profile::issuable unconditionally refuses service profiles; no config enables them.
    let (status, body) = auth_response(&node, &grants[6]).await;
    let baseline_refuses_service_profiles = status == 403;
    if baseline_refuses_service_profiles {
        let error: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(error["code"], "forbidden", "{body}");
        assert_eq!(error["message"], "service grant issuance is disabled", "{body}");
        eprintln!(
            "baseline release already refuses service profiles: the profile-blind transition \
             is behind this fleet; skipping only the baseline counterexample acceptance assertion"
        );
    } else {
        assert_eq!(status, 400, "{} grant {}: {body}\n{}", node.name, grants[6].id, node.logs());
    }
    let owner = Client::unix_as(node.socket(), PERSON).unwrap();
    if baseline_refuses_service_profiles {
        // v0.3.31 also validates the stored principal while projecting an owner read.
        // A plugin is not an assistant: only the historical projection assertion is skipped.
        let response = reqwest::Client::builder().unix_socket(node.socket()).build().unwrap()
            .get(format!("http://localhost/v1/client/assistant/grants/{}", grants[0].id))
            .header("x-st3-person", PERSON).send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status.as_u16(), 403, "{body}");
        let error: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(error["code"], "forbidden", "{body}");
        assert_eq!(error["message"], "invalid assistant principal", "{body}");
        eprintln!(
            "baseline release already validates assistant principals: skipping only the \
             historical plugin-grant projection assertions"
        );
    } else {
        let inspected: Value = owner
            .get(&format!("/v1/client/assistant/grants/{}", grants[0].id)).await.unwrap();
        assert_eq!(inspected["kind"], "assistant-grant", "{inspected}");
        assert_eq!(inspected["principal"], "assistant/chatgpt", "{inspected}");
        assert_eq!(inspected["scopes"], json!(["read.service.status"]), "{inspected}");
    }
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
    assert_candidate_counterexample(&node, &grants).await;
    assert_retained(&node, &grants).await;
}

// Use the release's real issuance and policy routes: this history must predate the
// candidate's first Store::open, rather than being seeded by candidate store code.
async fn issue_with_policy(node: &Node, key: u8) -> Grant {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    let owner = Client::unix_as(node.socket(), PERSON).unwrap();
    let policy = json!({
        "api_version":"st3.client.v0",
        "scopes":["read.assistant.summary", "read.assistant.attention",
            "read.assistant.missions", "read.assistant.agents"],
        "expires_in_seconds":3600, "minimal_read_mode":"approval-once",
        "disclosure_consent":true, "consent_evidence":"Disposable rollback fixture owner consent"
    });
    let mut begin = policy.clone();
    begin["assistant_id"] = json!("assistant/chatgpt");
    begin["device_name"] = json!(format!("rollback fixture {key}"));
    let challenge: Value = owner.post("/v1/client/pairings", &begin).await.unwrap();
    let challenge = challenge.get("value").unwrap_or(&challenge);
    let pairing = challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/");
    let response = reqwest::Client::builder().unix_socket(node.socket()).build().unwrap()
        .post(format!("http://localhost/v1/client/pairings/{pairing}/complete"))
        .json(&json!({"api_version":"st3.client.v0", "code":challenge["code"],
            "device_public_key":URL_SAFE_NO_PAD.encode([key; 32]), "key_storage":"software"}))
        .send().await.unwrap();
    let status = response.status();
    let paired: Value = response.json().await.unwrap();
    assert_eq!(status.as_u16(), 200, "baseline assistant issuance: {paired}");
    let paired = paired.get("value").unwrap_or(&paired);
    let id = paired["grant"]["id"].as_str().unwrap().trim_start_matches("assistant-grant/").to_owned();
    let changed: Value = owner.post(
        &format!("/v1/client/assistant/grants/{id}/policy"), &policy,
    ).await.unwrap();
    let changed = changed.get("value").unwrap_or(&changed);
    assert_eq!(changed["policy_revision"], 2, "{changed}");
    let subject = format!("custom/client/assistant-v1-{id}");
    let claims = node.claims().await.into_iter()
        .filter(|claim| claim["subject"] == subject)
        .map(|claim| claim["id"].as_str().unwrap().to_owned()).collect();
    Grant { id, credential: paired["credential"].as_str().unwrap().to_owned(), claims }
}

fn authority_history(node: &Node) -> Vec<(String, String)> {
    let connection = rusqlite::Connection::open_with_flags(
        node.state_dir().join("claims.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    connection.prepare(
        "SELECT id, body FROM claims WHERE kind IN (?1, ?2, ?3) ORDER BY id",
    ).unwrap().query_map([GRANTED, POLICY, REVOKED], |row| {
        Ok((row.get(0)?, row.get(1)?))
    }).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn assert_authority_projection_ready(node: &Node) {
    let version: String = rusqlite::Connection::open_with_flags(
        node.state_dir().join("claims.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap().query_row(
        "SELECT value FROM meta WHERE key='assistant_grant_authority_version'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(version, "1", "candidate must complete authority backfill before rollback");
}

fn authority_projection_snapshot(connection: &rusqlite::Connection) -> (Vec<String>, Vec<String>) {
    let authority = connection.prepare(
        "SELECT json_array(claim_id,base_claim_id,grant_subject,kind,origin,actor,principal,profile,
            expires_at,store_index,credential_hash,record)
         FROM assistant_grant_authority ORDER BY claim_id,base_claim_id",
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<Vec<String>>>().unwrap();
    let effective = connection.prepare(
        "SELECT json_array(claim_id,grant_subject,principal,owner,profile,expires_at,revoked,grant_record)
         FROM assistant_grant_effective ORDER BY claim_id",
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<Vec<String>>>().unwrap();
    (authority, effective)
}

fn authority_backfill_progress(connection: &rusqlite::Connection) -> (u64, bool) {
    connection.query_row(
        "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1),0),
            EXISTS(SELECT 1 FROM meta WHERE key='assistant_grant_authority_version')",
        [format!("assistant_grant_authority_cursor/{GRANTED}")],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: the pinned previous published release"]
async fn authority_projection_resumes_after_daemon_is_killed_with_committed_partial_backfill() {
    use smallclaims::store::assistant_grant_authority as authority;

    if fractal_daemon::test_support::supervise_test() { return; }
    let root = tempfile::tempdir().unwrap();
    let mut node = Node::new(root.path(), "issuer");
    node.binary = compat_binary();
    node.start().await;
    let active = issue_with_policy(&node, 7).await;
    let revoked = issue_with_policy(&node, 8).await;
    let owner = Client::unix_as(node.socket(), PERSON).unwrap();
    let _: Value = owner.post(
        &format!("/v1/client/assistant/grants/{}/revoke", revoked.id), &json!({}),
    ).await.unwrap();
    let history = authority_history(&node);
    assert_eq!(history.len(), 5);
    node.stop();

    let database = node.state_dir().join("claims.sqlite3");
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    fractal_daemon::store::configure_projection_writer(&connection).unwrap();
    authority::install(&connection, &node.name).unwrap();
    let reference_path = root.path().join("uninterrupted.sqlite3");
    connection.execute("VACUUM INTO ?1", [reference_path.to_str().unwrap()]).unwrap();
    let mut reference = rusqlite::Connection::open(&reference_path).unwrap();
    fractal_daemon::store::configure_projection_writer(&reference).unwrap();
    loop {
        let transaction = reference.transaction().unwrap();
        let done = authority::backfill_slice(&transaction, Duration::from_secs(2)).unwrap();
        transaction.commit().unwrap();
        if done { break; }
    }
    let expected = authority_projection_snapshot(&reference);
    assert!(expected.0.is_empty(), "pre-marker Legacy grants lack authenticated writer provenance");
    assert!(expected.1.is_empty());
    drop(reference);

    // Commit one real backfill step rather than making the fixture's size depend on
    // machine speed. The writer fence holds startup at this incomplete migration,
    // so an actual process kill cannot race a fast daemon finishing all five rows.
    let transaction = connection.transaction().unwrap();
    assert!(!authority::backfill_slice(&transaction, Duration::ZERO).unwrap());
    transaction.commit().unwrap();
    let partial = authority_backfill_progress(&connection);
    assert!(partial.0 > 0, "must kill with durable forward progress");
    assert!(!partial.1, "the committed projection must still be incomplete");
    assert!(authority_projection_snapshot(&connection).0.is_empty(),
        "a partial cursor must not turn ambiguous Legacy evidence into authority");
    let blocker = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    node.binary = PathBuf::from(st3());
    let pty = node.root.join("bin/pty");
    let mut daemon = DaemonProcess::spawn_isolated(
        node.command(&["up", "--pty-binary", pty.to_str().unwrap(), "--web-listen", "off"]),
        fs::File::create(node.root.join("daemon.log")).unwrap(),
        fs::File::create(node.root.join("daemon.stderr.log")).unwrap(),
    ).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(daemon.try_wait().unwrap().is_none(), "candidate exited before interrupted startup:\n{}", node.logs());
        // Startup owns the directory fence before opening/migrating the store.
        if fractal_daemon::node_identity::lock_directory(&node.state_dir()).is_err() {
            break;
        }
        assert!(Instant::now() < deadline, "candidate did not acquire its startup state fence:\n{}", node.logs());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(authority_backfill_progress(&blocker), partial);
    daemon.kill().unwrap();
    assert!(!daemon.wait().unwrap().success(), "the interrupted daemon must not exit gracefully");
    assert_eq!(authority_backfill_progress(&blocker), partial, "kill must retain the committed cursor");
    blocker.rollback().unwrap();
    drop(connection);
    assert_eq!(authority_history(&node), history, "process kill must not alter raw evidence");

    node.start().await;
    wait_until("authority backfill becomes ready after daemon restart", 30, || async {
        let reader = rusqlite::Connection::open_with_flags(
            &database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        authority_backfill_progress(&reader).1
    }).await;
    assert_authority_projection_ready(&node);
    let reader = rusqlite::Connection::open_with_flags(
        &database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    assert_eq!(authority_projection_snapshot(&reader), expected,
        "restart must converge exactly to uninterrupted authority and effective rows");
    assert_auth(&node, &active, false).await;
    assert_auth(&node, &revoked, false).await;
    assert_eq!(authority_history(&node), history, "restart must preserve every raw claim body");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: the pinned previous published release"]
async fn previous_release_authorizes_and_revokes_after_authority_projection_rollback() {
    if fractal_daemon::test_support::supervise_test() { return; }
    let root = tempfile::tempdir().unwrap();
    let mut node = Node::new(root.path(), "issuer");
    node.binary = compat_binary();
    node.start().await;
    let active = issue_with_policy(&node, 7).await;
    let revoked = issue_with_policy(&node, 8).await;
    let owner = Client::unix_as(node.socket(), PERSON).unwrap();
    let _: Value = owner.post(
        &format!("/v1/client/assistant/grants/{}/revoke", revoked.id), &json!({}),
    ).await.unwrap();
    assert_auth(&node, &active, true).await;
    assert_auth(&node, &revoked, false).await;
    let history = authority_history(&node);
    assert_eq!(history.len(), 5, "two grants, two policy renewals and one revocation");
    let projection_exists: bool = rusqlite::Connection::open_with_flags(
        node.state_dir().join("claims.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='assistant_grant_authority')",
        [], |row| row.get(0),
    ).unwrap();
    assert!(!projection_exists, "baseline history must predate authority projection initialization");
    let state = node.state_dir();

    node.stop();
    node.binary = PathBuf::from(st3());
    node.start().await;
    assert_eq!(node.state_dir(), state);
    assert_authority_projection_ready(&node);
    // The old standalone issuer had no member signature or local-append marker.
    // The upgrade preserves evidence but requires its owner to re-issue.
    assert_auth(&node, &active, false).await;
    assert_auth(&node, &revoked, false).await;
    assert_eq!(authority_history(&node), history, "backfill must not rewrite raw evidence");

    node.stop();
    node.binary = compat_binary();
    node.start().await;
    assert_eq!(node.state_dir(), state);
    assert_auth(&node, &active, true).await;
    assert_auth(&node, &revoked, false).await;
    assert_eq!(authority_history(&node), history, "baseline must retain the backfilled history");
    let owner = Client::unix_as(node.socket(), PERSON).unwrap();
    let _: Value = owner.post(
        &format!("/v1/client/assistant/grants/{}/revoke", active.id), &json!({}),
    ).await.unwrap();
    assert_auth(&node, &active, false).await;
    let after_revoke = authority_history(&node);
    assert_eq!(after_revoke.len(), history.len() + 1);
    for claim in &history {
        assert!(after_revoke.contains(claim), "baseline revoke changed prior evidence");
    }

    node.stop();
    node.binary = PathBuf::from(st3());
    node.start().await;
    assert_authority_projection_ready(&node);
    assert_auth(&node, &active, false).await;
    assert_auth(&node, &revoked, false).await;
    assert_retained(&node, &[active, revoked]).await;
    assert_eq!(authority_history(&node), after_revoke, "reopen must retain the baseline revoke");
}

fn compat_binary() -> PathBuf {
    PathBuf::from(std::env::var("ST3_COMPAT_BIN").expect("ST3_COMPAT_BIN names the pinned baseline"))
}

#[tokio::test(flavor = "multi_thread")]
async fn candidate_offline_service_fixtures_replicate_without_authorizing_credentials() {
    if fractal_daemon::test_support::supervise_test() { return; }
    mixed_pair(None, false).await;
}

// Candidate-only reproduction of the rollback fixture, without a baseline binary.
#[tokio::test(flavor = "multi_thread")]
async fn candidate_suspends_the_offline_counterexample_profile_conflict() {
    if fractal_daemon::test_support::supervise_test() { return; }
    let root = tempfile::tempdir().unwrap();
    let mut node = Node::new(root.path(), "issuer");
    node.start().await;
    let grants = fixtures(&mut node, true).await;
    assert_candidate_counterexample(&node, &grants).await;
    assert_retained(&node, &grants).await;
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
