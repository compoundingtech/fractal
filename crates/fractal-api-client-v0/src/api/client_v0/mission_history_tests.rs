//! The mission detail's step history, per-step ETA and requested run, against runs driven through
//! claims, failures, retries, declared waits and asks.
#![allow(unused_imports)]
use super::*;
use crate::PersonWorkStore;
use crate::model::{MissionRunRequest, PersonAskRequest, PersonStepResponse, WorkRequest};

const AGENT: &str = "agent/history.worker";

fn state(root: &Path) -> AppState {
    super::tests::test_state_named(root, "history-node")
}

fn publish(state: &AppState, source: &str, key: &str) {
    let intent = crate::graph::parse_intent(source, "history-node").unwrap();
    let preview = state
        .store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    state
        .store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            key,
            Some("person/operator"),
        )
        .unwrap();
}

fn start(state: &AppState, mission: &str, key: &str) -> crate::model::MissionRunView {
    state
        .store
        .create_mission_run(&MissionRunRequest {
            mission: mission.into(),
            revision: None,
            workspace: "/work/history".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: key.into(),
        })
        .unwrap()
}

fn act(state: &AppState, subject: &str, action: &str, wait_ms: Option<u64>, key: &str) {
    state
        .store
        .work_action(
            subject,
            action,
            &WorkRequest {
                wait_ms,
                actor: Some(AGENT.into()),
                incarnation: Some(format!("{AGENT}-1")),
                summary: Some(format!("{action} summary")),
                reason: Some(format!("{action} reason")),
                evidence: Vec::new(),
                idempotency_key: key.into(),
            },
        )
        .unwrap();
}

fn detail(state: &AppState, mission: &str, run: Option<&str>) -> Value {
    let index = state.store.index().unwrap();
    let mut items =
        mission_resources_for_run(&state.store, index, true, Some(mission), run).unwrap();
    assert_eq!(items.len(), 1);
    items.remove(0)
}

fn run_detail<'a>(mission: &'a Value, run: &str) -> &'a Value {
    mission["run_details"]
        .as_array()
        .unwrap()
        .iter()
        .find(|detail| detail["id"] == run)
        .unwrap()
}

fn step<'a>(run: &'a Value, path: &str) -> &'a Value {
    run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["path"] == path)
        .unwrap()
}

