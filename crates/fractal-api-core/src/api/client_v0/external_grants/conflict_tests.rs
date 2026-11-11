use super::*;
use fractal_client::ErrorCode;
use smallclaims::fleet::MemberKey;
use std::collections::BTreeSet;
use std::sync::Arc;

const FLEET: &str = "86700000-0000-4000-8000-000000000001";
const OWNER: &str = "person/owner";
const ASSISTANT_ID: &str = "111111111111111111111111";
const SERVICE_ID: &str = "222222222222222222222222";

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
    while !to.store.advance_assistant_grant_authority_backfill().unwrap() {}
}

fn admitted_pair(root: &std::path::Path) -> (AppState, AppState) {
    admitted_pair_with_floor(root, 0)
}

fn admitted_pair_with_floor(root: &std::path::Path, floor: u64) -> (AppState, AppState) {
    let a = member(root, "alder");
    let b = member(root, "birch");
    b.store.set_writer_floor(floor).unwrap();
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let peer = Arc::new(MemberKey::generate().unwrap().0);
    for (state, key) in [(&a, &anchor), (&b, &peer)] {
        state.store.pin_fleet_anchor(anchor.public()).unwrap();
        state.store.set_member_key(Some(key.clone())).unwrap();
    }
    a.store.admit_fleet_anchor(FLEET, anchor.public(), "listening").unwrap();
    raw_claim(&a, "host/birch", "fleet.member-admitted", OWNER, json!({
        "fleet_id":FLEET,"member_key":peer.public(),"via":"invite","mode":"listening",
        "writer_floor":floor
    }));
    replicate(&a, &b);
    assert!(matches!(b.store.fleet_membership().unwrap().state("birch"),
        smallclaims::fleet::MemberState::Current(_)));
    (a, b)
}

// Service rows with this assistant principal are legacy persisted conflict
// evidence, never a service credential accepted by the strict bearer parser.
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

