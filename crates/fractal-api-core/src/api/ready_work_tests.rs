//! Normal Workspace runtime coverage: real graph declarations and work-list publications.
use super::*;
use crate::{AttentionListStore, PersonWorkStore};
use crate::api::published_lists::tests_shared_client_v0_tests::publish;
use crate::api::tests_shared_client_v0_tests::{get_request, state};
use crate::model::{MissionRunRequest, PersonAskRequest, WorkHandoffRequest, WorkRequest};

const ASH: &str = "agent/garden/ash";
const BIRCH: &str = "agent/garden/birch";
const PATH: &str = "/v1/client/ready-work?cohort=agent/garden/ash";

// Current-thread API tests resume the handler on their own thread. Key the one-shot seam
// by store as well, so another router on that thread cannot consume this invalidation.
thread_local! {
    static FORGET_AFTER_COUNT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

pub(super) fn after_count(state: &AppState) {
    FORGET_AFTER_COUNT.with(|hook| {
        if hook.get() == Some(Arc::as_ptr(&state.store) as usize) {
            hook.set(None);
            state.store.published_work_list().forget();
        }
    });
}

fn fixture() -> (tempfile::TempDir, AppState, String) {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    publish(&state.store, r#"version 2
agent "garden/ash" { workspace "/tmp"; command "true" }
agent "garden/birch" { workspace "/tmp"; command "true" }
mission "garden/ready-demand" state="ready" {
  goal "Exercise ready demand through the client API."
  step "plant" { assigned-to "agent/garden/ash" }
}"#, "ready-demand-declarations");
    let run = state.store.create_mission_run(&MissionRunRequest {
        mission: "garden/ready-demand".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/operator".into()), mode: None, inputs: BTreeMap::new(),
        idempotency_key: "ready-demand-run".into(),
    }).unwrap();
    let step = run.steps[0].subject.clone();
    state.store.set_step_state(&step, "ready", None).unwrap();
    (root, state, step)
}

fn publish_work(state: &AppState) {
    let store = &state.store;
    store.set_projection_frontier_for_test(store.index().unwrap());
    store.published_work_list().start();
    published_lists::refresh_work_once(store);
    let publication = store.published_work().unwrap();
    assert_eq!(publication.cut, store.index().unwrap());
    assert_eq!(store.attention_projection_frontier().unwrap(), Some(("healthy".into(), publication.rows.frontier)));
}

fn claim(state: &AppState, step: &str) {
    state.store.work_action(step, "claim", &WorkRequest {
        wait_ms: None, actor: Some(ASH.into()), incarnation: Some("ash-1".into()),
        summary: None, reason: None, evidence: Vec::new(), idempotency_key: "claim-ready-demand".into(),
    }).unwrap();
}

async fn refused(state: &AppState, path: &str, status: StatusCode, code: &str) {
    let (actual, error) = get_request(router(state.clone()), path).await;
    assert_eq!(actual, status, "{error}");
    if code == "work-list-not-ready" {
        assert_eq!(error["code"], "internal", "{error}");
        assert_eq!(error["retryable"], true, "{error}");
        assert_eq!(error["details"]["reason"], code, "{error}");
    } else {
        assert_eq!(error["code"], code, "{error}");
    }
}

#[tokio::test]
async fn ready_work_happy_path_deduplicates_cohort_and_reports_publication_cut() {
    let (_root, state, _) = fixture();
    grant_claim(&state, "custom/client/ready-demand-authorized", "custom.client.pairing-completed", json!({
        "credential_hash": client_v0::credential_digest("ready-demand-authorized"),
        "session_actor": "client/ready-demand-authorized", "person_id": "person/operator",
        "expires_at_unix_ms": u64::MAX, "scopes": ["read.projections"]
    }));
    publish_work(&state);
    let publication = state.store.published_work().unwrap();
    for cohort in [ASH.to_owned(), format!("{ASH},{ASH},{BIRCH},{BIRCH}")] {
        let response = router(state.clone()).oneshot(Request::builder()
            .uri(format!("/v1/client/ready-work?cohort={cohort}"))
            .header("authorization", "Bearer ready-demand-authorized")
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let envelope: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(envelope["api_version"], "st3.client.v0");
        assert_eq!(envelope["snapshot"]["store_index"], publication.cut);
        assert_eq!(envelope["value"]["ready_count"], 1);
        assert_eq!(envelope["value"]["coverage"], "local-graph-cut");
        assert_eq!(envelope["value"]["projection_frontier"], publication.rows.frontier);
        assert_eq!(envelope["value"]["evaluated_at"], client_timestamp(publication.rows.time_unix_ms));
    }
}

#[tokio::test]
async fn ready_work_rejects_malformed_and_oversized_cohorts_before_reading_work() {
    let (_root, state, _) = fixture();
    for cohort in [String::new(), "agent/".into(), "garden/ash".into(),
        format!("{ASH},"), format!("agent/{}", "a".repeat(123)),
        std::iter::repeat_n(ASH, 65).collect::<Vec<_>>().join(",")] {
        refused(&state, &format!("/v1/client/ready-work?cohort={cohort}"),
            StatusCode::UNPROCESSABLE_ENTITY, "validation-failed").await;
    }
    // The inclusive size bound accepts 64 entries, including duplicates.
    publish_work(&state);
    let cohort = std::iter::repeat_n(ASH, 64).collect::<Vec<_>>().join(",");
    let (status, value) = get_request(router(state), &format!("/v1/client/ready-work?cohort={cohort}")).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["ready_count"], 1);
}

#[tokio::test]
async fn ready_work_requires_publication_and_declared_cohort() {
    let (_root, state, _) = fixture();
    refused(&state, PATH, StatusCode::SERVICE_UNAVAILABLE, "work-list-not-ready").await;
    publish_work(&state);
    refused(&state, "/v1/client/ready-work?cohort=agent/garden/absent",
        StatusCode::SERVICE_UNAVAILABLE, "ready-work-unavailable").await;
}

#[tokio::test]
async fn ready_work_refuses_graph_head_and_frontier_staleness_independently() {
    for graph_head in [true, false] {
        let (_root, state, _) = fixture();
        publish_work(&state);
        let publication = state.store.published_work().unwrap();
        if graph_head {
            state.store.append_claim(&ClaimInput {
                subject: "custom/test/ready-demand-marker".into(), kind: "custom.test.marker".into(),
                actor: None, fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None,
                idempotency_key: None,
            }).unwrap();
            assert!(state.store.index().unwrap() > publication.cut);
            assert_eq!(state.store.attention_projection_frontier().unwrap().unwrap().1, publication.rows.frontier);
        } else {
            state.store.set_projection_frontier_for_test(publication.rows.frontier + 1);
            assert_eq!(state.store.index().unwrap(), publication.cut);
        }
        refused(&state, PATH, StatusCode::SERVICE_UNAVAILABLE, "ready-work-unavailable").await;
    }
}

#[tokio::test]
async fn ready_work_refuses_generation_invalidated_after_count() {
    let (_root, state, _) = fixture();
    publish_work(&state);
    FORGET_AFTER_COUNT.with(|hook| hook.set(Some(Arc::as_ptr(&state.store) as usize)));
    refused(&state, PATH, StatusCode::SERVICE_UNAVAILABLE, "work-list-not-ready").await;
    FORGET_AFTER_COUNT.with(|hook| assert!(hook.get().is_none(), "the post-count seam was reached"));
}

#[cfg(test)]
#[tokio::test]
async fn ready_work_excludes_claimants_future_work_and_pending_handoffs() {
    for exclusion in ["claimant", "not-before", "handoff"] {
        let (_root, state, step) = fixture();
        ready_count(&state, &format!("{PATH},{BIRCH}"), 1).await;
        state.store.published_work_list().forget();
        match exclusion {
            "not-before" => {
                // Seed a future-ready projection without advancing wall time. A cold real
                // work publication must retain and exclude its not-before constraint.
                state.store.connection.lock().unwrap().execute(
                    "UPDATE step_runs SET not_before_unix_ms=?1 WHERE subject=?2",
                    rusqlite::params!["9999999999999", step],
                ).unwrap();
            }
            _ => claim(&state, &step),
        }
        if exclusion == "handoff" {
            state.store.handoff_work(&step, &WorkHandoffRequest {
                actor: ASH.into(), incarnation: Some("ash-1".into()), to: BIRCH.into(),
                note: "Please take over.".into(), evidence: Vec::new(),
                idempotency_key: "ready-demand-handoff".into(),
            }).unwrap();
        }
        publish_work(&state);
        let publication = state.store.published_work().unwrap();
        let view = &publication.rows.rows[&step].view;
        match exclusion {
            "claimant" => assert_eq!(view.claimant.as_deref(), Some(ASH)),
            "not-before" => {
                assert_eq!(view.status, "ready");
                assert!(view.not_before_unix_ms.unwrap() > publication.rows.time_unix_ms);
            }
            "handoff" => {
                assert_eq!(view.status, "ready");
                assert_eq!(view.assigned_to.as_deref(), Some(BIRCH));
                assert!(view.claimant.is_none());
            }
            _ => unreachable!(),
        }
        let (status, value) = get_request(router(state), &format!("{PATH},{BIRCH}")).await;
        assert_eq!(status, StatusCode::OK, "{exclusion}: {value}");
        assert_eq!(value["ready_count"], 0, "{exclusion}: {value}");
    }
}

async fn ready_count(state: &AppState, path: &str, expected: usize) {
    state.store.published_work_list().forget();
    publish_work(state);
    let (status, value) = get_request(router(state.clone()), path).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["ready_count"], expected, "{value}");
}

fn otherwise_ready(view: &crate::model::StepRunView) {
    assert_eq!(view.status, "ready");
    assert!(view.claimant.is_none());
    assert!(view.blocked_reason.is_none());
    assert!(view.blockers.is_empty());
    assert!(view.not_before_unix_ms.is_none());
}

#[cfg(test)]
#[tokio::test]
async fn ready_work_excludes_only_the_agentless_flag_on_an_otherwise_ready_row() {
    let (_root, state, step) = fixture();
    ready_count(&state, PATH, 1).await;
    // Isolate the flag without also removing the cohort assignment or changing status.
    state.store.connection.lock().unwrap().execute(
        "UPDATE step_runs SET agentless=1 WHERE subject=?1", [&step],
    ).unwrap();
    ready_count(&state, PATH, 0).await;
    let publication = state.store.published_work().unwrap();
    let view = &publication.rows.rows[&step].view;
    otherwise_ready(view);
    assert!(view.agentless);
    assert_eq!(view.assigned_to.as_deref(), Some(ASH));
}

#[tokio::test]
async fn ready_work_excludes_non_member_of_a_valid_cohort() {
    let (_root, state, step) = fixture();
    ready_count(&state, PATH, 1).await;
    ready_count(&state, "/v1/client/ready-work?cohort=agent/garden/birch", 0).await;
    let publication = state.store.published_work().unwrap();
    let view = &publication.rows.rows[&step].view;
    otherwise_ready(view);
    assert!(!view.agentless);
    assert_eq!(view.assigned_to.as_deref(), Some(ASH));
    // Changing only cohort membership restores the same row to the answer.
    ready_count(&state, PATH, 1).await;
}

#[tokio::test]
async fn ready_work_counts_an_acknowledged_handoff_as_ready() {
    let (_root, state, step) = fixture();
    let path = format!("{PATH},{BIRCH}");
    ready_count(&state, &path, 1).await;
    claim(&state, &step);
    state.store.handoff_work(&step, &WorkHandoffRequest {
        actor: ASH.into(), incarnation: Some("ash-1".into()), to: BIRCH.into(),
        note: "Please take over.".into(), evidence: Vec::new(),
        idempotency_key: "ready-demand-acknowledged-handoff".into(),
    }).unwrap();
    ready_count(&state, &path, 0).await;
    let publication = state.store.published_work().unwrap();
    let view = &publication.rows.rows[&step].view;
    otherwise_ready(view);
    assert!(!view.agentless);
    assert_eq!(view.assigned_to.as_deref(), Some(BIRCH));
    let messages = state.store.messages(Some(BIRCH), true).unwrap();
    let note = messages.iter().find(|message| {
        message.tags.contains(&format!("st3-work-handoff:{step}"))
    }).expect("the real handoff publishes its acknowledgment note");
    state.store.acknowledge_work(&step, &crate::model::WorkAcknowledgeRequest {
        actor: BIRCH.into(), message: note.subject.clone(),
    }).unwrap();
    ready_count(&state, &path, 1).await;
    otherwise_ready(&state.store.published_work().unwrap().rows.rows[&step].view);
}

#[tokio::test]
async fn ready_work_person_blocker_isolated_from_status_and_claimant() {
    let (_root, state, step) = fixture();
    ready_count(&state, PATH, 1).await;
    claim(&state, &step);
    let ask = state.store.ask_person(&PersonAskRequest {
        deadline_ms: None, default_answer: None, legacy_request: None,
        person: "person/operator".into(), title: "Choose a planting date".into(),
        reason: "The owner must choose.".into(), actor: ASH.into(), step: Some(step.clone()),
        new_run: None, incarnation: Some("ash-1".into()), request: None,
        idempotency_key: "ready-demand-isolated-person-blocker".into(),
    }).unwrap();
    // Restore ready and clear the lease through the typed state path, leaving the
    // open person ask as the sole exclusion instead of testing waiting-person.
    state.store.set_step_state(&step, "ready", None).unwrap();
    ready_count(&state, PATH, 0).await;
    let publication = state.store.published_work().unwrap();
    let view = &publication.rows.rows[&step].view;
    assert_eq!(view.status, "ready");
    assert!(view.claimant.is_none());
    assert!(view.blocked_reason.is_none());
    assert!(view.not_before_unix_ms.is_none());
    assert!(!view.agentless);
    assert_eq!(view.assigned_to.as_deref(), Some(ASH));
    assert!(!view.blockers.is_empty());
    // Resolve only the blocking ask; the owner is already ready and unclaimed.
    state.store.set_step_state(&ask.subject, "completed", None).unwrap();
    ready_count(&state, PATH, 1).await;
    otherwise_ready(&state.store.published_work().unwrap().rows.rows[&step].view);
}

fn grant_claim(state: &AppState, subject: &str, kind: &str, fields: Value) {
    state.store.append_claim(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: Some("person/operator".into()),
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap();
}

#[tokio::test]
async fn ready_work_denies_paired_client_without_projection_scope_and_head_is_405() {
    let (_root, state, _) = fixture();
    grant_claim(&state, "custom/client/ready-demand", "custom.client.pairing-completed", json!({
        "credential_hash": client_v0::credential_digest("ready-demand-device"),
        "session_actor": "client/ready-demand", "person_id": "person/operator",
        "expires_at_unix_ms": u64::MAX, "scopes": ["terminal.read"]
    }));
    for method in ["GET", "HEAD"] {
        let response = router(state.clone()).oneshot(Request::builder().method(method).uri(PATH)
            .header("authorization", "Bearer ready-demand-device")
            .body(Body::empty()).unwrap()).await.unwrap();
        if method == "HEAD" {
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert!(to_bytes(response.into_body(), usize::MAX).await.unwrap().is_empty());
        } else {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let error: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
            assert_eq!(error["code"], "forbidden");
            assert!(error["message"].as_str().unwrap().contains("read.projections"));
        }
    }
}

#[tokio::test]
async fn ready_work_refuses_authenticated_assistant_and_external_service_grants() {
    use client_v0::external_grants::{self, GRANTED};
    for profile in ["assistant", "service"] {
        let (_root, mut state, _) = fixture();
        state.owning_person = Some("person/operator".into());
        let id = "111111111111111111111111";
        grant_claim(&state, &external_grants::subject(id).unwrap(), GRANTED, json!({
            "profile": profile, "principal": "assistant/chatgpt", "grant_id": id,
            "session_actor": format!("assistant/chatgpt/session/{id}"), "issuer": "person/operator",
            "credential_hash": client_v0::credential_digest("ready-demand-external"),
            "expires_at_unix_ms": u64::MAX, "minimal_read_mode": "approval-once",
            "disclosure_consent": true, "scopes": ["read.assistant.summary"]
        }));
        if profile == "assistant" {
            // Prove this bearer authenticates: refusal is the explicit route allowlist,
            // not an absent, malformed, expired, or otherwise unusable grant.
            let session = external_grants::authenticate(&state,
                &client_v0::credential_digest("ready-demand-external"), "unix").unwrap().unwrap();
            assert_eq!(session.authority_actor, "assistant/chatgpt");
        }
        let response = router(state).oneshot(Request::builder().uri(PATH)
            .header("authorization", "Bearer ready-demand-external")
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let error: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(error["code"], "forbidden");
        assert!(error["message"].as_str().unwrap().contains(if profile == "assistant" {
            "explicit proof-bearing read projections"
        } else { "service grant issuance is disabled" }), "{error}");
    }
}
