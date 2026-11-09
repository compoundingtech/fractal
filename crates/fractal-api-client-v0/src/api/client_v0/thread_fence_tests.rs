use super::*;

#[test]
fn extra_thread_fences_use_the_authorized_semantic_version_and_recheck_revocation() {
    let root = tempfile::tempdir().unwrap();
    let state = tests::test_state_named(root.path(), "thread-fence");
    let person = "person/operator";
    let thread = "thread/fence";
    let (created, _) = crate::store::thread_metadata::apply(
        &state.store, person, thread, "create-fence",
        crate::store::thread_metadata::Edit::Create { agent: "agent/reviewer", title: "Private title" },
    ).unwrap();
    let current = crate::store::thread_metadata::summary(&state.store, person, thread).unwrap();
    let fence: Fence = serde_json::from_value(json!({
        "snapshot_id":new_client_snapshot(&state).id,
        "subject_revisions":{thread:current.revision},
    })).unwrap();
    let owner = ClientSession::local(Some(person)).unwrap();
    assert!(validate_fence(&state, &fence, &owner).is_ok());
    let mut raw = fence.clone();
    raw.subject_revisions.insert(thread.into(), created.id);
    assert!(validate_fence(&state, &raw, &owner).is_err());
    let mut no_read = owner.clone();
    no_read.scopes.remove("read.projections");
    assert_eq!(validate_fence(&state, &fence, &no_read).unwrap_err().status, StatusCode::FORBIDDEN);
    let other = ClientSession::local(Some("person/other")).unwrap();
    let denied = validate_fence(&state, &fence, &other).unwrap_err();
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
    assert!(!denied.message.contains("Private title"));
    let grant = "custom/client/pairing-extra-thread-fence";
    let completed = state.store.append_client_operation_claim(&ClaimInput {
        subject: grant.into(), kind: "custom.client.pairing-completed".into(), actor: Some(person.into()),
        fields: serde_json::from_value(json!({
            "person_id":person,"session_actor":"person/operator/session/fence",
            "device_id":"device/fence","credential_hash":"test-not-a-secret",
            "expires_at_unix_ms":client_now_ms()+60_000,"scopes":["read.projections"],
        })).unwrap(), evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    let paired = paired_client_session(&state, &completed, "fabric-loopback", false).unwrap();
    assert!(validate_fence(&state, &fence, &paired).is_ok());
    state.store.append_client_operation_claim(&ClaimInput {
        subject:grant.into(),kind:"custom.client.pairing-revoked".into(),actor:Some(person.into()),
        fields:BTreeMap::new(),evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert_eq!(validate_fence(&state, &fence, &paired).unwrap_err().status, StatusCode::FORBIDDEN);

    let ordinary_message = |id: &str| ClaimInput {
        subject:format!("message/{id}"),kind:"message.sent".into(),actor:Some(person.into()),
        fields:serde_json::from_value(json!({"from":person,"to":"agent/reviewer",
            "content":"Fenced ordinary message","status":"sent"})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:Some(id.into()),
    };
    crate::store::append_claim_with_commit_context(&state.store.graph, &ordinary_message("before-change"),
        None,None,Some(&fence.subject_revisions),None,Default::default()).unwrap();
    crate::store::thread_metadata::apply(&state.store, person, thread, "close-fence",
        crate::store::thread_metadata::Edit::Close { revision: &current.revision }).unwrap();
    let error = crate::store::append_claim_with_commit_context(&state.store.graph,
        &ordinary_message("after-change"),None,None,Some(&fence.subject_revisions),None,Default::default()).unwrap_err();
    assert_eq!(error.code, "stale-subject", "the admitting writer also uses the semantic thread fence");
    assert!(state.store.claims_for("message/after-change",None).unwrap().is_empty());
}