fn legacy_service(state: &AppState, id: &str) -> ClaimRecord {
    raw_claim(state, &subject(id).unwrap(), GRANTED, OWNER, grant_fields(id, "service"))
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

fn assert_legacy_bearer_forbidden(state: &AppState, id: &str) {
    let error = authenticate_id(state, id).expect_err("legacy service evidence is not a bearer");
    assert_eq!(error.code, "forbidden");
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(error.details.is_empty(), "legacy rejection must not expose owner witnesses");
}

fn assert_conflict(error: &ApiError) {
    let typed: ErrorCode = serde_json::from_value(json!(error.code)).unwrap();
    assert_eq!(typed, ErrorCode::GrantProfileConflict);
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.details.is_empty(), "bearer errors must not expose owner witnesses");
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
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let assistant = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let service = legacy_service(&b, SERVICE_ID);
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
    assert_legacy_bearer_forbidden(&b, SERVICE_ID);
    replicate(&a, &b);
    replicate(&b, &a);
    for state in [&a, &b] {
        for grant in [&assistant, &service] {
            assert!(state.store.claim_has_authenticated_member_writer(&grant.id).unwrap());
            if fields(grant).unwrap()["profile"] == "assistant" {
                assert_conflict(&authenticate_id(state, ASSISTANT_ID)
                    .expect_err("the assistant credential must be suspended on either member"));
            } else {
                assert_legacy_bearer_forbidden(state, SERVICE_ID);
            }
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
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let assistant = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let service = legacy_service(&b, SERVICE_ID);
    // Birch never receives Alder's assistant grant: it remains a disconnected
    // issuer even as Alder receives its grants and policy transitions.
    replicate(&b, &a);
    assert_witness(&profile_conflict(&a, &assistant).unwrap().unwrap());

    let mut policy = fields(&service).unwrap().clone();
    policy["expires_at_unix_ms"] = json!(100_u64);
    raw_claim(&b, &service.subject, POLICY, OWNER, policy);
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
    let replacement = legacy_service(&b, replacement_id);
    replicate(&b, &a);
    let witness = profile_conflict(&a, &assistant).unwrap().unwrap();
    let rows = witness["grants"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&json!({"grant_id":ASSISTANT_ID,"origin":"alder","profile":"assistant"})));
    assert!(rows.contains(&json!({"grant_id":replacement_id,"origin":"birch","profile":"service"})));
    assert!(!rows.iter().any(|row| row["grant_id"] == SERVICE_ID));
    assert!(profile_conflict(&a, &service).unwrap().is_none());
    assert!(profile_conflict(&a, &replacement).unwrap().is_some());
    assert_conflict(&authenticate_id(&a, ASSISTANT_ID).err().unwrap());
    assert_legacy_bearer_forbidden(&a, replacement_id);
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
fn plugin_service_bearer_is_independent_of_assistant_profiles_and_bound_to_its_issuer() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let assistant = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let mut source = grant_fields(SERVICE_ID, "service");
    source["principal"] = json!("plugin/test/reader");
    source["session_actor"] = json!(format!("plugin/test/reader/session/{SERVICE_ID}"));
    source["scopes"] = json!(["read.claims", "read.work", "read.documents"]);
    let service = raw_claim(&b, &subject(SERVICE_ID).unwrap(), GRANTED, OWNER, source);
    let bearer = authenticate_id(&b, SERVICE_ID).unwrap().unwrap();
    assert_eq!(bearer.authority_actor, "plugin/test/reader");
    assert_eq!(bearer.actor, format!("plugin/test/reader/session/{SERVICE_ID}"));
    assert_eq!(bearer.scopes, BTreeSet::from([
        "read.claims".to_owned(), "read.work".to_owned(), "read.documents".to_owned()]));
    replicate(&a, &b);
    replicate(&b, &a);
    for state in [&a, &b] {
        for grant in [&assistant, &service] {
            assert!(state.store.claim_has_authenticated_member_writer(&grant.id).unwrap());
            assert!(profile_conflict(state, grant).unwrap().is_none());
            assert!(projection_with_witness(state, grant).unwrap()["conflict"].is_null());
        }
        assert_eq!(candidate_grants(state, PRINCIPAL, Some(0)).unwrap().len(), 1);
        assert_eq!(candidate_grants(state, "plugin/test/reader", Some(0)).unwrap().len(), 1);
    }
    assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
    assert!(authenticate_id(&b, SERVICE_ID).unwrap().is_some());
    for (state, id) in [(&a, SERVICE_ID), (&b, ASSISTANT_ID)] {
        let error = authenticate_id(state, id).unwrap_err();
        assert_eq!(error.code, "forbidden");
        assert!(error.message.contains("authoritative grant issuer"));
    }
}

#[test]
fn admission_refuses_a_known_authenticated_foreign_opposite_profile() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    legacy_service(&b, SERVICE_ID);
    replicate(&b, &a);
    let error = issue(&a, ASSISTANT_ID, "assistant").err().unwrap();
    assert_conflict(&error);
    assert!(latest(&a, &subject(ASSISTANT_ID).unwrap(), GRANTED).unwrap().is_none());
}

#[test]
fn malformed_local_owner_actor_grants_cannot_suspend_a_local_credential() {
    let root = tempfile::tempdir().unwrap();
    for invalid in ["profile", "grant_id", "principal", "issuer", "session_actor",
        "minimal_read_mode", "disclosure_consent", "scopes", "credential_hash"] {
        let state = member(root.path(), "alder");
        let local = issue(&state, ASSISTANT_ID, "assistant").unwrap();
        let mut source = grant_fields(SERVICE_ID, "service");
        source[invalid] = Value::Null;
        let malformed = raw_claim(&state, &subject(SERVICE_ID).unwrap(), GRANTED, OWNER, source);
        assert_eq!(malformed.origin, local.origin);
        assert_eq!(malformed.actor.as_deref(), Some(OWNER));
        assert!(profile_conflict(&state, &local).unwrap().is_none(), "{invalid}");
        assert!(authenticate_id(&state, ASSISTANT_ID).unwrap().is_some(), "{invalid}");
        assert!(projection_with_witness(&state, &local).unwrap()["conflict"].is_null(), "{invalid}");
    }
}

