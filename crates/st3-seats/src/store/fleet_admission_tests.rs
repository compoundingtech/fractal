#![allow(unused_imports)]
use crate::HarnessQuestionsStore;
use super::*;
use crate::fleet::{MemberKey, MemberState, envelope_signature_message, verify_signature};
use st3_store::store::fleet_admission_tests_shared_seats::*;
pub use st3_store::store::fleet_admission_tests_shared_seats::{FLEET, admit, admitted, append, key, node, note, sync};


#[test]
fn older_registry_projects_missions_while_provenance_waits_for_upgrade() {
    let anchor_key = key();
    let older_key = key();
    let current = node("current", Some(&anchor_key), Some(&anchor_key));
    admit(&current, "current", &anchor_key, "anchor", None);
    admit(&current, "older", &older_key, "invite", None);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("older.sqlite3");
    let older = Store::open(&path, "older").unwrap();
    older.bind_fleet(FLEET).unwrap();
    older.pin_fleet_anchor(anchor_key.public()).unwrap();
    older.set_member_key(Some(older_key.clone())).unwrap();
    let mut registry = st3_schema::registry().clone();
    registry.claims.remove("mission.provenance").unwrap();
    older.set_claim_registry(registry);
    sync(&current, &older);
    let source = crate::provenance::tests::source("Build.", Some("Approved source."));
    let intent = crate::parse_intent(&source, "current").unwrap();
    current
        .apply_internal(&intent, "provenance-publication")
        .unwrap();
    let sidecar = current
        .latest_claim(
            &format!("mission/orchard@{}", intent.missions["orchard"].revision),
            Some("mission.provenance"),
        )
        .unwrap()
        .unwrap();
    assert!(checkpoint_rules::slot_of(&sidecar).is_none());
    let admission = sync(&current, &older);
    assert_eq!(admission.invalid, 0);
    assert_eq!(admission.unknown, 1);
    let mission = older.mission_spec("orchard", None).unwrap().unwrap();
    assert_eq!(mission, intent.missions["orchard"]);
    assert!(
        older
            .mission_provenance("orchard", &mission.revision)
            .unwrap()
            .is_none()
    );
    let run = older
        .create_mission_run(&MissionRunRequest {
            mission: "orchard".into(),
            revision: None,
            workspace: directory.path().display().to_string(),
            requester: Some("person/avery".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "old-build-run".into(),
        })
        .unwrap();
    assert_eq!(run.revision, mission.revision);
    assert!(!run.steps.is_empty());
    let before = older.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(before.invalid_records, 0);
    assert_eq!(before.unhealthy_projections, 0);
    drop(older);
    let upgraded = Store::open(&path, "older").unwrap();
    upgraded.set_member_key(Some(older_key)).unwrap();
    upgraded.validate_replication_backlog().unwrap();
    upgraded.project_replication_backlog().unwrap();
    assert_eq!(
        upgraded
            .mission_provenance("orchard", &mission.revision)
            .unwrap(),
        Some(intent.mission_provenance["orchard"].clone())
    );
    assert_eq!(
        upgraded
            .mission_run(&run.subject)
            .unwrap()
            .unwrap()
            .revision,
        run.revision
    );
}

#[test]
fn submitted_incarnation_waits_on_older_members_and_legacy_submissions_replay() {
    let anchor_key = key();
    let older_key = key();
    let current = node("current", Some(&anchor_key), Some(&anchor_key));
    admit(&current, "current", &anchor_key, "anchor", None);
    admit(&current, "older", &older_key, "invite", None);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("older.sqlite3");
    let older = Store::open(&path, "older").unwrap();
    older.bind_fleet(FLEET).unwrap();
    older.pin_fleet_anchor(anchor_key.public()).unwrap();
    older.set_member_key(Some(older_key.clone())).unwrap();
    let mut registry = st3_schema::registry().clone();
    registry
        .claims
        .get_mut("work.submitted")
        .unwrap()
        .fields
        .remove("submitted_incarnation")
        .unwrap();
    older.set_claim_registry(registry.clone());
    sync(&current, &older);
    let intent = crate::graph::parse_test_intent(
        r#"version 2
agent "garden/worker" { workspace "/tmp"; command "true" }
mission "garden" state="ready" {
  goal "Record and verify garden work submissions."
  step "new" { assigned-to "agent/garden/worker" }
  step "legacy" { assigned-to "agent/garden/worker" }
}
"#,
        "current",
    )
    .unwrap();
    current.apply_internal(&intent, "garden").unwrap();
    let run = current
        .create_mission_run(&MissionRunRequest {
            mission: "garden".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/ada".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "garden-run".into(),
        })
        .unwrap();
    let actor = "agent/garden/worker";
    let incarnation = "garden:one";
    let step = &run
        .steps
        .iter()
        .find(|step| step.step == "new")
        .unwrap()
        .subject;
    let legacy_step = &run
        .steps
        .iter()
        .find(|step| step.step == "legacy")
        .unwrap()
        .subject;
    let claim_step = |subject: &str| {
        current.set_step_state(subject, "ready", None).unwrap();
        current
            .work_action(
                subject,
                "claim",
                &WorkRequest {
                    wait_ms: None,
                    actor: Some(actor.into()),
                    incarnation: Some(incarnation.into()),
                    summary: None,
                    reason: None,
                    evidence: vec![],
                    idempotency_key: format!("claim-{subject}"),
                },
            )
            .unwrap();
    };
    claim_step(step);
    current
        .work_action(
            step,
            "complete",
            &WorkRequest {
                wait_ms: None,
                actor: Some(actor.into()),
                incarnation: Some(incarnation.into()),
                summary: None,
                reason: None,
                evidence: vec![],
                idempotency_key: "garden-submit".into(),
            },
        )
        .unwrap();
    let submitted = current
        .latest_claim(step, Some("work.submitted"))
        .unwrap()
        .unwrap();
    claim_step(legacy_step);
    let legacy = current
        .append_claim(&ClaimInput {
            subject: legacy_step.clone(),
            kind: "work.submitted".into(),
            actor: Some(actor.into()),
            fields: BTreeMap::from([
                ("attempt".into(), json!(1)),
                ("status".into(), json!("verifying")),
                ("worker_reported".into(), json!(true)),
                ("claimant".into(), Value::Null),
                ("claim_incarnation".into(), Value::Null),
                ("claim_expires_at_unix_ms".into(), Value::Null),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some("legacy-submit".into()),
        })
        .unwrap();
    assert!(!crate::model::work_submission_matches(
        &legacy,
        legacy_step,
        1,
        actor,
        Some(incarnation),
        &[]
    ));
    assert!(!crate::model::work_submission_matches(
        &legacy,
        legacy_step,
        1,
        actor,
        None,
        &[]
    ));
    current.replay_replication_graph().unwrap();
    let legacy_view = current.step_run(legacy_step).unwrap().unwrap();
    assert_eq!(legacy_view.status, "verifying");
    assert!(legacy_view.submitted_incarnation.is_none());
    let fields = schema_fields_for_body(&submitted.kind, &submitted.body).unwrap();
    assert_eq!(
        registry
            .validate_claim(step, "work.submitted", &fields)
            .unwrap_err()
            .code,
        "unknown-claim-field"
    );
    assert!(matches!(
        classify_replicated_claim_with_registry(&submitted, &registry).unwrap(),
        ReplicatedClaimAdmission::UnknownField
    ));
    let later = note(&current, "A known claim after the submission");
    let admission = sync(&current, &older);
    assert_eq!(admission.unknown, 1);
    assert_eq!(admission.invalid, 0);
    assert!(!admitted(&older, &submitted));
    assert!(admitted(&older, &legacy));
    assert!(admitted(&older, &later));
    assert!(
        older
            .step_run(step)
            .unwrap()
            .unwrap()
            .submitted_incarnation
            .is_none()
    );
    assert_eq!(
        older.step_run(legacy_step).unwrap().unwrap().status,
        "verifying"
    );
    let local = note(&older, "The older member can still write known claims");
    sync(&older, &current);
    assert!(admitted(&current, &local));
    let before = older.replication_status(true, Some(FLEET), &[]).unwrap();
    let source = current.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(before.waiting_claims, 1);
    assert_eq!(before.invalid_records, 0);
    assert_eq!(before.unhealthy_projections, 0);
    assert_eq!(before.authority_digest, source.authority_digest);
    let waiting = older.replica_records(true).unwrap();
    assert_eq!(
        waiting[0].error_code.as_deref(),
        Some("unknown-claim-field")
    );
    drop(older);
    let upgraded = Store::open(&path, "older").unwrap();
    upgraded.set_member_key(Some(older_key)).unwrap();
    upgraded.validate_replication_backlog().unwrap();
    upgraded.apply_replication_repairs().unwrap();
    upgraded.project_replication_backlog().unwrap();
    let after = upgraded.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(after.waiting_claims, 0);
    assert_eq!(after.authority_digest, before.authority_digest);
    assert_eq!(after.graph_digest, source.graph_digest);
    assert_eq!(after.projection_digests, source.projection_digests);
    let learned = upgraded.claim_by_id(&submitted.id).unwrap().unwrap();
    assert_eq!(learned.body, submitted.body);
    let view = upgraded.step_run(step).unwrap().unwrap();
    assert_eq!(view.submitted_incarnation.as_deref(), Some(incarnation));
    assert!(view.claimant.is_none());
    assert!(view.claim_incarnation.is_none());
    assert!(
        upgraded
            .step_run(legacy_step)
            .unwrap()
            .unwrap()
            .submitted_incarnation
            .is_none()
    );
}

#[test]
fn a_step_wait_waits_on_older_members_and_converges_after_they_upgrade() {
    let anchor_key = key();
    let older_key = key();
    let current = node("current", Some(&anchor_key), Some(&anchor_key));
    admit(&current, "current", &anchor_key, "anchor", None);
    admit(&current, "older", &older_key, "invite", None);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("older.sqlite3");
    let older = Store::open(&path, "older").unwrap();
    older.bind_fleet(FLEET).unwrap();
    older.pin_fleet_anchor(anchor_key.public()).unwrap();
    older.set_member_key(Some(older_key.clone())).unwrap();
    // A build from before the wait: work.progress has neither new field.
    let mut registry = st3_schema::registry().clone();
    for field in ["clock", "wait_until_unix_ms"] {
        registry
            .claims
            .get_mut("work.progress")
            .unwrap()
            .fields
            .remove(field)
            .unwrap();
    }
    older.set_claim_registry(registry.clone());
    sync(&current, &older);
    let intent = crate::graph::parse_test_intent(
        r#"version 2
agent "garden/worker" { workspace "/tmp"; command "true" }
mission "garden" state="ready" {
  goal "Wait for a build slot without spending the execution clock."
  step "review" timeout="30m" { assigned-to "agent/garden/worker" }
}
"#,
        "current",
    )
    .unwrap();
    current.apply_internal(&intent, "garden").unwrap();
    let run = current
        .create_mission_run(&MissionRunRequest {
            mission: "garden".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/ada".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "garden-run".into(),
        })
        .unwrap();
    let step = run.steps[0].subject.clone();
    let request = |key: &str, wait_ms: Option<u64>, reason: Option<&str>| WorkRequest {
        wait_ms,
        actor: Some("agent/garden/worker".into()),
        incarnation: Some("garden:one".into()),
        summary: None,
        reason: reason.map(Into::into),
        evidence: vec![],
        idempotency_key: key.into(),
    };
    current.set_step_state(&step, "ready", None).unwrap();
    current
        .work_action(&step, "claim", &request("claim", None, None))
        .unwrap();
    current
        .work_action(
            &step,
            "wait",
            &request("wait", Some(60 * 60 * 1000), Some("build slot")),
        )
        .unwrap();
    let waiting = current
        .latest_claim(&step, Some("work.progress"))
        .unwrap()
        .unwrap();
    assert_eq!(waiting.body["fields"]["clock"], "paused");
    let fields = schema_fields_for_body(&waiting.kind, &waiting.body).unwrap();
    assert_eq!(
        registry
            .validate_claim(&step, "work.progress", &fields)
            .unwrap_err()
            .code,
        "unknown-claim-field"
    );
    assert!(matches!(
        classify_replicated_claim_with_registry(&waiting, &registry).unwrap(),
        ReplicatedClaimAdmission::UnknownField
    ));
    assert!(
        current
            .step_run(&step)
            .unwrap()
            .unwrap()
            .execution_paused_until_unix_ms
            .is_some()
    );

    // The older member keeps every claim it knows, holds the wait as an unknown record, and
    // never applies it, so it does not stop the clock until it is upgraded.
    let later = note(&current, "A known claim after the wait");
    let admission = sync(&current, &older);
    assert_eq!(admission.unknown, 1);
    assert_eq!(admission.invalid, 0);
    assert!(!admitted(&older, &waiting));
    assert!(admitted(&older, &later));
    let held = older.step_run(&step).unwrap().unwrap();
    assert_eq!(held.execution_paused_until_unix_ms, None);
    assert_eq!(held.execution_waited_ms, 0);
    let local = note(&older, "The older member can still write known claims");
    sync(&older, &current);
    assert!(admitted(&current, &local));
    let before = older.replication_status(true, Some(FLEET), &[]).unwrap();
    let source = current.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(before.waiting_claims, 1);
    assert_eq!(before.invalid_records, 0);
    assert_eq!(before.unhealthy_projections, 0);
    assert_eq!(before.authority_digest, source.authority_digest);
    assert_eq!(
        older.replica_records(true).unwrap()[0].error_code.as_deref(),
        Some("unknown-claim-field")
    );
    drop(older);

    // Upgrading retries the held claim with the new registry and converges on the same graph.
    let upgraded = Store::open(&path, "older").unwrap();
    upgraded.set_member_key(Some(older_key)).unwrap();
    upgraded.validate_replication_backlog().unwrap();
    upgraded.apply_replication_repairs().unwrap();
    upgraded.project_replication_backlog().unwrap();
    let after = upgraded.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(after.waiting_claims, 0);
    assert_eq!(after.authority_digest, before.authority_digest);
    assert_eq!(after.graph_digest, source.graph_digest);
    assert_eq!(after.projection_digests, source.projection_digests);
    let learned = upgraded.claim_by_id(&waiting.id).unwrap().unwrap();
    assert_eq!(learned.body, waiting.body);
    let view = upgraded.step_run(&step).unwrap().unwrap();
    assert!(view.execution_paused_until_unix_ms.is_some());
    assert_eq!(view.blocked_reason.as_deref(), Some("build slot"));
}

#[test]
fn harness_questions_wait_on_older_members_without_a_heal_and_converge_after_upgrade() {
    use st3_schema::harness_questions::{OBSERVED, Question, RESOLVED, Status};
    let anchor_key = key();
    let older_key = key();
    let current = node("current", Some(&anchor_key), Some(&anchor_key));
    admit(&current, "current", &anchor_key, "anchor", None);
    admit(&current, "older", &older_key, "invite", None);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("older.sqlite3");
    let older = Store::open(&path, "older").unwrap();
    older.bind_fleet(FLEET).unwrap();
    older.pin_fleet_anchor(anchor_key.public()).unwrap();
    older.set_member_key(Some(older_key.clone())).unwrap();
    // A build from before harness questions: neither kind is registered.
    let mut registry = st3_schema::registry().clone();
    for kind in [OBSERVED, RESOLVED] {
        registry.claims.remove(kind).unwrap();
    }
    older.set_claim_registry(registry);
    sync(&current, &older);

    let question = current
        .observe_harness_question(&harness_questions::QuestionObservation {
            agent: "agent/ada".into(),
            session: "tester".into(),
            native_id: "call-1".into(),
            runtime_incarnation: "inc-1".into(),
            questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }],
        })
        .unwrap();
    let (resolved, _) = current
        .resolve_harness_question(&question.id, Status::Cancelled, &question.revision, &BTreeMap::new(), None)
        .unwrap();

    // The older member holds both claims as unknown records and keeps every claim it knows.
    let later = note(&current, "A known claim after the question");
    let admission = sync(&current, &older);
    assert_eq!(admission.unknown, 2);
    assert_eq!(admission.invalid, 0);
    assert!(admitted(&older, &later));
    assert!(older.harness_question(&question.id, 0).unwrap().is_none());
    let local = note(&older, "The older member can still write known claims");
    sync(&older, &current);
    assert!(admitted(&current, &local));
    // Equal wire summaries across different registries preserve the older member's waiting
    // claims without invalid records.
    for (from, to) in [(&current, &older), (&older, &current)] {
        let exchange = from.export_replication_summary(FLEET).unwrap();
        to.receive_replication_exchange_asking(&from.origin, FLEET, &exchange, true)
            .unwrap();
    }
    let before = older.replication_status(true, Some(FLEET), &[]).unwrap();
    let source = current.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(before.waiting_claims, 2);
    assert_eq!(before.invalid_records, 0);
    assert_eq!(before.unhealthy_projections, 0);
    assert_eq!(before.authority_digest, source.authority_digest);
    drop(older);

    // Upgrading retries the held claims with the new registry and converges on the same graph.
    let upgraded = Store::open(&path, "older").unwrap();
    upgraded.set_member_key(Some(older_key)).unwrap();
    upgraded.validate_replication_backlog().unwrap();
    upgraded.apply_replication_repairs().unwrap();
    upgraded.project_replication_backlog().unwrap();
    let after = upgraded.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(after.waiting_claims, 0);
    assert_eq!(after.graph_digest, source.graph_digest);
    assert_eq!(after.projection_digests, source.projection_digests);
    let at = resolved.resolution.as_ref().unwrap().resolved_at_unix_ms;
    assert_eq!(upgraded.harness_question(&question.id, at).unwrap(), Some(resolved));
}
