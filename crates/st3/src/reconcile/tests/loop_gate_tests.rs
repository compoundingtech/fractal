use super::*;

const SOURCE: &str = r#"version 2
resource "result" { kind "custom.test.result" }
resource "review-ready" { kind "custom.test.review-ready" }
mission "dotfiles/ui-design/machine-filter" state="ready" {
  goal "Build, then review the result."
  loop "refine" {
    max-rounds 2
    until { gate "done" { field "state" "resource/result" is "done" } }
    round {
      completion { when "all-steps-exhausted" }
      agent "builder" { workspace "."; command "true"; restart "never" }
      agent "gate" { workspace "."; command "true"; restart "never" }
      step "build" { assigned-to "builder" }
      step "review" {
        depends-on { step "build" completed; field "state" "resource/review-ready" is "ready" }
        assigned-to "gate"
      }
    }
  }
}"#;

#[test]
fn loop_gate_starts_only_after_builder_work_is_ready_for_review() {
    // Pins eager round materialization introduced by 0f108e59644 on the direct KDL lifecycle.
    for incremental in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        apply_source(&store, SOURCE, "gate-demand-source");
        let parent = store.create_mission_run(&MissionRunRequest {
            mission: "dotfiles/ui-design/machine-filter".into(),
            revision: None,
            workspace: root.path().to_string_lossy().into_owned(),
            requester: Some("person/test".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "gate-demand-parent".into(),
        }).unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()))
            .skipping_unneeded(incremental);
        for _ in 0..8 { reconciler.reconcile_once().unwrap(); }
        let child = store.mission_run_for_parent_step(&parent.steps[0].subject).unwrap().unwrap();
        let gate = store.desired_subjects_for_owner_run(&child.subject).unwrap().into_iter()
            .find(|subject| subject.subject.ends_with("/gate")).unwrap();
        let member = gate.member.as_ref().unwrap();
        assert!(store.is_loop_gate(&gate).unwrap());
        assert!(store.loop_gate_ready_at(&gate).unwrap().is_none());
        assert!(!runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).contains(&member.runtime_id), "gate pre-started with round");
        assert!(store.runtime_start_for(&gate.subject).unwrap().is_none());
        let build = child.steps.iter().find(|step| step.step.ends_with("build")).unwrap();
        store.set_step_state(&build.subject, "completed", None).unwrap();
        for _ in 0..8 { reconciler.reconcile_once().unwrap(); }
        assert!(store.loop_gate_ready_at(&gate).unwrap().is_none());
        assert!(!runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).contains(&member.runtime_id),
            "dependency completion alone must not bypass the readiness predicate");
        store.append_claim(&ClaimInput {
            subject: "resource/review-ready".into(), kind: "resource.observed".into(), actor: None,
            fields: BTreeMap::from([("state".into(), serde_json::json!("ready"))]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: Some("allow-review".into()),
        }).unwrap();
        for _ in 0..8 { reconciler.reconcile_once().unwrap(); }
        assert!(store.loop_gate_ready_at(&gate).unwrap().is_some());
        assert_eq!(runtime.starts.lock().unwrap_or_else(PoisonError::into_inner).iter().filter(|id| *id == &member.runtime_id).count(), 1);
        assert_eq!(reconciler.idle_loop_gate_count(&[gate.clone()]).unwrap(), 0);
        let started = runtime.started_members.lock().unwrap_or_else(PoisonError::into_inner).iter()
            .find(|started| started.runtime_id == member.runtime_id).unwrap().clone();
        let incarnation = "gate-idle";
        runtime.execs.lock().unwrap_or_else(PoisonError::into_inner).insert(member.runtime_id.clone(), RuntimeObservation {
            runtime_id: member.runtime_id.clone(), terminal: false, status: "running".into(),
            exit_code: None, incarnation_id: Some(incarnation.into()),
        });
        store.append_claim(&ClaimInput {
            subject: gate.subject.clone(), kind: "runtime.observed".into(), actor: None,
            fields: member_fields(&started, "running", Some(incarnation), true),
            evidence: Vec::new(), expected_subject: None, idempotency_key: Some("gate-idle-runtime".into()),
        }).unwrap();
        store.append_claim(&ClaimInput {
            subject: gate.subject.clone(), kind: "harness.observed".into(), actor: Some(gate.subject.clone()),
            fields: BTreeMap::from([
                ("state".into(), serde_json::json!("idle")),
                ("driver".into(), serde_json::json!("omp")),
                ("incarnation_id".into(), serde_json::json!(incarnation)),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: Some("gate-idle-harness".into()),
        }).unwrap();
        assert_eq!(reconciler.idle_loop_gate_count(&[gate]).unwrap(), 1);
    }
}
