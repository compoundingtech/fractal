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
  step "source" {{ agentless }}
  step "middle" {{ agentless }}
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

fn sibling(store: &Store, step: &StepRunView, name: &str) -> StepRunView {
    let subject = format!("{}/{name}", step.subject.strip_suffix("/work").unwrap());
    step_run_row_tx(&store.connection.write(), &subject)
        .unwrap()
        .unwrap()
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
    // Applied run-scoped declarations require the later fractal-missions slice.
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
        let source = sibling(&store, &step, "source");
        let mut bare = json!({"attempt": 1, "status": "ready", "claimant": "agent/example/old"});
        if kind == "step-run.carried" {
            bare["source_step_run"] = json!(source.subject);
        }
        for (kind, body) in [
            (kind, bare),
            (
                "work.submitted",
                json!({"fields": {"attempt": 1, "claimant": null}}),
            ),
            (
                "work.claimed",
                json!({"fields": {"attempt": 1, "claimant": "agent/example/new"}}),
            ),
        ] {
            event(&store, &step, kind, Some(PERSON), &body);
        }
        for worker in ["agent/example/old", "agent/example/new"] {
            assert_eq!(check(&store, &step, PERSON, worker), CONFLICT);
        }
    }
}

#[test]
fn agent_review_step_work_actor_without_prior_claim() {
    for (kind, actor, expected) in [
        ("work.progress", Some(REVIEWER), CONFLICT),
        ("work.progress", Some(PERSON), "ok"),
        ("work.progress", None, "ok"),
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

#[test]
fn agent_review_step_carried_chain_bare_and_wrapped() {
    for bare in [false, true] {
        let (store, step) = fixture(r#"assigned-to "agent/example/new""#);
        let middle = sibling(&store, &step, "middle");
        let source = sibling(&store, &step, "source");
        // Only the oldest source's work actor names this reviewer.
        event(&store, &source, "work.progress", Some(REVIEWER), &json!({}));
        for (target, source) in [(&step, &middle), (&middle, &source)] {
            let fields =
                json!({"source_step_run": source.subject, "attempt": 1, "status": "ready"});
            let body = if bare {
                fields
            } else {
                json!({"fields": fields})
            };
            event(&store, target, "step-run.carried", Some(PERSON), &body);
        }
        assert_eq!(check(&store, &step, PERSON, REVIEWER), CONFLICT);
        assert_eq!(check(&store, &step, PERSON, "agent/example/old"), "ok");
    }
}

#[test]
fn agent_review_step_carried_source_fails_closed() {
    for source in [
        json!("step-run/${SOURCE}/work"),
        json!("agent/example/new"),
        json!("step-run/missing/work"),
    ] {
        let (store, step) = fixture("agentless");
        event(
            &store,
            &step,
            "step-run.carried",
            Some(PERSON),
            &json!({"fields": {"source_step_run": source, "attempt": 1, "status": "ready"}}),
        );
        assert_eq!(check(&store, &step, PERSON, REVIEWER), UNKNOWN);
    }
    let (store, step) = fixture("agentless");
    let source = sibling(&store, &step, "source");
    for (target, source) in [(&step, &source), (&source, &step)] {
        event(
            &store,
            target,
            "step-run.carried",
            Some(PERSON),
            &json!({"source_step_run": source.subject, "attempt": 1, "status": "ready"}),
        );
    }
    assert_eq!(check(&store, &step, PERSON, REVIEWER), UNKNOWN);
    let (store, step) = fixture("agentless");
    let source = sibling(&store, &step, "source");
    event(
        &store,
        &step,
        "step-run.carried",
        Some(PERSON),
        &json!({"source_step_run": source.subject, "attempt": 1, "status": "ready"}),
    );
    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-hf97024592eee owner=fractal_store::references::step_review::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM step_runs WHERE subject=?1", [&source.subject])
        .unwrap();
    assert_eq!(check(&store, &step, PERSON, REVIEWER), UNKNOWN);
}

#[test]
fn agent_review_step_owner_rename_and_pending_naming() {
    use crate::store::identities::{IdentityKind, IdentityStore, RenameRequest};
    let (store, mut step) = fixture(r#"assigned-to "agent/example/old""#);
    store
        .rename_identity(
            IdentityKind::Agent,
            &RenameRequest {
                from: "agent/example/old".into(),
                to: "agent/example/renamed".into(),
                actor: PERSON.into(),
                reason: "Clarify responsibility".into(),
                expected_naming_revision: None,
                expected_declaration: None,
                idempotency_key: "rename".into(),
            },
            &[],
        )
        .unwrap();
    step.assigned_to = Some("agent/example/renamed".into());
    assert_eq!(check(&store, &step, PERSON, "agent/example/old"), CONFLICT);
    assert_eq!(check(&store, &step, PERSON, REVIEWER), "ok");
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hdc4eec22f34b owner=fractal_store::references::step_review::tests TODO=chunk-or-reference-exact-wire-values */
        "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,subject_hint,kind_hint,updated_at_unix_ms)
         VALUES('record/pending','example-peer',1,'pending',0,'{}','unknown',?1,'agent.renamed','1')",
        ["agent/example/old"],
    ).unwrap();
    assert_eq!(check(&store, &step, PERSON, REVIEWER), UNKNOWN);
    // Reviewer naming classification is intentionally unchanged.
    step.assigned_to = Some("agent/example/new".into());
    assert_eq!(
        check(&store, &step, PERSON, "agent/example/renamed"),
        "lifecycle-unavailable"
    );
}