// Same shape as fleet::service_grants' rollback counterexample: a service grant
// with a non-assistant scope (3), an assistant grant (4) and legacy service
// conflict evidence (7), all active for assistant/chatgpt on one member.
#[test]
fn compat_counterexample_suspends_the_valid_pair_and_refuses_the_malformed_service() {
    let root = tempfile::tempdir().unwrap();
    let state = member(root.path(), "issuer");
    let [malformed, assistant, service] = ["000000000000000000000003",
        "000000000000000000000004", "000000000000000000000007"];
    let mut source = grant_fields(malformed, "service");
    source["scopes"] = json!(["read.service.status"]);
    raw_claim(&state, &subject(malformed).unwrap(), GRANTED, OWNER, source);
    issue(&state, assistant, "assistant").unwrap();
    raw_claim(&state, &subject(service).unwrap(), GRANTED, OWNER, grant_fields(service, "service"));
    assert_conflict(&authenticate_id(&state, assistant)
        .expect_err("valid legacy opposite-profile evidence suspends the assistant"));
    assert_legacy_bearer_forbidden(&state, service);
    assert!(authenticate_id(&state, malformed).unwrap().is_none(),
        "malformed service evidence must not enter bearer authority");
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
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let foreign = legacy_service(&b, SERVICE_ID);
    let mut policy = fields(&foreign).unwrap().clone();
    policy["session_actor"] = json!("assistant/chatgpt/session/333333333333333333333333");
    raw_claim(&b, &foreign.subject, POLICY, OWNER, policy);
    replicate(&b, &a);
    assert!(profile_conflict(&a, &local).unwrap().is_some());
    assert_conflict(&authenticate_id(&a, ASSISTANT_ID).err().unwrap());
}

#[test]
fn wrong_origin_revocation_and_expired_policy_cannot_clear_conflict() {
    let root = tempfile::tempdir().unwrap();
    for kind in [REVOKED, POLICY] {
        let (a, b) = admitted_pair(root.path());
        let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
        let foreign = legacy_service(&b, SERVICE_ID);
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
            assert_conflict(&authenticate_id(state, ASSISTANT_ID).err().unwrap());
            assert_legacy_bearer_forbidden(state, SERVICE_ID);
            let inspected = projection_with_witness(state, &local).unwrap();
            assert!(inspected["conflict"].is_object());
            assert!(inspected["cleared_conflicts"].is_null());
        }
    }
}

