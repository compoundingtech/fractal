#![cfg(unix)]
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fractal_daemon::api::{AppState, router, serve_unix};
use fractal_daemon::client::{Client, Endpoint};
use fractal_daemon::fleet::MemberKey;
use fractal_daemon::model::{ClaimInput, PlannerSpec};
use fractal_daemon::store::Store;
use fractal_schema::deploy::{PackageIdentity, VerifyGoReceipt, VerifyGoRequest};
use fractal_schema::deploy_policy::Policy;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
const MANAGER: &str = "agent/example-manager";
const COMPAT: &str = "agent/example-reviewer";
const EXECUTOR: &str = "agent/example-executor";

fn publish(store: &Store, kind: &str, actor: Option<&str>, fields: Value) -> String {
    if let Some(actor) = actor {
        store.ensure_principal_key(actor).unwrap();
    }
    let record = store.append_claim(&ClaimInput {
        subject: if kind == "fleet.member-admitted" { "host/host-b" } else { "resource/deploy/example" }.into(),
        kind: kind.into(), actor: actor.map(str::to_owned),
        fields: serde_json::from_value::<BTreeMap<String, Value>>(fields).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    record.id
}

fn fixture() -> (Store, VerifyGoRequest) {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let issuer = Store::open_memory("host-b").unwrap();
    issuer.bind_fleet(FLEET).unwrap();
    issuer.pin_fleet_anchor(key.public()).unwrap();
    issuer.set_member_key(Some(key.clone())).unwrap();
    publish(&issuer, "fleet.member-admitted", None, json!({
        "fleet_id":FLEET,"member_key":key.public(),"via":"anchor","mode":"listening"
    }));
    let old = PackageIdentity { package_path:"/nix/store/old-example".into(), nar_hash:"sha256-example-old".into(), commit:"a".repeat(40) };
    let new = PackageIdentity { package_path:"/nix/store/new-example".into(), nar_hash:"sha256-example-new".into(), commit:"b".repeat(40) };
    let compat = publish(&issuer, "deploy.compat-signoff", Some(COMPAT), json!({
        "version":1,"trainId":"train/example","oldCommit":old.commit,"newCommit":new.commit
    }));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let timestamp = |ms: u128| chrono::DateTime::from_timestamp_millis(i64::try_from(ms).unwrap()).unwrap().to_rfc3339();
    let go_id = publish(&issuer, "deploy.go", Some(MANAGER), json!({
        "version":1,"host":"host-a","actor":EXECUTOR,"operation":"install","trainId":"train/example",
        "old":old,"new":new,"issuedAt":timestamp(now - 1000),"expiresAt":timestamp(now + 3_600_000),
        "previewDigest":"example-preview","failureRestore":false,"compatSignoffId":compat
    }));
    let local = Store::open_memory("host-a").unwrap();
    local.bind_fleet(FLEET).unwrap();
    local.pin_fleet_anchor(key.public()).unwrap();
    let exchange = issuer.export_replication_exchange_answering(FLEET, &local.replication_inventory().unwrap(), &local.replication_signature_requests().unwrap()).unwrap();
    local.receive_replication_exchange("host-b", FLEET, &exchange).unwrap();
    local.validate_replication_backlog().unwrap();
    local.project_replication_backlog().unwrap();
    (local, VerifyGoRequest {
        go_id, host:"host-a".into(), actor:EXECUTOR.into(), operation:"install".into(), train_id:"train/example".into(),
        old, new, preview_digest:"example-preview".into(), failure_restore:false,
    })
}

fn command(root: &Path, endpoint: &str, request: &VerifyGoRequest) -> std::process::Command {
    let mut command = fractal_daemon::test_support::command(test_env!("CARGO_BIN_EXE_fractal-fixture"));
    command.env_clear().env("HOME", root).env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_RUNTIME_DIR", root).current_dir(root)
        .args(["--endpoint", endpoint, "--json", "deploy", "verify-go", &request.go_id,
            "--host", &request.host, "--actor", &request.actor, "--operation", &request.operation,
            "--train-id", &request.train_id, "--old-package-path", &request.old.package_path,
            "--old-nar-hash", &request.old.nar_hash, "--old-commit", &request.old.commit,
            "--new-package-path", &request.new.package_path, "--new-nar-hash", &request.new.nar_hash,
            "--new-commit", &request.new.commit, "--preview-digest", &request.preview_digest,
            "--failure-restore", "false"]);
    command
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verifier_cli_api_uses_signed_local_replica_and_fixed_policy() {
    if fractal_daemon::test_support::supervise_test() { return; }
    let root = tempfile::tempdir().unwrap();
    let (store, request) = fixture();
    let store = Arc::new(store);
    let before = store.event_bounds().unwrap();
    let state = AppState {
        owning_person:None, store:store.clone(), notify:Arc::new(Notify::new()), event_notify:watch::channel(0).0,
        node:"host-a".into(), state_dir:root.path().into(), pty_root:root.path().join("pty"), pty_binary:"pty".into(),
        fleet_id:Some(FLEET.into()), configured_peers:vec![], client_relay:None, native_session_home:None,
        planner_default:PlannerSpec::default(),
        harness_question_sink: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    };
    // The Fabric router must refuse this route even if a caller bypasses the Unix-only client.
    use tower::ServiceExt as _;
    let response = fractal_daemon::api::fabric_router(state.clone()).oneshot(
        axum::http::Request::builder().method("POST").uri("/v1/deploy/verify-go")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&request).unwrap())).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    let socket = root.path().join("verify.sock");
    let serving_socket = socket.clone();
    let server = tokio::spawn(async move { serve_unix(&serving_socket, router(state)).await });
    let client = Client::unix(&socket);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while client.get::<Value>("/v1/health").await.is_err() {
        assert!(!server.is_finished(), "API server exited");
        assert!(tokio::time::Instant::now() < deadline, "API did not become ready");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let endpoint = socket.to_str().unwrap();
    // Runtime attempts point at a valid, permissive file but cannot alter either binary's policy.
    let override_path = root.path().join("override.json");
    std::fs::write(&override_path, serde_json::to_vec(&json!({
        "version":1,"refusedVerifierHosts":[],"consumptionSigners":[EXECUTOR],
        "goSigners":[MANAGER],"stopSigners":[MANAGER],"compatSignoffSigners":[COMPAT]
    })).unwrap()).unwrap();
    for flag in ["--policy", "--policy-file", "--deploy-policy", "--deploy-policy-file"] {
        let output = command(root.path(), endpoint, &request).args([flag, override_path.to_str().unwrap()]).output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    }
    for field in ["policy", "policyFile", "deployPolicyFile"] {
        let mut overridden = serde_json::to_value(&request).unwrap();
        overridden[field] = json!(override_path);
        assert!(client.post::<_, Value>("/v1/deploy/verify-go", &overridden).await.is_err(),
            "API must reject caller-provided policy fields");
    }
    let api = client.verify_deploy_go(&request).await;
    let output = command(root.path(), endpoint, &request)
        .env("ST3_DEPLOY_POLICY_FILE", &override_path).env("ST_DEPLOY_POLICY_FILE", &override_path)
        .env("ST3_DEPLOY_POLICY", &override_path).output().unwrap();
    if Policy::packaged_path().is_none() {
        let error = api.unwrap_err();
        assert!(error.to_string().contains("deploy-policy-unavailable"), "{error:#}");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("deploy-policy-unavailable"));
    } else {
        let receipt = api.unwrap();
        assert_eq!(receipt.go_id, request.go_id);
        assert_eq!(receipt.origin, "host-b");
        assert!(!receipt.consumed);
        assert_eq!(receipt.diagnostics.future_dated_count, 0);
        assert_eq!(receipt.diagnostics.max_future_skew_ms, 0);
        assert!(receipt.policy_path.is_some());
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let printed: VerifyGoReceipt = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(printed.go_id, request.go_id);
        assert_eq!(printed.origin, "host-b");
        assert!(!printed.consumed);
        assert_eq!(printed.diagnostics, receipt.diagnostics);
        let mut mismatched = request.clone();
        mismatched.preview_digest.push_str("-changed");
        assert!(client.verify_deploy_go(&mismatched).await.unwrap_err().to_string().contains("deploy-request-mismatch"));
    }
    assert_eq!(store.event_bounds().unwrap(), before, "verification must not append claims");
    let remote = Client::new(Endpoint::Http("http://127.0.0.1:1".into()));
    assert!(remote.verify_deploy_go(&request).await.unwrap_err().to_string().contains("local Unix"));
    let output = command(root.path(), "http://127.0.0.1:1", &request).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("local Unix"));
    server.abort();
}
