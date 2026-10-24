use super::*;
use crate::store::fleet_admission_tests_shared_seats::{admit, key, node, sync};

const MANAGER: &str = "agent/example-manager";
const COMPAT: &str = "agent/example-compat";
const EXECUTOR: &str = "agent/example-executor";
const OTHER: &str = "agent/example-other";

struct Fixture {
    issuer: Store,
    local: Store,
    policy: Policy,
    request: VerifyGoRequest,
    go: Value,
    now: u128,
}

fn publish(store: &Store, kind: &str, actor: &str, fields: Value) -> ClaimRecord {
    store.ensure_principal_key(actor).unwrap();
    let claim = store.append_claim(&ClaimInput {
        subject: "resource/deploy/example".into(), kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    claim
}
fn time(ms: u128) -> String {
    chrono::DateTime::from_timestamp_millis(i64::try_from(ms).unwrap()).unwrap().to_rfc3339()
}
impl Fixture {
    fn new() -> Self {
        let root = key();
        let member = key();
        let issuer = node("host-b", Some(&root), Some(&root));
        admit(&issuer, "host-b", &root, "anchor", None);
        admit(&issuer, "host-a", &member, "host-b", None);
        let local = node("host-a", Some(&member), Some(&root));
        sync(&issuer, &local);
        let now = now_ms();
        let compat = publish(&issuer, "deploy.compat-signoff", COMPAT, json!({
            "version":1,"trainId":"train/example","oldCommit":"a".repeat(40),"newCommit":"b".repeat(40)
        }));
        let go = json!({
            "version":1,"host":"host-a","actor":EXECUTOR,"operation":"install","trainId":"train/example",
            "old":{"packagePath":"/nix/store/old-example","narHash":"sha256-example-old","commit":"a".repeat(40)},
            "new":{"packagePath":"/nix/store/new-example","narHash":"sha256-example-new","commit":"b".repeat(40)},
            "issuedAt":time(now-1000),"expiresAt":time(now+60*60*1000),"failureRestore":false,
            "previewDigest":"example-preview","compatSignoffId":compat.id
        });
        let claim = publish(&issuer, "deploy.go", MANAGER, go.clone());
        sync(&issuer, &local);
        let policy = Policy::from_json(&serde_json::to_vec(&json!({
            "version":1,"refusedVerifierHosts":[],"consumptionSigners":[EXECUTOR,OTHER],
            "goSigners":[MANAGER],"stopSigners":[MANAGER],"compatSignoffSigners":[COMPAT]
        })).unwrap()).unwrap();
        let request = VerifyGoRequest {
            go_id: claim.id, host:"host-a".into(), actor:EXECUTOR.into(), operation:"install".into(), train_id:"train/example".into(),
            old:serde_json::from_value(go["old"].clone()).unwrap(), new:serde_json::from_value(go["new"].clone()).unwrap(),
            preview_digest:"example-preview".into(), failure_restore:false,
        };
        let fixture = Self { issuer, local, policy, request, go, now };
        fixture.receipt(Some(now));
        fixture
    }
    fn receipt(&self, time: Option<u128>) {
        self.local.connection.write().execute(
            "UPDATE replication_peers SET last_success_at_unix_ms=?1 WHERE peer='host-b'", [time.map(|t|t.to_string())]
        ).unwrap();
    }
    fn replace_go(&mut self, fields: Value) {
        self.request.go_id = publish(&self.issuer, "deploy.go", MANAGER, fields).id;
        sync(&self.issuer, &self.local);
        self.receipt(Some(self.now));
    }
    fn record(&self, kind: &str, actor: &str, fields: Value) {
        publish(&self.issuer, kind, actor, fields);
        sync(&self.issuer, &self.local);
        self.receipt(Some(self.now));
    }
    fn verify(&self) -> Checked<VerifyGoReceipt> { self.local.verify_deploy_go(&self.policy, &self.request, self.now) }
    fn stop(&self, id: &str, scope: &str, host: &str) {
        self.record("deploy.stop", MANAGER, json!({"version":1,"stopId":id,"scope":scope,"hosts":[host],"liftCondition":"example approval"}));
    }
}

#[test]
fn approved_offline_verification_is_read_only_and_exactly_bound() {
    let fixture = Fixture::new();
    let before = fixture.local.event_bounds().unwrap();
    let receipt = fixture.verify().unwrap();
    assert_eq!(receipt.go_id, fixture.request.go_id);
    assert_eq!(receipt.origin, "host-b");
    assert_eq!(receipt.policy_hash, fixture.policy.content_hash());
    fixture.verify().unwrap(); // Verification does not itself consume the GO.
    assert_eq!(fixture.local.event_bounds().unwrap(), before);
}

#[test]
fn every_caller_fact_must_match() {
    let fixture = Fixture::new();
    for name in ["host", "actor", "operation", "trainId", "previewDigest", "failureRestore", "old.packagePath", "old.narHash", "old.commit", "new.packagePath", "new.narHash", "new.commit"] {
        let mut value = serde_json::to_value(&fixture.request).unwrap();
        if let Some((parent, child)) = name.split_once('.') { value[parent][child] = json!("different"); }
        else { value[name] = if name == "failureRestore" { json!(true) } else { json!("different") }; }
        let request = serde_json::from_value(value).unwrap();
        assert!(fixture.local.verify_deploy_go(&fixture.policy, &request, fixture.now).is_err(), "{name}");
    }
}

#[test]
fn forbidden_local_host_cannot_claim_an_allowed_host() {
    let mut fixture = Fixture::new();
    fixture.policy.refused_verifier_hosts.push("host-a".into());
    assert!(fixture.verify().is_err());
    fixture.request.host = "host-b".into();
    assert!(fixture.verify().is_err());
}

#[test]
fn origin_receipt_requires_recent_nonfuture_success() {
    let fixture = Fixture::new();
    for time in [None, Some(fixture.now - 11*60*1000), Some(fixture.now+1)] {
        fixture.receipt(time);
        assert!(fixture.verify().is_err());
    }
    fixture.receipt(Some(fixture.now - 10*60*1000));
    fixture.verify().unwrap();
    fixture.local.connection.write().execute("DELETE FROM replication_peers WHERE peer='host-b'", []).unwrap();
    assert!(fixture.verify().is_err());
}

#[test]
fn issue_expiry_and_four_hour_boundary() {
    let mut fixture = Fixture::new();
    for (issued, expires, approved) in [
        (fixture.now+1,fixture.now+1000,false), (fixture.now-1000,fixture.now,false),
        (fixture.now,fixture.now+4*60*60*1000,true),
    ] {
        let mut go = fixture.go.clone(); go["issuedAt"] = json!(time(issued)); go["expiresAt"] = json!(time(expires));
        fixture.replace_go(go);
        assert_eq!(fixture.verify().is_ok(), approved);
    }
    let mut go = fixture.go.clone(); go["issuedAt"] = json!(time(fixture.now)); go["expiresAt"] = json!(time(fixture.now+4*60*60*1000+1));
    assert!(fixture.issuer.append_claim(&ClaimInput { subject:"resource/deploy/example".into(),kind:"deploy.go".into(),actor:Some(MANAGER.into()),fields:serde_json::from_value(go).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None }).is_err());
}

#[test]
fn compatibility_requires_authorized_typed_exact_reference() {
    let mut fixture = Fixture::new();
    for (kind, signer, mut fields) in [
        ("deploy.compat-signoff", OTHER, json!({"version":1,"trainId":"train/example","oldCommit":"a".repeat(40),"newCommit":"b".repeat(40)})),
        ("deploy.compat-signoff", COMPAT, json!({"version":1,"trainId":"train/other","oldCommit":"a".repeat(40),"newCommit":"b".repeat(40)})),
        ("deploy.compat-signoff", COMPAT, json!({"version":1,"trainId":"train/example","oldCommit":"c".repeat(40),"newCommit":"b".repeat(40)})),
        ("deploy.compat-signoff", COMPAT, json!({"version":1,"trainId":"train/example","oldCommit":"a".repeat(40),"newCommit":"c".repeat(40)})),
        ("deploy.go-revoked", MANAGER, json!({"version":1,"goId":"missing"})),
    ] {
        let claim = publish(&fixture.issuer, kind, signer, fields.take());
        let mut go = fixture.go.clone(); go["compatSignoffId"] = json!(claim.id);
        fixture.replace_go(go);
        assert!(fixture.verify().is_err());
    }
    let mut go = fixture.go.clone(); go["compatSignoffId"] = json!("missing"); fixture.replace_go(go);
    assert!(fixture.verify().is_err());
}

#[test]
fn revocation_and_any_consumption_refuse_reuse() {
    let fixture = Fixture::new();
    fixture.record("deploy.go-revoked", MANAGER, json!({"version":1,"goId":fixture.request.go_id}));
    assert!(fixture.verify().is_err());
    for (actor, operation, correct_origin) in [(EXECUTOR,"install",true),(EXECUTOR,"rollback",true),(OTHER,"install",true),(EXECUTOR,"install",false)] {
        let fixture = Fixture::new();
        let data = json!({"version":1,"goId":fixture.request.go_id,"operation":operation,"result":"example"});
        if correct_origin { publish(&fixture.local,"deploy.go-consumed",actor,data); }
        else { fixture.record("deploy.go-consumed",actor,data); }
        assert!(fixture.verify().is_err());
    }
}

#[test]
fn active_stop_combined_binding_and_second_stop() {
    let mut fixture = Fixture::new();
    fixture.stop("example-stop", "installer-entry", "host-a");
    assert!(fixture.verify().is_err());
    let mut go = fixture.go.clone(); go["combined"] = json!({"stopId":"example-stop","planId":"example-plan"});
    fixture.replace_go(go);
    fixture.verify().unwrap();
    fixture.stop("example-second", "train/example", "host-a");
    assert!(fixture.verify().is_err());
}

#[test]
fn combined_reference_requires_active_covering_stop() {
    for (id, scope, host) in [("different","installer-entry","host-a"),("example-stop","unrelated","host-a"),("example-stop","installer-entry","host-b")] {
        let mut fixture = Fixture::new(); fixture.stop(id,scope,host);
        let mut go = fixture.go.clone(); go["combined"] = json!({"stopId":"example-stop","planId":"example-plan"}); fixture.replace_go(go);
        assert!(fixture.verify().is_err());
    }
}

#[test]
fn typed_lift_removes_stop_but_not_dangling_or_combined_ambiguity() {
    let mut fixture = Fixture::new(); fixture.stop("example-stop","installer-entry","host-a");
    fixture.record("deploy.stop-lifted",MANAGER,json!({"version":1,"stopId":"example-stop","releaseGoId":fixture.request.go_id}));
    fixture.verify().unwrap();
    let mut go = fixture.go.clone(); go["combined"] = json!({"stopId":"example-stop","planId":"example-plan"}); fixture.replace_go(go);
    assert!(fixture.verify().is_err());
    for (stop, release) in [("missing",fixture.request.go_id.as_str()),("example-stop","missing")] {
        let other = Fixture::new(); other.stop("example-stop","installer-entry","host-a");
        other.record("deploy.stop-lifted",MANAGER,json!({"version":1,"stopId":stop,"releaseGoId":release}));
        assert!(other.verify().is_err());
    }
}

#[test]
fn signer_authority_is_not_go_actor_text() {
    let mut fixture = Fixture::new();
    let unauthorized = publish(&fixture.issuer,"deploy.go",OTHER,fixture.go.clone());
    fixture.request.go_id = unauthorized.id; sync(&fixture.issuer,&fixture.local); fixture.receipt(Some(fixture.now));
    assert!(fixture.verify().is_err());
    let mut record = fixture.local.claim_by_id(&fixture.request.go_id).unwrap().unwrap();
    record.body["fields"]["version"] = json!(2);
    assert!(fields(&record).is_err()); // Decode unknown versions even before considering authority.
}

#[test]
fn unrelated_valid_lifts_do_not_block_and_historical_lifts_do_not_expire() {
    let fixture = Fixture::new();
    fixture.stop("other-stop", "installer-entry", "host-b");
    let mut release = fixture.go.clone();
    release["host"] = json!("host-b");
    release["issuedAt"] = json!(time(fixture.now - 2*60*60*1000));
    release["expiresAt"] = json!(time(fixture.now - 60*60*1000));
    let release_id = publish(&fixture.issuer, "deploy.go", MANAGER, release).id;
    fixture.record("deploy.stop-lifted", MANAGER, json!({"version":1,"stopId":"other-stop","releaseGoId":release_id}));
    fixture.verify().unwrap();
    fixture.stop("local-stop", "installer-entry", "host-a");
    let mut release = fixture.go.clone();
    release["issuedAt"] = json!(time(fixture.now - 2*60*60*1000));
    release["expiresAt"] = json!(time(fixture.now - 60*60*1000));
    let release_id = publish(&fixture.issuer, "deploy.go", MANAGER, release).id;
    fixture.record("deploy.stop-lifted", MANAGER, json!({"version":1,"stopId":"local-stop","releaseGoId":release_id}));
    fixture.verify().unwrap();
    // A release for another train cannot authorize this request's covering STOP.
    fixture.stop("train-stop", "installer-entry", "host-a");
    let compat_id = publish(&fixture.issuer, "deploy.compat-signoff", COMPAT, json!({
        "version":1,"trainId":"train/other","oldCommit":"a".repeat(40),"newCommit":"b".repeat(40)
    })).id;
    let mut release = fixture.go.clone();
    release["trainId"] = json!("train/other");
    release["compatSignoffId"] = json!(compat_id);
    let release_id = publish(&fixture.issuer, "deploy.go", MANAGER, release).id;
    fixture.record("deploy.stop-lifted", MANAGER, json!({"version":1,"stopId":"train-stop","releaseGoId":release_id}));
    assert!(fixture.verify().is_err());
}

#[test]
fn unsigned_claims_are_not_authority() {
    let fixture = Fixture::new();
    let unsigned = Store::open_memory("host-a").unwrap();
    let claim = publish(&unsigned, "deploy.go", MANAGER, fixture.go.clone());
    let mut request = fixture.request.clone();
    request.go_id = claim.id;
    assert!(unsigned.verify_deploy_go(&fixture.policy, &request, fixture.now).is_err());
}

#[test]
fn unreadable_deployment_history_fails_closed_after_successful_sync() {
    for state in ["pending", "unknown", "invalid"] {
        let fixture = Fixture::new();
        // Model the retained transport/admission row for an unsupported v2 revocation.
        // The approved GO and its principal chain above remain genuinely signed.
        fixture.local.connection.write().execute(
            "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,kind_hint,error_code,updated_at_unix_ms)
             VALUES('example-unsupported','host-b',9999,'example-envelope',0,?1,?2,'deploy.go-revoked','unsupported-deploy-version',?3)",
            params![serde_json::to_vec(&json!({"kind":"deploy.go-revoked","fields":{"version":2,"goId":fixture.request.go_id}})).unwrap(),state,fixture.now.to_string()]
        ).unwrap();
        assert!(fixture.verify().is_err(), "{state}");
    }
}

#[test]
fn incomplete_principal_model_is_never_authority() {
    let fixture = Fixture::new();
    fixture.local.connection.write().execute(
        "UPDATE claim_signature_migration SET phase='legacy',serving_legacy=0", []
    ).unwrap();
    assert!(matches!(fixture.local.claim_authorization(&fixture.request.go_id).unwrap().verdict, Verdict::Held(_)));
    assert!(fixture.verify().is_err());
}

#[test]
fn repaired_release_records_cannot_lift_a_stop() {
    let fixture = Fixture::new();
    fixture.stop("example-stop","installer-entry","host-a");
    let lift = publish(&fixture.issuer,"deploy.stop-lifted",MANAGER,json!({
        "version":1,"stopId":"example-stop","releaseGoId":fixture.request.go_id
    }));
    sync(&fixture.issuer,&fixture.local); fixture.receipt(Some(fixture.now));
    fixture.verify().unwrap();
    fixture.local.connection.write().execute(
        "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)", [&lift.id]
    ).unwrap();
    assert!(fixture.verify().is_err());
}

#[test]
fn cryptographically_signed_go_from_revoked_key_is_invalid() {
    let mut fixture = Fixture::new();
    let signing_key = fixture.issuer.claim_authorization(&fixture.request.go_id).unwrap().signature.unwrap().key;
    fixture.issuer.revoke_device_key(MANAGER, &signing_key, "example revocation").unwrap();
    fixture.replace_go(fixture.go.clone());
    assert!(matches!(fixture.local.claim_authorization(&fixture.request.go_id).unwrap().verdict, Verdict::Invalid(_)));
    assert!(fixture.verify().is_err());
}
