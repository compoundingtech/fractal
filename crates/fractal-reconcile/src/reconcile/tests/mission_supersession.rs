//! Human gates exercise the cancellation barrier without a libtest process acting as an exec launcher.
use super::*;

#[test]
fn an_unadmitted_replacement_cannot_run_finalizers_before_its_source_is_terminal() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = r#"version 2
mission "supersession-finally" state="ready" {
  goal "Finish the original revision."
  concurrent-runs max=4
  step "work" {
    agentless
    gate "approve work" type="human" { reviewer "person/operator" }
  }
  finally {
    step "cleanup" {
      agentless
      gate "approve cleanup" type="human" { reviewer "person/operator" }
    }
  }
}"#;
    apply_source(&store, source, "supersession-finally-original");
    let old = store.create_mission_run(&MissionRunRequest {
        mission: "supersession-finally".into(), revision: None,
        workspace: root.path().display().to_string(),
        requester: Some("person/operator".into()), mode: None,
        inputs: BTreeMap::new(), idempotency_key: "supersession-finally-source".into(),
    }).unwrap();
    let runtime = Arc::new(NativeRuntime::new(root.path(), None, &std::env::current_exe().unwrap()));
    let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
    for _ in 0..3 { reconciler.reconcile_once().unwrap(); }

    apply_source(&store, &source.replace("original revision", "intermediate revision"),
        "supersession-finally-intermediate");
    let intermediate = store.mission_run(&old.id).unwrap().unwrap().superseded_by.unwrap();
    for _ in 0..3 { reconciler.reconcile_once().unwrap(); }
    assert_ne!(store.mission_run(&old.id).unwrap().unwrap().phase, "terminal");
    assert_eq!(store.mission_run(&intermediate).unwrap().unwrap().phase, "supersession-waiting");

    apply_source(&store, &source.replace("original revision", "newest revision"),
        "supersession-finally-newest");
    for _ in 0..3 {
        let run = store.mission_run(&intermediate).unwrap().unwrap();
        reconciler.evaluate_active_mission_run(&run, None).unwrap();
    }
    let run = store.mission_run(&intermediate).unwrap().unwrap();
    assert_eq!(run.phase, "final-cancelled");
    assert!(!store.mission_run_supersession_predecessors_terminal(&run).unwrap());
    assert_eq!(run.steps.iter().find(|step| step.step == "cleanup").unwrap().status, "pending");
    let newest = store.mission_run(run.superseded_by.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!(newest.phase, "supersession-waiting");
    assert!(newest.steps.iter().all(|step| step.status == "pending"));
}

fn observe_superseded_runtime(store: &Store, subject: &str, status: &str, incarnation: &str, reachability: &str) {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: "runtime.observed".into(), actor: None,
        fields: BTreeMap::from([
            ("status".into(), Value::from(status)),
            ("host".into(), Value::from("remote")),
            ("runtime_id".into(), Value::from("superseded-worker")),
            ("terminal".into(), Value::Bool(false)),
            ("incarnation_id".into(), Value::from(incarnation)),
            ("reachability".into(), Value::from(reachability)),
        ]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

#[test]
fn expired_supersession_cleanup_holds_effects_until_the_exact_runtime_ends() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = r#"version 2
mission "runtime-fence" state="ready" {
  goal "Original execution."
  concurrent-runs max=4
  step "work" {
    agentless
    gate "approve" type="human" { reviewer "person/operator" }
  }
}"#;
    apply_source(&store, source, "runtime-fence-original");
    let old = store.create_mission_run(&MissionRunRequest {
        mission: "runtime-fence".into(), revision: None,
        workspace: root.path().display().to_string(), requester: Some("person/operator".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: "runtime-fence-source".into(),
    }).unwrap();
    let subject = "agent/runtime-fence/remote";
    let mut stop = crate::graph::parse_internal_intent(
        &format!("version 2\nstop {subject:?}\n"), "node",
    ).unwrap();
    stop.subjects.get_mut(subject).unwrap().owner_run = Some(old.subject.clone());
    store.apply_internal(&stop, "runtime-fence-stop").unwrap();
    observe_superseded_runtime(&store, subject, "running", "current", "reachable");
    apply_source(&store, &source.replace("Original execution", "Replacement execution"),
        "runtime-fence-replacement");
    let replacement = store.mission_run(&old.id).unwrap().unwrap().superseded_by.unwrap();
    store.set_mission_run_state(&old.id, "running", "cleanup-cancelled", None).unwrap();
    // The evaluator's lightweight view must carry the same execution fences as
    // the presentation view, without needing a second run lookup.
    let cleanup = store.mission_run_for_reconcile(&old.id).unwrap();
    assert_eq!(cleanup.superseded_by.as_deref(), Some(replacement.as_str()));
    let waiting = store.mission_run(&replacement).unwrap().unwrap();
    assert_eq!(
        store.mission_run_for_reconcile(&waiting.id).unwrap().supersedes,
        Some(old.subject.clone()),
    );
    let runtime = Arc::new(NativeRuntime::new(root.path(), None, &std::env::current_exe().unwrap()));
    let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()))
        .with_cleanup_deadline(Duration::ZERO).tolerating_faults();
    for _ in 0..3 { reconciler.reconcile_once().unwrap(); }
    let held = || {
        let replacement = store.mission_run(&replacement).unwrap().unwrap();
        assert_eq!(replacement.phase, "supersession-waiting");
        assert!(replacement.steps.iter().all(|step| step.status == "pending"));
        assert!(store.open_reconcile_faults("node").unwrap()
            .contains_key(&(old.subject.clone(), "cleanup-runtime-quiescence".into())));
    };
    held();
    // A stale incarnation and an unreachable terminal report are neither proof
    // that the executing incarnation ended nor permission to begin effects.
    observe_superseded_runtime(&store, subject, "stopped", "earlier", "reachable");
    for _ in 0..2 { reconciler.reconcile_once().unwrap(); }
    held();
    observe_superseded_runtime(&store, subject, "unknown", "current", "unreachable");
    for _ in 0..2 { reconciler.reconcile_once().unwrap(); }
    held();
    observe_superseded_runtime(&store, subject, "stopped", "current", "unreachable");
    // A later reachable stop for an older incarnation must not launder the
    // unconfirmed terminal report for the execution that still owns effects.
    observe_superseded_runtime(&store, subject, "stopped", "earlier", "reachable");
    for _ in 0..2 { reconciler.reconcile_once().unwrap(); }
    held();
    observe_superseded_runtime(&store, subject, "stopped", "current", "reachable");
    for _ in 0..4 { reconciler.reconcile_once().unwrap(); }
    assert_eq!(store.mission_run(&old.id).unwrap().unwrap().phase, "terminal");
    assert_eq!(store.mission_run(&replacement).unwrap().unwrap().phase, "normal");
    assert!(!store.open_reconcile_faults("node").unwrap()
        .contains_key(&(old.subject, "cleanup-runtime-quiescence".into())));
}

