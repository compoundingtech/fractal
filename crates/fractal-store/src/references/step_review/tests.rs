use super::*;
use crate::model::MissionRunRequest;
use crate::store::tests_shared_missions::publish_carry;
use crate::store::{Store, append_claim_tx};
use serde_json::json;
use std::collections::BTreeMap;

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
  step "source" {{ agentless }}
}}"#
    );
    publish_carry(&store, &source, "declare");
    let run = store
        .create_mission_run_inner(
            &MissionRunRequest {
                mission: "carry".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some(PERSON.into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: "run".into(),
            },
            None,
            None,
            None,
        )
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
    .map_or_else(|error| error.code.to_owned(), |_| "ok".into())
}

#[test]
fn agent_review_step_concrete_boundaries() {
    for selector in ["agentless", r#"assigned-to "agent/${ST_MISSION_RUN}/worker""#] {
        let (store, step) = fixture(selector);
        assert_eq!(check(&store, &step, PERSON, REVIEWER), "ok");
        assert_eq!(check(&store, &step, REVIEWER, REVIEWER), CONFLICT);
        if let Some(worker) = &step.assigned_to {
            assert!(!worker.contains("${"));
            assert_eq!(check(&store, &step, PERSON, worker), CONFLICT);
        } else {
            assert!(step.agentless);
        }
    }
    let (store, step) = fixture(
        r#"available-to "agent/example/new"; available-to "agent/example/reviewer""#,
    );
    assert!(step.claimant.is_none());
    assert_eq!(check(&store, &step, PERSON, REVIEWER), CONFLICT);
}

#[test]
fn agent_review_step_unknown_requires_later_validation() {
    let (store, mut step) = fixture("agentless");
    assert!(step.assigned_to.is_none());
    step.agentless = false;
    for owner in [
        None,
        Some("agent/example/missing"),
        Some("agent/example/st-run-value-owner"),
        Some("agent/${OWNER}"),
    ] {
        step.assigned_to = owner.map(str::to_owned);
        assert_eq!(check(&store, &step, PERSON, REVIEWER), UNKNOWN);
    }
    step.assigned_to = Some("agent/example/new".into());
    assert_eq!(check(&store, &step, PERSON, REVIEWER), "ok");
    step.assigned_to = Some(REVIEWER.into());
    assert_eq!(check(&store, &step, PERSON, REVIEWER), CONFLICT);
}

#[test]
fn agent_review_step_carried_owner_survives_new_claim() {
    let (store, step) = fixture(r#"assigned-to "agent/example/new""#);
    let source = store
        .mission_run(&step.run)
        .unwrap()
        .unwrap()
        .steps
        .into_iter()
        .find(|s| s.step == "source")
        .unwrap()
        .subject;
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    for (subject, kind, fields) in [
        (
            &source,
            "work.submitted",
            json!({"attempt": 1, "claimant": "agent/example/old"}),
        ),
        (
            &step.subject,
            "step-run.carried",
            json!({"attempt": 1, "status": "ready", "claimant": "agent/example/old", "source_step_run": source}),
        ),
        (
            &step.subject,
            "work.claimed",
            json!({"attempt": 1, "claimant": "agent/example/new"}),
        ),
    ] {
        append_claim_tx(
            &transaction,
            store.origin(),
            subject,
            kind,
            None,
            &json!({"fields": fields}),
            &[],
            None,
        )
        .unwrap();
    }
    transaction.commit().unwrap();
    drop(connection);
    for worker in ["agent/example/old", "agent/example/new"] {
        assert_eq!(check(&store, &step, PERSON, worker), CONFLICT);
    }
    assert_eq!(check(&store, &step, PERSON, REVIEWER), "ok");
}
