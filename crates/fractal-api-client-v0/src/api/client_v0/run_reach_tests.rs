//! A bound run's client session is no acting party: it cancels no run through the client action,
//! its own child or any other. The store's cancellation entry point holds a run to its subtree
//! as well (`a_run_requests_cancellation_only_within_its_subtree`).
use super::*;
use crate::model::MissionRunRequest;

const NODE: &str = "reach-node";

fn publish(state: &AppState, source: &str, key: &str) {
    let intent = crate::graph::parse_intent(source, NODE).unwrap();
    let preview = state
        .store
        .mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None })
        .unwrap();
    state.store.apply_as(&intent, &preview.subject_tokens, key, Some("person/operator")).unwrap();
}

fn chore(key: &str) -> MissionRunRequest {
    MissionRunRequest {
        mission: "chore".into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/operator".into()),
        mode: None,
        inputs: BTreeMap::new(),
        idempotency_key: key.into(),
    }
}

#[tokio::test]
async fn a_bound_run_cancels_no_run_through_the_client() {
    let root = tempfile::tempdir().unwrap();
    let state = tests::test_state_named(root.path(), NODE);
    let step = state.store.start_program_step_for_tests();
    let run = state.store.mission_run(&step.run).unwrap().unwrap();
    publish(&state, "version 2\nmission \"chore\" state=\"ready\" {\n  goal \"Do a chore.\"\n  concurrent-runs max=4\n  step \"work\" { agentless }\n}\n", "chore");
    let child = state.store.create_child_mission_run(&chore("reach-child"), &run, &step.subject, None).unwrap();
    let unrelated = state.store.create_mission_run(&chore("reach-unrelated")).unwrap();
    let session = ClientSession::for_tests_scoped(&step.run, &step.run, &["control.missions"]);
    let phase = |target: &crate::model::MissionRunView| state.store.mission_run(&target.subject).unwrap().unwrap().phase;
    for (target, key) in [(&unrelated, "reach-cancel-unrelated"), (&child, "reach-cancel-child")] {
        let snapshot = new_client_snapshot(&state);
        let request = ActionRequest {
            api_version: CLIENT_API_VERSION.into(),
            id: format!("action/{key}"),
            action_type: "mission.cancel".into(),
            idempotency_key: key.into(),
            fence: Fence {
                snapshot_id: snapshot.id.clone(),
                mission_generation: Some(target.generation.clone()),
                ..Default::default()
            },
            parameters: json!({"target_id": target.subject, "reason": "the test cancels it"}),
        };
        let before = state.store.index().unwrap();
        let refused = action_inner(State(state.clone()), Extension(snapshot), Extension(session.clone()), Json(request), None)
            .await
            .unwrap_err();
        assert_eq!(refused.code, "forbidden", "{key}");
        assert_eq!(state.store.index().unwrap(), before, "{key}");
        assert_eq!(phase(target), "normal", "{key}");
    }
}
