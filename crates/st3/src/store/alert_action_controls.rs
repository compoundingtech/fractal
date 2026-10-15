//! Invented real-store controls; no live person policy or harness is touched.
use super::*;
use crate::model::{DelegationPolicyRequest, DelegationProof};

const SEAT: &str = "agent/alder.worker";
const ACTOR: &str = "agent/alder.helper";

fn append(
    store: &Store,
    subject: &str,
    kind: &str,
    actor: &str,
    fields: Value,
    key: &str,
) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: Some(actor.into()),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(key.into()),
        })
        .unwrap()
}

fn denial_fixture(actions: &[&str]) -> (Store, ClaimRecord, DelegationProof) {
    let store = Store::open_memory("alder").unwrap();
    let (denial, proof) = seed_denial(&store, actions);
    (store, denial, proof)
}

pub(crate) fn seed_denial(store: &Store, actions: &[&str]) -> (ClaimRecord, DelegationProof) {
    let source = "version 2\nagent \"worker\" { workspace \"/tmp\"; harness \"claude\" {} }\n";
    let intent = crate::graph::parse_internal_intent(source, "alder").unwrap();
    let preview = store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "declared",
            Some("person/avery"),
        )
        .unwrap();
    append(
        store,
        SEAT,
        "runtime.observed",
        SEAT,
        json!({"status":"running","incarnation_id":"one"}),
        "runtime",
    );
    let denial = append(
        store,
        SEAT,
        "harness.diagnostic",
        SEAT,
        json!({"code":"permission-denied","severity":"warning","status":"Bash","driver":"claude",
            "reason":"invented denied call","session_id":"session-one"}),
        "denial",
    );
    let instruction = append(
        store,
        "message/instruction",
        "message.sent",
        "person/avery",
        json!({"from":"person/avery","to":ACTOR,"content":"Dismiss that denied call.","status":"sent"}),
        "instruction",
    );
    let policy = store
        .set_delegation_policy(&DelegationPolicyRequest {
            person: "person/avery".into(),
            actor: "person/avery".into(),
            actions: actions.iter().map(|s| (*s).into()).collect(),
            evidence: vec![instruction.id],
            idempotency_key: "policy".into(),
        })
        .unwrap();
    let proof = DelegationProof {
        person: "person/avery".into(),
        policy: policy.id,
        message: "message/instruction".into(),
        quote: "Dismiss that denied call.".into(),
        episode: denial.id.clone(),
    };
    (denial, proof)
}

#[test]
fn granted_denial_dismissal_records_agent_person_and_closes_the_source_once() {
    let (store, denial, proof) = denial_fixture(&["close-item"]);
    assert!(
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .iter()
            .any(|item| item.episode == denial.id)
    );
    let result = store
        .respond_native_prompt(SEAT, &denial.id, "dismiss", ACTOR, Some(&proof))
        .unwrap();
    assert_eq!(result.actor.as_deref(), Some(ACTOR));
    assert_eq!(result.body["fields"]["acted_for"], proof.person);
    assert_eq!(result.body["fields"]["delegation"], json!(proof));
    assert_eq!(
        store.prompt_answer(&denial.id).unwrap().answer.as_deref(),
        Some("dismiss")
    );
    for _ in 0..2 {
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .all(|item| item.episode != denial.id)
        );
        assert_eq!(
            store
                .respond_native_prompt(SEAT, &denial.id, "dismiss", ACTOR, Some(&proof))
                .unwrap()
                .id,
            result.id
        );
    }
    let notices = store.messages(Some(SEAT), true).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("Do not run it"));
    assert!(notices[0].content.contains(ACTOR));
}

#[test]
fn dismissal_refuses_missing_wrong_revoked_and_forged_delegation_without_source_effects() {
    for case in 0..8 {
        let (store, denial, mut proof) = denial_fixture(if case == 0 {
            &["answer-ask"]
        } else {
            &["close-item"]
        });
        let actor = if case == 1 {
            "agent/alder.stranger"
        } else {
            ACTOR
        };
        match case {
            2 => proof.person = "person/other".into(),
            3 => proof.policy = "missing-policy".into(),
            4 => proof.quote = "Not in the person's message".into(),
            5 => proof.episode = "missing-episode".into(),
            6 => {
                store
                    .set_delegation_policy(&DelegationPolicyRequest {
                        person: proof.person.clone(),
                        actor: proof.person.clone(),
                        actions: vec![],
                        evidence: vec![proof.policy.clone()],
                        idempotency_key: "revoke".into(),
                    })
                    .unwrap();
            }
            _ => {}
        }
        assert!(
            store
                .respond_native_prompt(
                    SEAT,
                    &proof.episode,
                    "dismiss",
                    actor,
                    (case != 7).then_some(&proof)
                )
                .is_err(),
            "case {case}"
        );
        assert_eq!(
            store.prompt_answer(&denial.id).unwrap().answer,
            None,
            "case {case}"
        );
        assert!(
            store.messages(Some(SEAT), true).unwrap().is_empty(),
            "case {case}"
        );
    }
}