#[test]
fn malformed_same_origin_expired_policy_keeps_the_last_valid_live_policy() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let foreign = legacy_service(&b, SERVICE_ID);
    let mut live = fields(&foreign).unwrap().clone();
    live["policy_revision"] = json!(2);
    let live = raw_claim(&b, &foreign.subject, POLICY, OWNER, live);
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
fn unauthenticated_legacy_transitions_preserve_authenticated_lifecycle_history() {
    let root = tempfile::tempdir().unwrap();
    for (scenario, kind) in [("renewal", POLICY), ("renewal", REVOKED),
        ("narrowing", POLICY), ("revocation", REVOKED)] {
        let (a, b) = admitted_pair_with_floor(root.path(), 10);
        let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
        let mut expired = grant_fields(SERVICE_ID, "service");
        expired["expires_at_unix_ms"] = json!(if scenario == "renewal" { 1_u64 } else { u64::MAX });
        let foreign = raw_claim(&b, &subject(SERVICE_ID).unwrap(), GRANTED, OWNER, expired);
        let mut live = fields(&foreign).unwrap().clone();
        live["expires_at_unix_ms"] = json!(if scenario == "narrowing" { 1_u64 } else { u64::MAX });
        let live = raw_claim(&b, &foreign.subject, POLICY, OWNER, live);
        let authentic_revoke = (scenario == "revocation").then(||
            raw_claim(&b, &foreign.subject, REVOKED, OWNER, fields(&foreign).unwrap().clone()));
        replicate(&b, &a);
        assert!(a.store.claim_has_authenticated_member_writer(&foreign.id).unwrap());
        assert!(a.store.claim_has_authenticated_member_writer(&live.id).unwrap());
        if let Some(revoke) = &authentic_revoke {
            assert!(a.store.claim_has_authenticated_member_writer(&revoke.id).unwrap());
        }
        assert_eq!(effective(&a, &foreign).unwrap().id, live.id);
        assert_eq!(revoked(&a, &foreign).unwrap(), scenario == "revocation");

        // A different incarnation of the same writer sends a signed envelope
        // below the admission floor. It is visible Legacy data, not authority,
        // even though origin, owner actor and immutable grant identity match.
        let legacy = member(root.path(), "birch");
        legacy.store.set_member_key(Some(Arc::new(MemberKey::generate().unwrap().0))).unwrap();
        let mut source = fields(&foreign).unwrap().clone();
        source["expires_at_unix_ms"] = json!(if scenario == "narrowing" { u64::MAX } else { 1_u64 });
        let overlay = raw_claim(&legacy, &foreign.subject, kind, OWNER, source);
        replicate(&legacy, &a);
        let overlay = a.store.claim_by_id(&overlay.id).unwrap().unwrap();
        let retained = a.store.claim_by_id(&live.id).unwrap().unwrap();
        assert!(overlay.store_index > retained.store_index);
        assert_eq!(overlay.origin, foreign.origin);
        assert_eq!(overlay.actor, foreign.actor);
        let connection = a.store.readers.get();
        let (signatures, sequence): (u64, u64) = connection.query_row(
            "SELECT count(*),max(sequence) FROM replica_records JOIN replica_envelope_signatures
             USING(writer,sequence,envelope_hash) WHERE claim_id=?1",
            [&overlay.id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert!(signatures > 0, "this regression must pass the signature-row prefilter");
        drop(connection);
        assert!(matches!(a.store.fleet_membership().unwrap().window("birch", sequence),
            smallclaims::fleet::Window::Legacy));
        assert!(!a.store.claim_has_authenticated_member_writer(&overlay.id).unwrap());
        assert_eq!(tests::reference_transition(&a, &foreign, POLICY).unwrap().unwrap().id, live.id);
        assert_eq!(tests::reference_transition(&a, &foreign, REVOKED).unwrap().map(|claim| claim.id),
            authentic_revoke.as_ref().map(|claim| claim.id.clone()));
        // Maintenance has finished, but this below-floor incarnation cannot
        // authenticate. Its retained transition still fences its own subject;
        // it must neither grant bearer access nor replace the last authenticated
        // snapshot used for cross-subject conflict admission.
        assert!(authenticate_id(&a, SERVICE_ID).unwrap().is_none());
        for error in [
            effective(&a, &foreign).unwrap_err(),
            revoked(&a, &foreign).unwrap_err(),
        ] {
            assert_eq!(error.code, "forbidden");
            assert_eq!(error.details["reason"], "credential-rejected");
        }
        let candidate = json!({"profile":"assistant","principal":PRINCIPAL});
        let reference = tests::rust_only_admission(&a, &candidate).map_err(|error| error.code);
        let production = admit_profile(&a, &candidate).map_err(|error| error.code);
        assert_eq!(production, reference, "{scenario}/{kind}: independent provenance-aware oracle");
        assert_eq!(production.is_err(), scenario == "renewal");
        if scenario == "renewal" {
            assert_witness(&profile_conflict(&a, &local).unwrap().unwrap());
            assert_conflict(&authenticate_id(&a, ASSISTANT_ID).expect_err("the old renewal remains active"));
        } else {
            assert!(profile_conflict(&a, &local).unwrap().is_none());
            assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
        }
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
fn admission_reads_the_authority_projection_index() {
    let root = tempfile::tempdir().unwrap();
    let state = member(root.path(), "alder");
    let grant = issue(&state, ASSISTANT_ID, "assistant").unwrap();
    let candidates = candidate_grants(&state, PRINCIPAL, Some(client_now_ms() as u64)).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].0.id, grant.id);
    let connection = state.store.readers.get();
    let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-ha14d9df71ca8 owner=fractal_api_core::api::client_v0::external_grants::conflict_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {}", authority::CANDIDATES_QUERY)).unwrap();
    let expiry = authority::expiry_key(client_now_ms() as u64);
    let plan = query.query_map(rusqlite::params![PRINCIPAL, OWNER, expiry,
        Option::<&str>::None, Option::<&str>::None], |row| row.get::<_, String>(3)).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap();
    assert!(plan.iter().any(|step| step.contains("assistant_grant_effective_active_index")),
        "admission must use the effective authority projection: {plan:?}");
    assert!(plan.iter().all(|step| !step.contains("claims")),
        "admission must never scan raw claim history: {plan:?}");
}

#[test]
fn p1_policy_candidates_keep_read_acquisitions_bounded_with_expired_history() {
    // Profiling is process-global, so isolate it using the profiler's own
    // subprocess convention instead of mutating parallel tests' environment.
    const CHILD: &str = "FRACTAL_POLICY_CANDIDATE_PROFILE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact",
                "api::client_v0::external_grants::conflict_tests::p1_policy_candidates_keep_read_acquisitions_bounded_with_expired_history",
                "--nocapture"])
            .env(CHILD, "1")
            .env("ST3_PROFILE_DIR", directory.path())
            .env("ST3_PROFILE_SLOW_MS", "0")
            .status().unwrap();
        assert!(status.success(), "isolated policy candidate regression failed");
        let log = std::fs::read_to_string(directory.path().join("slow.jsonl")).unwrap();
        let rows = log.lines().map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let acquisitions = |label: &str| rows.iter().find(|row| row["label"] == label)
            .unwrap()["read_acquisitions"].as_u64().unwrap();
        let before = acquisitions("p1-policy-candidates-before");
        let after = acquisitions("p1-policy-candidates-after");
        assert!(before > 0, "the measured scan must acquire real read connections");
        assert_eq!(after, before, "expired policy history and wrong-origin renewals must be pruned before per-grant reads");
        let two = acquisitions("p1-authentication-n2");
        let six = acquisitions("p1-authentication-n6");
        assert!(two > 0, "authentication must acquire real read connections");
        assert_eq!(six, two, "authenticated foreign candidates must use constant-N batched reads");
        return;
    }
    smallclaims::profile::init_from_env();
    let root = tempfile::tempdir().unwrap();
    let a = member(root.path(), "alder");
    let b = member(root.path(), "birch");
    let local = issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let measure = |label: &'static str| {
        let operation = smallclaims::profile::Op::start(label, None).unwrap();
        {
            let _entered = operation.enter();
            let candidates = candidate_grants(&a, PRINCIPAL, Some(100)).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].0.id, local.id);
        }
        operation.finish();
    };
    measure("p1-policy-candidates-before");
    for index in 0..128 {
        let id = format!("{:024x}", index + 1000);
        let subject = subject(&id).unwrap();
        let mut expired = grant_fields(&id, "service");
        expired["expires_at_unix_ms"] = json!(1);
        raw_claim(&a, &subject, GRANTED, OWNER, expired.clone());
        raw_claim(&a, &subject, POLICY, OWNER, expired.clone());
        // A future policy from another origin must not pull this expired
        // issuer grant back into the per-candidate Rust authority fold.
        expired["expires_at_unix_ms"] = json!(u64::MAX);
        raw_claim(&b, &subject, POLICY, OWNER, expired);
    }
    replicate(&b, &a);
    measure("p1-policy-candidates-after");

    let (a, b) = admitted_pair(root.path());
    issue(&a, ASSISTANT_ID, "assistant").unwrap();
    for (count, label) in [(1, "p1-authentication-n2"), (5, "p1-authentication-n6")] {
        let start = if count == 1 { 0 } else { 1 };
        for index in start..count {
            let id = format!("{:024x}", index + 2000);
            let source = grant_fields(&id, "service");
            let grant = raw_claim(&b, &subject(&id).unwrap(), GRANTED, OWNER, source.clone());
            raw_claim(&b, &grant.subject, POLICY, OWNER, source);
        }
        replicate(&b, &a);
        let operation = smallclaims::profile::Op::start(label, None).unwrap();
        {
            let _entered = operation.enter();
            assert_conflict(&authenticate_id(&a, ASSISTANT_ID)
                .expect_err("all measured opposite-profile grants remain active"));
        }
        operation.finish();
    }
}