#[test]
fn terminal_supersession_ancestors_and_their_children_all_fence_replacement_admission() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = r#"version 2
mission "ancestry-fence" state="ready" {
  goal "Original execution."
  concurrent-runs max=8
  step "work" {
    agentless
    gate "approve" type="human" { reviewer "person/operator" }
  }
}
mission "ancestry-child" state="ready" {
  goal "Child execution."
  step "work" {
    agentless
    gate "approve" type="human" { reviewer "person/operator" }
  }
}"#;
    apply_source(&store, source, "ancestry-original");
    let request = |mission: &str, key: &str| MissionRunRequest {
        mission: mission.into(), revision: None,
        workspace: root.path().display().to_string(), requester: Some("person/operator".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: key.into(),
    };
    let old = store.create_mission_run(&request("ancestry-fence", "ancestry-root")).unwrap();
    let child = store.create_child_mission_run(
        &request("ancestry-child", "ancestry-child"), &old, &old.steps[0].subject, None,
    ).unwrap();
    let own_stop = |owner: &str, subject: &str| {
        let mut stop = crate::graph::parse_internal_intent(
            &format!("version 2\nstop {subject:?}\n"), "node",
        ).unwrap();
        stop.subjects.get_mut(subject).unwrap().owner_run = Some(owner.into());
        store.apply_internal(&stop, &format!("ancestry-stop:{subject}")).unwrap();
        observe_superseded_runtime(&store, subject, "running", subject, "reachable");
    };
    own_stop(&child.subject, "agent/ancestry-child/remote");
    apply_source(&store, &source.replace("Original execution", "Intermediate execution"), "ancestry-intermediate");
    let intermediate = store.mission_run(&old.id).unwrap().unwrap().superseded_by.unwrap();
    own_stop(&intermediate, "agent/ancestry-intermediate/remote");
    apply_source(&store, &source.replace("Original execution", "Newest execution"), "ancestry-newest");
    let newest = store.mission_run(&intermediate).unwrap().unwrap().superseded_by.unwrap();
    for run in [&old.subject, &child.subject, &intermediate] {
        store.set_mission_run_state(run, "cancelled", "terminal", None).unwrap();
    }
    assert!(!store.resume_superseding_mission_run(&newest).unwrap());
    assert!(store.mission_run(&newest).unwrap().unwrap().steps.iter().all(|step| step.status == "pending"));
    let runtime = Arc::new(NativeRuntime::new(root.path(), None, &std::env::current_exe().unwrap()));
    let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()))
        .tolerating_faults();
    for _ in 0..2 { reconciler.reconcile_once().unwrap(); }
    assert!(store.open_reconcile_faults("node").unwrap()
        .contains_key(&(newest.clone(), "supersession-runtime-quiescence".into())));
    // Ending only the nearest predecessor is insufficient: the oldest
    // predecessor's terminal child still owns execution in this workspace.
    observe_superseded_runtime(&store, "agent/ancestry-intermediate/remote", "stopped",
        "agent/ancestry-intermediate/remote", "reachable");
    assert!(!store.resume_superseding_mission_run(&newest).unwrap());
    reconciler.reconcile_once().unwrap();
    assert!(store.open_reconcile_faults("node").unwrap()
        .contains_key(&(newest.clone(), "supersession-runtime-quiescence".into())));
    observe_superseded_runtime(&store, "agent/ancestry-child/remote", "stopped",
        "agent/ancestry-child/remote", "reachable");
    for _ in 0..3 { reconciler.reconcile_once().unwrap(); }
    assert_eq!(store.mission_run(&newest).unwrap().unwrap().phase, "normal");
    assert!(!store.open_reconcile_faults("node").unwrap()
        .contains_key(&(newest, "supersession-runtime-quiescence".into())));
}