#[test]
fn close_item_never_authorizes_allow_kind_or_an_unproved_generic_harness_answer() {
    let (store, denial, proof) = denial_fixture(&["close-item"]);
    for answer in ["allow", "deny", "approve", "approve-once", "approve-kind"] {
        assert_eq!(
            store
                .respond_native_prompt(SEAT, &denial.id, answer, ACTOR, Some(&proof))
                .unwrap_err()
                .code,
            "delegation-refused"
        );
    }
    let input = ClaimInput {
        subject: SEAT.into(),
        kind: "harness.diagnostic".into(),
        actor: Some(ACTOR.into()),
        fields: serde_json::from_value(
            json!({"code":"native-prompt-answered","status":"dismiss","driver":"claude"}),
        )
        .unwrap(),
        evidence: vec![denial.id],
        expected_subject: None,
        idempotency_key: Some("unproved-answer".into()),
    };
    assert_eq!(
        store.append_claim(&input).unwrap_err().code,
        "delegation-refused"
    );
}

#[test]
fn delegated_dismissal_fences_a_restarted_runtime_and_changed_person() {
    for changed_person in [false, true] {
        let (store, denial, proof) = denial_fixture(&["close-item"]);
        if changed_person {
            let source =
                "version 2\nagent \"worker\" { workspace \"/tmp/other\"; harness \"claude\" {} }\n";
            let intent = crate::graph::parse_internal_intent(source, "alder").unwrap();
            let preview = store
                .mission(
                    &intent,
                    crate::model::IntentInput {
                        kdl: source.into(),
                        source_name: None,
                    },
                )
                .unwrap();
            store
                .apply_as(
                    &intent,
                    &preview.subject_tokens,
                    "changed-person",
                    Some("person/other"),
                )
                .unwrap();
        } else {
            append(
                &store,
                SEAT,
                "runtime.observed",
                SEAT,
                json!({"status":"running","incarnation_id":"two"}),
                "restarted",
            );
        }
        assert!(
            store
                .respond_native_prompt(SEAT, &denial.id, "dismiss", ACTOR, Some(&proof))
                .is_err()
        );
        assert_eq!(store.prompt_answer(&denial.id).unwrap().answer, None);
    }
}

#[test]
fn a_failed_denial_notice_rolls_back_the_delegated_close_and_operation() {
    let (store, denial, proof) = denial_fixture(&["close-item"]);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute_batch("CREATE TEMP TRIGGER refuse_denial_notice BEFORE INSERT ON claims WHEN NEW.kind='message.sent' BEGIN SELECT RAISE(ABORT,'authored notice failure'); END")?;
        Ok(())
    }).unwrap().unwrap();
    assert!(
        store
            .respond_native_prompt(SEAT, &denial.id, "dismiss", ACTOR, Some(&proof))
            .is_err()
    );
    assert_eq!(store.prompt_answer(&denial.id).unwrap().answer, None);
    store
        .connection
        .batched(|tx| -> Result<()> {
            tx.execute_batch("DROP TRIGGER refuse_denial_notice")?;
            Ok(())
        })
        .unwrap()
        .unwrap();
    store
        .respond_native_prompt(SEAT, &denial.id, "dismiss", ACTOR, Some(&proof))
        .unwrap();
}

#[test]
fn sender_withdraws_its_update_without_releasing_its_held_step_and_retains_history() {
    let (store, origin, mut input) = person_work::tests::fixture();
    input.step = None;
    input.incarnation = None;
    input.request = Some(json!({"version":1,"type":"update","about":origin.run}));
    let update = store.ask_person(&input).unwrap();
    let episode = store
        .claims_for(&update.subject, Some("work.person-asked"))
        .unwrap()[0]
        .id
        .clone();
    let before = store.step_run(&origin.subject).unwrap().unwrap();
    let mut request = crate::model::PersonStepResponse {
        delegation: None,
        subject: update.subject.clone(),
        actor: input.actor.clone(),
        summary: "The old update is withdrawn".into(),
        evidence: vec![],
        episode: Some(episode),
        answer: None,
        idempotency_key: "withdraw-own-update".into(),
    };
    request.actor = "agent/alder.stranger".into();
    assert_eq!(
        store.withdraw_person_update(&request).unwrap_err().code,
        "forbidden"
    );
    request.actor = input.actor;
    let ended = store.withdraw_person_update(&request).unwrap();
    assert_eq!(ended.status, "cancelled");
    assert_eq!(
        store.withdraw_person_update(&request).unwrap().status,
        "cancelled"
    );
    let after = store.step_run(&origin.subject).unwrap().unwrap();
    assert_eq!(
        (after.status, after.claimant, after.claim_expires_at_unix_ms),
        (
            before.status,
            before.claimant,
            before.claim_expires_at_unix_ms
        )
    );
    assert!(
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .iter()
            .all(|item| item.subject != update.subject)
    );
    let history = store.claims_for(&update.subject, None).unwrap();
    assert!(
        history
            .iter()
            .any(|claim| claim.kind == "work.person-asked")
    );
    assert_eq!(
        history
            .iter()
            .filter(|claim| claim.kind == "work.person-cancelled")
            .count(),
        1
    );
}