#[test]
fn dirty_grant_reads_and_bearers_fail_closed_without_blocking_an_unrelated_credential() {
    let root = tempfile::tempdir().unwrap();
    let state = member(root.path(), "issuer");
    let grant = issue(&state, ASSISTANT_ID, "assistant").unwrap();
    let independent = issue(&state, SERVICE_ID, "assistant").unwrap();
    while !state.store.advance_assistant_grant_authority_backfill().unwrap() {}
    authenticate_id(&state, ASSISTANT_ID).unwrap().unwrap();
    authenticate_id(&state, SERVICE_ID).unwrap().unwrap();
    state.store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hda4b15518ef0 owner=fractal_api_core::api::client_v0::external_grants::conflict_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id) VALUES(?1)", [&grant.id],
    ).unwrap();
    assert!(authenticate_id(&state, ASSISTANT_ID).unwrap().is_none());
    authenticate_id(&state, SERVICE_ID).unwrap().unwrap();
    let denied = credential_rejected("the client credential is unknown or expired");
    for error in [
        session(&state, &grant, "http").unwrap_err(),
        effective(&state, &grant).unwrap_err(),
        revoked(&state, &grant).unwrap_err(),
        projection(&state, &grant).unwrap_err(),
        latest(&state, &grant.subject, GRANTED).unwrap_err(),
    ] {
        assert_eq!(error.status, denied.status);
        assert_eq!(error.code, denied.code);
        assert_eq!(error.message, denied.message);
        assert_eq!(error.details, denied.details);
    }
    assert_eq!(candidate_grants(&state, PRINCIPAL, Some(0)).unwrap()
        .iter().map(|(claim, _)| claim.id.as_str()).collect::<BTreeSet<_>>(),
        BTreeSet::from([grant.id.as_str(), independent.id.as_str()]));
}

