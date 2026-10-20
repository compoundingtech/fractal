


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
fn batched_mission_reads_preserve_alias_lifecycle_fences() {
    use st3_store::store::identities::{IdentityKind, IdentityStore as _, RenameRequest};
    for fence in ["head-conflict", "unknown", "invalid"] {
        let store = Store::open_memory("example").unwrap();
        let mission = publish_mission(
            &store,
            "version 2\nmission \"example-before\" state=\"ready\" { goal \"Keep continuity.\"; step \"work\" { agentless } }",
            "example-mission",
        );
        store.rename_identity(IdentityKind::Mission, &RenameRequest {
            from: "mission/example-before".into(), to: "mission/example-after".into(),
            actor: "person/robin".into(), reason: "Clarify responsibility".into(),
            expected_naming_revision: None, expected_declaration: None,
            idempotency_key: "example-rename".into(),
        }, &[]).unwrap();
        for id in ["example-before", "mission/example-before", "example-after", "mission/example-after"] {
            assert_eq!(store.mission_spec(id, Some(&mission.revision)).unwrap().unwrap(), mission);
            assert_eq!(store.mission_definitions_for_ids(&[id.into()]).unwrap().len(), 1);
            assert!(store.mission_run_headers_for_missions(&[id.into()]).unwrap().is_empty());
        }
        let connection = store.connection.write();
        if fence == "head-conflict" {
            // The naming fold exposes a losing partition reservation through this head.
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-hc707a8ad70dc owner=st3_missions::store::run_capacity_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE identity_heads SET unavailable='conflict' WHERE identity='mission/example-before'", [],
            ).unwrap();
        } else {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h207736d22c9c owner=st3_missions::store::run_capacity_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,subject_hint,kind_hint,updated_at_unix_ms)
                 VALUES('record/pending','example-peer',1,'pending',0,'{}',?1,'mission/example-before','mission.renamed','1')",
                [fence],
            ).unwrap();
        }
        drop(connection);
        let expected = if fence == "unknown" { "pending-replication" } else { "conflict" };
        for id in ["example-before", "mission/example-before", "example-after", "mission/example-after"] {
            for error in [
                store.mission_spec(id, None).unwrap_err(),
                store.mission_spec(id, Some(&mission.revision)).unwrap_err(),
                store.mission_definitions_for_ids(&[id.into()]).unwrap_err(),
                store.mission_run_headers_for_missions(&[id.into()]).unwrap_err(),
            ] {
                let error = error.downcast_ref::<St3Error>().unwrap();
                assert_eq!(error.code, "lifecycle-unavailable", "{id}: {fence}");
                assert_eq!(error.details["subject"], "mission/example-before");
                assert_eq!(error.details["reason"], expected);
            }
        }
    }
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
