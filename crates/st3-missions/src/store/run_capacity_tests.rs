


#[test]
fn cancellation_releases_admission_capacity_before_final_work_and_cleanup_finish() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "upstream" state="ready" {
  goal "Hold admission."
  step "work" { agentless }
}
"#,
        "capacity-upstream",
    );
    let mission = publish_mission(
        &store,
        r#"version 2
mission "capacity" state="ready" {
  concurrent-runs max=1
  goal "Count runs admitted for work."
  step "work" { agentless }
  finally { step "cleanup" { agentless } }
}
"#,
        "capacity-mission",
    );
    let request = |mission: &str, key: &str| MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/operator".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: key.into(),
    };
    let upstream = store
        .create_mission_run(&request("upstream", "capacity-predecessor"))
        .unwrap();
    let source = format!(
        "version 2\nmission-run \"queued-capacity\" {{\n mission \"mission/capacity@{}\"\n workspace \"/tmp\"\n requester \"person/operator\"\n after {:?}\n}}\n",
        mission.revision, upstream.subject
    );
    let intent = parse_intent(&source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "capacity-queued")
        .unwrap();
    assert_eq!(
        store
            .create_mission_run(&request("capacity", "capacity-refused-live"))
            .unwrap_err()
            .code,
        "mission-run-capacity"
    );
    store
        .request_mission_run_cancellation("queued-capacity", "replace the waiting run")
        .unwrap();
    let cancelled = store.mission_run("queued-capacity").unwrap().unwrap();
    assert_eq!(cancelled.status, "running");
    assert_eq!(cancelled.phase, "final-cancelled");
    assert!(
        cancelled
            .steps
            .iter()
            .any(|step| step.step == "cleanup" && step.status == "pending")
    );
    let replacement = store
        .create_mission_run(&request("capacity", "capacity-replacement"))
        .expect("cancelled waiting run must release capacity before its final work finishes");
    assert_eq!(
        store
            .create_mission_run(&request("capacity", "capacity-refused-replacement"))
            .unwrap_err()
            .code,
        "mission-run-capacity"
    );
    store
        .set_mission_run_state("queued-capacity", "running", "cleanup-cancelled", None)
        .unwrap();
    store
        .request_mission_run_cancellation(&replacement.id, "replace admitted work")
        .unwrap();
    let running_cancel = store.mission_run(&replacement.id).unwrap().unwrap();
    assert_eq!(running_cancel.phase, "final-cancelled");
    store
        .create_mission_run(&request("capacity", "capacity-second-replacement"))
        .expect("neither final-cancelled nor cleanup-cancelled consumes admission capacity");
    assert_eq!(
        store
            .create_mission_run(&request("capacity", "capacity-refused-second-replacement"))
            .unwrap_err()
            .code,
        "mission-run-capacity"
    );
}

#[test]
fn mission_rename_keeps_revision_generation_and_schedule_occurrence() {
    use st3_store::store::identities::{IdentityKind, IdentityStore as _, RenameRequest};
    let store = Store::open_memory("example").unwrap();
    let mission = publish_mission(
        &store,
        "version 2\nmission \"example-before\" state=\"ready\" {\n goal \"Keep continuity.\"\n step \"work\" { agentless }\n}\n",
        "example-mission",
    );
    let mut request = MissionRunRequest {
        mission: "mission/example-before".into(),
        revision: Some(mission.revision.clone()),
        workspace: "/tmp".into(),
        requester: Some("person/robin".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: "example-occurrence".into(),
    };
    let before = store.create_scheduled_mission_run(
        &request, None, "schedule/example", 7,
    ).unwrap();
    // Release the default single-run capacity before creating the independent retry fixture.
    store.set_mission_run_state(&before.id, "completed", "terminal", None).unwrap();
    let mut retry_request = request.clone();
    retry_request.idempotency_key = "example-retry".into();
    let original = store.create_mission_run(&retry_request).unwrap();
    store.rename_identity(IdentityKind::Mission, &RenameRequest {
        from: "mission/example-before".into(),
        to: "mission/example-after".into(),
        actor: "person/robin".into(),
        reason: "Clarify responsibility".into(),
        expected_naming_revision: None,
        expected_declaration: None,
        idempotency_key: "example-rename".into(),
    }, &[]).unwrap();
    request.mission = "mission/example-after".into();
    let after = store.create_scheduled_mission_run(
        &request, None, "schedule/example", 7,
    ).unwrap();
    assert_eq!(before.subject, after.subject);
    assert_eq!(before.generation, after.generation);
    assert_eq!(before.initial_revision, after.initial_revision);
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.steps[0].subject, after.steps[0].subject);
    retry_request.mission = request.mission.clone();
    let retried = store.create_mission_run(&retry_request).unwrap();
    assert_eq!(original.subject, retried.subject);
    assert_eq!(original.generation, retried.generation);
    assert_eq!(
        store.mission_spec("example-after", Some(&mission.revision)).unwrap().unwrap(),
        mission,
    );
    let definitions = store.mission_definitions().unwrap();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].mission.subject, "mission/example-after");
    assert_eq!(definitions[0].mission.revision, mission.revision);
    // The same current spelling also starts a new run against the original rows.
    store.set_mission_run_state(&original.id, "completed", "terminal", None).unwrap();
    request.idempotency_key = "example-next-run".into();
    let next = store.create_mission_run(&request).unwrap();
    assert_eq!(next.mission, "mission/example-before");
    assert_eq!(next.revision, mission.revision);
    let mut republished = parse_intent(
        "version 2\nmission \"example-after\" state=\"ready\" {\n goal \"Keep continuity.\"\n step \"work\" { agentless }\n}\n",
        store.origin(),
    ).unwrap();
    st3_store::store::missions::normalize_intent_missions(
        &store.readers.get(), &mut republished,
    ).unwrap();
    let normalized = republished.missions.values().next().unwrap();
    assert_eq!(normalized.id, mission.id);
    assert_eq!(normalized.subject, mission.subject);
    assert_eq!(normalized.revision, mission.revision);
}