#[test]
fn concurrent_delegated_dismissals_have_one_source_answer_and_one_notice() {
    let (store, denial, proof) = denial_fixture(&["close-item"]);
    let store = std::sync::Arc::new(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let (store, barrier, proof, episode) = (
                store.clone(),
                barrier.clone(),
                proof.clone(),
                denial.id.clone(),
            );
            std::thread::spawn(move || {
                barrier.wait();
                store.respond_native_prompt(SEAT, &episode, "dismiss", ACTOR, Some(&proof))
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert!(results.iter().any(|result| result.is_ok()));
    let answers = store
        .claims_for(SEAT, Some("harness.diagnostic"))
        .unwrap()
        .into_iter()
        .filter(|claim| claim.body["fields"]["code"] == "native-prompt-answered")
        .collect::<Vec<_>>();
    assert_eq!(answers.len(), 1);
    for result in results.into_iter().flatten() {
        assert_eq!(result.id, answers[0].id);
    }
    assert_eq!(store.messages(Some(SEAT), true).unwrap().len(), 1);
}

#[test]
fn a_person_read_racing_sender_withdrawal_has_one_terminal_source_outcome() {
    let (store, origin, mut input) = person_work::tests::fixture();
    input.step = None;
    input.incarnation = None;
    input.request = Some(json!({"version":1,"type":"update","about":origin.run}));
    let update = store.ask_person(&input).unwrap();
    let episode = store
        .claims_for(&update.subject, Some("work.person-asked"))
        .unwrap()[0]
        .id
        .clone();
    let request = crate::model::PersonStepResponse {
        delegation: None,
        subject: update.subject.clone(),
        actor: input.actor,
        summary: "Withdrawn".into(),
        evidence: vec![],
        episode: Some(episode),
        answer: None,
        idempotency_key: "racing-withdraw".into(),
    };
    let store = std::sync::Arc::new(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let reader = {
        let (store, barrier, mut request) = (store.clone(), barrier.clone(), request.clone());
        std::thread::spawn(move || {
            request.actor = "person/avery".into();
            request.summary = "Read".into();
            request.answer = Some(crate::person_request::AnswerInput {
                id: Some("read".into()),
                text: None,
            });
            request.idempotency_key = "racing-read".into();
            barrier.wait();
            store.finish_person_step(&request, false)
        })
    };
    barrier.wait();
    let withdrawn = store.withdraw_person_update(&request);
    let read = reader.join().unwrap();
    assert_ne!(withdrawn.is_ok(), read.is_ok());
    let history = store.claims_for(&update.subject, None).unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|c| matches!(
                c.kind.as_str(),
                "work.person-done" | "work.person-cancelled"
            ))
            .count(),
        1
    );
    assert_eq!(
        store.step_run(&origin.subject).unwrap().unwrap().status,
        "claimed"
    );
}

#[test]
fn update_only_withdrawal_cannot_cancel_an_attached_question() {
    let (store, origin, input) = person_work::tests::fixture();
    let ask = store.ask_person(&input).unwrap();
    let request = crate::model::PersonStepResponse {
        delegation: None,
        subject: ask.subject.clone(),
        actor: input.actor,
        summary: "Wrong route".into(),
        evidence: vec![],
        episode: None,
        answer: None,
        idempotency_key: "wrong-kind-withdraw".into(),
    };
    assert_eq!(
        store.withdraw_person_update(&request).unwrap_err().code,
        "invalid-update-withdrawal"
    );
    assert_eq!(
        store.step_run(&ask.subject).unwrap().unwrap().status,
        "ready"
    );
    assert_eq!(
        store.step_run(&origin.subject).unwrap().unwrap().status,
        "waiting-person"
    );
}
