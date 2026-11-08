use super::*;
use fractal_client::ErrorCode;
use smallclaims::fleet::MemberKey;
use std::sync::Arc;

const FLEET: &str = "86700000-0000-4000-8000-000000000001";
const OWNER: &str = "person/owner";
const ASSISTANT_ID: &str = "111111111111111111111111";
const SERVICE_ID: &str = "222222222222222222222222";

// This override is thread-local: the reserved service profile remains closed in
// production, and another test thread cannot accidentally inherit the fixture.
struct ServiceFixture;
impl ServiceFixture {
    fn enable() -> Self {
        assert!(!tests::SERVICE_PROFILE.replace(true));
        Self
    }
}
impl Drop for ServiceFixture {
    fn drop(&mut self) {
        tests::SERVICE_PROFILE.set(false);
    }
}

fn member(root: &std::path::Path, name: &str) -> AppState {
    let mut state = crate::api::tests_shared::state(root);
    state.store = Arc::new(Store::open_memory(name).unwrap());
    state.node = name.into();
    state.owning_person = Some(OWNER.into());
    state.fleet_id = Some(FLEET.into());
    state.store.bind_fleet(FLEET).unwrap();
    state
}

fn replicate(from: &AppState, to: &AppState) {
    let exchange = from.store.export_replication_exchange_answering(
        FLEET, &to.store.replication_inventory().unwrap(),
        &to.store.replication_signature_requests().unwrap(),
    ).unwrap();
    to.store.receive_replication_exchange(from.store.origin(), FLEET, &exchange).unwrap();
    to.store.validate_replication_backlog().unwrap();
    to.store.project_replication_backlog().unwrap();
}

fn admitted_pair(root: &std::path::Path) -> (AppState, AppState) {
    let a = member(root, "alder");
    let b = member(root, "birch");
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let peer = Arc::new(MemberKey::generate().unwrap().0);
    for (state, key) in [(&a, &anchor), (&b, &peer)] {
        state.store.pin_fleet_anchor(anchor.public()).unwrap();
        state.store.set_member_key(Some(key.clone())).unwrap();
    }
    a.store.admit_fleet_anchor(FLEET, anchor.public(), "listening").unwrap();
    raw_claim(&a, "host/birch", "fleet.member-admitted", OWNER, json!({
        "fleet_id":FLEET,"member_key":peer.public(),"via":"invite","mode":"listening"
    }));
    replicate(&a, &b);
    assert!(matches!(b.store.fleet_membership().unwrap().state("birch"),
        smallclaims::fleet::MemberState::Current(_)));
    (a, b)
}

fn grant_fields(id: &str, profile: &str) -> Value {
    json!({"profile":profile,"grant_id":id,"principal":PRINCIPAL,
        "issuer":OWNER,"session_actor":format!("{PRINCIPAL}/session/{id}"),
        "expires_at_unix_ms":u64::MAX,"minimal_read_mode":"approval-once",
        "disclosure_consent":true,"scopes":["read.assistant.summary"],
        "credential_hash":credential_digest(id)})
}

fn issue(state: &AppState, id: &str, profile: &str) -> Result<ClaimRecord, ApiError> {
    let source = grant_fields(id, profile);
    let subject = subject(id).unwrap();
    let begun = append(state, subject.clone(), BEGUN, OWNER, source.clone(), None)?;
    append(state, subject, GRANTED, OWNER, source, Some(&begun))
}

fn raw_claim(state: &AppState, subject: &str, kind: &str, actor: &str, source: Value) -> ClaimRecord {
    state.store.append_claim(&ClaimInput {
        subject:subject.into(),kind:kind.into(),actor:Some(actor.into()),
        fields:serde_json::from_value(source).unwrap(),evidence:Vec::new(),
        expected_subject:None,idempotency_key:None,
    }).unwrap()
}

fn authenticate_id(state: &AppState, id: &str) -> Result<Option<ClientSession>, ApiError> {
    authenticate(state, &credential_digest(id), "http")
}

fn assert_conflict(error: &ApiError) {
    let typed: ErrorCode = serde_json::from_value(json!(error.code)).unwrap();
    assert_eq!(typed, ErrorCode::GrantProfileConflict);
    assert_eq!(error.status, StatusCode::CONFLICT);
}