#[test]
fn dirty_opposite_profile_witness_does_not_clear_an_established_conflict() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = admitted_pair(root.path());
    issue(&a, ASSISTANT_ID, "assistant").unwrap();
    let service = legacy_service(&b, SERVICE_ID);
    replicate(&a, &b);
    replicate(&b, &a);
    assert_conflict(&authenticate_id(&a, ASSISTANT_ID).unwrap_err());
    a.store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h830bfc82281f owner=fractal_api_core::api::client_v0::external_grants::conflict_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id) VALUES(?1)", [&service.id],
    ).unwrap();
    assert!(authenticate_id(&a, SERVICE_ID).unwrap().is_none(),
        "the dirty subject's own credential is fenced");
    assert_conflict(&authenticate_id(&a, ASSISTANT_ID).unwrap_err());
    assert_eq!(candidate_grants(&a, PRINCIPAL, Some(0)).unwrap().len(), 2,
        "pending work cannot erase the last authenticated opposite-profile snapshot");
}

#[test]
fn clean_grant_inspection_omits_pending_expired_or_revoked_opposite_history() {
    for cause in ["expired", "revoked"] {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = admitted_pair(root.path());
        let assistant = issue(&a, ASSISTANT_ID, "assistant").unwrap();
        let service = legacy_service(&b, SERVICE_ID);
        let mut source = fields(&service).unwrap().clone();
        let kind = if cause == "expired" {
            source["expires_at_unix_ms"] = json!(1_u64);
            POLICY
        } else { REVOKED };
        raw_claim(&b, &service.subject, kind, OWNER, source);
        replicate(&b, &a);
        let inspected = projection_with_witness(&a, &assistant).unwrap();
        assert_eq!(inspected["cleared_conflicts"][0]["cause"], cause);
        a.store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-hb8b3d12c32f5 owner=fractal_api_core::api::client_v0::external_grants::conflict_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id) VALUES(?1)", [&service.id],
        ).unwrap();
        assert!(authenticate_id(&a, SERVICE_ID).unwrap().is_none());
        assert!(authenticate_id(&a, ASSISTANT_ID).unwrap().is_some());
        let inspected = projection_with_witness(&a, &assistant).unwrap();
        assert!(inspected["conflict"].is_null());
        assert_eq!(inspected["cleared_conflicts"], json!([]));
        assert_eq!(inspected["revoked"], false);
        assert_eq!(inspected["id"], ASSISTANT_ID);
    }
}
