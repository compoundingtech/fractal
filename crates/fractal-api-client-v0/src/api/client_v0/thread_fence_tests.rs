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

#[tokio::test]
async fn both_arrangement_actions_authorize_extra_thread_fences_before_the_writer() {
    for family in ["arrangement.edit", "arrangement.membership.edit"] {
        let root = tempfile::tempdir().unwrap();
        let state = tests::test_state_named(root.path(), "arrangement-thread-fence");
        let person = "person/operator";
        let arrangement = "arrangement/person/operator/019a0000-0000-7000-8000-000000000001";
        let own_thread = "thread/arrangement-owned";
        let private_thread = "thread/arrangement-private";
        for (thread, owner) in [(own_thread, person), (private_thread, "person/blair")] {
            crate::store::thread_metadata::apply(&state.store, owner, thread, thread,
                crate::store::thread_metadata::Edit::Create {
                    agent:"agent/reviewer",title:"Private sentinel title",
                }).unwrap();
        }
        let own_revision = crate::store::thread_metadata::summary(&state.store,person,own_thread).unwrap().revision;
        let private_revision = crate::store::thread_metadata::summary(&state.store,"person/blair",private_thread).unwrap().revision;
        let owner = ClientSession::local(Some(person)).unwrap();
        let request = |key: &str, selected: Option<(&str,&str)>, create: bool| -> ActionRequest {
            let subject_revisions = selected.into_iter().map(|(id,revision)|(id.to_owned(),revision.to_owned()))
                .collect::<BTreeMap<_,_>>();
            let operations = if create { json!([{"op":"create","name":"Layout"}]) }
                else if family == "arrangement.edit" { json!([{"op":"rename","name":key}]) }
                else { json!([{"op":"place","member":"agent/member","bucket":null,"key":"a0"}]) };
            let mut parameters = json!({"subject":arrangement,"owner":person,"operations":operations});
            if create { parameters["version"]=json!(2); }
            serde_json::from_value(json!({
                "api_version":CLIENT_API_VERSION,"id":format!("action/{key}"),
                "type":if create {"arrangement.edit"} else {family},
                "idempotency_key":format!("arrangement-thread-fence-{key}"),
                "fence":{"snapshot_id":new_client_snapshot(&state).id,"subject_revisions":subject_revisions},
                "parameters":parameters,
            })).unwrap()
        };
        let _ = action(State(state.clone()),Extension(new_client_snapshot(&state)),Extension(owner.clone()),
            Json(request("create",None,true))).await.unwrap();
        let before = state.store.claims_for(arrangement,None).unwrap().len();
        let denied = action(State(state.clone()),Extension(new_client_snapshot(&state)),Extension(owner.clone()),
            Json(request("foreign",Some((private_thread,"guessed-private-version")),false))).await.unwrap_err();
        assert_eq!(denied.status,StatusCode::NOT_FOUND,"{family}: {denied:?}");
        let rendered = format!("{denied:?}");
        assert!(!rendered.contains(&private_revision));
        assert!(!rendered.contains("current_head"));
        assert!(!rendered.contains("Private sentinel title"));
        let mut no_read = owner.clone();
        no_read.scopes.remove("read.projections");
        let denied = action(State(state.clone()),Extension(new_client_snapshot(&state)),Extension(no_read),
            Json(request("no-read",Some((own_thread,&own_revision)),false))).await.unwrap_err();
        assert_eq!(denied.status,StatusCode::FORBIDDEN,"{family}");
        assert_eq!(state.store.claims_for(arrangement,None).unwrap().len(),before);
        let _ = action(State(state.clone()),Extension(new_client_snapshot(&state)),Extension(owner),
            Json(request("owner-allowed",Some((own_thread,&own_revision)),false))).await.unwrap();

        let grant = "custom/client/pairing-arrangement-thread-fence";
        let completed = state.store.append_client_operation_claim(&ClaimInput {
            subject:grant.into(),kind:"custom.client.pairing-completed".into(),actor:Some(person.into()),
            fields:serde_json::from_value(json!({"person_id":person,"session_actor":"person/operator/session/arrangement",
                "device_id":"device/arrangement","credential_hash":"test-not-a-secret",
                "expires_at_unix_ms":client_now_ms()+60_000,"scopes":["read.projections","control.arrangements"]})).unwrap(),
            evidence:vec![],expected_subject:None,idempotency_key:None,
        }).unwrap();
        let paired = paired_client_session(&state,&completed,"fabric-loopback",false).unwrap();
        let paired_request = request("paired-allowed",Some((own_thread,&own_revision)),false);
        let _ = action(State(state.clone()),Extension(new_client_snapshot(&state)),Extension(paired.clone()),
            Json(paired_request.clone())).await.unwrap();
        state.store.append_client_operation_claim(&ClaimInput {
            subject:grant.into(),kind:"custom.client.pairing-revoked".into(),actor:Some(person.into()),
            fields:BTreeMap::new(),evidence:vec![],expected_subject:None,idempotency_key:None,
        }).unwrap();
        let before = state.store.claims_for(arrangement,None).unwrap().len();
        let denied = action(State(state.clone()),Extension(new_client_snapshot(&state)),Extension(paired.clone()),
            Json(request("revoked",Some((own_thread,&own_revision)),false))).await.unwrap_err();
        assert_eq!(denied.status,StatusCode::FORBIDDEN,"{family}");
        assert_eq!(state.store.claims_for(arrangement,None).unwrap().len(),before);
        // Generic action-result recovery must also reauthorize the extra thread read.
        let refused_replay = action(State(state.clone()),Extension(new_client_snapshot(&state)),Extension(paired),
            Json(paired_request)).await.unwrap_err();
        assert_eq!(refused_replay.status,StatusCode::FORBIDDEN,"{family} receipt recovery");
    }
}