fn assert_witness(value: &Value) {
    let rows = value["grants"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for (id, origin, profile) in [(ASSISTANT_ID, "alder", "assistant"),
        (SERVICE_ID, "birch", "service")] {
        assert!(rows.contains(&json!({"grant_id":id,"origin":origin,"profile":profile})),
            "missing grant witness: {value}");
    }
}

#[test]
fn disconnected_opposite_profiles_conflict_on_both_members_and_revocation_recovers() {
    let _service = ServiceFixture::enable();
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let assistant = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let service = issue(&b, SERVICE_ID, "service").unwrap();
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
    assert!(authenticate_id(&b, SERVICE_ID).unwrap().is_some());
    replicate(&a, &b);
    replicate(&b, &a);
    for state in [&a, &b] {
        for grant in [&assistant, &service] {
            assert!(state.store.claim_has_authenticated_member_writer(&grant.id).unwrap());
            let error = authenticate_id(state, fields(grant).unwrap()["grant_id"].as_str().unwrap())
                .err().expect("both credentials must be suspended on either member");
            assert_conflict(&error);
            assert_witness(&Value::Object((*error.details).clone())["conflict"]);
            assert_witness(&profile_conflict(state, grant).unwrap().unwrap());
            assert_witness(&projection_with_witness(state, grant).unwrap()["conflict"]);
        }
    }
    append(&b, service.subject.clone(), REVOKED, OWNER,
        fields(&service).unwrap().clone(), Some(&service)).unwrap();
    replicate(&b, &a);
    for state in [&a, &b] {
        for grant in [&assistant, &service] {
            assert!(profile_conflict(state, grant).unwrap().is_none());
            assert!(projection_with_witness(state, grant).unwrap()["conflict"].is_null());
        }
        assert!(authenticate_id(state, SERVICE_ID).is_err());
        let inspected = projection_with_witness(state, &assistant).unwrap();
        let cleared = inspected["cleared_conflicts"].as_array().unwrap();
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0]["cause"], "revoked");
        assert_witness(&cleared[0]);
    }
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
    let foreign = authenticate_id(&b, ASSISTANT_ID).err().unwrap();
    assert_eq!(foreign.code, "forbidden");
    assert!(foreign.message.contains("authoritative grant issuer"));
}

#[test]
fn expiry_boundary_clears_conflict_and_disconnected_reissue_recomputes_it() {
    let _service = ServiceFixture::enable();
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let assistant = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let service = issue(&b, SERVICE_ID, "service").unwrap();
    // Birch never receives Alder's assistant grant: it remains a disconnected
    // issuer even as Alder receives its grants and policy transitions.
    replicate(&b, &a);
    assert_witness(&profile_conflict(&a, &assistant).unwrap().unwrap());

    let mut policy = fields(&service).unwrap().clone();
    policy["expires_at_unix_ms"] = json!(100_u64);
    append(&b, service.subject.clone(), POLICY, OWNER, policy, Some(&service)).unwrap();
    replicate(&b, &a);
    for (now, expected_count, conflicted) in [(99_u64, 2, true), (100, 1, false), (101, 1, false)] {
        let candidates = candidate_grants(&a, PRINCIPAL, Some(now)).unwrap();
        assert_eq!(candidates.len(), expected_count, "SQL/Rust expiry boundary at {now}");
        assert_eq!(candidates.iter().any(|(grant, _)| grant.id == service.id), conflicted);
        assert_eq!(profile_conflict_at(&a, &assistant, now).unwrap().is_some(), conflicted);
        assert_eq!(profile_conflict_at(&a, &service, now).unwrap().is_some(), conflicted);
        assert_eq!(unexpired(fields(&effective(&a, &service).unwrap()).unwrap(), now), conflicted);
    }
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
    let inspected = projection_with_witness(&a, &assistant).unwrap();
    assert!(inspected["conflict"].is_null());
    let cleared = inspected["cleared_conflicts"].as_array().unwrap();
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0]["cause"], "expired");
    assert_witness(&cleared[0]);

    let replacement_id = "333333333333333333333333";
    let replacement = issue(&b, replacement_id, "service").unwrap();
    replicate(&b, &a);
    let witness = profile_conflict(&a, &assistant).unwrap().unwrap();
    let rows = witness["grants"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&json!({"grant_id":ASSISTANT_ID,"origin":"alder","profile":"assistant"})));
    assert!(rows.contains(&json!({"grant_id":replacement_id,"origin":"birch","profile":"service"})));
    assert!(!rows.iter().any(|row| row["grant_id"] == SERVICE_ID));
    assert!(profile_conflict(&a, &service).unwrap().is_none());
    assert!(profile_conflict(&a, &replacement).unwrap().is_some());
    for id in [ASSISTANT_ID, replacement_id] {
        assert_conflict(&authenticate_id(&a, id).err().unwrap());
    }
    let inspected = projection_with_witness(&a, &assistant).unwrap();
    assert_eq!(inspected["conflict"], witness);
    assert!(inspected["cleared_conflicts"].is_null());
}

