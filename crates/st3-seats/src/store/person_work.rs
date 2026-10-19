//! The part of `store::person_work` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use super::*;
use crate::model::{MessageAttachment, PersonAskRequest, PersonStepResponse};
use crate::person_request::{StructuredRequest, answer_summary};
pub use st3_missions::store::person_work::*;
pub use st3_missions::store::person_work::{AskImages, current, declaration_live, enrich_responses, is_update, project, project_minimal_run, request, run_live, run_liveness, step, validate_delegated_answer};
#[cfg(test)]
mod reconcile_candidates_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use st3_missions::store::person_work_tests_shared_seats::*;
    use st3_missions::store::person_work_tests_shared_seats::{TEST_FLEET, gateway_decision, receive_exchange};
    pub use st3_missions::store::person_work::tests::*;

    pub fn fixture() -> (Store, StepRunView, PersonAskRequest) {
        let store = Store::open_memory("alder").unwrap();
        let intent = crate::graph::parse_internal_intent(
            r#"version 2
agent "alder.asker" { workspace "/tmp"; command "true"; restart always; }
mission "person-work" state="ready" {
  goal "Review the release.";
  step "prepare" { assigned-to "agent/alder.asker"; goal "Prepare the release."; }
  step "review" { assigned-to "person/avery"; goal "Review the release."; }
}
"#,
            "alder",
        )
        .unwrap();
        store.apply_internal(&intent, "person-fixture").unwrap();
        let run = store
            .create_mission_run(&MissionRunRequest {
                mission: "person-work".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/avery".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "person-run".into(),
            })
            .unwrap();
        let origin = run
            .steps
            .iter()
            .find(|step| step.step == "prepare")
            .unwrap()
            .clone();
        store.connection.batched(|tx| -> Result<()> {
            let claim = append_claim_tx(tx, "alder", &origin.subject, "work.claimed", Some("agent/alder.asker"),
                &json!({"fields": {"attempt": 1, "status": "claimed", "claimant": "agent/alder.asker",
                    "claim_incarnation": "asker-one", "claim_expires_at_unix_ms": (now_ms()+600_000) as u64}}), &[], None)?;
            project_mission_run_update(tx, &claim).unwrap();
            Ok(())
        }).unwrap().unwrap();
        let input = PersonAskRequest {
            legacy_request: None,
            person: "person/avery".into(),
            title: "Choose a release date".into(),
            reason: "Reply with the release date.".into(),
            actor: "agent/alder.asker".into(),
            step: Some(origin.subject.clone()),
            new_run: None,
            incarnation: Some("asker-one".into()),
            idempotency_key: "release-date".into(),
            request: None,
        };
        (store, origin, input)
    }

    fn delivery_fixture(kind: &str) -> (Store, StepRunView, PersonAskRequest, MissionRunView) {
        let (store, origin, mut input) = fixture();
        let root = store.mission_run(&origin.run).unwrap().unwrap();
        let source = r#"version 2
mission "child-person-work" state="ready" {
  goal "Gather one person decision."
  step "prepare" { assigned-to "agent/alder.asker"; goal "Prepare the question." }
}
resource "issues" { kind "vcs.repository" }
observer "issues" { resource "resource/issues"; provider "github.repository"; locator "example/repo"; field "issues" }
subscription "intake" {
  observer "observer/issues"; on "issues"
  delivery "mission" { mission "child-person-work"; resource "source"; workspace "/tmp" }
}
schedule "intake" {
  calendar { at "08:00"; timezone "UTC" }
  work { mission "person-work@REVISION"; workspace "/tmp" }
}"#.replace("REVISION", &root.revision);
        let mut intent = crate::graph::parse_internal_intent(&source, "alder").unwrap();
        let parent = format!("{kind}/intake");
        let subscription = intent.subjects.get_mut(&parent).unwrap();
        subscription.owner_run = Some(root.subject.clone());
        subscription.owner_generation = Some(root.generation.clone());
        store.apply_internal(&intent, "owned-subscription").unwrap();
        let child = store
            .create_child_mission_run(
                &MissionRunRequest {
                    mission: "child-person-work".into(),
                    revision: None,
                    workspace: "/tmp/child".into(),
                    requester: Some(input.actor.clone()),
                    mode: Some("run".into()),
                    inputs: BTreeMap::new(),
                    idempotency_key: "subscription-child".into(),
                },
                &root,
                &parent,
                None,
            )
            .unwrap();
        let origin = child
            .steps
            .iter()
            .find(|step| step.step == "prepare")
            .unwrap()
            .clone();
        store.connection.batched(|tx| -> Result<()> {
            let claim = append_claim_tx(tx, "alder", &origin.subject, "work.claimed", Some(&input.actor),
                &json!({"fields": {"attempt": 1, "status": "claimed", "claimant": input.actor,
                    "claim_incarnation": "asker-one", "claim_expires_at_unix_ms": (now_ms()+600_000) as u64}}), &[], None)?;
            project_mission_run_update(tx, &claim).unwrap();
            Ok(())
        }).unwrap().unwrap();
        input.step = Some(origin.subject.clone());
        (store, origin, input, root)
    }

    #[test]
    fn delivery_child_person_ask_survives_reconciliation_and_replay() {
        for kind in ["subscription", "schedule"] {
            let (store, origin, input, _) = delivery_fixture(kind);
            let ask = store.ask_person(&input).unwrap();
            for replay in [false, true] {
                if replay {
                    store.replay_replication_graph().unwrap();
                }
                store.reconcile_person_asks().unwrap();
                assert_eq!(
                    store.step_run(&ask.subject).unwrap().unwrap().status,
                    "ready"
                );
                assert!(
                    store
                        .attention_items(Some("person/avery"))
                        .unwrap()
                        .iter()
                        .any(|item| item.subject == ask.subject)
                );
            }
            let response = PersonStepResponse {
                delegation: None,
                subject: ask.subject,
                actor: "person/avery".into(),
                summary: "Friday".into(),
                evidence: vec![],
                episode: None,
                idempotency_key: "subscription-answer".into(),
                answer: None,
            };
            store.finish_person_step(&response, false).unwrap();
            assert_eq!(
                store.step_run(&origin.subject).unwrap().unwrap().status,
                "ready"
            );
        }
    }

    #[test]
    fn subscription_child_person_asks_keep_owner_fences() {
        for invalidation in [
            "stop",
            "root-ended",
            "stale-generation",
            "missing-parent",
            "missing-step",
        ] {
            let (store, origin, input, root) = delivery_fixture("subscription");
            let ask = store.ask_person(&input).unwrap();
            match invalidation {
                "stop" => {
                    let mut intent = crate::graph::parse_internal_intent(
                        "version 2\nsubscription \"intake\" { stop }",
                        "alder",
                    )
                    .unwrap();
                    let stopped = intent.subjects.get_mut("subscription/intake").unwrap();
                    stopped.owner_run = Some(root.subject.clone());
                    stopped.owner_generation = Some(root.generation.clone());
                    store.apply_internal(&intent, "stop-subscription").unwrap();
                }
                "root-ended" => {
                    store
                        .set_mission_run_state(
                            &root.subject,
                            "cancelled",
                            "terminal",
                            Some("intake ended"),
                        )
                        .unwrap();
                }
                "stale-generation" => {
                    store.connection.batched(|tx| -> Result<()> {
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h941280992955 owner=st3_seats::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_generation='run-generation/old' WHERE subject='subscription/intake'", [])?;
                        Ok(())
                    }).unwrap().unwrap();
                }
                missing => {
                    let parent = if missing == "missing-parent" {
                        "step-run/subscription/missing"
                    } else {
                        "step-run/missing/prepare"
                    };
                    store
                        .connection
                        .batched(|tx| -> Result<()> {
                            tx.execute(
                                /* sqlite-large-value: allow-for-now lv-h7f86d64acfa2 owner=st3_seats::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET parent_step_run=?1 WHERE id=?2",
                                params![parent, origin.run.trim_start_matches("mission-run/")],
                            )?;
                            Ok(())
                        })
                        .unwrap()
                        .unwrap();
                }
            }
            assert!(
                store
                    .attention_items(Some("person/avery"))
                    .unwrap()
                    .iter()
                    .all(|item| item.subject != ask.subject),
                "{invalidation}"
            );
            store.reconcile_person_asks().unwrap();
            assert_eq!(
                store.step_run(&ask.subject).unwrap().unwrap().status,
                "cancelled",
                "{invalidation}"
            );
        }
    }

    #[test]
    fn person_ask_suspends_without_lease_and_response_resumes_same_attempt() {
        let (store, origin, input) = fixture();
        let ask = store.ask_person(&input).unwrap();
        let paused = store.step_run(&origin.subject).unwrap().unwrap();
        assert_eq!(paused.status, "waiting-person");
        assert!(paused.claimant.is_none() && paused.claim_expires_at_unix_ms.is_none());
        assert_eq!(store.ask_person(&input).unwrap().subject, ask.subject);
        let mut conflict = input.clone();
        conflict.reason = "Another question".into();
        assert_eq!(
            store.ask_person(&conflict).unwrap_err().code,
            "idempotency-conflict"
        );
        let items = store
            .attention_snapshot(Some("person/avery"), now_ms() + 7 * 86_400_000)
            .unwrap();
        let item = items
            .iter()
            .find(|item| item.subject == ask.subject)
            .unwrap();
        assert!(
            store
                .attention_snapshot(Some("person/robin"), now_ms())
                .unwrap()
                .is_empty()
        );
        let mut response = PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: "person/robin".into(),
            summary: "Friday".into(),
            evidence: Vec::new(),
            episode: Some(item.episode.clone()),
            idempotency_key: "release-response".into(),
            answer: None,
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "forbidden"
        );
        response.actor = "person/avery".into();
        response.episode = Some("old-episode".into());
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "stale-fence"
        );
        response.episode = Some(item.episode.clone());
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        let resumed = store.step_run(&origin.subject).unwrap().unwrap();
        assert_eq!(resumed.status, "ready");
        assert_eq!(resumed.attempt, origin.attempt);
        assert!(
            resumed
                .constraints
                .iter()
                .any(|constraint| constraint == "Person response: Friday")
        );
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .all(|item| item.subject != ask.subject)
        );
        store.replay_replication_graph().unwrap();
        assert_eq!(
            store.step_run(&origin.subject).unwrap().unwrap().status,
            "ready"
        );
        assert_eq!(
            store.step_run(&ask.subject).unwrap().unwrap().status,
            "completed"
        );
    }

    /// Revise the fixture's run so `prepare` moves into a new generation: carried with its
    /// claim when only `review` changes, or issued again and claimed anew when `prepare` changes.
    fn revise_fixture(store: &Store, origin: &StepRunView, prepare_changes: bool) -> StepRunView {
        let (prepare, review) = if prepare_changes {
            ("Prepare the release notes too.", "Review the release.")
        } else {
            ("Prepare the release.", "Review the release notes.")
        };
        let source = format!(
            r#"version 2
mission "person-work" state="ready" {{
  goal "Review the release.";
  step "prepare" {{ assigned-to "agent/alder.asker"; goal "{prepare}"; }}
  step "review" {{ assigned-to "person/avery"; goal "{review}"; }}
}}
"#
        );
        let intent = crate::graph::parse_internal_intent(&source, "alder").unwrap();
        store.apply_internal(&intent, "person-revision").unwrap();
        let revised = store
            .adopt_mission_revision(
                &origin.run,
                &intent.missions["person-work"],
                "person/avery",
                "the release needs notes",
                "person-revision",
            )
            .unwrap();
        let moved = revised
            .steps
            .iter()
            .find(|step| step.step == "prepare")
            .unwrap()
            .clone();
        assert_ne!(moved.subject, origin.subject);
        if prepare_changes {
            store.set_step_state(&moved.subject, "ready", None).unwrap();
            store
                .work_action(
                    &moved.subject,
                    "claim",
                    &crate::model::WorkRequest {
                        wait_ms: None,
                        actor: Some("agent/alder.asker".into()),
                        incarnation: Some("asker-one".into()),
                        summary: None,
                        reason: None,
                        evidence: Vec::new(),
                        idempotency_key: "reclaim-prepare".into(),
                    },
                )
                .unwrap();
        }
        let moved = store.step_run(&moved.subject).unwrap().unwrap();
        assert_eq!(moved.claimant.as_deref(), Some("agent/alder.asker"));
        moved
    }

    #[test]
    fn a_worker_asks_from_the_step_a_revision_moved_by_its_old_name() {
        for prepare_changes in [false, true] {
            let (store, origin, input) = fixture();
            let moved = revise_fixture(&store, &origin, prepare_changes);
            // The worker still names the step it claimed before the revision.
            assert_eq!(input.step.as_deref(), Some(origin.subject.as_str()));
            let ask = store
                .ask_person(&input)
                .unwrap_or_else(|error| panic!("prepare changes: {prepare_changes}: {error:?}"));
            let paused = store.step_run(&moved.subject).unwrap().unwrap();
            assert_eq!(paused.status, "waiting-person");
            assert_eq!(
                store.ask_person(&input).unwrap().subject,
                ask.subject,
                "a retry is the same ask"
            );
            assert!(
                ask.subject.starts_with(&format!(
                    "step-run/{}/",
                    moved.generation.trim_start_matches("run-generation/")
                )),
                "{}",
                ask.subject
            );
            // Another seat cannot ask through a step it never held.
            let mut stranger = input.clone();
            stranger.actor = "agent/alder.other".into();
            stranger.idempotency_key = "stranger".into();
            assert_eq!(
                store.ask_person(&stranger).unwrap_err().code,
                "stale-work-ask"
            );
        }
    }

    #[test]
    fn person_ask_disappears_on_requester_retirement_without_cleanup() {
        let (store, _origin, input) = fixture();
        let ask = store.ask_person(&input).unwrap();
        let stop =
            crate::graph::parse_internal_intent("version 2\nstop \"agent/alder.asker\"", "alder")
                .unwrap();
        store.apply_internal(&stop, "retire-asker").unwrap();
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .all(|item| item.subject != ask.subject)
        );
        let response = PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: "person/avery".into(),
            summary: "Friday".into(),
            evidence: vec![],
            episode: None,
            idempotency_key: "late-response".into(),
            answer: None,
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "stale-fence"
        );
        store.reconcile_person_asks().unwrap();
        assert_eq!(
            store.step_run(&ask.subject).unwrap().unwrap().status,
            "cancelled"
        );
    }

    #[test]
    fn new_run_person_ask_is_durable_and_bound_to_requester() {
        let (store, origin, mut input) = fixture();
        store
            .set_step_state(&origin.subject, "completed", None)
            .unwrap();
        input.step = None;
        input.new_run = Some("release-question".into());
        let ask = store.ask_person(&input).unwrap();
        assert_eq!(store.ask_person(&input).unwrap().subject, ask.subject);
        store.replay_replication_graph().unwrap();
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .any(|item| item.subject == ask.subject)
        );
        let response = PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: input.actor,
            summary: "No longer needed".into(),
            evidence: vec![],
            episode: None,
            idempotency_key: "cancel-question".into(),
            answer: None,
        };
        assert_eq!(
            store.finish_person_step(&response, true).unwrap().status,
            "cancelled"
        );
        assert_eq!(
            store.mission_run(&ask.run).unwrap().unwrap().status,
            "cancelled"
        );
        assert!(
            store
                .messages(Some("agent/alder.asker"), true)
                .unwrap()
                .is_empty(),
            "the requester's own cancel tells nobody"
        );
    }

    /// A structured ask shows its request to the person, takes only a named answer, and gives
    /// the asker that answer as data when its step resumes.
    #[test]
    fn structured_ask_returns_the_named_answer_to_the_asker_as_data() {
        let (store, origin, mut input) = fixture();
        input.title = "Land the browser gateway?".into();
        input.request = Some(gateway_decision());
        let mut malformed = input.clone();
        malformed.request.as_mut().unwrap()["answers"][1]["outcome"] = json!("accept");
        assert_eq!(
            store.ask_person(&malformed).unwrap_err().code,
            "invalid-person-request"
        );
        let ask = store.ask_person(&input).unwrap();
        let mut conflict = input.clone();
        conflict.request.as_mut().unwrap()["question"] = json!("Land it now?");
        assert_eq!(
            store.ask_person(&conflict).unwrap_err().code,
            "idempotency-conflict"
        );
        let item = store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .find(|item| item.subject == ask.subject)
            .unwrap();
        let request = item.request.clone().unwrap();
        assert_eq!(request["answers"][2]["id"], "revise-auth");
        assert_eq!(request["subjects"][0]["revision"], "abc123");
        assert!(item.actions[0].argv.iter().any(|arg| arg == "--answer"));
        let mut response = PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: "person/avery".into(),
            summary: "yes".into(),
            evidence: vec![],
            episode: Some(item.episode.clone()),
            idempotency_key: "gateway-answer".into(),
            answer: None,
        };
        // Authority comes before the answer's shape.
        response.actor = "person/robin".into();
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "forbidden"
        );
        response.actor = "person/avery".into();
        // A reply in words never selects an answer.
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "answer-required"
        );
        response.summary = String::new();
        response.answer = Some(crate::person_request::AnswerInput {
            id: Some("revise-auth".into()),
            text: Some("Pair browsers like the native clients.".into()),
        });
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        let mut other = response.clone();
        other.answer.as_mut().unwrap().id = Some("land".into());
        other.answer.as_mut().unwrap().text = None;
        assert_eq!(
            store.finish_person_step(&other, false).unwrap_err().code,
            "idempotency-conflict"
        );
        let expected = json!({
            "type": "decision", "outcome": "request_changes", "id": "revise-auth",
            "label": "Revise auth and review", "text": "Pair browsers like the native clients."
        });
        let check = |store: &Store| {
            let resumed = store.step_run(&origin.subject).unwrap().unwrap();
            assert_eq!(resumed.status, "ready");
            assert_eq!(resumed.person_answers.len(), 1);
            let answer = &resumed.person_answers[0];
            assert_eq!(answer.ask, ask.subject);
            assert_eq!(answer.respondent, "person/avery");
            assert_eq!(answer.answer.as_ref(), Some(&expected));
            assert_eq!(
                answer.summary,
                "Revise auth and review: Pair browsers like the native clients."
            );
            assert!(resumed.constraints.iter().any(|constraint| constraint
                == "Person response: Revise auth and review: Pair browsers like the native clients."));
            let own = store.step_run(&ask.subject).unwrap().unwrap();
            assert_eq!(own.person_answers.len(), 1);
            assert_eq!(own.person_answers[0].answer.as_ref(), Some(&expected));
        };
        check(&store);
        store.replay_replication_graph().unwrap();
        check(&store);
        assert!(
            store
                .messages(Some("agent/alder.asker"), true)
                .unwrap()
                .is_empty(),
            "a step-owned ask resumes its step instead of sending a message"
        );
    }

    #[test]
    fn new_run_decision_pushes_the_named_answer_to_the_requester() {
        let (store, origin, mut input) = fixture();
        store
            .set_step_state(&origin.subject, "completed", None)
            .unwrap();
        input.step = None;
        input.new_run = Some("gateway".into());
        input.title = "Land the browser gateway?".into();
        input.request = Some(gateway_decision());
        let ask = store.ask_person(&input).unwrap();
        store
            .finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: ask.subject.clone(),
                    actor: "person/avery".into(),
                    summary: String::new(),
                    evidence: vec![],
                    episode: None,
                    idempotency_key: "gateway-answer".into(),
                    answer: Some(crate::person_request::AnswerInput {
                        id: Some("revise-auth".into()),
                        text: Some("Pair browsers like the native clients.".into()),
                    }),
                },
                false,
            )
            .unwrap();
        let messages = store.messages(Some("agent/alder.asker"), true).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains(
            "Answer: Revise auth and review (revise-auth)\nPair browsers like the native clients."
        ));
    }

    /// An update brings the person what they asked for: the poster keeps working, the update
    /// stays on the person's home after the poster stops, and reading it clears it without
    /// telling anyone. Work the person did not ask for takes no update.
    #[test]
    fn an_update_waits_for_nobody_stays_until_read_and_needs_the_persons_own_work() {
        let (store, origin, mut input) = fixture();
        input.step = None;
        input.incarnation = None;
        input.title = "Release notes are ready".into();
        input.reason = "The notes cover all three fixes.".into();
        input.request = Some(json!({"version": 1, "type": "update", "about": origin.run}));
        let update = store.ask_person(&input).unwrap();
        assert!(update.subject.ends_with("/update"));
        assert_eq!(store.ask_person(&input).unwrap().subject, update.subject);
        // Nothing waits on an update: the poster's step keeps its claim.
        assert_eq!(
            store.step_run(&origin.subject).unwrap().unwrap().status,
            "claimed"
        );

        // Not the person's work, not the person's message: refused.
        let mut stranger = input.clone();
        stranger.person = "person/someone-else".into();
        assert_eq!(
            store.ask_person(&stranger).unwrap_err().code,
            "update-not-asked"
        );
        let message = |subject: &str, from: &str, to: &str| {
            store.connection.batched(|tx| -> Result<()> {
                append_claim_tx(tx, "alder", subject, "message.sent", Some(from),
                    &json!({"fields": {"from": from, "to": to, "content": "How did the release go?", "status": "sent"}}), &[], None)?;
                Ok(())
            }).unwrap().unwrap();
        };
        message(
            "message/0000000000000001",
            "person/avery",
            "agent/alder.asker",
        );
        message(
            "message/0000000000000002",
            "person/avery",
            "agent/alder.other",
        );
        let mut asked = input.clone();
        asked.idempotency_key = "asked-in-message".into();
        asked.request =
            Some(json!({"version": 1, "type": "update", "about": "message/0000000000000001"}));
        let answered = store.ask_person(&asked).unwrap();
        asked.request =
            Some(json!({"version": 1, "type": "update", "about": "message/0000000000000002"}));
        assert_eq!(
            store.ask_person(&asked).unwrap_err().code,
            "update-not-asked"
        );
        asked.request = Some(json!({"version": 1, "type": "update", "about": origin.subject}));
        asked.idempotency_key = "asked-in-step".into();
        store.ask_person(&asked).unwrap();
        let mut with_step = input.clone();
        with_step.step = Some(origin.subject.clone());
        assert_eq!(
            store.ask_person(&with_step).unwrap_err().code,
            "ambiguous-ask-owner"
        );

        let stop =
            crate::graph::parse_internal_intent("version 2\nstop \"agent/alder.asker\"", "alder")
                .unwrap();
        store.apply_internal(&stop, "retire-poster").unwrap();
        store.reconcile_person_asks().unwrap();
        let items = store.attention_items(Some("person/avery")).unwrap();
        let item = items
            .iter()
            .find(|item| item.subject == update.subject)
            .expect("the update outlives its poster");
        assert_eq!(item.request.as_ref().unwrap()["type"], "update");
        assert_eq!(item.actions[0].label, "read");
        assert_eq!(item.actions[0].argv[6..], ["--answer", "read"]);

        let read = |subject: &str, answer: Option<&str>, text: Option<&str>| {
            store.finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: subject.into(),
                    actor: "person/avery".into(),
                    summary: String::new(),
                    evidence: vec![],
                    episode: None,
                    idempotency_key: format!("read:{subject}:{answer:?}:{text:?}"),
                    answer: (answer.is_some() || text.is_some()).then(|| {
                        crate::person_request::AnswerInput {
                            id: answer.map(str::to_owned),
                            text: text.map(str::to_owned),
                        }
                    }),
                },
                false,
            )
        };
        assert_eq!(
            read(&update.subject, None, Some("Thanks, now ship it"))
                .unwrap_err()
                .code,
            "invalid-person-answer"
        );
        read(&update.subject, Some("read"), None).unwrap();
        read(&answered.subject, None, None).unwrap();
        let done = store.step_run(&update.subject).unwrap().unwrap();
        assert_eq!(done.status, "completed");
        assert_eq!(done.person_answers[0].summary, "Read");
        assert_eq!(
            done.person_answers[0].answer,
            Some(json!({"type": "update", "outcome": "read"}))
        );
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .all(|item| item.subject != update.subject && item.subject != answered.subject)
        );
        // Reading tells nobody: an update is not a question. The one message is the person's.
        let messages = store.messages(Some("agent/alder.asker"), true).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "How did the release go?");
    }

    /// A free-text ask keeps working, takes no named answer, and keeps its claim body unchanged
    /// so older members still admit it.
    #[test]
    fn free_text_ask_takes_words_and_refuses_a_named_answer() {
        let (store, origin, input) = fixture();
        let ask = store.ask_person(&input).unwrap();
        let claim = request(&store.readers.get(), &ask.subject)
            .unwrap()
            .unwrap();
        assert!(claim.body["fields"].get("request").is_none());
        let item = store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .find(|item| item.subject == ask.subject)
            .unwrap();
        assert!(item.request.is_none());
        let mut response = PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: "person/avery".into(),
            summary: String::new(),
            evidence: vec![],
            episode: Some(item.episode),
            idempotency_key: "free-text".into(),
            answer: Some(crate::person_request::AnswerInput {
                id: Some("friday".into()),
                text: None,
            }),
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "invalid-person-answer"
        );
        response.answer = None;
        response.summary = "Friday".into();
        store.finish_person_step(&response, false).unwrap();
        let resumed = store.step_run(&origin.subject).unwrap().unwrap();
        assert_eq!(resumed.person_answers[0].summary, "Friday");
        assert!(resumed.person_answers[0].answer.is_none());
        let done = store
            .readers
            .get()
            .query_row(
                "SELECT body FROM claims WHERE subject=?1 AND kind='work.person-done'",
                [&ask.subject],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert!(!done.contains("\"answer\""));
    }

    /// A feedback ask in a run of its own: the person's text is the answer, and the asker reads
    /// it from the ask step.
    #[test]
    fn new_run_feedback_ask_keeps_the_answer_on_the_ask() {
        let (store, origin, mut input) = fixture();
        store
            .set_step_state(&origin.subject, "completed", None)
            .unwrap();
        input.step = None;
        input.new_run = Some("page-feedback".into());
        input.request = Some(json!({
            "version": 1, "type": "feedback", "question": "What should the landing page say?",
            "why_person": "It speaks in the owner's voice.",
            "subjects": [{"kind": "document", "label": "Draft", "ref": "doc/example/landing"}]
        }));
        let ask = store.ask_person(&input).unwrap();
        // An older client sends only words; feedback takes them as its text.
        store
            .finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: ask.subject.clone(),
                    actor: "person/avery".into(),
                    summary: "Lead with the phone.".into(),
                    evidence: vec![],
                    episode: None,
                    idempotency_key: "page-feedback".into(),
                    answer: None,
                },
                false,
            )
            .unwrap();
        let own = store.step_run(&ask.subject).unwrap().unwrap();
        assert_eq!(
            own.person_answers[0].answer,
            Some(
                json!({"type": "feedback", "outcome": "feedback", "text": "Lead with the phone."})
            )
        );
        // No step waits on a new-run ask, so the answer is pushed to its requester.
        let messages = store.messages(Some("agent/alder.asker"), true).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].from, "daemon/runtime");
        assert_eq!(
            messages[0].title.as_deref(),
            Some("Answered: Choose a release date")
        );
        assert!(messages[0].content.contains("Lead with the phone."));
        assert!(
            messages[0]
                .content
                .contains(&format!("st work show {} --json", ask.subject))
        );
        // A replayed response does not send it again.
        store
            .finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: ask.subject.clone(),
                    actor: "person/avery".into(),
                    summary: "Lead with the phone.".into(),
                    evidence: vec![],
                    episode: None,
                    idempotency_key: "page-feedback".into(),
                    answer: None,
                },
                false,
            )
            .unwrap();
        store.replay_replication_graph().unwrap();
        assert_eq!(
            store
                .messages(Some("agent/alder.asker"), true)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn person_lifecycle_replication_rebuilds_only_its_tree() {
        for mode in ["step", "standalone", "owned"] {
            let standalone = mode != "step";
            for cancel in [false, true] {
                let (source, origin, mut input) = fixture();
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("claims.sqlite3");
                let target = Store::open(&path, "birch").unwrap();
                // The first graph has no healthy frontier; its initial replay is intentional.
                target.project_replication_backlog().unwrap();
                receive(&source, &target);
                if standalone {
                    source
                        .set_step_state(&origin.subject, "completed", None)
                        .unwrap();
                    input.step = None;
                    input.new_run = Some("release-question".into());
                }
                if mode == "owned" {
                    let mut owned = crate::graph::parse_internal_intent(
                        r#"version 2
agent "alder.asker" { workspace "/tmp"; command "true"; restart always; }
"#,
                        "alder",
                    )
                    .unwrap();
                    let declaration = owned.subjects.get_mut("agent/alder.asker").unwrap();
                    declaration.owner_run = Some(origin.run.clone());
                    declaration.owner_generation = Some(origin.generation.clone());
                    source.apply_internal(&owned, "own-the-requester").unwrap();
                }
                let ask = source.ask_person(&input).unwrap();
                receive(&source, &target);
                assert_eq!(
                    target.step_run(&ask.subject).unwrap().unwrap().status,
                    "ready"
                );
                if !standalone {
                    assert_eq!(
                        target.step_run(&origin.subject).unwrap().unwrap().status,
                        "waiting-person"
                    );
                }
                source
                    .finish_person_step(
                        &PersonStepResponse {
                            delegation: None,
                            subject: ask.subject.clone(),
                            actor: if cancel { input.actor } else { input.person },
                            summary: "Release reviewed".into(),
                            evidence: vec![],
                            episode: None,
                            idempotency_key: "review-release".into(),
                            answer: None,
                        },
                        cancel,
                    )
                    .unwrap();
                receive(&source, &target);
                assert_eq!(
                    target.step_run(&ask.subject).unwrap().unwrap().status,
                    if cancel { "cancelled" } else { "completed" }
                );
                if !standalone {
                    assert_eq!(
                        target.step_run(&origin.subject).unwrap().unwrap().status,
                        "ready"
                    );
                }
                target
                    .replication_status(true, Some(TEST_FLEET), &[])
                    .unwrap();
                let before = projection_digest::tables(&target.readers.get()).unwrap();
                drop(target);
                let reopened = Store::open(&path, "birch").unwrap();
                assert_eq!(
                    before,
                    projection_digest::tables(&reopened.readers.get()).unwrap()
                );
                assert_eq!(
                    before,
                    projection_digest::oracle(&reopened.readers.get()).unwrap()
                );
            }
        }
    }

    #[test]
    fn person_response_before_standalone_ask_matches_full_replay() {
        let (source, origin, mut input) = fixture();
        source
            .set_step_state(&origin.subject, "completed", None)
            .unwrap();
        let target = Store::open_memory("birch").unwrap();
        target.project_replication_backlog().unwrap();
        receive(&source, &target);
        input.step = None;
        input.new_run = Some("release-question".into());
        let ask = source.ask_person(&input).unwrap();
        let asking = source
            .export_replication_exchange_answering(
                TEST_FLEET,
                &target.replication_inventory().unwrap(),
                &[],
            )
            .unwrap();
        source
            .finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: ask.subject.clone(),
                    actor: input.actor,
                    summary: "No longer needed".into(),
                    evidence: vec![],
                    episode: None,
                    idempotency_key: "cancel-release".into(),
                    answer: None,
                },
                true,
            )
            .unwrap();
        let mut response = source
            .export_replication_exchange_answering(
                TEST_FLEET,
                &target.replication_inventory().unwrap(),
                &[],
            )
            .unwrap();
        response
            .envelopes
            .retain(|envelope| !asking.envelopes.iter().any(|old| old.hash == envelope.hash));
        receive_exchange(&target, &response);
        assert!(target.step_run(&ask.subject).unwrap().is_none());
        receive_exchange(&target, &asking);
        assert_eq!(
            target.step_run(&ask.subject).unwrap().unwrap().status,
            "cancelled"
        );
    }

    #[test]
    fn populated_person_receive_does_not_block_claims_and_renewals() {
        use std::sync::{Arc, Barrier};
        use std::time::{Duration, Instant};
        let (source, origin, mut input) = fixture();
        source
            .set_step_state(&origin.subject, "completed", None)
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let target = Store::open(&directory.path().join("claims.sqlite3"), "birch").unwrap();
        target.project_replication_backlog().unwrap();
        receive(&source, &target);
        let intent = crate::graph::parse_internal_intent(
            r#"version 2
agent "birch.worker" { workspace "/tmp"; command "true"; restart always; }
mission "writer-load" state="ready" {
 goal "Write while a peer receives.";
 step "write" { assigned-to "agent/birch.worker"; goal "Keep working."; }
}"#,
            "birch",
        )
        .unwrap();
        target.apply_internal(&intent, "writer-load").unwrap();
        let run = target
            .create_mission_run(&MissionRunRequest {
                mission: "writer-load".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/avery".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "writer-load-run".into(),
            })
            .unwrap();
        let worker = &run.steps[0];
        target.connection.batched(|tx| -> Result<()> {
            let claim = append_claim_tx(tx, "birch", &worker.subject, "work.claimed", Some("agent/birch.worker"),
                &json!({"fields":{"attempt":1,"status":"claimed","claimant":"agent/birch.worker",
                    "claim_incarnation":"worker-one","claim_expires_at_unix_ms":(now_ms()+600_000) as u64}}), &[], None)?;
            project_mission_run_update(tx, &claim).unwrap();
            // These are valid retained claims on unrelated subjects, not the run being updated.
            for n in 0..20_000 {
                append_claim_tx(tx, "birch", "daemon/birch", "daemon.diagnostic", None,
                    &json!({"fields":{"severity":"warning","code":"retained","reason":format!("history {n}")}}), &[], None)?;
            }
            Ok(())
        }).unwrap().unwrap();
        target.project_replication_backlog().unwrap();
        input.step = None;
        input.new_run = Some("release-question".into());
        let ask = source.ask_person(&input).unwrap();
        source
            .finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: ask.subject.clone(),
                    actor: input.actor,
                    summary: "No longer needed".into(),
                    evidence: vec![],
                    episode: None,
                    idempotency_key: "cancel-release".into(),
                    answer: None,
                },
                true,
            )
            .unwrap();
        let exchange = source
            .export_replication_exchange_answering(
                TEST_FLEET,
                &target.replication_inventory().unwrap(),
                &[],
            )
            .unwrap();
        let gate = Arc::new(Barrier::new(3));
        std::thread::scope(|scope| {
            let receiving = scope.spawn(|| {
                gate.wait();

                let before = FULL_REPLAYS.with(|count| count.get());
                let start = Instant::now();
                target
                    .receive_replication_exchange(&exchange.peer, TEST_FLEET, &exchange)
                    .unwrap();
                target.validate_replication_backlog().unwrap();
                target.apply_replication_repairs().unwrap();
                target.project_replication_backlog().unwrap();
                assert_eq!(FULL_REPLAYS.with(|count| count.get()), before);
                start.elapsed()
            });
            let writing = scope.spawn(|| {
                gate.wait();

                let start = Instant::now();
                for n in 0..10 {
                    target.append_claim(&ClaimInput {
                        subject: "daemon/birch".into(), kind: "daemon.diagnostic".into(), actor: None,
                        fields: serde_json::from_value(json!({"severity":"warning","code":"live","reason":format!("claim {n}")})).unwrap(),
                        evidence: vec![], expected_subject: None, idempotency_key: None,
                    }).unwrap();
                }
                start.elapsed()
            });
            gate.wait();

            let start = Instant::now();
            for n in 0..10 {
                target
                    .work_action(
                        &worker.subject,
                        "renew",
                        &WorkRequest {
                            wait_ms: None,
                            actor: Some("agent/birch.worker".into()),
                            incarnation: Some("worker-one".into()),
                            summary: None,
                            reason: None,
                            evidence: vec![],
                            idempotency_key: format!("load-renewal-{n}"),
                        },
                    )
                    .unwrap();
            }
            let renewing = start.elapsed();

            for (label, elapsed) in [
                ("receive/apply", receiving.join().unwrap()),
                ("claims", writing.join().unwrap()),
                ("renewals", renewing),
            ] {
                assert!(
                    elapsed < Duration::from_secs(2),
                    "{label} queued for {elapsed:?}"
                );
            }
        });
        assert_eq!(
            target.step_run(&ask.subject).unwrap().unwrap().status,
            "cancelled"
        );
        target.project_replication_backlog().unwrap();
        let incremental = projection_digest::tables(&target.readers.get()).unwrap();
        target.replay_replication_graph().unwrap();
        assert_eq!(
            incremental,
            projection_digest::tables(&target.readers.get()).unwrap()
        );
        assert_eq!(
            incremental,
            projection_digest::oracle(&target.readers.get()).unwrap()
        );
    }

    #[test]
    fn authored_person_work_waits_for_readiness_and_only_assignee_can_finish() {
        let (store, origin, _input) = fixture();
        let run = store.mission_run(&origin.run).unwrap().unwrap();
        let review = run.steps.iter().find(|step| step.step == "review").unwrap();
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
        store
            .set_step_state(&review.subject, "ready", None)
            .unwrap();
        let item = store
            .attention_items(Some("person/avery"))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(item.subject, review.subject);
        let mut response = PersonStepResponse {
            delegation: None,
            subject: review.subject.clone(),
            actor: "person/robin".into(),
            summary: "Reviewed the release".into(),
            evidence: vec![],
            episode: Some(item.episode),
            idempotency_key: "authored-response".into(),
            answer: None,
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "forbidden"
        );
        response.actor = "person/avery".into();
        assert_eq!(
            store.finish_person_step(&response, false).unwrap().status,
            "completed"
        );
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
        response.evidence.push("doc/release-review".into());
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "idempotency-conflict"
        );
        store.replay_replication_graph().unwrap();
        assert_eq!(
            store.step_run(&review.subject).unwrap().unwrap().status,
            "completed"
        );
    }

    #[test]
    fn cancelled_completed_and_failed_owners_hide_person_asks_before_cleanup() {
        for status in ["cancelled", "completed", "failed"] {
            let (store, origin, input) = fixture();
            let ask = store.ask_person(&input).unwrap();
            store
                .set_mission_run_state(&origin.run, status, "terminal", Some("the owner ended"))
                .unwrap();
            assert!(
                store
                    .attention_items(Some("person/avery"))
                    .unwrap()
                    .iter()
                    .all(|item| item.subject != ask.subject)
            );
            let response = PersonStepResponse {
                delegation: None,
                subject: ask.subject.clone(),
                actor: "person/avery".into(),
                summary: "Friday".into(),
                evidence: vec![],
                episode: None,
                idempotency_key: format!("ended-{status}"),
                answer: None,
            };
            assert_eq!(
                store.finish_person_step(&response, false).unwrap_err().code,
                "stale-fence"
            );
            store.replay_replication_graph().unwrap();
            assert!(
                store
                    .attention_items(Some("person/avery"))
                    .unwrap()
                    .iter()
                    .all(|item| item.subject != ask.subject)
            );
        }
    }

    /// A top-level seat that restarts onto another harness keeps the asks it raised: they stay
    /// on the person's alerts, and asking again finds the same ask instead of a second one.
    #[test]
    fn a_top_level_seats_ask_survives_its_redeclaration_and_ends_only_when_it_stops() {
        let (store, origin, mut input) = fixture();
        store
            .set_step_state(&origin.subject, "completed", None)
            .unwrap();
        input.step = None;
        input.new_run = Some("release-question".into());
        let ask = store.ask_person(&input).unwrap();
        let moved = crate::graph::parse_internal_intent("version 2\nagent \"alder.asker\" { workspace \"/tmp\"; command \"false\"; restart always; }", "alder").unwrap();
        store.apply_internal(&moved, "asker-moves-harness").unwrap();
        assert!(!store.reconcile_person_asks().unwrap());
        assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().status, "ready");
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .any(|item| item.subject == ask.subject)
        );
        assert_eq!(store.ask_person(&input).unwrap().subject, ask.subject);
        let stop =
            crate::graph::parse_internal_intent("version 2\nstop \"agent/alder.asker\"", "alder")
                .unwrap();
        store.apply_internal(&stop, "stop-asker").unwrap();
        assert!(store.reconcile_person_asks().unwrap());
        assert_eq!(
            store.step_run(&ask.subject).unwrap().unwrap().status,
            "cancelled"
        );
    }

    /// An ask whose requester ended does not vanish silently: the requester hears once, and the
    /// person gets one update in place of the alert.
    #[test]
    fn an_ask_cancelled_because_its_owner_ended_tells_the_requester_and_the_person_once() {
        let (store, _origin, input) = fixture();
        let ask = store.ask_person(&input).unwrap();
        let stop =
            crate::graph::parse_internal_intent("version 2\nstop \"agent/alder.asker\"", "alder")
                .unwrap();
        store.apply_internal(&stop, "stop-asker").unwrap();
        assert!(store.reconcile_person_asks().unwrap());
        assert!(!store.reconcile_person_asks().unwrap());
        let told = store
            .messages(Some("agent/alder.asker"), true)
            .unwrap()
            .into_iter()
            .filter(|message| message.from == "daemon/runtime")
            .filter_map(|message| message.title)
            .collect::<Vec<_>>();
        assert_eq!(told, vec!["Ask cancelled: Choose a release date".to_owned()]);
        let home = store.attention_items(Some("person/avery")).unwrap();
        assert!(home.iter().all(|item| item.subject != ask.subject));
        let updates = home
            .iter()
            .filter(|item| item.request.as_ref().is_some_and(|r| r["type"] == "update"))
            .map(|item| item.title.clone())
            .collect::<Vec<_>>();
        assert_eq!(updates, vec!["Cancelled: Choose a release date".to_owned()]);
    }

    /// A notice that cannot be written never undoes the cancellation: the ask still ends, and
    /// nothing half-told is left behind.
    #[test]
    fn a_notice_that_fails_does_not_undo_the_cancellation() {
        let (store, _origin, input) = fixture();
        let ask = store.ask_person(&input).unwrap();
        let asked = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        // The update the notice would post already exists with other words, so posting it fails.
        store
            .connection
            .batched(|tx| {
                store
                    .append_update_tx(
                        tx,
                        "daemon/runtime",
                        "person/avery",
                        "Something else",
                        "Something else.",
                        &format!("st3-ask-ended:{}", asked.id),
                        &json!({"version": 1, "type": "update", "about": ask.subject}),
                    )
                    .map(|_| ())
            })
            .unwrap()
            .unwrap();
        let stop =
            crate::graph::parse_internal_intent("version 2\nstop \"agent/alder.asker\"", "alder")
                .unwrap();
        store.apply_internal(&stop, "stop-asker").unwrap();
        assert!(store.reconcile_person_asks().unwrap());
        assert_eq!(
            store.step_run(&ask.subject).unwrap().unwrap().status,
            "cancelled"
        );
        assert!(
            store
                .messages(Some("agent/alder.asker"), true)
                .unwrap()
                .iter()
                .all(|message| message.from != "daemon/runtime"),
            "the message rolls back with the failed update"
        );
    }

    #[test]
    fn a_redeclared_requester_does_not_revive_its_old_person_episode() {
        let (store, _origin, input) = fixture();
        let ask = store.ask_person(&input).unwrap();
        let stop =
            crate::graph::parse_internal_intent("version 2\nstop \"agent/alder.asker\"", "alder")
                .unwrap();
        store.apply_internal(&stop, "retire-old-episode").unwrap();
        let renewed = crate::graph::parse_internal_intent("version 2\nagent \"alder.asker\" { workspace \"/tmp\"; command \"true\"; restart always; }", "alder").unwrap();
        store
            .apply_internal(&renewed, "new-requester-declaration")
            .unwrap();
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .all(|item| item.subject != ask.subject)
        );
        store.replay_replication_graph().unwrap();
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .all(|item| item.subject != ask.subject)
        );
    }

    #[test]
    fn a_later_question_in_the_same_attempt_does_not_revive_the_previous_ask() {
        let (store, origin, input) = fixture();
        let old = store.ask_person(&input).unwrap();
        store
            .set_step_state(&origin.subject, "ready", None)
            .unwrap();
        store.connection.batched(|tx| -> Result<()> {
            let claim = append_claim_tx(tx, "alder", &origin.subject, "work.claimed", Some(&input.actor),
                &json!({"fields":{"attempt":1,"status":"claimed","claimant":input.actor,
                    "claim_incarnation":"asker-one","claim_expires_at_unix_ms":(now_ms()+600_000) as u64}}), &[], None)?;
            project_mission_run_update(tx, &claim).unwrap();
            Ok(())
        }).unwrap().unwrap();
        let mut next = input;
        next.idempotency_key = "a-later-question".into();
        let fresh = store.ask_person(&next).unwrap();
        let items = store.attention_items(Some("person/avery")).unwrap();
        assert!(items.iter().any(|item| item.subject == fresh.subject));
        assert!(items.iter().all(|item| item.subject != old.subject));
        store.reconcile_person_asks().unwrap();
        assert_eq!(
            store.step_run(&origin.subject).unwrap().unwrap().status,
            "waiting-person"
        );
        assert_eq!(
            store
                .step_run(&origin.subject)
                .unwrap()
                .unwrap()
                .blocked_reason,
            Some(fresh.subject)
        );
    }

    #[test]
    fn legacy_requests_remain_audit_claims_without_creating_new_person_work() {
        let (store, origin, input) = fixture();
        let legacy = store
            .request_attention_closing(
                "attention/legacy-date",
                &AttentionRequest {
                    reviewer: input.person.clone(),
                    title: input.title.clone(),
                    reason: input.reason.clone(),
                    severity: "warning".into(),
                    targets: vec![origin.subject.clone()],
                    actor: input.actor.clone(),
                    idempotency_key: "legacy-date".into(),
                },
                &AttentionClosing {
                    step: Some(origin.subject.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .work_action(
                &origin.subject,
                "release",
                &WorkRequest {
                    wait_ms: None,
                    actor: Some(input.actor.clone()),
                    incarnation: input.incarnation.clone(),
                    summary: None,
                    reason: Some("Waiting for a person".into()),
                    evidence: vec![],
                    idempotency_key: "release-legacy-ask".into(),
                },
            )
            .unwrap();
        assert!(!store.reconcile_person_asks().unwrap());
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.step_run(&origin.subject).unwrap().unwrap().status,
            "ready"
        );
        store.replay_replication_graph().unwrap();
        let retained = store.attention_request(&legacy.subject).unwrap().unwrap();
        assert_eq!(retained.request, legacy.request);
        assert_eq!(retained.status, "pending");
        assert_eq!(retained.requested_at_unix_ms, legacy.requested_at_unix_ms);
        assert!(!store.reconcile_person_asks().unwrap());
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn origin_retry_invalidates_old_ask_and_response_without_cleanup() {
        let (store, origin, input) = fixture();
        let ask = store.ask_person(&input).unwrap();
        store
            .set_step_state(&origin.subject, "failed", Some("try again"))
            .unwrap();
        store.retry_step(&origin.subject, "new attempt", 0).unwrap();
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .iter()
                .all(|item| item.subject != ask.subject)
        );
        let response = PersonStepResponse {
            delegation: None,
            subject: ask.subject,
            actor: "person/avery".into(),
            summary: "Friday".into(),
            evidence: vec![],
            episode: None,
            idempotency_key: "late-old-attempt".into(),
            answer: None,
        };
        assert_eq!(
            store.finish_person_step(&response, false).unwrap_err().code,
            "stale-fence"
        );
    }
}
