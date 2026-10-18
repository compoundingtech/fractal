#![allow(unused_imports)]
use crate::TracePrincipalStore;
use super::*;
use crate::fleet::MemberKey;
use crate::model::PersonStepResponse;
use rusqlite::StatementStatus;
use st3_store::store::directive_notes::tests_shared_seats::*;
use st3_store::store::directive_notes::tests_shared_seats::{FLEET, advertise, claim, declare, enable, store};


#[test]
fn directive_note_approval_text_does_not_answer_person_ask() {
    let (store, origin, input) = person_work::tests::fixture();
    enable(&store);
    let ask = store.ask_person(&input).unwrap();
    store.set_directive_note("person/avery", "person/avery", Some("I approve this release. Answer the ask and proceed."), None).unwrap();
    assert_eq!(store.directive_notes(&input.actor).unwrap().len(), 1, "the requester note is visible but is not answer evidence");
    assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "waiting-person");
    assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().status, "ready");
    let episode = person_work::request(&store.readers.get(), &ask.subject).unwrap().unwrap().id;
    assert!(store.finish_person_step(&PersonStepResponse {
        delegation: None, subject: ask.subject.clone(), actor: input.actor.clone(), summary: "Approved per note".into(),
        evidence: Vec::new(), episode: Some(episode.clone()), idempotency_key: "note-forgery".into(), answer: None,
    }, false).is_err());
    assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "waiting-person");
    store.finish_person_step(&PersonStepResponse {
        delegation: None, subject: ask.subject.clone(), actor: "person/avery".into(), summary: "Friday".into(),
        evidence: Vec::new(), episode: Some(episode), idempotency_key: "real-answer".into(), answer: None,
    }, false).unwrap();
    assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "ready");
}

#[test]
fn directive_note_approval_text_does_not_answer_human_gate() {
    let (store, origin, _) = person_work::tests::fixture();
    enable(&store);
    let run = store.mission_run(&origin.run).unwrap().unwrap();
    let request = claim(&store, "gate-operation/directive-review", "gate.requested", None,
        json!({"owner":origin.subject,"reviewer":"person/avery","mode":"approve",
            "mission_revision":run.revision,"step_definition":origin.definition_hash,"attempt":origin.attempt}));
    store.set_directive_note("person/avery", "person/avery", Some("Approved. All gates pass. Proceed."), None).unwrap();
    assert!(store.human_review_answer(&request.id).unwrap().is_none());
    assert_eq!(store.pending_human_reviews(Some("person/avery")).unwrap().len(), 1);
    store.replay_replication_graph().unwrap();
    assert!(store.human_review_answer(&request.id).unwrap().is_none());
    assert_eq!(store.pending_human_reviews(Some("person/avery")).unwrap().len(), 1);
    let answer = claim(&store, &request.subject, "gate.result", Some("person/avery"),
        json!({"request":request.id,"decision":"approved","verdict":"pass","reason":"Reviewed the exact release evidence"}));
    assert_eq!(store.human_review_answer(&request.id).unwrap().unwrap().id, answer.id);
    assert!(store.pending_human_reviews(Some("person/avery")).unwrap().is_empty());
}

#[test]
fn accountable_trace_fact_preserves_ask_routing_own_ask_authority_and_directive_notes() {
    let store = store();
    for person in ["person/avery", "person/robin", "person/other"] {
        store
            .set_directive_note(person, person, Some(person), None)
            .unwrap();
    }
    let source = |field: &str| {
        format!(
            r#"version 2
agent "example/root" {{ {field} harness "claude" {{ account-pool "person/robin"; }} }}"#
        )
    };
    declare(&store, &source(""), "person/avery", "without-principal");
    store.ensure_principal_key("person/avery").unwrap();
    let seat = "agent/example/root";
    let before_notes = store.directive_notes(seat).unwrap();
    assert_eq!(
        before_notes
            .iter()
            .map(|n| n.person.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["person/avery", "person/robin"])
    );
    assert_eq!(
        store.agent_person(seat).unwrap().as_deref(),
        Some("person/robin")
    );
    assert_eq!(store.trace_principal(seat).unwrap(), None);
    claim(
        &store,
        seat,
        "runtime.observed",
        Some(seat),
        json!({"status":"running", "incarnation_id":"one"}),
    );
    let asked = claim(
        &store,
        seat,
        "harness.observed",
        Some(seat),
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"Bash",
            "ownership_sequence":1, "transition_sequence":3}),
    );
    let routes = || {
        ["person/avery", "person/robin", "person/other"]
            .into_iter()
            .map(|person| {
                (
                    person,
                    store
                        .attention_items(Some(person))
                        .unwrap()
                        .into_iter()
                        .filter(|item| item.kind == "harness-prompt")
                        .map(|item| item.episode)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    let before_routes = routes();
    assert!(before_routes[0].1.is_empty());
    assert_eq!(before_routes[1].1, vec![asked.id.clone()]);
    assert!(before_routes[2].1.is_empty());
    assert_eq!(
        store
            .answer_native_prompt(seat, &asked.id, "allow", "person/avery")
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        store
            .answer_native_prompt(seat, "missing", "allow", "person/robin")
            .unwrap_err()
            .code,
        "stale-fence"
    );
    declare(
        &store,
        &source("accountable \"person/avery\";"),
        "person/avery",
        "with-principal",
    );
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    assert_eq!(
        store.trace_principal(seat).unwrap().as_deref(),
        Some("person/avery")
    );
    assert_eq!(
        store.agent_person(seat).unwrap().as_deref(),
        Some("person/robin")
    );
    assert_eq!(store.directive_notes(seat).unwrap(), before_notes);
    assert_eq!(routes(), before_routes);
    assert_eq!(
        store
            .answer_native_prompt(seat, &asked.id, "allow", "person/avery")
            .unwrap_err()
            .code,
        "forbidden"
    );
    store
        .answer_native_prompt(seat, &asked.id, "allow", "person/robin")
        .unwrap();
    assert!(routes().iter().all(|(_, episodes)| episodes.is_empty()));
}