#[test]
fn same_profile_foreign_grants_do_not_conflict_or_authenticate_on_the_wrong_member() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let first = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let second = issue(&b, SERVICE_ID, "assistant").unwrap();
    replicate(&a, &b);
    replicate(&b, &a);
    for state in [&a, &b] {
        for grant in [&first, &second] {
            assert!(profile_conflict(state, grant).unwrap().is_none());
        }
    }
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
    assert!(authenticate_id(&b, SERVICE_ID).unwrap().is_some());
    assert_eq!(authenticate_id(&a, SERVICE_ID).err().unwrap().code, "forbidden");
    assert_eq!(authenticate_id(&b, ASSISTANT_ID).err().unwrap().code, "forbidden");
}

#[test]
fn admission_refuses_a_known_authenticated_foreign_opposite_profile() {
    let _service = ServiceFixture::enable();
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    issue(&b, SERVICE_ID, "service").unwrap();
    replicate(&b, &a);
    let error = issue(&a, ASSISTANT_ID, "assistant").err().unwrap();
    assert_conflict(&error);
    assert_witness(&Value::Object((*error.details).clone())["conflict"]);
    assert!(latest(&a, &subject(ASSISTANT_ID).unwrap(), GRANTED).unwrap().is_none());
}

#[test]
fn malformed_authenticated_foreign_grants_cannot_suspend_a_local_credential() {
    let root = tempfile::tempdir().unwrap();
    // Each case is independently a signed admitted writer's claim, rather than
    // an unsigned claim rejected before it can exercise the conflict scanner.
    for invalid in ["profile", "grant_id", "principal", "issuer", "session_actor",
        "minimal_read_mode", "disclosure_consent", "scopes", "credential_hash", "actor"] {
        let (a, b) = admitted_pair(root.path());
        let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
        let mut source = grant_fields(SERVICE_ID, "service");
        if invalid != "actor" {
            source[invalid] = Value::Null;
        }
        let foreign = raw_claim(&b, &subject(SERVICE_ID).unwrap(), GRANTED,
            if invalid == "actor" { "person/intruder" } else { OWNER }, source);
        replicate(&b, &a);
        assert!(a.store.claim_by_id(&foreign.id).unwrap().is_some(), "{invalid}");
        assert!(a.store.claim_has_authenticated_member_writer(&foreign.id).unwrap(), "{invalid}");
        assert!(profile_conflict(&a, &local).unwrap().is_none(), "{invalid}");
        assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some(), "{invalid}");
    }
}

#[test]
fn foreign_policy_with_mismatched_identity_cannot_clear_a_conflict() {
    let _service = ServiceFixture::enable();
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let foreign = issue(&b, SERVICE_ID, "service").unwrap();
    let mut policy = fields(&foreign).unwrap().clone();
    policy["session_actor"] = json!("assistant/chatgpt/session/333333333333333333333333");
    raw_claim(&b, &foreign.subject, POLICY, OWNER, policy);
    replicate(&b, &a);
    assert!(profile_conflict(&a, &local).unwrap().is_some());
    assert_conflict(&authenticate_id(&a, ASSISTANT_ID).err().unwrap());
}

#[test]
fn wrong_origin_revocation_and_expired_policy_cannot_clear_conflict() {
    let _service = ServiceFixture::enable();
    let root = tempfile::tempdir().unwrap();
    for kind in [REVOKED, POLICY] {
        let (a, b) = admitted_pair(root.path());
        let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
        let foreign = issue(&b, SERVICE_ID, "service").unwrap();
        replicate(&a, &b);
        replicate(&b, &a);
        let mut overlay = fields(&foreign).unwrap().clone();
        if kind == POLICY {
            overlay["expires_at_unix_ms"] = json!(1_u64);
        }
        // An admitted, signed writer with an owner-shaped actor still cannot
        // mutate another member's grant lifecycle.
        let malicious = raw_claim(&a, &foreign.subject, kind, OWNER, overlay);
        replicate(&a, &b);
        for state in [&a, &b] {
            assert!(state.store.claim_has_authenticated_member_writer(&malicious.id).unwrap());
            assert_eq!(effective(state, &foreign).unwrap().id, foreign.id);
            assert_witness(&profile_conflict(state, &local).unwrap().unwrap());
            for id in [ASSISTANT_ID, SERVICE_ID] {
                assert_conflict(&authenticate_id(state, id).err().unwrap());
            }
            let inspected = projection_with_witness(state, &local).unwrap();
            assert!(inspected["conflict"].is_object());
            assert!(inspected["cleared_conflicts"].is_null());
        }
    }
}