#[tokio::test]
async fn arrangement_thread_authority_is_rechecked_after_preflight_before_writes_or_receipts() {
    for family in ["arrangement.edit", "arrangement.membership.edit"] {
        for interruption in ["revoke-before-write", "revoke-before-receipt", "owner-change"] {
            let root = tempfile::tempdir().unwrap();
            let state = tests::test_state_named(root.path(), "thread-authority-race");
            let person = "person/operator";
            let thread = "thread/arrangement-race";
            let arrangement = "arrangement/person/operator/019a0000-0000-7000-8000-000000000002";
            let (creation, _) = crate::store::thread_metadata::apply(&state.store,person,thread,"race-thread-create",
                crate::store::thread_metadata::Edit::Create { agent:"agent/reviewer",title:"Owned before preflight" }).unwrap();
            let version = crate::store::thread_metadata::summary(&state.store,person,thread).unwrap().revision;
            let owner = ClientSession::local(Some(person)).unwrap();
            let creation_request: ActionRequest = serde_json::from_value(json!({
                "api_version":CLIENT_API_VERSION,"id":"action/create-race-arrangement",
                "type":"arrangement.edit","idempotency_key":"create-race-arrangement",
                "fence":{"snapshot_id":new_client_snapshot(&state).id},
                "parameters":{"subject":arrangement,"owner":person,"version":2,
                    "operations":[{"op":"create","name":"Before race"}]},
            })).unwrap();
            arrangements::edit(&state,&owner,&creation_request).await.unwrap();
            let grant = "custom/client/pairing-arrangement-race";
            let completed = state.store.append_client_operation_claim(&ClaimInput {
                subject:grant.into(),kind:"custom.client.pairing-completed".into(),actor:Some(person.into()),
                fields:serde_json::from_value(json!({"person_id":person,"session_actor":"person/operator/session/race",
                    "device_id":"device/race","credential_hash":"test-not-a-secret",
                    "expires_at_unix_ms":client_now_ms()+60_000,
                    "scopes":["read.projections","control.arrangements"]})).unwrap(),
                evidence:vec![],expected_subject:None,idempotency_key:None,
            }).unwrap();
            let paired = paired_client_session(&state,&completed,"fabric-loopback",false).unwrap();
            assert!(!paired.allows("control.messages"),"thread fence reads must not require message control");
            let operations = if family == "arrangement.edit" { json!([{"op":"rename","name":"Accepted after preflight"}]) }
                else { json!([{"op":"place","member":"agent/race-member","bucket":null,"key":"a0"}]) };
            let request: ActionRequest = serde_json::from_value(json!({
                "api_version":CLIENT_API_VERSION,"id":"action/arrangement-thread-race",
                "type":family,"idempotency_key":"arrangement-thread-race",
                "fence":{"snapshot_id":new_client_snapshot(&state).id,"subject_revisions":{thread:version}},
                "parameters":{"subject":arrangement,"owner":person,"operations":operations},
            })).unwrap();
            validate_fence(&state,&request.fence,&paired).unwrap();
            // This is the deterministic barrier: the route has completed authorization,
            // but the actual API writer has not entered its admitting transaction yet.
            if interruption == "revoke-before-receipt" {
                if family == "arrangement.edit" { arrangements::edit(&state,&paired,&request).await.unwrap(); }
                else { arrangements::edit_memberships(&state,&paired,&request).await.unwrap(); }
            }
            if interruption == "owner-change" {
                let peer = Store::open_memory("ownership-peer").unwrap();
                peer.set_write_clock_at(creation.accepted_at_unix_ms+1_000).unwrap();
                crate::store::thread_metadata::apply(&peer,"person/blair",thread,"concurrent-owner",
                    crate::store::thread_metadata::Edit::Create { agent:"agent/private",title:"Foreign after preflight" }).unwrap();
                state.store.import_replication(peer.origin(),&peer.export_replication(0).unwrap()).unwrap();
            } else {
                state.store.append_client_operation_claim(&ClaimInput {
                    subject:grant.into(),kind:"custom.client.pairing-revoked".into(),actor:Some(person.into()),
                    fields:BTreeMap::new(),evidence:vec![],expected_subject:None,idempotency_key:None,
                }).unwrap();
            }
            let before = state.store.claims_for(arrangement,None).unwrap().len();
            let refused = if family == "arrangement.edit" { arrangements::edit(&state,&paired,&request).await }
                else { arrangements::edit_memberships(&state,&paired,&request).await }.unwrap_err();
            assert_eq!(refused.status,if interruption == "owner-change" { StatusCode::NOT_FOUND } else { StatusCode::FORBIDDEN },
                "{family}/{interruption}: {refused:?}");
            let details = format!("{refused:?}");
            assert!(!details.contains("current_head"));
            assert!(!details.contains("Foreign after preflight"));
            assert_eq!(state.store.claims_for(arrangement,None).unwrap().len(),before,
                "no append or receipt recovery after authority changed");
        }
    }
}
