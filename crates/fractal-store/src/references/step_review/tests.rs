use super::*;
use crate::model::MissionRunRequest;
use crate::store::tests_shared_missions::publish_carry;
use crate::store::{Store, append_claim_tx};
use serde_json::{Value, json};

const REVIEWER: &str = "agent/example/reviewer";
const CONFLICT: &str = "reviewer-not-independent";
const UNKNOWN: &str = "reviewer-owner-unresolved";
const PERSON: &str = "person/operator";

fn fixture(selector: &str) -> (Store, StepRunView) {
    let store = Store::open_memory("node").unwrap();
    let source = format!(
        r#"version 2
agent "example/reviewer" {{ command "true"; workspace "/tmp" }}
agent "example/old" {{ command "true"; workspace "/tmp" }}
agent "example/new" {{ command "true"; workspace "/tmp" }}
mission "carry" state="ready" {{
  goal "Check responsibility."
  agent "worker" {{ command "true"; workspace "/tmp" }}
  step "work" {{ {selector} }}
}}"#
    );
    publish_carry(&store, &source, "declare");
    let request = MissionRunRequest {
        mission: "carry".into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some(PERSON.into()),
        mode: None,
        inputs: Default::default(),
        idempotency_key: "run".into(),
    };
    let run = store
        .create_mission_run_inner(&request, None, None, None)
        .unwrap();
    let step = run.steps.into_iter().find(|s| s.step == "work").unwrap();
    (store, step)
}

fn check(store: &Store, step: &StepRunView, requester: &str, reviewer: &str) -> String {
    validate_step_reviewer(
        &Scope {
            run: BTreeSet::new(),
            intent: None,
        },
        &store.connection.write(),
        step,
        requester,
        reviewer,
    )
    .inspect_err(|error| eprintln!("{error:?}"))
    .map_or_else(|error| error.code.to_owned(), |_| "ok".into())
}

#[test]
fn agent_review_step_concrete_boundaries() {
    for selector in ["agentless", r#"assigned-to "agent/example/old""#] {
        let (store, step) = fixture(selector);
        assert_eq!(check(&store, &step, PERSON, REVIEWER), "ok");
        assert_eq!(check(&store, &step, REVIEWER, REVIEWER), CONFLICT);
        assert_eq!(step.assigned_to.is_none(), selector == "agentless");
        if let Some(worker) = &step.assigned_to {
            assert_eq!(check(&store, &step, PERSON, worker), CONFLICT);
        }
    }
    let (store, step) =
        fixture(r#"available-to "agent/example/new"; available-to "agent/example/reviewer""#);
    assert!(step.claimant.is_none());
    assert_eq!(check(&store, &step, PERSON, REVIEWER), CONFLICT);
    // Applied run-scoped declarations are covered by slice 5b.
    let (store, step) = fixture(r#"assigned-to "agent/${ST_MISSION_RUN}/worker""#);
    assert_eq!(check(&store, &step, PERSON, REVIEWER), UNKNOWN);
}

#[test]
fn agent_review_step_unknown_requires_later_validation() {
    let (store, mut step) = fixture("agentless");
    assert_eq!(
        check(&store, &step, "person/${REQUESTER}", REVIEWER),
        UNKNOWN
    );
    step.agentless = false;
    for owner in [
        None,
        Some("agent/example/missing"),
        Some("agent/example/st-run-value-owner"),
        Some("agent/${OWNER}"),
        Some("person/${OWNER}"),
    ] {
        step.assigned_to = owner.map(str::to_owned);
        assert_eq!(check(&store, &step, PERSON, REVIEWER), UNKNOWN);
    }
    step.assigned_to = Some("agent/example/new".into());
    assert_eq!(check(&store, &step, PERSON, REVIEWER), "ok");
    step.assigned_to = Some(REVIEWER.into());
    assert_eq!(check(&store, &step, PERSON, REVIEWER), CONFLICT);
}

fn event(store: &Store, step: &StepRunView, kind: &str, actor: Option<&str>, body: &Value) {
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    append_claim_tx(
        &transaction,
        store.origin(),
        &step.subject,
        kind,
        actor,
        body,
        &[],
        None,
    )
    .unwrap();
    transaction.commit().unwrap();
}

#[test]
fn agent_review_step_local_history_survives_new_claim() {
    for kind in ["work.claimed", "step-run.carried"] {
        let (store, step) = fixture(r#"assigned-to "agent/example/new""#);
        for (kind, body) in [
            (
                kind,
                json!({"attempt": 1, "status": "ready", "claimant": "agent/example/old"}),
            ),
            (
                "work.submitted",
                json!({"fields": {"attempt": 1, "claimant": null}}),
            ),
            (
                "work.claimed",
                json!({"fields": {"attempt": 1, "claimant": "agent/example/new"}}),
            ),
        ] {
            event(&store, &step, kind, Some("agent/example/old"), &body);
        }
        for worker in ["agent/example/old", "agent/example/new"] {
            assert_eq!(check(&store, &step, PERSON, worker), CONFLICT);
        }
    }
}

#[test]
fn agent_review_step_work_actor_without_prior_claim() {
    for (kind, actor, expected) in [
        ("work.person-done", Some(REVIEWER), CONFLICT),
        ("work.person-done", Some(PERSON), "ok"),
        ("work.person-done", None, "ok"),
        ("work.submitted", Some(REVIEWER), CONFLICT),
        ("work.submitted", Some("agent/example/missing"), UNKNOWN),
        ("work.submitted", None, UNKNOWN),
    ] {
        let (store, step) = fixture(r#"assigned-to "agent/example/new""#);
        event(
            &store,
            &step,
            kind,
            actor,
            &json!({"fields": {"attempt": 1, "status": "verifying", "claimant": null}}),
        );
        assert_eq!(check(&store, &step, PERSON, REVIEWER), expected);
    }
}
