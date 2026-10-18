use super::*;

const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

fn sync(source: &AppState, receiver: &AppState) {
    let exchange = source.store.export_replication_exchange_answering(
        FLEET,
        &receiver.store.replication_inventory().unwrap(),
        &receiver.store.replication_signature_requests().unwrap(),
    ).unwrap();
    receiver.store.receive_replication_exchange(source.store.origin(), FLEET, &exchange).unwrap();
    receiver.store.validate_replication_backlog().unwrap();
    receiver.store.project_replication_backlog().unwrap();
}

async fn caller_bound_grant(source: &AppState) -> Value {
    let mut request = request();
    request.minimal_read_mode = "caller-bound-allow".into();
    let Json(challenge) = begin(source.clone(), ClientSession::local(Some(ISSUER)).unwrap(), request).await.unwrap();
    let complete_request = serde_json::from_value(json!({
        "api_version": CLIENT_API_VERSION, "code": challenge["code"],
        "device_public_key": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8; 32]),
    })).unwrap();
    complete(source, challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap(),
        &complete_request, "fabric-loopback").await.unwrap().unwrap().0
}

async fn summary_status(receiver: &AppState, credential: &str) -> StatusCode {
    super::super::super::super::fabric_router(receiver.clone())
        .oneshot(Request::builder().uri("/v1/client/assistant/summary")
            .header(AUTHORIZATION, format!("Bearer {credential}"))
            .body(Body::empty()).unwrap()).await.unwrap().status()
}

async fn unsigned_lifecycle_is_refused(anchored: bool) {
    let source_root = tempfile::tempdir().unwrap();
    let receiver_root = tempfile::tempdir().unwrap();
    // Same name deliberately models a forged receiving-issuer origin, not a foreign grant
    // that session() would already reject. All state and credentials are isolated fixtures.
    let source = state(source_root.path());
    let receiver = state(receiver_root.path());
    source.store.bind_fleet(FLEET).unwrap();
    receiver.store.bind_fleet(FLEET).unwrap();
    if anchored {
        let anchor = crate::fleet::MemberKey::generate().unwrap().0;
        receiver.store.pin_fleet_anchor(anchor.public()).unwrap();
    }
    let grant = caller_bound_grant(&source).await;
    let credential = grant["credential"].as_str().unwrap();
    assert_eq!(summary_status(&source, credential).await, StatusCode::OK);
    let id = grant["grant"]["id"].as_str().unwrap();
    let grant_subject = subject(id).unwrap();
    let granted = latest(&source, &grant_subject, GRANTED).unwrap().unwrap();
    let mut policy = fields(&granted).unwrap().clone();
    policy["policy_revision"] = json!(2);
    append(&source, grant_subject.clone(), POLICY, ISSUER, policy, Some(&granted)).unwrap();
    let compatibility = source.store.append_claim(&ClaimInput {
        subject: "custom/legacy-compatibility".into(), kind: "custom.legacy.note".into(),
        actor: None, fields: serde_json::from_value(json!({"text":"legacy remains supported"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    sync(&source, &receiver);
    for kind in [BEGUN, GRANTED, POLICY] {
        assert!(latest(&receiver, &grant_subject, kind).unwrap().is_none(), "{kind}");
    }
    assert!(receiver.store.claim_by_id(&compatibility.id).unwrap().is_some());
    assert!(authenticate(&receiver, &credential_digest(credential), "fabric-loopback").unwrap().is_none());
    assert_eq!(summary_status(&receiver, credential).await, StatusCode::FORBIDDEN);
    revoke(State(source.clone()), Extension(ClientSession::local(Some(ISSUER)).unwrap()),
        AxumPath(id.to_owned())).await.unwrap();
    sync(&source, &receiver);
    assert!(latest(&receiver, &grant_subject, REVOKED).unwrap().is_none());
}

#[tokio::test]
async fn assistant_replication_no_anchor_refuses_forged_issuer_lifecycle_and_bearer() {
    unsigned_lifecycle_is_refused(false).await;
}

#[tokio::test]
async fn assistant_replication_anchored_legacy_refuses_forged_issuer_lifecycle_and_bearer() {
    unsigned_lifecycle_is_refused(true).await;
}

#[tokio::test]
async fn assistant_replication_keyed_issuer_grant_allows_caller_bound_read() {
    let source_root = tempfile::tempdir().unwrap();
    let receiver_root = tempfile::tempdir().unwrap();
    let source = state(source_root.path());
    let receiver = state(receiver_root.path());
    let member = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
    for state in [&source, &receiver] {
        state.store.bind_fleet(FLEET).unwrap();
        state.store.pin_fleet_anchor(member.public()).unwrap();
    }
    source.store.set_member_key(Some(member.clone())).unwrap();
    source.store.append_claim(&ClaimInput {
        subject: format!("host/{}", source.store.origin()), kind: "fleet.member-admitted".into(), actor: None,
        fields: serde_json::from_value(json!({"fleet_id":FLEET,"member_key":member.public(),
            "via":"anchor","mode":"listening"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let grant = caller_bound_grant(&source).await;
    sync(&source, &receiver);
    let credential = grant["credential"].as_str().unwrap();
    let grant_subject = subject(grant["grant"]["id"].as_str().unwrap()).unwrap();
    assert!(latest(&receiver, &grant_subject, GRANTED).unwrap().is_some());
    assert!(authenticate(&receiver, &credential_digest(credential), "fabric-loopback").unwrap().is_some());
    assert_eq!(summary_status(&receiver, credential).await, StatusCode::OK);
}