#[test]
fn mission_detail_folds_each_attempt_wait_and_transition_of_a_step() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    publish(
        &state,
        "version 2\nagent \"history.worker\" { workspace \"/tmp\"; command \"true\"; restart always; }\n\
         mission \"history\" state=\"ready\" {\n  goal \"Build.\"\n  step \"build\" { assigned-to \"agent/history.worker\" }\n}\n",
        "publish-history",
    );
    let run = start(&state, "history", "history-1");
    let build = run
        .steps
        .iter()
        .find(|step| step.step == "build")
        .unwrap()
        .subject
        .clone();

    // Attempt 1 fails, and its retry backs off for a minute.
    state.store.set_step_state(&build, "ready", None).unwrap();
    act(&state, &build, "claim", None, "claim-1");
    act(&state, &build, "fail", None, "fail-1");
    assert!(
        state
            .store
            .retry_step(&build, "the tests failed", 60_000)
            .unwrap()
    );
    // Attempt 2 declares a wait, resumes, asks a person and finishes.
    state.store.set_step_state(&build, "ready", None).unwrap();
    act(&state, &build, "claim", None, "claim-2");
    act(&state, &build, "wait", Some(3_600_000), "wait-2");
    act(&state, &build, "resume", None, "resume-2");
    let ask = state
        .store
        .ask_person(&PersonAskRequest {
            priority: None,
            legacy_request: None,
            person: "person/ada".into(),
            title: "Which target?".into(),
            reason: "Two are named.".into(),
            actor: AGENT.into(),
            step: Some(build.clone()),
            new_run: None,
            incarnation: Some(format!("{AGENT}-1")),
            request: None,
            deadline_ms: None,
            default_answer: None,
            idempotency_key: "ask-target".into(),
        })
        .unwrap();
    state
        .store
        .finish_person_step(
            &PersonStepResponse {
                delegation: None,
                subject: ask.subject.clone(),
                actor: "person/ada".into(),
                summary: "The first.".into(),
                evidence: Vec::new(),
                episode: None,
                answer: None,
                idempotency_key: "answer-target".into(),
            },
            false,
        )
        .unwrap();
    // The answer readies the step again for its holder, which claims it in the same attempt.
    act(&state, &build, "claim", None, "reclaim-2");
    act(&state, &build, "complete", None, "complete-2");

    let mission = detail(&state, "mission/history", None);
    let shown = step(run_detail(&mission, &run.subject), "build");
    assert_eq!(shown["history_complete"], true);
    let attempts = shown["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2, "{shown:#}");
    assert_eq!(attempts[0]["n"], 1);
    assert_eq!(attempts[0]["claimant"], AGENT);
    assert_eq!(attempts[0]["outcome"], "failed");
    assert_eq!(attempts[0]["reason"], "fail reason");
    assert!(attempts[0]["started_at"].as_u64() <= attempts[0]["ended_at"].as_u64());
    assert_eq!(attempts[1]["n"], 2);
    assert_eq!(
        attempts[1]["outcome"], "running",
        "submitted, awaiting verification"
    );
    assert!(attempts[1]["ended_at"].is_null());

    let waits = shown["waits"].as_array().unwrap();
    let wait = |kind: &str| {
        waits
            .iter()
            .find(|wait| wait["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} wait in {waits:#?}"))
    };
    let retry = wait("retry");
    assert_eq!(
        (retry["attempt"].as_u64(), retry["reason"].as_str()),
        (Some(2), Some("the tests failed"))
    );
    assert!(retry["until"].as_u64() > retry["start"].as_u64());
    assert!(retry["end"].is_u64(), "the second claim ended the backoff");
    let paused = wait("paused");
    assert_eq!(paused["reason"], "wait reason");
    assert!(paused["end"].is_u64() && paused["until"].is_u64());
    let person = wait("you");
    assert_eq!(
        (person["attempt"].as_u64(), person["reason"].as_str()),
        (Some(2), Some("Which target?"))
    );
    assert!(person["end"].is_u64(), "the answer closed it");

    let transitions = shown["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|transition| {
            (
                transition["status"].as_str().unwrap(),
                transition["attempt"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    for expected in [
        ("claimed", 1),
        ("failed", 1),
        ("waiting", 2),
        ("waiting-person", 2),
        ("ready", 2),
        ("verifying", 2),
    ] {
        assert!(
            transitions.contains(&expected),
            "{expected:?} in {transitions:?}"
        );
    }
    // The ask is a step of the run too, and waits on its person itself.
    let ask_step = run_detail(&mission, &run.subject)["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == ask.subject.as_str())
        .unwrap();
    assert!(
        ask_step["waits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|wait| wait["kind"] == "you")
    );
    assert_eq!(ask_step["transitions"].as_array().unwrap().last().unwrap()["status"], "completed");
    assert!(ask_step["waits"].as_array().unwrap().iter().all(|wait| wait["end"].is_u64()));

    // Collection reads never carry the history.
    let index = state.store.index().unwrap();
    let listed = mission_resources(&state.store, index, true, None).unwrap();
    let listed = listed
        .iter()
        .find(|item| item["id"] == "mission/history")
        .unwrap();
    for run in listed["run_details"].as_array().unwrap() {
        for step in run["steps"].as_array().into_iter().flatten() {
            for field in [
                "attempts",
                "waits",
                "transitions",
                "history_complete",
                "eta",
            ] {
                assert!(step.get(field).is_none(), "{field} on a list read");
            }
        }
    }
}

#[test]
fn person_responses_close_waits_and_emit_terminal_transitions() {
    for (cancel_run, cancel, terminal) in [(false, false, "completed"), (false, true, "cancelled"), (true, true, "cancelled")] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let source = if cancel {
            "version 2\nagent \"history.worker\" { workspace \"/tmp\"; command \"true\"; restart always; }\nmission \"person-history\" state=\"ready\" {\n goal \"Decide.\"\n step \"decide\" { assigned-to \"agent/history.worker\" }\n}\n"
        } else {
            "version 2\nmission \"person-history\" state=\"ready\" {\n goal \"Decide.\"\n step \"decide\" { assigned-to \"person/ada\" }\n}\n"
        };
        publish(&state, source, "publish-person-history");
        let run = start(&state, "person-history", "person-history-1");
        let origin = &run.steps.iter().find(|step| step.step == "decide").unwrap().subject;
        state.store.set_step_state(origin, "ready", None).unwrap();
        let subject = if cancel {
            act(&state, origin, "claim", None, "claim-before-ask");
            state.store.ask_person(&PersonAskRequest {
                priority: None,
                legacy_request: None,
                person: "person/ada".into(),
                title: "Which target?".into(),
                reason: "Two are named.".into(),
                actor: AGENT.into(),
                step: Some(origin.clone()),
                new_run: None,
                incarnation: Some(format!("{AGENT}-1")),
                request: None,
                deadline_ms: None,
                default_answer: None,
                idempotency_key: "ask-to-cancel".into(),
            }).unwrap().subject
        } else {
            origin.clone()
        };
        if cancel_run {
            state.store.request_mission_run_cancellation(&run.id, "cancel the parent").unwrap();
        } else {
        state.store.finish_person_step(
            &PersonStepResponse {
                delegation: None,
                subject: subject.clone(),
                actor: if cancel { AGENT } else { "person/ada" }.into(),
                summary: "Decision recorded.".into(),
                evidence: Vec::new(),
                episode: None,
                answer: None,
                idempotency_key: "person-response".into(),
            },
            cancel,
        ).unwrap();
        }
        let mission = detail(&state, "mission/person-history", None);
        let shown = run_detail(&mission, &run.subject)["steps"].as_array().unwrap()
            .iter().find(|step| step["id"] == subject).unwrap();
        let transition = shown["transitions"].as_array().unwrap().last().unwrap();
        assert_eq!(transition["status"], terminal);
        assert_eq!(transition["event"], "state");
        let waits = shown["waits"].as_array().unwrap();
        assert!(waits.iter().any(|wait| wait["kind"] == if cancel { "you" } else { "queue" }));
        assert!(waits.iter().all(|wait| wait["end"].is_u64()), "{shown:#}");
        if cancel_run {
            let origin = step(run_detail(&mission, &run.subject), "decide");
            assert!(origin["waits"].as_array().unwrap().iter().all(|wait| wait["end"].is_u64()), "{origin:#}");
            assert_eq!(origin["history_complete"], true);
        }
    }
}

#[tokio::test]
async fn a_requested_run_lifts_the_timing_and_history_bound_for_an_old_run() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    publish(
        &state,
        "version 2\nmission \"old-runs\" state=\"ready\" {\n  goal \"Run.\"\n  concurrent-runs max=10\n  step \"work\" { agentless }\n}\n",
        "publish-old-runs",
    );
    publish(
        &state,
        "version 2\nmission \"other\" state=\"ready\" { goal \"Other.\" }\n",
        "publish-other",
    );
    let mut runs = Vec::new();
    for index in 0..3 {
        let run = start(&state, "old-runs", &format!("old-{index}"));
        state
            .store
            .set_step_state(&run.steps[0].subject, "working", None)
            .unwrap();
        state
            .store
            .set_step_state(&run.steps[0].subject, "completed", None)
            .unwrap();
        for phase in ["cleanup-completed", "terminal"] {
            state
                .store
                .set_mission_run_state(&run.id, "completed", phase, Some("done"))
                .unwrap();
        }
        runs.push(run.subject);
    }
    let oldest = &runs[0];

    let bounded = detail(&state, "mission/old-runs", None);
    let shown = step(run_detail(&bounded, oldest), "work");
    assert!(shown["started_at"].is_null());
    assert!(
        shown.get("attempts").is_none(),
        "an older finished run stays lightweight"
    );
    assert!(step(run_detail(&bounded, &runs[2]), "work")["attempts"].is_array());

    let session =
        ClientSession::for_tests_scoped("client/test", "person/operator", &["read.projections"]);
    let read = |run: Option<String>| {
        let state = state.clone();
        let session = session.clone();
        async move {
            mission_detail(
                State(state.clone()),
                Extension(new_client_snapshot(&state)),
                Extension(session),
                AxumPath("mission/old-runs".to_owned()),
                Query(MissionDetailQuery { run }),
            )
            .await
            .map(|Json(value)| value)
        }
    };
    for requested in [
        oldest.clone(),
        oldest.trim_start_matches("mission-run/").to_owned(),
    ] {
        let lifted = read(Some(requested)).await.unwrap();
        let shown = step(run_detail(&lifted, oldest), "work");
        assert!(
            shown["started_at"].is_u64() && shown["completed_at"].is_u64(),
            "{shown:#}"
        );
        let attempts = shown["attempts"].as_array().unwrap();
        assert_eq!(
            (attempts.len(), attempts[0]["outcome"].as_str()),
            (1, Some("done"))
        );
        // The bound stays on the runs not asked for.
        assert!(
            step(run_detail(&lifted, &runs[1]), "work")
                .get("attempts")
                .is_none()
        );
    }
    let other = start(&state, "other", "other-1");
    let error = read(Some(other.subject.clone())).await.unwrap_err();
    assert_eq!(error.status, StatusCode::NOT_FOUND);
    assert!(read(None).await.is_ok());
}

#[test]
fn mission_detail_estimates_remaining_steps_that_have_samples_without_changing_the_run_eta() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    publish(
        &state,
        "version 2\nmission \"estimate\" state=\"ready\" {\n  goal \"Estimate.\"\n  concurrent-runs max=10\n  step \"sampled\" { agentless }\n  step \"unsampled\" { agentless }\n}\n",
        "publish-estimate",
    );
    // A prior run that finished only one of its steps leaves a sample for that step alone.
    let prior = start(&state, "estimate", "prior");
    let sampled = prior
        .steps
        .iter()
        .find(|step| step.step == "sampled")
        .unwrap();
    state
        .store
        .set_step_state(&sampled.subject, "working", None)
        .unwrap();
    state
        .store
        .set_step_state(&sampled.subject, "completed", None)
        .unwrap();
    for phase in ["cleanup-completed", "terminal"] {
        state
            .store
            .set_mission_run_state(&prior.id, "completed", phase, Some("done"))
            .unwrap();
    }
    let current = start(&state, "estimate", "current");

    let mission = detail(&state, "mission/estimate", None);
    let run = run_detail(&mission, &current.subject);
    assert!(
        run["eta"].is_null(),
        "a remaining step without samples still nulls the run ETA"
    );
    let eta = &step(run, "sampled")["eta"];
    assert_eq!(eta["samples"], 1, "{run:#}");
    assert!(eta["low_ms"].is_u64() && eta["low_ms"] == eta["high_ms"]);
    assert!(step(run, "unsampled").get("eta").is_none());
    // A finished run estimates nothing.
    assert!(
        step(run_detail(&mission, &prior.subject), "sampled")
            .get("eta")
            .is_none()
    );
}
