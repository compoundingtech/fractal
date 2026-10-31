//! The part of `store::person_work` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use super::*;
use crate::model::{MessageAttachment, PersonAskRequest, PersonStepResponse};
use crate::person_request::{StructuredRequest, answer_summary};
use crate::model::PersonRenameRequest;
pub use fractal_missions::store::person_work::*;
pub use fractal_missions::store::person_work::{AskImages, current, declaration_live, enrich_responses, is_update, project, project_minimal_run, request, run_live, run_liveness, step, validate_delegated_answer};
#[cfg(test)]
mod reconcile_candidates_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use fractal_missions::store::person_work_tests_shared_seats::*;
    use fractal_missions::store::person_work_tests_shared_seats::{TEST_FLEET, gateway_decision, receive_exchange};
    pub use fractal_missions::store::person_work::tests::*;

    fn rename_request() -> PersonRenameRequest {
        PersonRenameRequest {
            old_person: "person/avery".into(),
            new_person: "person/robin".into(),
            actor: "person/avery".into(),
            idempotency_key: "repair-avery".into(),
        }
    }

    fn known_rename_person(store: &Store) {
        store.append_claim(&ClaimInput {
            subject: "message/rename-destination-known".into(),
            kind: "message.sent".into(),
            actor: Some("person/robin".into()),
            fields: BTreeMap::from([
                ("from".into(), json!("person/robin")),
                ("to".into(), json!("agent/alder.asker")),
                ("content".into(), json!("This person already participates in the graph.")),
                ("status".into(), json!("sent")),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("rename-destination-known".into()),
        }).unwrap();
    }

    fn rename_claim_count(store: &Store) -> u64 {
        store.readers.get().query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn rename_moves_authored_and_asked_inboxes_without_changing_history() {
        let (store, origin, input) = fixture();
        known_rename_person(&store);
        let ask = store.ask_person(&input).unwrap();
        let original = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        let run = store.mission_run(&origin.run).unwrap().unwrap();
        let review = run.steps.iter().find(|step| step.step == "review").unwrap();
        store.set_step_state(&review.subject, "ready", None).unwrap();
        let before = store.attention_items(Some("person/avery")).unwrap();
        let repair = rename_request();
        let report = store.rename_person(&repair, &[]).unwrap();
        let mut expected = vec![ask.subject.clone(), review.subject.clone()];
        expected.sort();
        assert_eq!(report.reassigned_steps, expected);
        assert_eq!(report.run_requesters, vec![origin.run.clone()]);
        assert!(store.attention_items(Some("person/avery")).unwrap().is_empty());
        let after = store.attention_items(Some("person/robin")).unwrap();
        assert_eq!(after.len(), 2);
        for old in &before {
            let new = after.iter().find(|item| item.subject == old.subject).unwrap();
            assert_eq!(new.episode, old.episode);
            assert_eq!(new.requester_id, old.requester_id);
            assert_eq!(new.person, "person/robin");
        }
        let preserved = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        assert_eq!(preserved.body, original.body);
        assert_eq!(preserved.actor, original.actor);
        assert_eq!(preserved.id, original.id);
        let count = rename_claim_count(&store);
        assert_eq!(store.rename_person(&repair, &[]).unwrap(), report);
        assert_eq!(rename_claim_count(&store), count);
        let mut conflicting = repair.clone();
        conflicting.new_person = "person/blair".into();
        assert_eq!(store.rename_person(&conflicting, &[]).unwrap_err().code, "idempotency-conflict");
        conflicting = repair.clone();
        conflicting.actor = "person/blair".into();
        assert_eq!(store.rename_person(&conflicting, &[]).unwrap_err().code, "idempotency-conflict");
        store.replay_replication_graph().unwrap();
        assert_eq!(store.rename_person(&repair, &[]).unwrap(), report);
        for item in after {
            let mut response = PersonStepResponse {
                delegation: None,
                subject: item.subject.clone(),
                actor: "person/avery".into(),
                summary: "Reviewed".into(),
                evidence: vec![],
                episode: Some(item.episode),
                idempotency_key: format!("renamed-answer:{}", item.subject),
                answer: None,
            };
            assert_eq!(store.finish_person_step(&response, false).unwrap_err().code, "forbidden");
            response.actor = "person/robin".into();
            assert_eq!(store.finish_person_step(&response, false).unwrap().status, "completed");
        }
        assert_eq!(request(&store.readers.get(), &ask.subject).unwrap().unwrap().actor, original.actor);
        assert_eq!(store.claim_by_id(&original.id).unwrap().unwrap().body, original.body);
    }

    #[test]
    fn rename_preserves_pending_episode_and_requester_cancel_authority() {
        let (store, origin, input) = fixture();
        known_rename_person(&store);
        let ask = store.ask_person(&input).unwrap();
        let review = store.mission_run(&origin.run).unwrap().unwrap().steps.into_iter()
            .find(|step| step.step == "review").unwrap();
        assert_eq!(review.status, "pending");
        let report = store.rename_person(&rename_request(), &[]).unwrap();
        assert!(report.reassigned_steps.contains(&review.subject));
        let moved = store.step_run(&review.subject).unwrap().unwrap();
        assert_eq!(moved.status, review.status);
        assert_eq!(moved.attempt, review.attempt);
        assert_eq!(moved.generation, review.generation);
        assert_eq!(moved.readiness_epoch, review.readiness_epoch);
        assert_eq!(moved.assigned_to.as_deref(), Some("person/robin"));
        let response = PersonStepResponse {
            delegation: None,
            subject: ask.subject,
            actor: input.actor,
            summary: "Not needed".into(),
            evidence: vec![],
            episode: None,
            idempotency_key: "cancel-renamed".into(),
            answer: None,
        };
        assert_eq!(store.finish_person_step(&response, true).unwrap().status, "cancelled");
        assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "ready");
    }

    #[test]
    fn rename_leaves_retired_stale_and_answered_asks_untouched() {
        for invalidation in ["retired", "retry", "answered", "owner-ended"] {
            let (store, origin, input) = fixture();
            known_rename_person(&store);
            let ask = store.ask_person(&input).unwrap();
            match invalidation {
                "retired" => {
                    let stop = crate::graph::parse_internal_intent("version 2\nstop \"agent/alder.asker\"", "alder").unwrap();
                    store.apply_internal(&stop, "retire-rename-requester").unwrap();
                }
                "retry" => {
                    store.set_step_state(&origin.subject, "failed", Some("retry")).unwrap();
                    store.retry_step(&origin.subject, "retry", 0).unwrap();
                }
                "answered" => {
                    store.finish_person_step(&PersonStepResponse {
                        delegation: None,
                        subject: ask.subject.clone(),
                        actor: "person/avery".into(),
                        summary: "Friday".into(),
                        evidence: vec![],
                        episode: None,
                        idempotency_key: "answer-before-rename".into(),
                        answer: None,
                    }, false).unwrap();
                }
                "owner-ended" => { store.set_mission_run_state(&origin.run, "cancelled", "terminal", Some("ended")).unwrap(); }
                _ => unreachable!(),
            }
            let prior = store.step_run(&ask.subject).unwrap().unwrap();
            let original = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
            let report = store.rename_person(&rename_request(), &[]).unwrap();
            assert!(!report.reassigned_steps.contains(&ask.subject), "{invalidation}");
            let unchanged = store.step_run(&ask.subject).unwrap().unwrap();
            assert_eq!(unchanged.assigned_to, prior.assigned_to, "{invalidation}");
            assert_eq!(unchanged.status, prior.status, "{invalidation}");
            assert_eq!(request(&store.readers.get(), &ask.subject).unwrap().unwrap().body, original.body);
            store.replay_replication_graph().unwrap();
            assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().assigned_to, prior.assigned_to);
        }
    }

    #[test]
    fn rename_before_ask_replication_converges_and_receipt_survives_reopen() {
        let (source, origin, input) = fixture();
        known_rename_person(&source);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rename.sqlite3");
        let target = Store::open(&path, "birch").unwrap();
        target.project_replication_backlog().unwrap();
        receive(&source, &target);
        let ask = source.ask_person(&input).unwrap();
        let asking = source.export_replication_exchange_answering(
            TEST_FLEET, &target.replication_inventory().unwrap(), &[]).unwrap();
        let repair = rename_request();
        let report = source.rename_person(&repair, &[]).unwrap();
        let mut renaming = source.export_replication_exchange_answering(
            TEST_FLEET, &target.replication_inventory().unwrap(), &[]).unwrap();
        renaming.envelopes.retain(|envelope| !asking.envelopes.iter().any(|old| old.hash == envelope.hash));
        receive_exchange(&target, &renaming);
        assert!(target.step_run(&ask.subject).unwrap().is_none());
        receive_exchange(&target, &asking);
        assert_eq!(target.step_run(&ask.subject).unwrap().unwrap().assigned_to.as_deref(), Some("person/robin"));
        assert_eq!(target.rename_person(&repair, &[]).unwrap(), report);
        assert_eq!(target.mission_run(&origin.run).unwrap().unwrap().requester, "person/avery");
        let count = rename_claim_count(&target);
        drop(target);
        let reopened = Store::open(&path, "birch").unwrap();
        assert_eq!(reopened.rename_person(&repair, &[]).unwrap(), report);
        assert_eq!(rename_claim_count(&reopened), count);
        assert!(reopened.attention_items(Some("person/avery")).unwrap().is_empty());
        assert!(reopened.attention_items(Some("person/robin")).unwrap().iter().any(|item| item.subject == ask.subject));
    }

    #[test]
    fn stale_reassignment_fences_cannot_move_a_different_episode_or_person() {
        for fence in ["generation", "attempt", "episode", "previous_person"] {
            let (store, origin, input) = fixture();
            let ask = store.ask_person(&input).unwrap();
            let original = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
            let mut fields = json!({
                "run": ask.run, "generation": ask.generation, "attempt": ask.attempt,
                "episode": original.id, "previous_person": "person/avery", "person": "person/robin",
            });
            fields[fence] = match fence {
                "generation" => json!("run-generation/not-current"),
                "attempt" => json!(ask.attempt + 1),
                "episode" => json!("another-episode"),
                "previous_person" => json!("person/blair"),
                _ => unreachable!(),
            };
            store.connection.batched(|tx| -> Result<()> {
                let claim = append_claim_tx(tx, "alder", &ask.subject, "work.person-reassigned",
                    Some("person/operator"), &json!({"fields": fields}), &[], None)?;
                project(tx, &claim).unwrap();
                Ok(())
            }).unwrap().unwrap();
            assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().assigned_to.as_deref(), Some("person/avery"), "{fence}");
            assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "waiting-person");
            store.replay_replication_graph().unwrap();
            assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().assigned_to.as_deref(), Some("person/avery"), "{fence}");
        }
    }
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
            deadline_ms: None,
            default_answer: None,
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
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h67888f2c1780 owner=fractal_seats::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_generation='run-generation/old' WHERE subject='subscription/intake'", [])?;
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
                                /* sqlite-large-value: allow-for-now lv-hd90855b784ec owner=fractal_seats::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET parent_step_run=?1 WHERE id=?2",
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
            let eta = fractal_store::work_eta::summary(
                Some("Release preparation checkpoint"),
                "2h",
                "agent/alder.asker",
                now_ms(),
            )
            .unwrap();
            store
                .work_action(
                    &origin.subject,
                    "progress",
                    &crate::model::WorkRequest {
                        wait_ms: None,
                        actor: Some("agent/alder.asker".into()),
                        incarnation: Some("asker-one".into()),
                        summary: Some(eta.clone()),
                        reason: None,
                        evidence: Vec::new(),
                        idempotency_key: "prepare-eta".into(),
                    },
                )
                .unwrap();
            let moved = revise_fixture(&store, &origin, prepare_changes);
            assert_eq!(
                moved.progress_summary.as_deref(),
                (!prepare_changes).then_some("Release preparation checkpoint"),
                "a carried step keeps material progress, but never the old ETA"
            );
            assert_eq!(
                store.step_run(&origin.subject).unwrap().unwrap().progress_summary.as_deref(),
                Some(eta.as_str()),
                "the old subject retains its report as history"
            );
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

    #[test]
    fn assistant_answer_keeps_attribution_on_origin_and_ask_after_replay() {
        let (store, origin, mut input) = fixture();
        input.request = Some(gateway_decision());
        let ask = store.ask_person(&input).unwrap();
        let grant_id = "assistant-grant/projection-test";
        let authorization = super::super::assistant_write::AssistantWriteAuthorization {
            grant_subject: "custom/client/assistant-projection-test".into(),
            actor: format!("assistant/chatgpt/session/{grant_id}"),
            owner: "person/avery".into(),
            policy_revision: 1,
            scope: "write.assistant.answers".into(),
            recipient: None,
        };
        store.append_claim(&ClaimInput {
            subject: authorization.grant_subject.clone(),
            kind: "custom.client.assistant.v1.granted".into(),
            actor: Some("person/avery".into()),
            fields: serde_json::from_value(json!({
                "grant_id": grant_id, "principal": "assistant/chatgpt",
                "session_actor": authorization.actor, "issuer": authorization.owner,
                "disclosure_consent": true, "policy_revision": 1,
                "expires_at_unix_ms": (now_ms() + 60_000) as u64,
                "scopes": ["write.assistant.answers"]
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let item = store.attention_items(Some("person/avery")).unwrap()
            .into_iter().find(|item| item.subject == ask.subject).unwrap();
        let response = PersonStepResponse {
            delegation: None, subject: ask.subject.clone(), actor: "person/avery".into(),
            summary: String::new(), evidence: vec![], episode: Some(item.episode),
            idempotency_key: "assistant-projection-answer".into(),
            answer: Some(crate::person_request::AnswerInput { id: Some("land".into()), text: None }),
        };
        store.finish_person_step_authorized(&response, false, Some(&authorization)).unwrap();
        // The answer is stored under the assistant principal, so a retry with the same key must
        // match that stored actor and return the stored view, not write a second answer.
        store.finish_person_step_authorized(&response, false, Some(&authorization)).unwrap();
        assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().person_answers.len(), 1);
        let check = |store: &Store| {
            for subject in [&origin.subject, &ask.subject] {
                let view = store.step_run(subject).unwrap().unwrap();
                let answer = &view.person_answers[0];
                assert_eq!(answer.respondent, "assistant/chatgpt");
                assert_eq!(answer.assistant_owner.as_deref(), Some(authorization.owner.as_str()));
                assert_eq!(answer.assistant_session_actor.as_deref(), Some(authorization.actor.as_str()));
                assert_eq!(answer.sender_class.as_deref(), Some("external-assistant"));
                assert_eq!(answer.assistant_grant_id.as_deref(), Some(grant_id));
                let json = serde_json::to_value(&view).unwrap();
                assert_eq!(json["person_answers"][0]["sender_class"], "external-assistant");
                assert_eq!(json["person_answers"][0]["assistant_grant_id"], grant_id);
                if subject == &origin.subject {
                    assert!(view.constraints.iter().any(|constraint|
                        constraint.starts_with("Person response:")
                            && constraint.contains("sender_class: external-assistant")
                            && constraint.contains(&format!("assistant_grant_id: {grant_id}"))));
                }
            }
        };
        check(&store);
        store.replay_replication_graph().unwrap();
        check(&store);
        input.step = None;
        input.new_run = Some("standalone-notification".into());
        input.idempotency_key = "standalone-notification".into();
        let standalone = store.ask_person(&input).unwrap();
        let item = store.attention_items(Some("person/avery")).unwrap()
            .into_iter().find(|item| item.subject == standalone.subject).unwrap();
        store.finish_person_step_authorized(&PersonStepResponse {
            delegation: None, subject: standalone.subject, actor: "person/avery".into(),
            summary: String::new(), evidence: vec![], episode: Some(item.episode),
            idempotency_key: "standalone-notification-answer".into(),
            answer: Some(crate::person_request::AnswerInput { id: Some("land".into()), text: None }),
        }, false, Some(&authorization)).unwrap();
        let notification = store.messages(Some("agent/alder.asker"), true).unwrap().into_iter()
            .find(|message| message.assistant_grant_id.as_deref() == Some(grant_id)).unwrap();
        let claim = store.latest_claim(&notification.subject, Some("message.sent")).unwrap().unwrap();
        super::super::assistant_write::validate_replicated(&claim.kind, claim.actor.as_deref(), &claim.body["fields"]).unwrap();
        assert!(matches!(classify_replicated_claim_with_registry(&claim, fractal_schema::registry()).unwrap(),
            ReplicatedClaimAdmission::Valid));
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

    /// Deadlines are off until `[asks] deadlines` turns them on, because a member on an older
    /// build keeps a claim with fields it does not know as retryable. While off, no claim
    /// carries a deadline field, an ask that names a deadline or default is refused, and no ask
    /// resolves by itself however long it waits.
    #[test]
    fn while_deadlines_are_off_nothing_writes_the_new_fields() {
        let (store, origin, mut input) = fixture();
        assert!(!store.ask_deadlines());
        input.request = Some(gateway_decision());
        for (deadline_ms, default) in [(Some(3_600_000), None), (None, Some("keep-open"))] {
            let mut named = input.clone();
            named.deadline_ms = deadline_ms;
            named.default_answer = default.map(str::to_owned);
            assert_eq!(store.ask_person(&named).unwrap_err().code, "ask-deadlines-off");
        }
        let ask = store.ask_person(&input).unwrap();
        // A new-run ask, from a requester that holds no claimed work.
        let (other, other_origin, mut new_run) = fixture();
        other.set_step_state(&other_origin.subject, "completed", None).unwrap();
        new_run.request = Some(gateway_decision());
        new_run.step = None;
        new_run.new_run = Some("gateway".into());
        let new_run_ask = other.ask_person(&new_run).unwrap();
        let far = now_ms() + 365 * u128::from(DEFAULT_ASK_DEADLINE_MS);
        for (store, subject) in [(&store, &ask.subject), (&other, &new_run_ask.subject)] {
            assert_eq!(store.reconcile_person_asks_at(far).unwrap(), PersonAskPass::default());
            let claim = request(&store.readers.get(), subject).unwrap().unwrap();
            for field in ["deadline_unix_ms", "deadline_ms", "default_answer"] {
                assert!(claim.body["fields"].get(field).is_none(), "{subject} {field}");
            }
            let view = store.step_run(subject).unwrap().unwrap();
            assert_eq!(view.status, "ready", "{subject}");
            assert!(view.person_answers.is_empty(), "{subject}");
            let connection = store.readers.get();
            let ended: i64 = connection.query_row(
                "SELECT COUNT(*) FROM claims WHERE kind IN ('work.person-done','work.person-cancelled')",
                [], |row| row.get(0)).unwrap();
            assert_eq!(ended, 0, "{subject}");
            assert_eq!(ask_deadlines_since(&connection).unwrap(), 0);
        }
        assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "waiting-person");
    }

    /// Every ask has a deadline. At it, an unanswered structured ask completes with its default
    /// answer, recorded as resolved by the deadline and not by the person: it leaves the
    /// person's home, and the asking step resumes reading that it timed out and which default
    /// applied. A person cannot answer it afterwards.
    #[test]
    fn a_structured_ask_resolves_with_its_default_answer_at_its_deadline() {
        let (store, origin, mut input) = fixture();
        store.set_ask_deadlines(true);
        input.title = "Land the browser gateway?".into();
        input.request = Some(gateway_decision());
        input.deadline_ms = Some(3_600_000);
        for (default, code) in [("revise-auth", "invalid-default-answer"), ("ship-it", "invalid-default-answer")] {
            let mut refused = input.clone();
            refused.default_answer = Some(default.into());
            assert_eq!(store.ask_person(&refused).unwrap_err().code, code, "{default}");
        }
        let mut too_soon = input.clone();
        too_soon.deadline_ms = Some(10_000);
        assert_eq!(store.ask_person(&too_soon).unwrap_err().code, "invalid-ask-deadline");
        input.default_answer = Some("keep-open".into());
        let asked = 1_800_000_000_000u128;
        smallclaims::store::set_thread_clock(Some(asked));
        let ask = store.ask_person(&input).unwrap();
        smallclaims::store::set_thread_clock(None);
        let claim = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        let deadline = u128::from(claim.body["fields"]["deadline_unix_ms"].as_u64().unwrap());
        assert_eq!(deadline, asked + 3_600_000);
        assert_eq!(claim.body["fields"]["default_answer"], "keep-open");
        assert_eq!(claim.body["fields"]["deadline_ms"], 3_600_000);
        let mut conflict = input.clone();
        conflict.default_answer = Some("land".into());
        assert_eq!(store.ask_person(&conflict).unwrap_err().code, "idempotency-conflict");
        // A retry with the same key keeps the first ask's deadline, so it may not name another,
        // nor fall back to the fleet's default; an identical retry finds the same ask.
        for deadline_ms in [Some(30 * 24 * 3_600_000), None] {
            let mut later = input.clone();
            later.deadline_ms = deadline_ms;
            assert_eq!(store.ask_person(&later).unwrap_err().code, "idempotency-conflict", "{deadline_ms:?}");
        }
        assert_eq!(store.ask_person(&input).unwrap().subject, ask.subject);
        let retried = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        assert_eq!(retried.body["fields"]["deadline_unix_ms"], claim.body["fields"]["deadline_unix_ms"]);

        let early = store.reconcile_person_asks_at(deadline - 1).unwrap();
        assert_eq!(early, PersonAskPass { changed: false, next_deadline: Some(deadline) });
        let item = store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .find(|item| item.subject == ask.subject)
            .unwrap();
        // The person's home shows when the ask resolves by itself and what then applies.
        assert_eq!(item.deadline_unix_ms, Some(deadline));
        assert_eq!(
            item.default_answer,
            Some(json!({"id": "keep-open", "label": "Keep it open"}))
        );
        // The stored attention projection does not read the deadline policy, so a policy change
        // cannot leave its rows stale: the published list adds the deadline at each fold.
        let row = fractal_store::store::attention_ivm::resource(&item).unwrap();
        assert!(row.get("deadline_at").is_none() && row.get("default_answer").is_none(), "{row}");
        assert!(store.reconcile_person_asks_at(deadline).unwrap().changed);
        assert!(
            store.attention_items(Some("person/avery")).unwrap().iter().all(|item| item.subject != ask.subject),
            "the resolved ask leaves the person's home"
        );
        // The default carries the outcome of the answer it stands for; resolved_by says it timed out.
        let expected = json!({"type": "decision", "outcome": "decline", "id": "keep-open", "label": "Keep it open"});
        let check = |store: &Store| {
            let resumed = store.step_run(&origin.subject).unwrap().unwrap();
            assert_eq!(resumed.status, "ready");
            assert_eq!(resumed.person_answers.len(), 1);
            let answer = &resumed.person_answers[0];
            assert_eq!(answer.ask, ask.subject);
            assert_eq!(answer.status, "completed");
            assert_eq!(answer.respondent, "daemon/runtime");
            assert_eq!(answer.resolved_by.as_deref(), Some("deadline"));
            assert_eq!(answer.answer.as_ref(), Some(&expected));
            assert_eq!(answer.summary, "Timed out without an answer; the default applied: Keep it open (keep-open)");
            assert!(resumed.constraints.iter().any(|constraint| constraint
                == "Person response: Timed out without an answer; the default applied: Keep it open (keep-open)"));
            assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().status, "completed");
        };
        check(&store);
        store.replay_replication_graph().unwrap();
        check(&store);
        assert_eq!(store.reconcile_person_asks_at(deadline + 1).unwrap(), PersonAskPass::default());
        let late = PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: "person/avery".into(),
            summary: String::new(),
            evidence: vec![],
            episode: Some(item.episode),
            idempotency_key: "gateway-late".into(),
            answer: Some(crate::person_request::AnswerInput { id: Some("land".into()), text: None }),
        };
        assert_eq!(store.finish_person_step(&late, false).unwrap_err().code, "stale-fence");
    }

    /// A plain ask names no answers, so at the fleet's default deadline it resolves as timed out
    /// with no answer.
    #[test]
    fn a_plain_ask_times_out_without_an_answer_at_the_fleet_default_deadline() {
        let (store, origin, mut input) = fixture();
        store.set_ask_deadlines(true);
        input.default_answer = Some("friday".into());
        assert_eq!(store.ask_person(&input).unwrap_err().code, "invalid-default-answer");
        input.default_answer = None;
        store.set_default_ask_deadline_ms(2 * 3_600_000).unwrap();
        let asked = 1_800_000_000_000u128;
        smallclaims::store::set_thread_clock(Some(asked));
        let ask = store.ask_person(&input).unwrap();
        smallclaims::store::set_thread_clock(None);
        let claim = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        let deadline = u128::from(claim.body["fields"]["deadline_unix_ms"].as_u64().unwrap());
        assert_eq!(deadline, asked + 2 * 3_600_000);
        assert!(claim.body["fields"].get("default_answer").is_none());
        assert!(!store.reconcile_person_asks_at(deadline - 1).unwrap().changed);
        assert!(store.reconcile_person_asks_at(deadline).unwrap().changed);
        let resumed = store.step_run(&origin.subject).unwrap().unwrap();
        assert_eq!(resumed.status, "ready");
        let answer = &resumed.person_answers[0];
        assert_eq!(answer.resolved_by.as_deref(), Some("deadline"));
        assert_eq!(answer.answer, None);
        assert_eq!(answer.summary, "Timed out without an answer");
        assert!(resumed.constraints.iter().any(|constraint| constraint == "Person response: Timed out without an answer"));
    }

    /// A person may answer on another member just before the deadline while the ask's own
    /// member, not yet holding that answer, applies the default. Once both members hold both
    /// responses, each keeps only the person's answer: the asking step reads it alone and is
    /// told the default was overridden, and its asker gets one message saying so. Past the
    /// deadline an answer is refused on any member, so it never races the default.
    #[test]
    fn an_answer_before_the_deadline_on_another_member_beats_the_default() {
        let (owner, origin, mut input) = fixture();
        owner.set_ask_deadlines(true);
        input.title = "Land the browser gateway?".into();
        input.request = Some(gateway_decision());
        input.deadline_ms = Some(3_600_000);
        input.default_answer = Some("keep-open".into());
        let ask = owner.ask_person(&input).unwrap();
        let claim = request(&owner.readers.get(), &ask.subject).unwrap().unwrap();
        let deadline = u128::from(claim.body["fields"]["deadline_unix_ms"].as_u64().unwrap());
        let episode = claim.id;
        let other = Store::open_memory("birch").unwrap();
        other.set_ask_deadlines(true);
        other.project_replication_backlog().unwrap();
        receive(&owner, &other);
        let answer = |key: &str, id: &str| PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: "person/avery".into(),
            summary: String::new(),
            evidence: vec![],
            episode: Some(episode.clone()),
            idempotency_key: key.into(),
            answer: Some(crate::person_request::AnswerInput { id: Some(id.into()), text: None }),
        };
        // The person answers on the other member before the deadline; the owner, not holding
        // that answer, applies the default at the deadline.
        other.finish_person_step(&answer("land-in-time", "land"), false).unwrap();
        assert!(owner.reconcile_person_asks_at(deadline).unwrap().changed);
        assert_eq!(owner.step_run(&origin.subject).unwrap().unwrap().person_answers[0].resolved_by.as_deref(), Some("deadline"));
        // The answer was made first, so it sorts before the default it reaches after: the
        // projection replays for it, which the incremental-only helper refuses.
        let exchange = |source: &Store, target: &Store| {
            let exchange = source.export_replication_exchange_answering(
                TEST_FLEET, &target.replication_inventory().unwrap(), &[]).unwrap();
            target.receive_replication_exchange(&exchange.peer, TEST_FLEET, &exchange).unwrap();
            target.validate_replication_backlog().unwrap();
            assert!(target.project_replication_backlog().unwrap());
        };
        exchange(&other, &owner);
        exchange(&owner, &other);
        assert!(owner.reconcile_person_asks_at(deadline + 1_000).unwrap().changed);
        for store in [&owner, &other] {
            let resumed = store.step_run(&origin.subject).unwrap().unwrap();
            assert_eq!(resumed.person_answers.len(), 1, "{}", store.origin);
            let standing = &resumed.person_answers[0];
            assert_eq!(standing.respondent, "person/avery");
            assert_eq!(standing.resolved_by, None);
            assert_eq!(standing.answer.as_ref().unwrap()["id"], "land");
            assert!(standing.answered_at_unix_ms < deadline);
            assert!(resumed.constraints.iter().any(|constraint| constraint.contains("replaces the default")), "{}", store.origin);
        }
        let told = |store: &Store| store.messages(Some("agent/alder.asker"), true).unwrap().into_iter()
            .filter(|message| message.content.contains("before its deadline")).count();
        assert_eq!(told(&owner), 1);
        // Telling is once, however many passes follow.
        owner.person_answers_checked.store(0, std::sync::atomic::Ordering::Relaxed);
        owner.reconcile_person_asks_at(deadline + 2_000).unwrap();
        assert_eq!(told(&owner), 1);
    }

    /// Past an ask's deadline a person's answer is refused on a member that does not hold the
    /// default yet, so a late answer never stands beside it. The time the answer is stored at
    /// decides, not when it was sent: one sent just before the deadline but written at it is
    /// refused, and nothing of it is kept.
    #[test]
    fn an_answer_stored_at_the_deadline_is_refused_on_another_member() {
        const ASKED: u128 = 1_800_000_000_000;
        let (owner, _, mut input) = fixture();
        owner.set_ask_deadlines(true);
        input.request = Some(gateway_decision());
        input.deadline_ms = Some(3_600_000);
        smallclaims::store::set_thread_clock(Some(ASKED));
        let ask = owner.ask_person(&input).unwrap();
        let episode = request(&owner.readers.get(), &ask.subject).unwrap().unwrap().id;
        let other = Store::open_memory("birch").unwrap();
        other.set_ask_deadlines(true);
        other.project_replication_backlog().unwrap();
        receive(&owner, &other);
        // Sent a moment before the deadline, and stored at it behind a queue of writes.
        smallclaims::store::set_thread_clock(Some(ASKED + 3_600_000 - 1));
        other.set_write_clock_at(ASKED + 3_600_000).unwrap();
        let late = PersonStepResponse {
            delegation: None,
            subject: ask.subject.clone(),
            actor: "person/avery".into(),
            summary: String::new(),
            evidence: vec![],
            episode: Some(episode),
            idempotency_key: "land-late".into(),
            answer: Some(crate::person_request::AnswerInput { id: Some("land".into()), text: None }),
        };
        assert_eq!(other.finish_person_step(&late, false).unwrap_err().code, "stale-fence");
        assert_eq!(other.step_run(&ask.subject).unwrap().unwrap().status, "ready");
        let answers: i64 = other.readers.get().query_row("SELECT COUNT(*) FROM claims WHERE subject=?1 AND kind='work.person-done'",
            [&ask.subject], |row| row.get(0)).unwrap();
        assert_eq!(answers, 0, "the refused answer's write rolled back");
        // With deadlines off on that member nothing would apply the default there, so it accepts.
        other.set_ask_deadlines(false);
        other.finish_person_step(&late, false).unwrap();
        smallclaims::store::set_thread_clock(None);
    }

    /// No step waits on a new-run ask, so its timeout reaches the requester as a message.
    #[test]
    fn a_new_run_ask_that_times_out_tells_the_requester_which_default_applied() {
        let (store, origin, mut input) = fixture();
        store.set_ask_deadlines(true);
        store.set_step_state(&origin.subject, "completed", None).unwrap();
        input.step = None;
        input.new_run = Some("gateway".into());
        input.title = "Land the browser gateway?".into();
        input.request = Some(gateway_decision());
        input.default_answer = Some("land".into());
        let ask = store.ask_person(&input).unwrap();
        let claim = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        let deadline = u128::from(claim.body["fields"]["deadline_unix_ms"].as_u64().unwrap());
        // The fleet's default applied, so a retry that names a deadline asks something else.
        assert!(claim.body["fields"]["deadline_ms"].is_null());
        let mut named = input.clone();
        named.deadline_ms = Some(60_000);
        assert_eq!(store.ask_person(&named).unwrap_err().code, "idempotency-conflict");
        assert_eq!(store.ask_person(&input).unwrap().subject, ask.subject);
        assert!(store.reconcile_person_asks_at(deadline).unwrap().changed);
        let own = store.step_run(&ask.subject).unwrap().unwrap();
        assert_eq!(own.status, "completed");
        assert_eq!(own.person_answers[0].resolved_by.as_deref(), Some("deadline"));
        let messages = store.messages(Some("agent/alder.asker"), true).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.starts_with(&format!(
            "person/avery did not answer, by its deadline, `{}`: Land the browser gateway?", ask.subject)));
        assert!(messages[0].content.contains("Answer: Land the gateway (land)"));
    }

    /// An ask made before deadlines were on (by an older build, or while `[asks] deadlines` was
    /// off) carries no deadline. Counting from when it was asked would resolve every old ask at
    /// once, so its deadline counts from the first pass that saw deadlines on.
    #[test]
    fn an_ask_open_when_deadlines_turn_on_counts_its_deadline_from_then() {
        let (store, origin, input) = fixture();
        // The claim and pause exactly as a build without deadlines wrote them.
        let subject = format!("step-run/{}/ask-legacy", generation_id_from_subject(&origin.generation));
        let asked = store.connection.batched(|tx| -> Result<ClaimRecord> {
            let claim = append_claim_tx(tx, "alder", &subject, "work.person-asked", Some(&input.actor),
                &json!({"fields": {"run": origin.run, "generation": origin.generation, "origin_step": origin.subject,
                    "origin_attempt": origin.attempt, "person": input.person, "title": input.title, "reason": input.reason,
                    "key": input.idempotency_key, "attempt": 1, "status": "ready", "waiting_since": null, "legacy_request": null}}), &[], None)?;
            project(tx, &claim).unwrap();
            let pause = append_claim_tx(tx, "alder", &origin.subject, "step-run.state", Some("daemon/runtime"),
                &json!({"fields": {"status": "waiting-person", "reason": subject, "attempt": origin.attempt}}), std::slice::from_ref(&claim.id), None)?;
            project_mission_run_update(tx, &pause).unwrap();
            Ok(claim)
        }).unwrap().unwrap();
        let day = DEFAULT_ASK_DEADLINE_MS as u128;
        // The person's home shows a deadline only while deadlines count.
        let home = |store: &Store| store.attention_items(Some("person/avery")).unwrap().into_iter()
            .find(|item| item.subject == subject).unwrap();
        // Opening the store starts no count, and nothing counts while deadlines are off.
        assert_eq!(ask_deadlines_since(&store.readers.get()).unwrap(), 0);
        assert_eq!(home(&store).deadline_unix_ms, None);
        assert_eq!(home(&store).requested_at_unix_ms, asked.accepted_at_unix_ms);
        let off = store.reconcile_person_asks_at(asked.accepted_at_unix_ms + 30 * day).unwrap();
        assert_eq!(off, PersonAskPass::default());
        assert_eq!(ask_deadlines_since(&store.readers.get()).unwrap(), 0);
        store.set_ask_deadlines(true);
        let on = asked.accepted_at_unix_ms + 10 * 3_600_000;
        let pass = store.reconcile_person_asks_at(on).unwrap();
        assert_eq!(pass, PersonAskPass { changed: false, next_deadline: Some(on + day) });
        assert_eq!(ask_deadlines_since(&store.readers.get()).unwrap(), on);
        assert_eq!(home(&store).deadline_unix_ms, Some(on + day), "the home shows the deadline counted from the turn-on");
        assert_eq!(home(&store).default_answer, None);
        assert_eq!(home(&store).requested_at_unix_ms, asked.accepted_at_unix_ms, "the home still shows when it was asked");
        // A later pass does not move the count.
        let pass = store.reconcile_person_asks_at(asked.accepted_at_unix_ms + day).unwrap();
        assert_eq!(pass, PersonAskPass { changed: false, next_deadline: Some(on + day) });
        assert_eq!(ask_deadlines_since(&store.readers.get()).unwrap(), on);
        assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "waiting-person");
        // Off, the next pass forgets the count, so turning deadlines on again counts afresh:
        // an ask made or left open meanwhile does not resolve at once.
        store.set_ask_deadlines(false);
        assert_eq!(store.reconcile_person_asks_at(on + day).unwrap(), PersonAskPass::default());
        assert_eq!(ask_deadlines_since(&store.readers.get()).unwrap(), 0);
        assert_eq!(home(&store).deadline_unix_ms, None);
        store.set_ask_deadlines(true);
        let again = on + 2 * day;
        assert_eq!(store.reconcile_person_asks_at(again).unwrap(), PersonAskPass { changed: false, next_deadline: Some(again + day) });
        assert_eq!(ask_deadlines_since(&store.readers.get()).unwrap(), again);
        assert_eq!(home(&store).deadline_unix_ms, Some(again + day));
        assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().status, "waiting-person");
        assert!(store.reconcile_person_asks_at(again + day).unwrap().changed);
        let resumed = store.step_run(&origin.subject).unwrap().unwrap();
        assert_eq!(resumed.status, "ready");
        assert_eq!(resumed.person_answers[0].resolved_by.as_deref(), Some("deadline"));
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
                    target.append_as_older_build_for_test(&ClaimInput {
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


