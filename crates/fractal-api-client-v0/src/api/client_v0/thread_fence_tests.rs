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

pub(crate) mod retained_discussion {
    use super::*;
    use crate::store::thread_metadata::{self, InboxOrigin, ThreadContext};

    const PERSON: &str = "person/operator";
    const AGENT: &str = "agent/garden/seed";
    const SOURCE: &str = "custom/garden/review/v1/retained";

    fn fixture(resolved: bool) -> (tempfile::TempDir, AppState, InboxOrigin) {
        let root = tempfile::tempdir().unwrap();
        let state = tests::test_state_named(root.path(), "retained-discussion");
        let origin = install_source(&state, resolved);
        (root, state, origin)
    }

    pub(crate) fn install_source(state: &AppState, resolved: bool) -> InboxOrigin {
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"garden/seed\" { workspace \"/work/garden\"; harness \"omp\" {} }",
            state.store.origin(),
        ).unwrap();
        state.store.apply_internal(&intent, "retained-declare").unwrap();
        state.store.configure_thread_write_rollout([]);
        state.store.register_custom_kind(&crate::store::custom::RegistrationRequest {
            manifest: serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h3b1364f4650e owner=fractal_api_client_v0::api::client_v0::thread_fence_tests TODO=trace-statement-and-value-representation */ include_str!("../../../../../examples/fractal/custom-review.json")).unwrap(),
            actor: AGENT.into(),
        }).unwrap();
        let opening = state.store.append_claim(&ClaimInput {
            subject: SOURCE.into(), kind: "custom.garden.review.v1.requested".into(),
            actor: Some(AGENT.into()),
            fields: serde_json::from_value(json!({
                "title":"Retained review", "detail":"Choose a path", "recipient":PERSON,
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: Some("retained-open".into()),
        }).unwrap();
        let origin = InboxOrigin { source_id: SOURCE.into(), episode: opening.id };
        if resolved {
            state.store.append_claim(&ClaimInput {
                subject: SOURCE.into(), kind: "custom.garden.review.v1.answered".into(),
                actor: Some(PERSON.into()),
                fields: serde_json::from_value(json!({
                    "request":origin.episode, "selection":"keep", "text":"Proceed.",
                })).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: Some("retained-answer".into()),
            }).unwrap();
        }
        origin
    }

    pub(crate) fn retire_original_with_fallback(state: &AppState) {
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"garden/fallback\" { workspace \"/work/garden\"; harness \"omp\" {} }",
            state.store.origin(),
        ).unwrap();
        state.store.apply_internal(&intent, "retained-fallback-declare").unwrap();
        state.store.configure_inbox_fallback_agent(Some("agent/garden/fallback")).unwrap();
        let token = state.store.selected_desired_token(AGENT).unwrap().unwrap();
        let (retirement, appended) = state.store.retire_agent(
            AGENT, PERSON, "Original responsibility ended", &token, None, "retained-retire-original", &[],
        ).unwrap();
        assert!(appended);
        assert_eq!(retirement.kind, "agent.retired");
        assert_eq!(retirement.body["fields"]["declaration"], token);
    }

    pub(crate) fn fallback_request(state: &AppState, origin: &InboxOrigin, thread: &str, key: &str) -> ActionRequest {
        let mut request = request(state, origin, thread, key);
        request.parameters["agent_id"] = json!("agent/garden/fallback");
        request
    }

    pub(crate) fn assert_first_result(replay: &Value, first: &Value) {
        assert_eq!(replay["affected_ids"], first["affected_ids"]);
        assert_eq!(replay["operation_id"], first["operation_id"]);
    }

    fn assert_first_message(state: &AppState, result: &Value, content: &str) {
        let message = result["affected_ids"][1].as_str().unwrap();
        let claims = state.store.claims_for(message, Some("message.sent")).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].body["fields"]["content"], content);
    }

    pub(crate) fn request(state: &AppState, origin: &InboxOrigin, thread: &str, key: &str) -> ActionRequest {
        serde_json::from_value(json!({
            "api_version":CLIENT_API_VERSION, "id":format!("action/{key}"),
            "type":"thread.start", "idempotency_key":format!("fixture-thread:{key}"),
            "fence":{"snapshot_id":new_client_snapshot(state).id},
            "parameters":{"target_id":thread, "agent_id":AGENT,
                "origin":origin, "title":"Discuss retained review", "content":"Explain the decision."},
        })).unwrap()
    }

    pub(crate) async fn start(state: &AppState, session: &ClientSession, request: ActionRequest) -> Result<Value, ApiError> {
        threads::message_action(state.clone(), session.clone(), request).await.map(|result| result.0)
    }

    fn assert_not_in_attention(state: &AppState) {
        assert!(!state.store.attention_items(Some(PERSON)).unwrap().iter()
            .any(|item| item.subject == SOURCE));
    }

    #[tokio::test]
    async fn resolved_custom_start_replays_same_key_but_new_key_preserves_independent_message() {
        let (_root, state, origin) = fixture(true);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        assert_not_in_attention(&state);
        let first_request = request(&state, &origin, "thread/retained-first", "retained-first");
        let first = start(&state, &owner, first_request.clone()).await.unwrap();
        let replay = start(&state, &owner, first_request.clone()).await.unwrap();
        assert_eq!(replay["affected_ids"], first["affected_ids"]);
        assert_eq!(replay["operation_id"], first["operation_id"]);
        let mut conflict = first_request;
        conflict.parameters["content"] = json!("Changed same-key content.");
        assert_eq!(start(&state, &owner, conflict).await.unwrap_err().status, StatusCode::CONFLICT);
        let mut second_request = request(&state, &origin, "thread/retained-second", "retained-different");
        second_request.parameters["content"] = json!("A distinct retained discussion.");
        let second = start(&state, &owner, second_request.clone()).await.unwrap();
        assert_ne!(second["affected_ids"], first["affected_ids"]);
        assert_ne!(second["operation_id"], first["operation_id"]);
        assert_eq!(second["affected_ids"][0], "thread/retained-second");
        assert_first_result(&start(&state, &owner, second_request).await.unwrap(), &second);
        assert_first_message(&state, &first, "Explain the decision.");
        assert_first_message(&state, &second, "A distinct retained discussion.");
        let history = thread_metadata::list_context(&state.store, PERSON,
            ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap();
        assert_eq!(history.items.len(), 2);
        for thread in ["thread/retained-first", "thread/retained-second"] {
            assert_eq!(thread_metadata::summary(&state.store, PERSON, thread).unwrap().message_count, 1);
        }
        assert_not_in_attention(&state);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_retained_starts_serialize_through_real_store_writer_and_create_independent_messages() {
        let (_root, state, origin) = fixture(true);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let mut a = request(&state, &origin, "thread/retained-race-a", "retained-race-a");
        a.parameters["content"] = json!("Concurrent discussion A.");
        let mut b = request(&state, &origin, "thread/retained-race-b", "retained-race-b");
        b.parameters["content"] = json!("Concurrent discussion B.");
        // Different action keys bypass the per-key API lock; both reach the batched store writer.
        let (a, b) = tokio::join!(start(&state, &owner, a), start(&state, &owner, b));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a["affected_ids"], b["affected_ids"]);
        assert_ne!(a["operation_id"], b["operation_id"]);
        assert_eq!(a["affected_ids"][0], "thread/retained-race-a");
        assert_eq!(b["affected_ids"][0], "thread/retained-race-b");
        assert_first_message(&state, &a, "Concurrent discussion A.");
        assert_first_message(&state, &b, "Concurrent discussion B.");
        let history = thread_metadata::list_context(&state.store, PERSON,
            ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap();
        assert_eq!(history.items.len(), 2);
        for thread in ["thread/retained-race-a", "thread/retained-race-b"] {
            assert_eq!(thread_metadata::summary(&state.store, PERSON, thread).unwrap().message_count, 1);
        }
        let creations = ["thread/retained-race-a", "thread/retained-race-b"].iter()
            .map(|thread| state.store.claims_for(thread, Some("thread.created")).unwrap().len()).sum::<usize>();
        assert_eq!(creations, 2);
        assert_not_in_attention(&state);
    }

    #[tokio::test]
    async fn retained_custom_rejects_wrong_person_fabricated_and_cross_source_episodes() {
        let (_root, state, origin) = fixture(true);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let other = ClientSession::local(Some("person/other")).unwrap();
        let error = start(&state, &other, request(&state, &origin, "thread/denied-owner", "denied-owner")).await.unwrap_err();
        assert_eq!(error.status, StatusCode::NOT_FOUND);
        for (key, denied) in [
            ("fabricated", InboxOrigin { episode:"claim/never-opened".into(), ..origin.clone() }),
            ("foreign", InboxOrigin { source_id:"custom/garden/review/v1/never-opened".into(), ..origin.clone() }),
        ] {
            let thread = format!("thread/denied-{key}");
            let error = start(&state, &owner, request(&state, &denied, &thread, key)).await.unwrap_err();
            assert_eq!(error.status, StatusCode::NOT_FOUND);
            assert!(state.store.claims_for(&thread, None).unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn current_custom_start_and_reply_remain_live_and_do_not_deduplicate_origins() {
        let (_root, state, origin) = fixture(false);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        for key in ["current-a", "current-b"] {
            start(&state, &owner, request(&state, &origin, &format!("thread/{key}"), key)).await.unwrap();
        }
        let reply: ActionRequest = serde_json::from_value(json!({
            "api_version":CLIENT_API_VERSION, "id":"action/current-reply", "type":"thread.reply",
            "idempotency_key":"fixture-thread:current-reply", "fence":{"snapshot_id":new_client_snapshot(&state).id},
            "parameters":{"target_id":"thread/current-a", "content":"A current follow-up."},
        })).unwrap();
        start(&state, &owner, reply).await.unwrap();
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/current-a").unwrap().message_count, 2);
        assert_eq!(thread_metadata::list_context(&state.store, PERSON,
            ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap().items.len(), 2);
        assert!(state.store.attention_items(Some(PERSON)).unwrap().iter().any(|item| item.subject == SOURCE));
    }

    #[tokio::test]
    async fn revoked_device_cannot_start_or_recover_retained_threads() {
        let (_root, state, origin) = fixture(true);
        let grant = "custom/client/pairing-retained-device";
        let completed = state.store.append_client_operation_claim(&ClaimInput {
            subject:grant.into(), kind:"custom.client.pairing-completed".into(), actor:Some(PERSON.into()),
            fields:serde_json::from_value(json!({
                "person_id":PERSON, "session_actor":"person/operator/session/retained",
                "device_id":"device/retained", "credential_hash":"test-not-a-secret",
                "expires_at_unix_ms":client_now_ms()+60_000, "scopes":["read.projections","control.messages"],
            })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
        let paired = paired_client_session(&state, &completed, "fabric-loopback", false).unwrap();
        let expiry = u128::from(completed.body["fields"]["expires_at_unix_ms"].as_u64().unwrap());
        assert!(paired_client_session_at(&state, &completed, "fabric-loopback", false, expiry - 1).is_ok());
        assert!(paired_client_session_at(&state, &completed, "fabric-loopback", false, expiry).is_err());
        let first = request(&state, &origin, "thread/retained-device", "retained-device");
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        start(&state, &owner, first.clone()).await.unwrap();
        state.store.append_client_operation_claim(&ClaimInput {
            subject:grant.into(), kind:"custom.client.pairing-revoked".into(), actor:Some(PERSON.into()),
            fields:BTreeMap::new(), evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
        for denied in [first, request(&state, &origin, "thread/retained-device-denied", "retained-device-denied")] {
            assert_eq!(start(&state, &paired, denied).await.unwrap_err().status, StatusCode::FORBIDDEN);
        }
        assert!(state.store.claims_for("thread/retained-device-denied", None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolved_gate_review_starts_after_its_real_owner_step_completes() {
        let (_root, state, _) = fixture(true);
        state.store.configure_inbox_fallback_agent(Some(AGENT)).unwrap();
        let intent = crate::graph::parse_intent(
            "version 2\nmission \"retained-gate\" state=\"ready\" { goal \"Review work\"; step \"review\" { goal \"Approve work\"; agentless } }",
            state.store.origin(),
        ).unwrap();
        state.store.apply_internal(&intent, "retained-gate-mission").unwrap();
        let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
            mission:"retained-gate".into(), revision:None, workspace:"/work/garden".into(),
            requester:Some(PERSON.into()), mode:Some("run".into()), inputs:BTreeMap::new(),
            idempotency_key:"retained-gate-run".into(),
        }).unwrap();
        let step = &run.steps[0];
        let opening = state.store.append_claim(&ClaimInput {
            subject:"gate-operation/retained-discussion/review".into(), kind:"gate.requested".into(), actor:None,
            fields:serde_json::from_value(json!({
                "owner":step.subject, "reviewer":PERSON, "question":"Approve work?",
                "review_targets":[], "decisions":["approved","rejected"],
                "operation":"gate-operation/retained-discussion/review",
                "mission_revision":run.revision, "step_definition":step.definition_hash, "attempt":step.attempt,
            })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:Some("retained-gate-open".into()),
        }).unwrap();
        assert_eq!(state.store.pending_human_reviews(Some(PERSON)).unwrap().len(), 1);
        state.store.set_step_state(&step.subject, "completed", None).unwrap();
        assert!(state.store.pending_human_reviews(Some(PERSON)).unwrap().is_empty());
        let origin = InboxOrigin { source_id:step.subject.clone(), episode:opening.id };
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let before = state.store.read_snapshot(Ok).unwrap();
        let (status, preview) = opening_preview(&state, PERSON, &origin).await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert_eq!(preview["value"]["source_id"], origin.source_id);
        assert_eq!(preview["value"]["episode"], origin.episode);
        assert_eq!(preview["value"]["start_recipient"],
            json!({"agent_id":AGENT,"basis":{"_tag":"configured"}}));
        assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
        let result = start(&state, &owner, request(&state, &origin, "thread/retained-gate", "retained-gate")).await.unwrap();
        assert_eq!(result["affected_ids"][0], "thread/retained-gate");
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/retained-gate").unwrap().message_count, 1);
        assert!(state.store.pending_human_reviews(Some(PERSON)).unwrap().is_empty());
    }

    #[tokio::test]
    async fn retained_start_creates_independent_thread_despite_existing_current_source_history() {
        let (_root, state, origin) = fixture(false);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let older = start(&state, &owner, request(&state, &origin, "thread/newest-old", "newest-old")).await.unwrap();
        let newer = start(&state, &owner, request(&state, &origin, "thread/newest-new", "newest-new")).await.unwrap();
        assert_ne!(older["affected_ids"], newer["affected_ids"]);
        state.store.append_claim(&ClaimInput {
            subject:SOURCE.into(), kind:"custom.garden.review.v1.answered".into(), actor:Some(PERSON.into()),
            fields:serde_json::from_value(json!({"request":origin.episode,"selection":"keep","text":"Proceed."})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:Some("newest-answer".into()),
        }).unwrap();
        let mut retained_request = request(&state, &origin, "thread/after-resolution", "after-resolution");
        retained_request.parameters["content"] = json!("Independent after resolution.");
        let retained = start(&state, &owner, retained_request).await.unwrap();
        assert_eq!(retained["affected_ids"][0], "thread/after-resolution");
        for previous in [&older, &newer] {
            assert_ne!(retained["affected_ids"], previous["affected_ids"]);
            assert_ne!(retained["operation_id"], previous["operation_id"]);
        }
        assert_first_message(&state, &retained, "Independent after resolution.");
        assert_eq!(thread_metadata::list_context(&state.store, PERSON,
            ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap().items.len(), 3);
        assert_not_in_attention(&state);
    }

    #[tokio::test]
    async fn resolved_literal_authored_step_with_proven_opening_can_start_and_list_history() {
        let (_root, state, _) = fixture(true);
        state.store.configure_inbox_fallback_agent(Some(AGENT)).unwrap();
        let intent = crate::graph::parse_intent(
            "version 2\nmission \"never-opened-authored\" state=\"ready\" { goal \"Choose\"; step \"review\" { assigned-to \"person/operator\"; goal \"Choose path\"; } }",
            state.store.origin(),
        ).unwrap();
        state.store.apply_internal(&intent, "never-opened-authored-mission").unwrap();
        let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
            mission:"never-opened-authored".into(), revision:None, workspace:"/work/garden".into(),
            requester:Some(PERSON.into()), mode:Some("run".into()), inputs:BTreeMap::new(),
            idempotency_key:"never-opened-authored-run".into(),
        }).unwrap();
        let step = &run.steps[0];
        state.store.set_step_state(&step.subject, "ready", None).unwrap();
        let current = state.store.attention_items(Some(PERSON)).unwrap().into_iter()
            .find(|item| item.subject == step.subject).unwrap();
        state.store.set_step_state(&step.subject, "completed", None).unwrap();
        let origin = InboxOrigin { source_id:step.subject.clone(), episode:current.episode };
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let never_opened = InboxOrigin {
            source_id: origin.source_id.clone(),
            episode: format!("{}:99:99", origin.episode.rsplit_once(':').unwrap().0.rsplit_once(':').unwrap().0),
        };
        let denied = start(&state, &owner, request(&state, &never_opened,
            "thread/never-opened-authored", "never-opened-authored")).await.unwrap_err();
        assert_eq!(denied.status, StatusCode::NOT_FOUND);
        assert!(state.store.claims_for("thread/never-opened-authored", None).unwrap().is_empty());
        let result = start(&state, &owner, request(&state, &origin,
            "thread/resolved-authored", "resolved-authored")).await.unwrap();
        assert_eq!(result["affected_ids"][0], "thread/resolved-authored");
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/resolved-authored").unwrap().message_count, 1);
        let history = thread_metadata::list_context(&state.store, PERSON,
            ThreadContext::Inbox { source:&origin.source_id, episode:&origin.episode }, None, 20).unwrap();
        assert_eq!(history.items.len(), 1);
        assert!(!state.store.attention_items(Some(PERSON)).unwrap().iter()
            .any(|item| item.subject == step.subject));
    }

    #[tokio::test]
    async fn manifest_derived_custom_episode_retains_authority_but_unproven_episode_still_refuses() {
        let root = tempfile::tempdir().unwrap();
        let state = tests::test_state_named(root.path(), "non-claim-custom-episode");
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"garden/seed\" { workspace \"/work/garden\"; harness \"omp\" {} }",
            state.store.origin(),
        ).unwrap();
        state.store.apply_internal(&intent, "non-claim-declare").unwrap();
        state.store.configure_thread_write_rollout([]);
        let mut manifest: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h9cebb1c8e2d9 owner=fractal_api_client_v0::api::client_v0::thread_fence_tests TODO=trace-statement-and-value-representation */ include_str!("../../../../../examples/fractal/custom-review.json")).unwrap();
        manifest["attention"]["episode"] = json!({"op":"field","slot":"request","field":"title"});
        state.store.register_custom_kind(&crate::store::custom::RegistrationRequest {
            manifest:serde_json::from_value(manifest).unwrap(), actor:AGENT.into(),
        }).unwrap();
        let opening = state.store.append_claim(&ClaimInput {
            subject:SOURCE.into(), kind:"custom.garden.review.v1.requested".into(), actor:Some(AGENT.into()),
            fields:serde_json::from_value(json!({"title":"non-claim-episode","detail":"Choose","recipient":PERSON})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:Some("non-claim-open".into()),
        }).unwrap();
        let origin = InboxOrigin { source_id:SOURCE.into(), episode:"non-claim-episode".into() };
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let unproven = InboxOrigin { episode:"unproven-title-episode".into(), ..origin.clone() };
        let denied = start(&state, &owner, request(&state, &unproven,
            "thread/non-claim-current-denied", "non-claim-current-denied")).await.unwrap_err();
        assert_eq!(denied.status, StatusCode::NOT_FOUND);
        let current = start(&state, &owner,
            request(&state, &origin, "thread/non-claim-current", "non-claim-current")).await.unwrap();
        state.store.append_claim(&ClaimInput {
            subject:SOURCE.into(), kind:"custom.garden.review.v1.answered".into(), actor:Some(PERSON.into()),
            fields:serde_json::from_value(json!({"request":opening.id,"selection":"keep","text":"Proceed."})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:Some("non-claim-answer".into()),
        }).unwrap();
        let retained = start(&state, &owner, request(&state, &origin,
            "thread/non-claim-retained", "non-claim-retained")).await.unwrap();
        assert_ne!(retained["affected_ids"], current["affected_ids"]);
        assert_ne!(retained["operation_id"], current["operation_id"]);
        assert_eq!(retained["affected_ids"][0], "thread/non-claim-retained");
        assert_first_message(&state, &retained, "Explain the decision.");
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/non-claim-retained").unwrap().message_count, 1);
        let denied = start(&state, &owner, request(&state, &unproven,
            "thread/non-claim-retained-denied", "non-claim-retained-denied")).await.unwrap_err();
        assert_eq!(denied.status, StatusCode::NOT_FOUND);
        for thread in ["thread/non-claim-current-denied", "thread/non-claim-retained-denied"] {
            assert!(state.store.claims_for(thread, None).unwrap().is_empty());
        }
        assert_not_in_attention(&state);
    }

    #[tokio::test]
    async fn expired_persisted_device_grant_cannot_start_a_retained_discussion() {
        let (_root, state, origin) = fixture(true);
        let expired_at = client_now_ms() - 1;
        let completed = state.store.append_client_operation_claim(&ClaimInput {
            subject:"custom/client/pairing-expired-retained".into(),
            kind:"custom.client.pairing-completed".into(), actor:Some(PERSON.into()),
            fields:serde_json::from_value(json!({
                "person_id":PERSON, "session_actor":"person/operator/session/expired-retained",
                "device_id":"device/expired-retained", "credential_hash":"test-not-a-secret",
                "expires_at_unix_ms":expired_at, "scopes":["read.projections","control.messages"],
            })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
        // Recover the session at its actual valid side of the expiry boundary, then pass
        // that cached session to the API at wall-clock now. No fabricated session fields.
        let session = paired_client_session_at(&state, &completed, "fabric-loopback", false, expired_at - 1).unwrap();
        let denied = start(&state, &session, request(&state, &origin,
            "thread/expired-retained", "expired-retained")).await.unwrap_err();
        assert_eq!(denied.status, StatusCode::FORBIDDEN);
        assert!(state.store.claims_for("thread/expired-retained", None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn retained_discussion_unread_positions_batch_exact_episodes_without_restoring_attention() {
        let (_root, state, origin) = fixture(true);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let result = start(&state, &owner,
            request(&state, &origin, "thread/retained-unread", "retained-unread")).await.unwrap();
        let thread = result["affected_ids"][0].as_str().unwrap();
        crate::store::thread_delivery::append_reply(&state.store, &ClaimInput {
            subject:"message/retained-unread-incoming".into(), kind:"message.sent".into(),
            actor:Some(AGENT.into()),
            fields:serde_json::from_value(json!({
                "from":AGENT, "to":PERSON, "content":"The retained discussion reply.",
                "thread_id":thread, "status":"sent",
            })).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:Some("retained-unread-incoming".into()),
        }, thread_metadata::PersonAuthority::BoundAgent { actor:AGENT }, None).unwrap();
        let keys = [
            (SOURCE, origin.episode.as_str()),
            (SOURCE, "claim/never-opened"),
            ("custom/garden/review/v1/foreign", origin.episode.as_str()),
            (SOURCE, origin.episode.as_str()),
        ];
        assert_eq!(thread_metadata::unread_inbox_positions(&state.store, PERSON, &keys).unwrap(),
            std::collections::BTreeSet::from([0, 3]));
        assert!(thread_metadata::unread_inbox_positions(&state.store, "person/other", &keys).unwrap().is_empty());
        assert_not_in_attention(&state);
    }

    #[tokio::test]
    async fn retained_discussion_prefers_participating_original_agent_over_configured_fallback() {
        let (_root, state, origin) = fixture(true);
        state.store.configure_inbox_fallback_agent(Some("agent/garden/fallback")).unwrap();
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let mut wrong = request(&state, &origin, "thread/retained-wrong-fallback", "retained-wrong-fallback");
        wrong.parameters["agent_id"] = json!("agent/garden/fallback");
        assert_eq!(start(&state, &owner, wrong).await.unwrap_err().status, StatusCode::NOT_FOUND);
        let actual = start(&state, &owner,
            request(&state, &origin, "thread/retained-original", "retained-original")).await.unwrap();
        assert_eq!(actual["affected_ids"][0], "thread/retained-original");
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/retained-original").unwrap().agent_id, AGENT);
    }

    #[tokio::test]
    async fn terminal_retirement_uses_fallback_for_current_and_retained_sources() {
        for resolved in [false, true] {
            let (_root, state, origin) = fixture(resolved);
            retire_original_with_fallback(&state);
            let owner = ClientSession::local(Some(PERSON)).unwrap();
            let stale = request(&state, &origin, "thread/retired-original-denied", "retired-original-denied");
            assert_eq!(start(&state, &owner, stale).await.unwrap_err().status, StatusCode::NOT_FOUND);
            let first = fallback_request(&state, &origin, "thread/terminal-fallback", "terminal-fallback");
            let actual = start(&state, &owner, first.clone()).await.unwrap();
            assert_eq!(actual["affected_ids"][0], "thread/terminal-fallback");
            let summary = thread_metadata::summary(&state.store, PERSON, "thread/terminal-fallback").unwrap();
            assert_eq!(summary.agent_id, "agent/garden/fallback");
            assert_eq!(summary.message_count, 1);
            assert_first_result(&start(&state, &owner, first).await.unwrap(), &actual);
            assert!(state.store.claims_for("thread/retired-original-denied", None).unwrap().is_empty());
            assert_eq!(state.store.attention_items(Some(PERSON)).unwrap().iter()
                .any(|item| item.subject == SOURCE), !resolved);
        }
    }

    #[tokio::test]
    async fn terminal_retirement_preserves_frozen_thread_and_replay_but_new_keys_create_independent_fallback_threads() {
        for resolved in [false, true] {
            let (_root, state, origin) = fixture(resolved);
            let owner = ClientSession::local(Some(PERSON)).unwrap();
            let original_request = request(&state, &origin, "thread/before-retirement", "before-retirement");
            let original = start(&state, &owner, original_request.clone()).await.unwrap();
            let before = thread_metadata::summary(&state.store, PERSON, "thread/before-retirement").unwrap();
            retire_original_with_fallback(&state);
            assert_first_result(&start(&state, &owner, original_request).await.unwrap(), &original);
            let fallback = fallback_request(&state, &origin, "thread/after-retirement", "after-retirement");
            let created = start(&state, &owner, fallback.clone()).await.unwrap();
            assert_eq!(created["affected_ids"][0], "thread/after-retirement");
            assert_ne!(created["affected_ids"], original["affected_ids"]);
            assert_first_result(&start(&state, &owner, fallback).await.unwrap(), &created);
            let old = thread_metadata::summary(&state.store, PERSON, "thread/before-retirement").unwrap();
            assert_eq!(old.agent_id, AGENT);
            assert_eq!(old.revision, before.revision);
            assert_eq!(old.message_count, 1);
            assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/after-retirement").unwrap().agent_id,
                "agent/garden/fallback");
            let history = thread_metadata::list_context(&state.store, PERSON,
                ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap();
            assert_eq!(history.items.len(), 2);
            let mut another_request = fallback_request(&state, &origin,
                "thread/after-retirement-independent", "after-retirement-new-key");
            another_request.parameters["content"] = json!("A separate fallback discussion.");
            let another = start(&state, &owner, another_request).await.unwrap();
            assert_eq!(another["affected_ids"][0], "thread/after-retirement-independent");
            assert_ne!(another["affected_ids"], created["affected_ids"]);
            assert_ne!(another["operation_id"], created["operation_id"]);
            assert_first_message(&state, &created, "Explain the decision.");
            assert_first_message(&state, &another, "A separate fallback discussion.");
            assert_eq!(thread_metadata::summary(&state.store, PERSON,
                "thread/after-retirement-independent").unwrap().agent_id, "agent/garden/fallback");
            assert_eq!(thread_metadata::list_context(&state.store, PERSON,
                ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap().items.len(), 3);
            if resolved { assert_not_in_attention(&state); }
        }
    }

    #[tokio::test]
    async fn offline_and_unknown_originals_are_not_terminal_retirements() {
        for resolved in [false, true] {
            for availability in ["unknown-runtime", "stopped", "unreachable", "undeclared"] {
                let (_root, state, origin) = fixture(resolved);
                let intent = crate::graph::parse_intent(
                    "version 2\nagent \"garden/fallback\" { workspace \"/work/garden\"; harness \"omp\" {} }",
                    state.store.origin(),
                ).unwrap();
                state.store.apply_internal(&intent, "availability-fallback-declare").unwrap();
                state.store.configure_inbox_fallback_agent(Some("agent/garden/fallback")).unwrap();
                let origin = if availability == "undeclared" {
                    let opening = state.store.append_claim(&ClaimInput {
                        subject:"custom/garden/review/v1/unknown-original".into(),
                        kind:"custom.garden.review.v1.requested".into(), actor:Some("agent/garden/undeclared".into()),
                        fields:serde_json::from_value(json!({
                            "title":"Unknown original", "detail":"Explain", "recipient":PERSON,
                        })).unwrap(),
                        evidence:vec![], expected_subject:None, idempotency_key:Some("unknown-original-open".into()),
                    }).unwrap();
                    if resolved {
                        state.store.append_claim(&ClaimInput {
                            subject:opening.subject.clone(), kind:"custom.garden.review.v1.answered".into(),
                            actor:Some(PERSON.into()), fields:serde_json::from_value(json!({
                                "request":opening.id, "selection":"keep", "text":"Proceed.",
                            })).unwrap(),
                            evidence:vec![], expected_subject:None, idempotency_key:Some("unknown-original-answer".into()),
                        }).unwrap();
                    }
                    InboxOrigin { source_id:opening.subject, episode:opening.id }
                } else {
                    if availability == "stopped" {
                        state.store.append_claim(&ClaimInput {
                            subject:AGENT.into(), kind:"runtime.observed".into(), actor:Some(AGENT.into()),
                            fields:serde_json::from_value(json!({
                                "status":"stopped", "incarnation_id":"offline-original",
                            })).unwrap(),
                            evidence:vec![], expected_subject:None, idempotency_key:None,
                        }).unwrap();
                    } else if availability == "unreachable" {
                        state.store.append_claim(&ClaimInput {
                            subject:AGENT.into(), kind:"agent.presence".into(), actor:Some(AGENT.into()),
                            fields:serde_json::from_value(json!({
                                "presence":"offline", "reachability":"unreachable",
                            })).unwrap(),
                            evidence:vec![], expected_subject:None, idempotency_key:None,
                        }).unwrap();
                    }
                    origin
                };
                let original_agent = if availability == "undeclared" { "agent/garden/undeclared" } else { AGENT };
                let owner = ClientSession::local(Some(PERSON)).unwrap();
                let refused = fallback_request(&state, &origin, "thread/availability-fallback", "availability-fallback");
                assert_eq!(start(&state, &owner, refused).await.unwrap_err().status, StatusCode::NOT_FOUND);
                let mut original = request(&state, &origin, "thread/availability-original", "availability-original");
                original.parameters["agent_id"] = json!(original_agent);
                if availability == "undeclared" {
                    assert_eq!(start(&state, &owner, original).await.unwrap_err().status, StatusCode::NOT_FOUND);
                } else {
                    let before = state.store.read_snapshot(Ok).unwrap();
                    let (status, preview) = opening_preview(&state, PERSON, &origin).await;
                    assert_eq!(status, StatusCode::OK, "{availability}: {preview}");
                    assert_eq!(preview["value"]["start_recipient"],
                        json!({"agent_id":original_agent,"basis":{"_tag":"original"}}));
                    assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
                    original.parameters["agent_id"] = preview["value"]["start_recipient"]["agent_id"].clone();
                    let result = start(&state, &owner, original).await.unwrap();
                    assert_eq!(result["affected_ids"][0], "thread/availability-original");
                    assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/availability-original").unwrap().agent_id,
                        original_agent);
                }
                assert!(state.store.claims_for("thread/availability-fallback", None).unwrap().is_empty());
                assert_eq!(state.store.attention_items(Some(PERSON)).unwrap().iter()
                    .any(|item| item.subject == origin.source_id), !resolved);
            }
        }
    }

    async fn opening_preview(state: &AppState, person: &str, origin: &InboxOrigin) -> (StatusCode, Value) {
        use tower::ServiceExt;
        let response = crate::api::router(state.clone()).oneshot(
            axum::http::Request::builder().uri(format!(
                "/v1/client/threads/start-context?source_id={}&episode={}",
                urlencoding::encode(&origin.source_id), urlencoding::encode(&origin.episode),
            )).header(LOCAL_PERSON_HEADER, person).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    async fn opening_post(state: &AppState, packet: ActionRequest) -> (StatusCode, Value) {
        use tower::ServiceExt;
        let response = crate::api::router(state.clone()).oneshot(
            axum::http::Request::builder().method("POST").uri("/v1/client/actions")
                .header(LOCAL_PERSON_HEADER, PERSON).header("Content-Type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&packet).unwrap())).unwrap(),
        ).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    fn assert_start_refusal_fixture(status: StatusCode, mut body: Value, name: &str) {
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body["request_id"].as_str().is_some_and(|id| id.starts_with("request/")));
        body["request_id"] = json!("request/normalized");
        let fixtures: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-hf5d474418b48 owner=fractal_api_client_v0::api::client_v0::thread_fence_tests TODO=trace-statement-and-value-representation */ include_str!(
            "../../../../../docs/st3/client-v0/fixtures/thread-start-refusals.json")).unwrap();
        assert_eq!(body, fixtures[name], "{name}");
        assert!(body["details"].get("reason").is_none());
    }

    #[tokio::test]
    async fn thread_start_refusal_stale_snapshot_router_matches_fixture() {
        let (_root, state, origin) = fixture(true);
        let mut packet = request(&state, &origin, "thread/stale-start", "stale-start");
        packet.fence.snapshot_id = "snapshot/another-host/0/digest".into();
        let before = state.store.read_snapshot(Ok).unwrap();
        let (status, body) = opening_post(&state, packet).await;
        assert_start_refusal_fixture(status, body, "stale-snapshot");
        assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
    }

    #[tokio::test]
    async fn thread_start_context_signed_retry_recovers_before_unassigned_refusal() {
        use crate::api::{ThreadMessageDraft, ThreadMessageOrigin, prepare_inline_thread_message};
        use smallclaims::principal::{ClaimSignature, FIELDS_FORMAT, fields_signing_bytes};
        use tower::ServiceExt;

        let (_root, state, _) = fixture(true);
        let intent = crate::graph::parse_intent(
            "version 2\nmission \"signed-system\" state=\"ready\" { goal \"Choose\"; step \"review\" { assigned-to \"person/operator\"; goal \"Choose a path\"; } }",
            state.store.origin(),
        ).unwrap();
        state.store.apply_internal(&intent, "signed-system-mission").unwrap();
        let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
            mission:"signed-system".into(), revision:None, workspace:"/work/garden".into(),
            requester:Some(PERSON.into()), mode:Some("run".into()), inputs:BTreeMap::new(),
            idempotency_key:"signed-system-run".into(),
        }).unwrap();
        state.store.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
        let item = state.store.attention_items(Some(PERSON)).unwrap().into_iter()
            .find(|item| item.subject == run.steps[0].subject).unwrap();
        assert!(item.conversation.is_none());
        let origin = InboxOrigin { source_id:item.subject, episode:item.episode };
        state.store.configure_inbox_fallback_agent(Some(AGENT)).unwrap();
        state.store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
        let (device, _) = smallclaims::fleet::MemberKey::generate().unwrap();
        let chain = state.store.enroll_device_key(PERSON, device.public(), "Start phone").unwrap();
        let credential = "fixture-signed-start-bearer";
        state.store.append_client_operation_claim(&ClaimInput {
            subject:"custom/client/pairing-signed-start".into(),
            kind:"custom.client.pairing-completed".into(), actor:Some(PERSON.into()),
            fields:serde_json::from_value(json!({
                "person_id":PERSON,"session_actor":"client/paired/signed-start",
                "device_public_key":device.public(),"credential_hash":credential_digest(credential),
                "expires_at_unix_ms":client_now_ms()+60_000,"scopes":["control.messages","read.projections"],
            })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
        let signed_packet = |thread: &str, key: &str| {
            let mut packet = request(&state, &origin, thread, key);
            let input = prepare_inline_thread_message(&state, ThreadMessageDraft {
                key:&packet.idempotency_key, thread, from:PERSON, to:AGENT,
                content:"Explain the decision.", title:Some("Discuss retained review"),
                quote:None, origin:Some(ThreadMessageOrigin::Inbox(&origin.source_id, &origin.episode)),
            }, None).unwrap();
            let mut signature = ClaimSignature::sign(&device, "", PERSON, None, chain.clone(), client_now_ms() as u64);
            signature.format = Some(FIELDS_FORMAT.into());
            signature.signed_fields = input.fields.keys().filter(|field| field.as_str() != "status").cloned().collect();
            signature.signature = device.sign(&fields_signing_bytes(
                &input.subject, &input.kind, input.actor.as_deref(), &json!(input.fields),
                &signature.signed_fields, &signature.signer, None, &signature.key,
                &signature.chain, &signature.nonce, signature.signed_at_unix_ms,
            ));
            packet.parameters["signature"] = serde_json::to_value(signature).unwrap();
            packet
        };
        let accepted = signed_packet("thread/signed-start", "signed-start");
        let fresh = signed_packet("thread/signed-refused", "signed-refused");
        for _ in 0..20 {
            let report = state.store.advance_signature_migration().unwrap();
            if report.done() { break; }
            if report.phase == "judging" { state.store.advance_signature_judgment().unwrap(); }
        }
        let post = |packet: ActionRequest| {
            let app = crate::api::router(state.clone());
            async move {
                let response = app.oneshot(axum::http::Request::builder()
                    .method("POST").uri("/v1/client/actions")
                    .header(AUTHORIZATION, format!("Bearer {credential}"))
                    .header("Content-Type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&packet).unwrap())).unwrap(),
                ).await.unwrap();
                let status = response.status();
                let bytes = axum::body::to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
                (status, serde_json::from_slice::<Value>(&bytes).unwrap())
            }
        };
        let (status, first) = post(accepted.clone()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        state.store.configure_inbox_fallback_agent(None).unwrap();
        let before = state.store.read_snapshot(Ok).unwrap();
        let (status, preview) = opening_preview(&state, PERSON, &origin).await;
        assert_start_refusal_fixture(status, preview, "recipient-unassigned");
        let (status, retry) = post(accepted).await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_first_result(&retry["value"], &first["value"]);
        assert_eq!(retry["value"]["affected_ids"][0], "thread/signed-start");
        assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
        let (status, refused) = post(fresh).await;
        assert_start_refusal_fixture(status, refused, "recipient-unassigned");
        assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
    }

    #[tokio::test]
    async fn thread_start_context_signed_retry_recovers_original_receipt_after_terminal_retirement() {
        use crate::api::{ThreadMessageDraft, ThreadMessageOrigin, prepare_inline_thread_message};
        use smallclaims::principal::{ClaimSignature, FIELDS_FORMAT, fields_signing_bytes};
        use tower::ServiceExt;

        for resolved in [false, true] {
            let (_root, state, origin) = fixture(resolved);
            state.store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
            let (device, _) = smallclaims::fleet::MemberKey::generate().unwrap();
            let chain = state.store.enroll_device_key(PERSON, device.public(), "Retirement phone").unwrap();
            let credential = "fixture-signed-retirement-bearer";
            state.store.append_client_operation_claim(&ClaimInput {
                subject:"custom/client/pairing-signed-retirement".into(),
                kind:"custom.client.pairing-completed".into(), actor:Some(PERSON.into()),
                fields:serde_json::from_value(json!({
                    "person_id":PERSON,"session_actor":"client/paired/signed-retirement",
                    "device_public_key":device.public(),"credential_hash":credential_digest(credential),
                    "expires_at_unix_ms":client_now_ms()+60_000,"scopes":["control.messages","read.projections"],
                })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
            }).unwrap();
            let signed_packet = |thread: &str, key: &str, agent: &str| {
                let mut packet = request(&state, &origin, thread, key);
                packet.parameters["agent_id"] = json!(agent);
                let input = prepare_inline_thread_message(&state, ThreadMessageDraft {
                    key:&packet.idempotency_key, thread, from:PERSON, to:agent,
                    content:"Explain the decision.", title:Some("Discuss retained review"),
                    quote:None, origin:Some(ThreadMessageOrigin::Inbox(&origin.source_id, &origin.episode)),
                }, None).unwrap();
                let mut signature = ClaimSignature::sign(&device, "", PERSON, None, chain.clone(), client_now_ms() as u64);
                signature.format = Some(FIELDS_FORMAT.into());
                signature.signed_fields = input.fields.keys().filter(|field| field.as_str() != "status").cloned().collect();
                signature.signature = device.sign(&fields_signing_bytes(
                    &input.subject, &input.kind, input.actor.as_deref(), &json!(input.fields),
                    &signature.signed_fields, &signature.signer, None, &signature.key,
                    &signature.chain, &signature.nonce, signature.signed_at_unix_ms,
                ));
                packet.parameters["signature"] = serde_json::to_value(signature).unwrap();
                serde_json::to_vec(&packet).unwrap()
            };
            let accepted = signed_packet("thread/signed-before-retirement", "signed-before-retirement", AGENT);
            for _ in 0..20 {
                let report = state.store.advance_signature_migration().unwrap();
                if report.done() { break; }
                if report.phase == "judging" { state.store.advance_signature_judgment().unwrap(); }
            }
            let post = |bytes: Vec<u8>| {
                let app = crate::api::router(state.clone());
                async move {
                    let response = app.oneshot(axum::http::Request::builder()
                        .method("POST").uri("/v1/client/actions")
                        .header(AUTHORIZATION, format!("Bearer {credential}"))
                        .header("Content-Type", "application/json")
                        .body(axum::body::Body::from(bytes)).unwrap(),
                    ).await.unwrap();
                    let status = response.status();
                    let bytes = axum::body::to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
                    (status, serde_json::from_slice::<Value>(&bytes).unwrap())
                }
            };
            let (status, first) = post(accepted.clone()).await;
            assert_eq!(status, StatusCode::OK, "{first}");
            let original = thread_metadata::summary(&state.store, PERSON, "thread/signed-before-retirement").unwrap();
            assert_eq!(original.agent_id, AGENT);
            assert_eq!(original.message_count, 1);
            retire_original_with_fallback(&state);
            let before = state.store.read_snapshot(Ok).unwrap();
            // Reuse the serialized body, including the signature and original fence.
            let (status, retry) = post(accepted).await;
            assert_eq!(status, StatusCode::OK, "{retry}");
            assert_first_result(&retry["value"], &first["value"]);
            assert_eq!(retry["value"]["affected_ids"][0], "thread/signed-before-retirement");
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            let frozen = thread_metadata::summary(&state.store, PERSON, "thread/signed-before-retirement").unwrap();
            assert_eq!(frozen.agent_id, AGENT);
            assert_eq!(frozen.revision, original.revision);
            assert_eq!(frozen.message_count, 1);
            assert_first_message(&state, &retry["value"], "Explain the decision.");
            let message = retry["value"]["affected_ids"][1].as_str().unwrap();
            assert_eq!(state.store.claims_for(message, Some("message.sent")).unwrap()[0].body["fields"]["to"], AGENT);

            let fresh_original = signed_packet("thread/signed-retired-refused", "signed-retired-refused", AGENT);
            let (status, refused) = post(fresh_original).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
            assert!(state.store.claims_for("thread/signed-retired-refused", None).unwrap().is_empty());
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            let fresh_fallback = signed_packet("thread/signed-after-retirement", "signed-after-retirement",
                "agent/garden/fallback");
            let (status, fallback) = post(fresh_fallback).await;
            assert_eq!(status, StatusCode::OK, "{fallback}");
            assert_eq!(fallback["value"]["affected_ids"][0], "thread/signed-after-retirement");
            assert_ne!(fallback["value"]["affected_ids"], first["value"]["affected_ids"]);
            assert_ne!(fallback["value"]["operation_id"], first["value"]["operation_id"]);
            let created = thread_metadata::summary(&state.store, PERSON, "thread/signed-after-retirement").unwrap();
            assert_eq!(created.agent_id, "agent/garden/fallback");
            assert_eq!(created.message_count, 1);
            assert_first_message(&state, &fallback["value"], "Explain the decision.");
            assert_eq!(thread_metadata::list_context(&state.store, PERSON,
                ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap().items.len(), 2);
            if resolved { assert_not_in_attention(&state); }
        }
    }

    #[tokio::test]
    async fn thread_start_context_person_owned_typed_notice_has_no_implicit_agent() {
        for suppressed in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let state = tests::test_state_named(root.path(), "notice-preview");
            let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
            state.store.bind_fleet("018f6f0d-4a5d-7b8c-9d0e-123456789abd").unwrap();
            state.store.pin_fleet_anchor(key.public()).unwrap();
            state.store.set_member_key(Some(key.clone())).unwrap();
            state.store.append_claim(&ClaimInput {
                subject:"host/notice-preview".into(), kind:"fleet.member-admitted".into(), actor:None,
                fields:serde_json::from_value(json!({
                    "fleet_id":"018f6f0d-4a5d-7b8c-9d0e-123456789abd",
                    "member_key":key.public(),"via":"anchor","mode":"listening",
                })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
            }).unwrap();
            let intent = crate::graph::parse_intent(&format!(
                "version 2\nnotice-owner {{ member \"notice-preview\"; member-key \"{}\"; always-on }}",
                key.public(),
            ), state.store.origin()).unwrap();
            state.store.apply_internal(&intent, "preview-notice-owner").unwrap();
            // Notice authority reads persisted envelope signatures, not the local signer
            // shortcut. Seal the anchor admission before asking it to validate snapshots.
            state.store.replication_inventory().unwrap();
            state.store.configure_thread_write_rollout([]);
            state.store.append_claim(&ClaimInput {
                subject:"daemon/notice-preview".into(), kind:"daemon.started".into(),
                actor:Some(fractal_schema::notice::RUNTIME_ACTOR.into()),
                fields:serde_json::from_value(json!({
                    "status":"running","features":{"thread_read_privacy":fractal_schema::threads::READ_PRIVACY_VERSION},
                })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
            }).unwrap();
            for manifest in fractal_schema::notice::manifests() {
                state.store.register_custom_kind(&crate::store::custom::RegistrationRequest {
                    manifest:manifest.clone(), actor:PERSON.into(),
                }).unwrap();
            }
            let fixture: Value = serde_json::from_str(
                /* sqlite-large-value: allow-for-now lv-h374311c02b6a owner=fractal_api_client_v0::api::client_v0::thread_fence_tests TODO=trace-statement-and-value-representation */
                include_str!("../../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
            let mut fields = fixture["snapshot"]["input"].clone();
            fields["recipient"] = json!(PERSON);
            fields["source"] = json!({"host":"host/notice-preview"});
            let source = "custom/attention/source/01900000-0000-7000-8000-000000000002";
            let opening = state.store.append_claim(&ClaimInput {
                subject:source.into(), kind:"custom.attention.notice-created".into(), actor:Some(PERSON.into()),
                fields:serde_json::from_value(fields).unwrap(), evidence:vec![],
                expected_subject:None, idempotency_key:Some("preview-person-notice".into()),
            }).unwrap();
            state.store.append_claim(&ClaimInput {
                subject:"custom/attention/snapshot/01900000-0000-7000-8000-000000000001".into(),
                kind:"custom.attention.snapshot-created".into(),
                actor:Some(fractal_schema::notice::RUNTIME_ACTOR.into()),
                fields:serde_json::from_value(json!({
                    "source_id":source,"creation_claim":opening.id,"producer":PERSON,"recipient":PERSON,
                    "runtime":"host/notice-preview",
                    "policy":{"disposition":if suppressed {"suppress"} else {"deliver"}},"delivery":[],
                })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
            }).unwrap();
            let origin = InboxOrigin { source_id:source.into(), episode:opening.id };
            let before = state.store.read_snapshot(Ok).unwrap();
            let (status, refused) = opening_preview(&state, PERSON, &origin).await;
            assert_start_refusal_fixture(status, refused.clone(), "recipient-unassigned");
            assert!(refused["value"]["start_recipient"].is_null());
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            let (status, refused) = opening_post(&state, request(&state, &origin,
                "thread/unconfigured-notice", "unconfigured-notice")).await;
            assert_start_refusal_fixture(status, refused, "recipient-unassigned");
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            let intent = crate::graph::parse_intent(
                "version 2\nagent \"garden/seed\" { workspace \"/work/garden\"; harness \"omp\" {} }",
                state.store.origin(),
            ).unwrap();
            state.store.apply_internal(&intent, "preview-notice-recipient").unwrap();
            state.store.configure_inbox_fallback_agent(Some(AGENT)).unwrap();
            let before = state.store.read_snapshot(Ok).unwrap();
            let (status, preview) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            assert_eq!(preview["value"]["source_id"], source);
            assert_eq!(preview["value"]["episode"], origin.episode);
            assert_eq!(preview["value"]["start_recipient"],
                json!({"agent_id":AGENT,"basis":{"_tag":"configured"}}));
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            let owner = ClientSession::local(Some(PERSON)).unwrap();
            let mut packet = request(&state, &origin, "thread/notice-preview", "notice-preview");
            packet.parameters["agent_id"] = preview["value"]["start_recipient"]["agent_id"].clone();
            let result = start(&state, &owner, packet).await.unwrap();
            assert_first_message(&state, &result, "Explain the decision.");
            assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/notice-preview").unwrap().agent_id, AGENT);
        }
    }

    #[tokio::test]
    async fn thread_start_context_previews_resolved_runtime_ask_original_recipient() {
        let (_root, state, _) = fixture(true);
        let ask = state.store.ask_person(&PersonAskRequest {
            person:PERSON.into(), actor:AGENT.into(), title:"Review runtime work".into(),
            reason:"May we proceed?".into(), step:None, new_run:None, incarnation:None,
            legacy_request:None, idempotency_key:"preview-runtime-ask".into(),
            deadline_ms:None, default_answer:None, request:None, priority:None,
        }).unwrap();
        let opening = state.store.newest_claim_for_subject_kind(&ask.subject, "work.person-asked")
            .unwrap().unwrap();
        let origin = InboxOrigin { source_id:ask.subject, episode:opening.id };
        state.store.finish_person_step(&PersonStepResponse {
            delegation:None, subject:origin.source_id.clone(), actor:PERSON.into(),
            summary:"Proceed.".into(), evidence:vec![], episode:Some(origin.episode.clone()),
            idempotency_key:"preview-runtime-answer".into(), answer:None,
        }, false).unwrap();
        assert!(!state.store.attention_items(Some(PERSON)).unwrap().iter()
            .any(|item| item.subject == origin.source_id));
        let before = state.store.read_snapshot(Ok).unwrap();
        let (status, preview) = opening_preview(&state, PERSON, &origin).await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert_eq!(preview["value"]["source_id"], origin.source_id);
        assert_eq!(preview["value"]["episode"], origin.episode);
        assert_eq!(preview["value"]["start_recipient"],
            json!({"agent_id":AGENT,"basis":{"_tag":"original"}}));
        assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
    }

    #[tokio::test]
    async fn thread_start_context_previews_retained_native_handoff_original_recipient() {
        for requester in [AGENT, PERSON] {
            let (_root, state, _) = fixture(true);
            let intent = crate::graph::parse_intent(
                "version 2\nmission \"preview-handoff\" state=\"ready\" { goal \"Review work\"; step \"review\" { assigned-to \"agent/garden/seed\"; goal \"Choose a path\"; } }",
                state.store.origin(),
            ).unwrap();
            state.store.apply_internal(&intent, "preview-handoff-mission").unwrap();
            let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
                mission:"preview-handoff".into(), revision:None, workspace:"/work/garden".into(),
                requester:Some(requester.into()), mode:Some("run".into()), inputs:BTreeMap::new(),
                idempotency_key:"preview-handoff-run".into(),
            }).unwrap();
            let step = &run.steps[0];
            state.store.set_step_state(&step.subject, "ready", None).unwrap();
            for (kind, fields) in [
                ("runtime.observed", json!({"status":"running","runtime_id":"preview-worker","incarnation_id":"preview-worker-one"})),
                ("harness.observed", json!({"state":"idle","driver":"codex","incarnation_id":"preview-worker-one"})),
            ] {
                state.store.append_claim(&ClaimInput {
                    subject:AGENT.into(), kind:kind.into(), actor:Some("daemon/runtime".into()),
                    fields:serde_json::from_value(fields).unwrap(), evidence:vec![],
                    expected_subject:None, idempotency_key:Some(format!("preview-handoff-{kind}")),
                }).unwrap();
            }
            state.store.work_action(&step.subject, "claim", &crate::model::WorkRequest {
                wait_ms:None, actor:Some(AGENT.into()), incarnation:Some("preview-worker-one".into()),
                summary:None, reason:None, evidence:vec![], idempotency_key:"preview-handoff-claim".into(),
            }).unwrap();
            let handed = state.store.handoff_work(&step.subject, &crate::model::WorkHandoffRequest {
                actor:AGENT.into(), incarnation:Some("preview-worker-one".into()), to:PERSON.into(),
                note:"Please review this request.".into(), evidence:vec![],
                idempotency_key:"preview-native-handoff".into(),
            }).unwrap();
            let origin = InboxOrigin { source_id:handed.subject,
                episode:format!("{}:{}:{}", handed.generation, handed.attempt, handed.readiness_epoch) };
            let released = state.store.newest_claim_for_subject_kind(&origin.source_id, "work.released")
                .unwrap().unwrap();
            assert_eq!(released.body["fields"]["status"], "ready");
            assert_eq!(released.body["fields"]["handoff_to"], PERSON);
            state.store.set_step_state(&origin.source_id, "completed", None).unwrap();
            assert!(!state.store.attention_items(Some(PERSON)).unwrap().iter()
                .any(|item| item.subject == origin.source_id));
            // Handoff proves the human opening, not an original discussion agent.
            // Once assigned to a person, only an agent requester supplies that association.
            if requester == PERSON {
                let before = state.store.read_snapshot(Ok).unwrap();
                let (status, refused) = opening_preview(&state, PERSON, &origin).await;
                assert_eq!(status, StatusCode::CONFLICT, "{refused}");
                assert_eq!(refused["code"], "recipient-unassigned");
                assert_eq!(refused["message"], "this Inbox source has no configured responsible agent");
                assert!(refused["value"]["start_recipient"].is_null());
                assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
                state.store.configure_inbox_fallback_agent(Some(AGENT)).unwrap();
            }
            let before = state.store.read_snapshot(Ok).unwrap();
            let (status, preview) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            assert_eq!(preview["value"]["source_id"], origin.source_id);
            assert_eq!(preview["value"]["episode"], origin.episode);
            assert_eq!(preview["value"]["start_recipient"],
                json!({"agent_id":AGENT,"basis":{"_tag":if requester == AGENT {"original"} else {"configured"}}}));
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn thread_start_context_system_source_requires_explicit_configured_recipient() {
        for resolved in [false, true] {
            let (_root, state, _) = fixture(true);
            let intent = crate::graph::parse_intent(
                "version 2\nmission \"preview-system\" state=\"ready\" { goal \"Choose\"; step \"review\" { assigned-to \"person/operator\"; goal \"Choose a path\"; } }",
                state.store.origin(),
            ).unwrap();
            state.store.apply_internal(&intent, "preview-system-mission").unwrap();
            let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
                mission:"preview-system".into(), revision:None, workspace:"/work/garden".into(),
                requester:Some(PERSON.into()), mode:Some("run".into()), inputs:BTreeMap::new(),
                idempotency_key:"preview-system-run".into(),
            }).unwrap();
            let step = &run.steps[0];
            state.store.set_step_state(&step.subject, "ready", None).unwrap();
            let item = state.store.attention_items(Some(PERSON)).unwrap().into_iter()
                .find(|item| item.subject == step.subject).unwrap();
            assert!(item.conversation.is_none(), "the system source has no original agent");
            let origin = InboxOrigin { source_id:step.subject.clone(), episode:item.episode };
            if resolved { state.store.set_step_state(&step.subject, "completed", None).unwrap(); }
            let before = state.store.read_snapshot(Ok).unwrap();
            let (status, refused) = opening_preview(&state, PERSON, &origin).await;
            assert_start_refusal_fixture(status, refused.clone(), "recipient-unassigned");
            assert!(refused["value"]["start_recipient"].is_null());
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            let (status, refused) = opening_post(&state, request(&state, &origin,
                "thread/unconfigured-system", "unconfigured-system")).await;
            assert_start_refusal_fixture(status, refused, "recipient-unassigned");
            let owner = ClientSession::local(Some(PERSON)).unwrap();
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            state.store.configure_inbox_fallback_agent(Some(AGENT)).unwrap();
            let before = state.store.read_snapshot(Ok).unwrap();
            let (status, preview) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            assert_eq!(preview["value"]["start_recipient"],
                json!({"agent_id":AGENT,"basis":{"_tag":"configured"}}));
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            let mut packet = request(&state, &origin, "thread/configured-system", "configured-system");
            packet.parameters["agent_id"] = preview["value"]["start_recipient"]["agent_id"].clone();
            let result = start(&state, &owner, packet).await.unwrap();
            assert_first_message(&state, &result, "Explain the decision.");
            assert_eq!(thread_metadata::summary(&state.store, PERSON,
                "thread/configured-system").unwrap().agent_id, AGENT);
        }
    }

    #[tokio::test]
    async fn thread_start_context_round_trip_rechecks_retirement_and_preserves_existing_audience() {
        for resolved in [false, true] {
            let (_root, state, origin) = fixture(resolved);
            let owner = ClientSession::local(Some(PERSON)).unwrap();
            let (status, preview) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            let mut first = request(&state, &origin, "thread/preview-original", "preview-original");
            first.parameters["agent_id"] = preview["value"]["start_recipient"]["agent_id"].clone();
            let original = start(&state, &owner, first).await.unwrap();
            assert_first_message(&state, &original, "Explain the decision.");
            let (status, stale_preview) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{stale_preview}");
            let mut stale = request(&state, &origin, "thread/preview-stale", "preview-stale");
            stale.parameters["agent_id"] = stale_preview["value"]["start_recipient"]["agent_id"].clone();
            retire_original_with_fallback(&state);
            let before = state.store.read_snapshot(Ok).unwrap();
            let refused = start(&state, &owner, stale).await.unwrap_err();
            assert_eq!(refused.status, StatusCode::NOT_FOUND);
            assert_eq!(refused.code, "not-found");
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before, "no message, thread or receipt appended");
            assert!(state.store.claims_for("thread/preview-stale", None).unwrap().is_empty());
            let (status, fallback) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{fallback}");
            assert_eq!(fallback["value"]["start_recipient"]["basis"]["_tag"], "fallback");
            assert_eq!(fallback["value"]["start_recipient"]["agent_id"], "agent/garden/fallback");
            let mut fresh = request(&state, &origin, "thread/preview-fallback", "preview-fallback");
            fresh.parameters["agent_id"] = fallback["value"]["start_recipient"]["agent_id"].clone();
            let newer = start(&state, &owner, fresh).await.unwrap();
            assert_ne!(newer["affected_ids"][0], original["affected_ids"][0]);
            assert_ne!(newer["operation_id"], original["operation_id"]);
            assert_first_message(&state, &newer, "Explain the decision.");
            for (thread, agent) in [
                ("thread/preview-original", AGENT), ("thread/preview-fallback", "agent/garden/fallback"),
            ] {
                let summary = thread_metadata::summary(&state.store, PERSON, thread).unwrap();
                assert_eq!(summary.agent_id, agent);
                assert_eq!(summary.message_count, 1);
                assert_eq!(crate::store::thread_delivery::reply_recipient(&state.store, thread, PERSON).unwrap(), agent);
            }
            assert_eq!(thread_metadata::list_context(&state.store, PERSON,
                ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap().items.len(), 2);
        }
    }

    #[tokio::test]
    async fn thread_start_context_previews_current_and_retained_terminal_recipient_without_writes() {
        for resolved in [false, true] {
            let (_root, state, origin) = fixture(resolved);
            let before = state.store.read_snapshot(Ok).unwrap();
            let (status, original) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{original}");
            assert_eq!(original["value"]["kind"], "thread-start-context");
            assert_eq!(original["value"]["start_recipient"]["agent_id"], AGENT);
            assert_eq!(original["value"]["start_recipient"]["basis"]["_tag"], "original");
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            retire_original_with_fallback(&state);
            let before = state.store.read_snapshot(Ok).unwrap();
            let (status, fallback) = opening_preview(&state, PERSON, &origin).await;
            assert_eq!(status, StatusCode::OK, "{fallback}");
            let recipient = &fallback["value"]["start_recipient"];
            assert_eq!(recipient["agent_id"], "agent/garden/fallback");
            assert_eq!(recipient["basis"]["_tag"], "fallback");
            assert_eq!(recipient["basis"]["original_agent_id"], AGENT);
            assert_eq!(recipient["basis"]["retirement"]["kind"], "agent.retired");
            assert_eq!(recipient["basis"]["retirement"]["state"], "retired");
            let claim = state.store.claims_for(AGENT, Some("agent.retired")).unwrap().pop().unwrap();
            assert_eq!(recipient["basis"]["retirement"]["claim_id"], claim.id);
            let owner = ClientSession::local(Some(PERSON)).unwrap();
            let mismatch = request(&state, &origin, "thread/preview-old-packet", "preview-old-packet");
            assert_eq!(start(&state, &owner, mismatch).await.unwrap_err().status, StatusCode::NOT_FOUND);
            assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
            assert_eq!(state.store.attention_items(Some(PERSON)).unwrap().iter()
                .any(|item| item.subject == SOURCE), !resolved);
        }
    }

    #[tokio::test]
    async fn thread_start_context_denied_and_unknown_keys_have_the_same_response() {
        let (_root, state, origin) = fixture(true);
        let before = state.store.read_snapshot(Ok).unwrap();
        let (wrong_status, wrong) = opening_preview(&state, "person/other", &origin).await;
        let fabricated = InboxOrigin { episode:"claim/never-opened-preview".into(), ..origin.clone() };
        let (unknown_status, unknown) = opening_preview(&state, PERSON, &fabricated).await;
        assert_eq!(wrong_status, StatusCode::NOT_FOUND);
        assert_eq!(unknown_status, wrong_status);
        assert_eq!(wrong["code"], unknown["code"]);
        assert_eq!(wrong["message"], unknown["message"]);
        assert_eq!(wrong["code"], "not-found");
        assert!(!wrong.to_string().contains(AGENT));
        assert!(!wrong.to_string().contains(&origin.episode));
        assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
    }

    #[tokio::test]
    async fn thread_start_context_unconfigured_terminal_fallback_uses_declared_recipient_retired_error() {
        let (_root, state, origin) = fixture(true);
        retire_original_with_fallback(&state);
        state.store.configure_inbox_fallback_agent(None).unwrap();
        let before = state.store.read_snapshot(Ok).unwrap();
        let (status, refused) = opening_preview(&state, PERSON, &origin).await;
        assert_start_refusal_fixture(status, refused, "recipient-retired");
        let (status, refused) = opening_post(&state,
            request(&state, &origin, "thread/no-retirement-fallback", "no-retirement-fallback")).await;
        assert_start_refusal_fixture(status, refused, "recipient-retired");
        assert_eq!(state.store.read_snapshot(Ok).unwrap(), before);
        assert_not_in_attention(&state);
    }

    #[tokio::test]
    async fn retained_discussion_unread_batch_preserves_frozen_threads_after_terminal_fallback() {
        let (_root, state, origin) = fixture(true);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        start(&state, &owner, request(&state, &origin,
            "thread/unread-original", "unread-original")).await.unwrap();
        retire_original_with_fallback(&state);
        start(&state, &owner, fallback_request(&state, &origin,
            "thread/unread-fallback", "unread-fallback")).await.unwrap();
        let fallback = "agent/garden/fallback";
        crate::store::thread_delivery::append_reply(&state.store, &ClaimInput {
            subject:"message/unread-terminal-fallback".into(), kind:"message.sent".into(),
            actor:Some(fallback.into()),
            fields:serde_json::from_value(json!({
                "from":fallback, "to":PERSON, "content":"The new participant replied.",
                "thread_id":"thread/unread-fallback", "status":"sent",
            })).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:Some("unread-terminal-fallback".into()),
        }, thread_metadata::PersonAuthority::BoundAgent { actor:fallback }, None).unwrap();
        let keys = [(SOURCE, origin.episode.as_str()), (SOURCE, "claim/never-opened-unread")];
        assert_eq!(thread_metadata::unread_inbox_positions(&state.store, PERSON, &keys).unwrap(),
            std::collections::BTreeSet::from([0]));
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/unread-original").unwrap().agent_id, AGENT);
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/unread-fallback").unwrap().agent_id, fallback);
        assert_not_in_attention(&state);
    }
}