#[test]
fn malformed_same_origin_expired_policy_keeps_the_last_valid_live_policy() {
    let _service = ServiceFixture::enable();
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let foreign = issue(&b, SERVICE_ID, "service").unwrap();
    let mut live = fields(&foreign).unwrap().clone();
    live["policy_revision"] = json!(2);
    let live = append(&b, foreign.subject.clone(), POLICY, OWNER, live, Some(&foreign)).unwrap();
    let mut malformed = fields(&live).unwrap().clone();
    malformed["expires_at_unix_ms"] = json!(1_u64);
    malformed["session_actor"] = json!("assistant/chatgpt/session/not-this-grant");
    raw_claim(&b, &foreign.subject, POLICY, OWNER, malformed);
    replicate(&a, &b);
    replicate(&b, &a);
    for state in [&a, &b] {
        assert_eq!(effective(state, &foreign).unwrap().id, live.id);
        assert_witness(&profile_conflict(state, &local).unwrap().unwrap());
        assert_conflict(&authenticate_id(state, ASSISTANT_ID).err().unwrap());
    }
}

#[test]
fn signed_revocation_arriving_before_its_grant_converges_without_conflict() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    replicate(&a, &b);
    let source = grant_fields(SERVICE_ID, "service");
    let subject = subject(SERVICE_ID).unwrap();
    // Replicated custom lifecycle claims are open data. Import the authentic
    // revocation first, while its grant is absent, then import that grant.
    let revoked = raw_claim(&b, &subject, REVOKED, OWNER, source.clone());
    replicate(&b, &a);
    assert!(latest(&a, &subject, GRANTED).unwrap().is_none());
    assert!(a.store.claim_by_id(&revoked.id).unwrap().is_some());
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
    let granted = raw_claim(&b, &subject, GRANTED, OWNER, source);
    replicate(&b, &a);
    for state in [&a, &b] {
        let revoked = state.store.claim_by_id(&revoked.id).unwrap().unwrap();
        let granted = state.store.claim_by_id(&granted.id).unwrap().unwrap();
        assert!(revoked.store_index < granted.store_index);
        assert!(state.store.claim_has_authenticated_member_writer(&revoked.id).unwrap());
        assert!(state.store.claim_has_authenticated_member_writer(&granted.id).unwrap());
        assert!(profile_conflict(state, &local).unwrap().is_none());
        assert!(profile_conflict(state, &granted).unwrap().is_none());
        let inspected = projection_with_witness(state, &local).unwrap();
        let cleared = inspected["cleared_conflicts"].as_array().unwrap();
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0]["cause"], "revoked");
        assert_witness(&cleared[0]);
    }
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
}

#[test]
fn unverifiable_foreign_grant_cannot_suspend_a_local_credential() {
    let root = tempfile::tempdir().unwrap();
    // Legacy replication projects unsigned envelopes. Mere visibility and an
    // owner-shaped actor are not sufficient provenance for a conflict witness.
    let a = member(root.path(), "alder");
    let b = member(root.path(), "birch");
    let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let foreign = raw_claim(&b, &subject(SERVICE_ID).unwrap(), GRANTED, OWNER,
        grant_fields(SERVICE_ID, "service"));
    replicate(&b, &a);
    assert!(a.store.claim_by_id(&foreign.id).unwrap().is_some());
    assert!(!a.store.claim_has_authenticated_member_writer(&foreign.id).unwrap());
    assert!(profile_conflict(&a, &local).unwrap().is_none());
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
}

#[test]
fn admission_scan_uses_the_real_principal_index() {
    let root = tempfile::tempdir().unwrap();
    let state = member(root.path(), "alder");
    issue(&state, ASSISTANT_ID, "assistant").unwrap();
    let connection = state.store.readers.get();
    let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-hf4201606586b owner=fractal_api_core::api::client_v0::external_grants::conflict_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {ADMISSION_QUERY}")).unwrap();
    let plan = query.query_map(rusqlite::params![GRANTED, PRINCIPAL, REVOKED, POLICY,
        Some(client_now_ms() as u64), state.store.origin()], |row| row.get::<_, String>(3)).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap();
    println!("external grant admission query plan:\n{}", plan.join("\n"));
    assert!(plan.iter().any(|step| step.contains("claims_assistant_v1_principal_index")),
        "principal admission must use its production index: {plan:?}");
    assert!(plan.iter().any(|step| step.contains("revoked USING INDEX claims_subject_kind_accepted_index")),
        "revocation must seek by subject rather than scan the principal: {plan:?}");
    assert!(plan.iter().any(|step| step.contains("claims_assistant_v1_principal_expiry_index")
        && step.contains("<expr>>?")),
        "expired grant bodies must be pruned by an index range: {plan:?}");
}
