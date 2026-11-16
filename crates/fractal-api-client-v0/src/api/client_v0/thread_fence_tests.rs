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
            manifest: serde_json::from_str(include_str!("../../../../../examples/fractal/custom-review.json")).unwrap(),
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

    pub(crate) fn request(state: &AppState, origin: &InboxOrigin, thread: &str, key: &str) -> ActionRequest {
        serde_json::from_value(json!({
            "api_version":CLIENT_API_VERSION, "id":format!("action/{key}"),
            "type":"thread.start", "idempotency_key":key,
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
    async fn resolved_custom_start_replays_and_different_key_opens_existing_thread_without_message() {
        let (_root, state, origin) = fixture(true);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        assert_not_in_attention(&state);
        let first_request = request(&state, &origin, "thread/retained-first", "retained-first");
        let first = start(&state, &owner, first_request.clone()).await.unwrap();
        let replay = start(&state, &owner, first_request).await.unwrap();
        assert_eq!(replay["affected_ids"], first["affected_ids"]);
        assert_eq!(replay["operation_id"], first["operation_id"]);
        let existing = start(&state, &owner,
            request(&state, &origin, "thread/retained-unused", "retained-different")).await.unwrap();
        assert_eq!(existing["affected_ids"], first["affected_ids"]);
        assert_eq!(existing["operation_id"], first["operation_id"]);
        assert!(state.store.claims_for("thread/retained-unused", None).unwrap().is_empty());
        let history = thread_metadata::list_context(&state.store, PERSON,
            ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/retained-first").unwrap().message_count, 1);
        assert_not_in_attention(&state);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_retained_starts_share_the_real_store_writer_and_return_one_creation() {
        let (_root, state, origin) = fixture(true);
        let owner = ClientSession::local(Some(PERSON)).unwrap();
        let a = request(&state, &origin, "thread/retained-race-a", "retained-race-a");
        let b = request(&state, &origin, "thread/retained-race-b", "retained-race-b");
        // Different action keys bypass the per-key API lock; both reach the batched store writer.
        let (a, b) = tokio::join!(start(&state, &owner, a), start(&state, &owner, b));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a["affected_ids"], b["affected_ids"]);
        assert_eq!(a["operation_id"], b["operation_id"]);
        let history = thread_metadata::list_context(&state.store, PERSON,
            ThreadContext::Inbox { source:SOURCE, episode:&origin.episode }, None, 20).unwrap();
        assert_eq!(history.items.len(), 1);
        let thread = a["affected_ids"][0].as_str().unwrap();
        assert_eq!(thread_metadata::summary(&state.store, PERSON, thread).unwrap().message_count, 1);
        let creations = ["thread/retained-race-a", "thread/retained-race-b"].iter()
            .map(|thread| state.store.claims_for(thread, Some("thread.created")).unwrap().len()).sum::<usize>();
        assert_eq!(creations, 1);
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
            "idempotency_key":"current-reply", "fence":{"snapshot_id":new_client_snapshot(&state).id},
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
        let result = start(&state, &owner, request(&state, &origin, "thread/retained-gate", "retained-gate")).await.unwrap();
        assert_eq!(result["affected_ids"][0], "thread/retained-gate");
        assert_eq!(thread_metadata::summary(&state.store, PERSON, "thread/retained-gate").unwrap().message_count, 1);
        assert!(state.store.pending_human_reviews(Some(PERSON)).unwrap().is_empty());
    }

    #[tokio::test]
    async fn retained_start_chooses_newest_owned_creation_from_current_source_history() {
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
        let retained = start(&state, &owner, request(&state, &origin, "thread/newest-unused", "newest-retained")).await.unwrap();
        assert_eq!(retained["affected_ids"], newer["affected_ids"]);
        assert_eq!(retained["operation_id"], newer["operation_id"]);
        assert!(state.store.claims_for("thread/newest-unused", None).unwrap().is_empty());
        assert_not_in_attention(&state);
    }

    #[tokio::test]
    async fn resolved_authored_step_without_recipient_bearing_opening_cannot_start() {
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
        let denied = start(&state, &owner, request(&state, &origin,
            "thread/never-opened-authored", "never-opened-authored")).await.unwrap_err();
        assert_eq!(denied.status, StatusCode::NOT_FOUND);
        assert!(state.store.claims_for("thread/never-opened-authored", None).unwrap().is_empty());
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
        let mut manifest: Value = serde_json::from_str(include_str!("../../../../../examples/fractal/custom-review.json")).unwrap();
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
        assert_eq!(retained["affected_ids"], current["affected_ids"]);
        assert_eq!(retained["operation_id"], current["operation_id"]);
        assert!(state.store.claims_for("thread/non-claim-retained", None).unwrap().is_empty());
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
}
