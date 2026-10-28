//! The part of `store::delegation` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::PersonWorkStore;
use super::*;
use crate::model::{DelegationPolicyRequest, DelegationProof};
pub use fractal_missions::store::delegation::*;
pub use fractal_missions::store::delegation::{ACTIONS, add_evidence, add_fields, policy_actions, validate_write};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PersonStepResponse;
    use fractal_missions::store::delegation_tests_shared_seats::*;
    use fractal_missions::store::delegation_tests_shared_seats::{policy, proof, sent};

    #[test]
    fn delegated_ask_records_actor_person_instruction_and_replays() {
        let (store, origin, input) = person_work::tests::fixture();
        let ask = store.ask_person(&input).unwrap();
        let episode = store
            .claims_for(&ask.subject, Some("work.person-asked"))
            .unwrap()[0]
            .id
            .clone();
        let proof = proof(&store, ACTIONS, episode.clone());
        let mut response = PersonStepResponse {
            subject: ask.subject.clone(),
            actor: input.actor.clone(),
            summary: "Friday".into(),
            evidence: vec![],
            episode: Some(episode),
            answer: None,
            idempotency_key: "answer".into(),
            delegation: Some(proof.clone()),
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        let record = &store
            .claims_for(&ask.subject, Some("work.person-done"))
            .unwrap()[0];
        assert_eq!(record.actor.as_deref(), Some(input.actor.as_str()));
        assert_eq!(record.body["fields"]["acted_for"], "person/avery");
        assert_eq!(record.body["fields"]["delegation"], json!(proof));
        assert_eq!(
            record.body["evidence"],
            json!([
                message_sent(&store.readers.get(), "message/instruction")
                    .unwrap()
                    .id,
                proof.policy
            ])
        );
        let resumed = store.step_run(&origin.subject).unwrap().unwrap();
        assert_eq!(resumed.status, "ready");
        assert_eq!(resumed.person_answers[0].respondent, input.actor);
        assert_eq!(
            resumed.person_answers[0].acted_for.as_deref(),
            Some("person/avery")
        );
        let peer = Store::open_memory("birch").unwrap();
        peer.project_replication_backlog().unwrap();
        person_work::tests::receive(&store, &peer);
        let replicated = peer.claim_by_id(&record.id).unwrap().unwrap();
        assert_eq!(replicated.actor, record.actor);
        assert_eq!(replicated.body, record.body);
        assert_eq!(
            peer.step_run(&origin.subject)
                .unwrap()
                .unwrap()
                .person_answers[0]
                .acted_for
                .as_deref(),
            Some("person/avery")
        );
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        response.delegation.as_mut().unwrap().quote = "Archive that message".into();
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "idempotency-conflict"
        );
    }

    #[test]
    fn delegated_document_instruction_quotes_the_original_pinned_body() {
        let (store, _, input) = person_work::tests::fixture();
        let ask = store.ask_person(&input).unwrap();
        let episode = store
            .claims_for(&ask.subject, Some("work.person-asked"))
            .unwrap()[0]
            .id
            .clone();
        let mut proof = proof(&store, ACTIONS, episode.clone());
        let document = store
            .put_document(
                "doc/example/instruction",
                b"Release Friday.",
                &None,
                "friday",
            )
            .unwrap();
        sent(
            &store,
            "document-instruction",
            "person/avery",
            "agent/alder.asker",
            &format!("{}@{}", document.name, document.hash),
        );
        store
            .put_document(
                "doc/example/instruction",
                b"Release Monday.",
                &Some(document.binding_claim_id),
                "monday",
            )
            .unwrap();
        proof.message = "message/document-instruction".into();
        let response = PersonStepResponse {
            subject: ask.subject.clone(),
            actor: input.actor,
            summary: "Friday".into(),
            evidence: vec![],
            episode: Some(episode),
            delegation: Some(proof),
            answer: None,
            idempotency_key: "document-answer".into(),
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
    }

    #[test]
    fn delegated_ask_refuses_bad_evidence_revoked_policy_and_authored_steps() {
        for invalid in [
            "absent-policy",
            "forged-quote",
            "wrong-person",
            "wrong-message",
            "stale-episode",
            "revoked",
            "authored-step",
            "cancel",
        ] {
            let (store, origin, input) = person_work::tests::fixture();
            let ask = store.ask_person(&input).unwrap();
            let episode = store
                .claims_for(&ask.subject, Some("work.person-asked"))
                .unwrap()[0]
                .id
                .clone();
            let mut proof = proof(&store, ACTIONS, episode.clone());
            let mut subject = ask.subject.clone();
            match invalid {
                "absent-policy" => proof.policy = "missing".into(),
                "forged-quote" => proof.quote = "Monday".into(),
                "wrong-person" => proof.person = "person/robin".into(),
                "wrong-message" => {
                    sent(
                        &store,
                        "fake",
                        "agent/alder.asker",
                        "person/avery",
                        "Friday",
                    );
                    proof.message = "message/fake".into();
                }
                "stale-episode" => proof.episode = "old".into(),
                "revoked" => {
                    policy(&store, &[], "revoke");
                }
                "authored-step" => {
                    subject = store
                        .mission_run(&origin.run)
                        .unwrap()
                        .unwrap()
                        .steps
                        .iter()
                        .find(|step| step.step == "review")
                        .unwrap()
                        .subject
                        .clone()
                }
                _ => {}
            }
            let response = PersonStepResponse {
                subject,
                actor: input.actor.clone(),
                summary: "Friday".into(),
                evidence: vec![],
                episode: Some(episode),
                answer: None,
                idempotency_key: "answer".into(),
                delegation: Some(proof),
            };
            assert!(
                store
                    .finish_person_step(&response, invalid == "cancel")
                    .is_err(),
                "{invalid}"
            );
            assert_eq!(
                store.step_run(&ask.subject).unwrap().unwrap().status,
                "ready",
                "{invalid}"
            );
            assert_eq!(
                store.step_run(&origin.subject).unwrap().unwrap().status,
                "waiting-person",
                "{invalid}"
            );
            assert!(
                store
                    .claims_for(&ask.subject, Some("work.person-done"))
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn delegated_message_closure_refuses_operations_outside_list_and_generic_forgery() {
        let (store, _, _) = person_work::tests::fixture();
        let target = sent(
            &store,
            "notice",
            "agent/alder.asker",
            "person/avery",
            "Notice",
        );
        let proof = proof(&store, &["answer-ask"], target.id.clone());
        let mut fields = BTreeMap::from([("status".into(), json!("closed"))]);
        add_fields(&mut fields, &proof);
        let mut evidence = vec![];
        add_evidence(&store.readers.get(), &mut evidence, &proof).unwrap();
        let mut input = ClaimInput {
            subject: target.subject.clone(),
            kind: "message.closed".into(),
            actor: Some("agent/alder.asker".into()),
            fields,
            evidence,
            expected_subject: None,
            idempotency_key: Some("close".into()),
        };
        assert_eq!(
            store.append_claim(&input).unwrap_err().code,
            "delegation-refused"
        );
        let policy = policy(&store, ACTIONS, "all");
        let mut allowed = proof.clone();
        allowed.policy = policy.id;
        allowed.quote = "Archive that message".into();
        add_fields(&mut input.fields, &allowed);
        input.evidence = vec![];
        add_evidence(&store.readers.get(), &mut input.evidence, &allowed).unwrap();
        assert_eq!(
            store.append_claim(&input).unwrap().actor.as_deref(),
            Some("agent/alder.asker")
        );
        assert_eq!(
            store.message(&target.subject).unwrap().unwrap().status,
            "closed"
        );
        assert_eq!(
            store.append_claim(&input).unwrap().body["fields"]["acted_for"],
            "person/avery"
        );
        let mut forged = input.clone();
        forged.subject = "message/other".into();
        forged.idempotency_key = Some("other".into());
        assert!(store.append_claim(&forged).is_err());
        // Supplying the same metadata on a deletion or external operation cannot authorize it.
        forged.kind = "glass.deleted".into();
        forged.subject = "glass/example".into();
        assert!(store.append_claim(&forged).is_err());
    }

    #[test]
    fn delegated_structured_ask_keeps_named_answer_validation() {
        let (store, _, mut input) = person_work::tests::fixture();
        input.request = Some(
            json!({"version":1,"type":"choice","question":"Choose a date","why_person":"Your date","answers":[{"id":"friday","label":"Friday","consequence":"Release Friday"},{"id":"monday","label":"Monday","consequence":"Release Monday"}]}),
        );
        let ask = store.ask_person(&input).unwrap();
        let episode = store
            .claims_for(&ask.subject, Some("work.person-asked"))
            .unwrap()[0]
            .id
            .clone();
        let proof = proof(&store, ACTIONS, episode.clone());
        let mut response = PersonStepResponse {
            subject: ask.subject.clone(),
            actor: input.actor,
            summary: String::new(),
            evidence: vec![],
            episode: Some(episode),
            delegation: Some(proof),
            answer: Some(crate::person_request::AnswerInput {
                id: Some("not-offered".into()),
                text: None,
            }),
            idempotency_key: "choice".into(),
        };
        assert!(store.finish_person_step(&response, false).is_err());
        response.answer.as_mut().unwrap().id = Some("friday".into());
        store.finish_person_step(&response, false).unwrap();
        let done = store.step_run(&ask.subject).unwrap().unwrap();
        assert_eq!(
            done.person_answers[0].answer.as_ref().unwrap()["id"],
            "friday"
        );
        assert_eq!(
            done.person_answers[0].acted_for.as_deref(),
            Some("person/avery")
        );
    }

    #[test]
    fn delegated_closure_reads_updates_without_answering_decisions() {
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
        let mut proof = proof(&store, &["close-item"], episode.clone());
        proof.quote = "read the update".into();
        let response = PersonStepResponse {
            subject: update.subject.clone(),
            actor: input.actor.clone(),
            summary: "Read".into(),
            evidence: vec![],
            episode: Some(episode),
            delegation: Some(proof.clone()),
            answer: None,
            idempotency_key: "read-update".into(),
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        let mut decision_input = input;
        decision_input.request = None;
        decision_input.step = Some(origin.subject.clone());
        decision_input.incarnation = Some("asker-one".into());
        decision_input.idempotency_key = "date".into();
        let ask = store.ask_person(&decision_input).unwrap();
        let ask_episode = store
            .claims_for(&ask.subject, Some("work.person-asked"))
            .unwrap()[0]
            .id
            .clone();
        let mut bypass = response;
        bypass.subject = ask.subject.clone();
        bypass.episode = Some(ask_episode.clone());
        bypass.delegation.as_mut().unwrap().episode = ask_episode;
        bypass.idempotency_key = "bypass".into();
        assert_eq!(
            store.finish_person_step(&bypass, false).unwrap_err().code,
            "delegation-refused"
        );
        assert_eq!(
            store.step_run(&ask.subject).unwrap().unwrap().status,
            "ready"
        );
    }

    #[test]
    fn delegated_go_stop_answers_gate_for_real_reviewer_and_refuses_stale_revision() {
        for verdict in ["pass", "fail", "stale"] {
            let (store, origin, _) = person_work::tests::fixture();
            let run = store.mission_run(&origin.run).unwrap().unwrap();
            let gate = store
                .append_claim(&ClaimInput {
                    subject: "gate-operation/review".into(),
                    kind: "gate.requested".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("owner".into(), json!(origin.subject)),
                        ("reviewer".into(), json!("person/avery")),
                        ("mode".into(), json!("approve")),
                        ("mission_revision".into(), json!(run.revision)),
                        ("step_definition".into(), json!(origin.definition_hash)),
                        ("attempt".into(), json!(1)),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: Some("gate".into()),
                })
                .unwrap();
            let mut proof = proof(&store, ACTIONS, gate.id.clone());
            proof.quote = if verdict == "fail" {
                "Stop the release"
            } else {
                "Approve the release"
            }
            .into();
            let mut fields = BTreeMap::from([
                ("request".into(), json!(gate.id)),
                (
                    "reason".into(),
                    json!("The person instructed this decision"),
                ),
                (
                    "verdict".into(),
                    json!(if verdict == "fail" { "fail" } else { "pass" }),
                ),
                (
                    "decision".into(),
                    json!(if verdict == "fail" {
                        "rejected"
                    } else {
                        "approved"
                    }),
                ),
            ]);
            add_fields(&mut fields, &proof);
            let mut evidence = vec![gate.id.clone()];
            add_evidence(&store.readers.get(), &mut evidence, &proof).unwrap();
            let result = ClaimInput {
                subject: gate.subject.clone(),
                kind: "gate.result".into(),
                actor: Some("agent/alder.asker".into()),
                fields,
                evidence,
                expected_subject: None,
                idempotency_key: Some("go".into()),
            };
            if verdict == "stale" {
                store
                    .connection
                    .batched(|tx| -> Result<()> {
                        tx.execute(
                            "UPDATE step_runs SET definition_hash='changed' WHERE subject=?1",
                            [&origin.subject],
                        )?;
                        Ok(())
                    })
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    store.append_claim(&result).unwrap_err().code,
                    "delegation-refused"
                );
                assert!(store.human_review_answer(&gate.id).unwrap().is_none());
            } else {
                let accepted = store.append_claim(&result).unwrap();
                assert_eq!(
                    store.human_review_answer(&gate.id).unwrap().unwrap().id,
                    accepted.id
                );
                assert!(
                    store
                        .pending_human_reviews(Some("person/avery"))
                        .unwrap()
                        .is_empty()
                );
                assert_eq!(accepted.actor.as_deref(), Some("agent/alder.asker"));
                assert_eq!(accepted.body["fields"]["acted_for"], "person/avery");
            }
        }
    }

    #[test]
    fn policy_can_only_be_replaced_by_person_and_raw_completion_needs_proof() {
        let (store, _, input) = person_work::tests::fixture();
        let decision = sent(
            &store,
            "decision",
            "person/avery",
            "agent/alder.asker",
            "Allow answers",
        );
        let mut request = DelegationPolicyRequest {
            person: "person/avery".into(),
            actor: input.actor.clone(),
            actions: vec!["answer-ask".into()],
            evidence: vec![decision.id],
            idempotency_key: "policy".into(),
        };
        assert_eq!(
            store.set_delegation_policy(&request).unwrap_err().code,
            "delegation-refused"
        );
        request.actor = "person/avery".into();
        request.actions = vec!["delete".into()];
        assert_eq!(
            store.set_delegation_policy(&request).unwrap_err().code,
            "delegation-refused"
        );
        let ask = store.ask_person(&input).unwrap();
        let raw = ClaimInput {
            subject: ask.subject.clone(),
            kind: "work.person-done".into(),
            actor: Some(input.actor),
            fields: BTreeMap::from([
                ("attempt".into(), json!(1)),
                ("status".into(), json!("completed")),
                ("summary".into(), json!("Friday")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some("raw".into()),
        };
        assert_eq!(
            store.append_claim(&raw).unwrap_err().code,
            "delegation-refused"
        );
        assert_eq!(
            store.step_run(&ask.subject).unwrap().unwrap().status,
            "ready"
        );
    }
}
