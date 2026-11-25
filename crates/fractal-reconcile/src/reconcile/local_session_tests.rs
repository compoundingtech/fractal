//! Real registry/process boundaries with the graph tables deliberately unavailable.
use super::*;
use crate::store::local_sessions::{LocalSession, SessionCustody};

fn without_graph(store: &Store) {
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h506fc9135132 owner=fractal_reconcile::reconcile::local_session_tests TODO=prepare-body-and-predecessor-values */ "ALTER TABLE claims RENAME TO unavailable_claims;
         ALTER TABLE desired RENAME TO unavailable_desired;"
    ).unwrap();
}

#[test]
fn owned_exec_resolves_after_restart_without_graph_and_stop_is_generation_fenced() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("custody.sqlite");
    let runtime = NativeRuntime::new(root.path(), None, Path::new("unused-pty"));
    let generation = runtime.exec.spawn("local-owned", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
        root.path(), &std::env::vars().collect()).unwrap();
    struct Cleanup<'a>(&'a NativeRuntime, String);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) { let _ = self.0.exec.kill_if("local-owned", Some(&self.1)); }
    }
    let _cleanup = Cleanup(&runtime, generation.generation_id.clone());
    {
        let store = Store::open(&database, "executor").unwrap();
        store.bind_local_session(&LocalSession {
            subject: "agent/example/local".into(), runtime_id: "local-owned".into(),
            incarnation: generation.generation_id.clone(), native_incarnation: generation.generation_id.clone(),
            terminal: false, driver: None, lifecycle: MemberLifecycle::Service, custody: SessionCustody::Owned,
            start_operation: None, invocation: None,
        }).unwrap();
    }
    let store = Store::open(&database, "executor").unwrap();
    without_graph(&store);
    let live = local_session(&store, &runtime, "agent/example/local", Some(&generation.generation_id)).unwrap();
    assert_eq!(live.native_incarnation, generation.generation_id);
    assert!(runtime.exec.read_log_bytes("local-owned", false).unwrap().is_some());
    assert!(stop_local_session(&store, &runtime, &live.subject, "reused-generation", "wrong-stop").is_err());
    assert!(matches!(runtime.exec.observe("local-owned").unwrap(), Some(fractal_runtime::ExecObservation::Running(_))));
    stop_local_session(&store, &runtime, &live.subject, &live.incarnation, "owned-stop").unwrap();
    runtime.exec.wait_for_exit("local-owned", &live.native_incarnation).unwrap();
    assert!(local_session(&store, &runtime, &live.subject, Some(&live.incarnation)).is_err());
    let replacement = runtime.exec.spawn("local-owned", &fractal_runtime::Launch::Shell("exec sleep 600".into()),
        root.path(), &std::env::vars().collect()).unwrap();
    let _replacement_cleanup = Cleanup(&runtime, replacement.generation_id.clone());
    assert_ne!(replacement.generation_id, live.native_incarnation);
    assert!(local_session(&store, &runtime, &live.subject, Some(&live.incarnation)).is_err(),
        "a reused runtime ID must not inherit the retired native generation");
}

#[tokio::test]
async fn a_managed_stopped_projection_from_absence_does_not_release_native_custody() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("executor").unwrap());
    let subject = "agent/example/missing-native";
    let incarnation = "unproven-native-generation";
    store.bind_local_session(&LocalSession {
        subject: subject.into(), runtime_id: "missing-native".into(),
        incarnation: incarnation.into(), native_incarnation: incarnation.into(),
        terminal: false, driver: None, lifecycle: MemberLifecycle::Service,
        custody: SessionCustody::Owned, start_operation: None, invocation: None,
    }).unwrap();
    let runtime = Arc::new(NativeRuntime::new(root.path(), None, Path::new("unused-pty")));
    let reconciler = Reconciler::new(store.clone(), runtime, "executor".into(), Arc::new(Notify::new()));
    assert!(reconciler.reconcile_runtime_stop(subject, "missing-native", false, Some(incarnation), 0, None).unwrap());
    assert!(store.has_current_local_session(subject).unwrap(),
        "the stopped projection is not an exact native closure witness");
    assert!(store.local_session_at(subject, incarnation).unwrap().is_some());
}

#[test]
fn local_session_probe_releases_only_exact_natural_pty_exit() {
    let store = Store::open_memory("executor").unwrap();
    let runtime = super::tests::FakeRuntime::default();
    let subject = "agent/example/probe-exit";
    let incarnation = "probe-generation";
    store.bind_local_session(&LocalSession {
        subject: subject.into(), runtime_id: "probe-exit".into(),
        incarnation: incarnation.into(), native_incarnation: incarnation.into(),
        terminal: true, driver: None, lifecycle: MemberLifecycle::Service,
        custody: SessionCustody::Owned, start_operation: None, invocation: None,
    }).unwrap();
    assert!(local_session(&store, &runtime, subject, None).is_err());
    assert!(store.has_current_local_session(subject).unwrap());
    let mut observation = RuntimeObservation {
        runtime_id: "probe-exit".into(), terminal: true, status: "vanished".into(),
        incarnation_id: Some(incarnation.into()), exit_code: None,
    };
    for status in ["vanished", "absent", "stopped"] {
        observation.status = status.into();
        *runtime.ptys.lock().unwrap() = vec![observation.clone()];
        assert!(local_session(&store, &runtime, subject, None).is_err());
        assert!(store.has_current_local_session(subject).unwrap());
    }
    observation.status = "exited".into();
    observation.incarnation_id = Some("older-generation".into());
    *runtime.ptys.lock().unwrap() = vec![observation.clone()];
    assert!(local_session(&store, &runtime, subject, None).is_err());
    assert!(store.has_current_local_session(subject).unwrap());
    observation.incarnation_id = Some(incarnation.into());
    observation.exit_code = Some(17);
    *runtime.ptys.lock().unwrap() = vec![observation];
    assert!(local_session(&store, &runtime, subject, Some("wrong-expected-generation")).is_err());
    assert!(store.has_current_local_session(subject).unwrap());
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h790ae64f7fad owner=fractal_reconcile::reconcile::local_session_tests TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE local_observations RENAME TO unavailable_local_observations;"
    ).unwrap();
    assert!(local_session(&store, &runtime, subject, None).is_err());
    assert!(store.has_current_local_session(subject).unwrap(),
        "an exit cannot release custody when its observation fails to persist");
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-hf682ac2a591a owner=fractal_reconcile::reconcile::local_session_tests TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE unavailable_local_observations RENAME TO local_observations;"
    ).unwrap();
    assert!(local_session(&store, &runtime, subject, None).is_err());
    assert!(!store.has_current_local_session(subject).unwrap());
    let receipt = store.latest_observation(subject, "runtime.observed").unwrap().unwrap();
    assert_eq!(receipt.body["fields"]["status"], "exited");
    assert_eq!(receipt.body["fields"]["incarnation_id"], incarnation);
    assert_eq!(receipt.body["fields"]["exit_code"], 17);
}

#[tokio::test]
async fn natural_pty_exit_settles_only_the_observed_incarnation_after_durable_observation() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    super::tests::apply_source(&store,
        "version 2\nagent \"example/natural-exit\" { workspace \"/tmp\"; command \"true\"; }",
        "natural-exit");
    let desired = store.desired_subjects().unwrap().into_iter()
        .find(|subject| subject.subject == "agent/example/natural-exit").unwrap();
    let member = desired.member.as_ref().unwrap();
    assert!(member.terminal);
    let incarnation = "natural-pty-generation";
    store.bind_local_session(&LocalSession {
        subject: desired.subject.clone(), runtime_id: member.runtime_id.clone(),
        incarnation: incarnation.into(), native_incarnation: incarnation.into(),
        terminal: true, driver: None, lifecycle: member.lifecycle.clone(),
        custody: SessionCustody::Owned, start_operation: None, invocation: None,
    }).unwrap();
    let reconciler = Reconciler::new(store.clone(), Arc::new(super::tests::FakeRuntime::default()),
        "node".into(), Arc::new(Notify::new()));
    let mut observation = RuntimeObservation {
        runtime_id: member.runtime_id.clone(), terminal: true, status: "vanished".into(),
        incarnation_id: Some(incarnation.into()), exit_code: None,
    };
    for status in ["vanished", "absent", "stopped"] {
        observation.status = status.into();
        reconciler.record_member(&desired, &observation, false).unwrap();
        assert!(store.has_current_local_session(&desired.subject).unwrap(),
            "{status} is not a positive PTY exit");
    }
    observation.status = "exited".into();
    observation.incarnation_id = Some("older-generation".into());
    reconciler.record_member(&desired, &observation, false).unwrap();
    assert!(store.has_current_local_session(&desired.subject).unwrap());
    observation.incarnation_id = Some(incarnation.into());
    observation.exit_code = Some(0);
    reconciler.record_member(&desired, &observation, false).unwrap();
    assert!(!store.has_current_local_session(&desired.subject).unwrap());
    let durable = store.latest_observation(&desired.subject, "runtime.observed").unwrap().unwrap();
    assert_eq!(durable.body["fields"]["status"], "exited");
    assert_eq!(durable.body["fields"]["incarnation_id"], incarnation);
    assert!(store.local_session_at(&desired.subject, incarnation).unwrap().is_some());
}

#[tokio::test]
async fn runtime_start_retirement_without_source_host_recovers_exact_local_custody() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("retirement.sqlite");
    let store = Arc::new(Store::open(&database, "node").unwrap());
    super::tests::apply_source(&store,
        "version 2\nagent \"example/start-retirement\" { workspace \"/tmp\"; command \"true\"; }",
        "start-retirement");
    let desired = store.desired_subjects().unwrap().into_iter()
        .find(|subject| subject.subject == "agent/example/start-retirement").unwrap();
    let member = desired.member.as_ref().unwrap();
    let token = store.selected_desired_token(&desired.subject).unwrap().unwrap();
    let mut operation = RuntimeStartOperation {
        id: uuid::Uuid::now_v7().to_string(), subject: desired.subject.clone(),
        owner: "fixture".into(),
        body: serde_json::json!({
            "subject": desired, "selected_token": token, "request": null,
            "member": member, "reason": "retirement recovery fixture",
        }),
        revoked_by: None, finished: false, result: None, applied: false,
        cleanup_verified: false, dispatched: false, receipted: false,
    };
    store.admit_runtime_start(&mut operation, 2).unwrap();
    assert!(store.dispatch_runtime_start(&operation.id, 2).unwrap());
    let incarnation = "retired-start-generation";
    store.finish_runtime_start(&operation.id,
        &serde_json::json!({"incarnation": incarnation})).unwrap();
    store.revoke_runtime_start(&operation.id, "fixture-stop").unwrap();
    let request = store.append_claim(&ClaimInput {
        subject: desired.subject.clone(), kind: "runtime.action.requested".into(), actor: None,
        fields: BTreeMap::from([
            ("action".into(), Value::from("terminate")),
            ("operation".into(), Value::from(operation.id.clone())),
            ("runtime_id".into(), Value::from(member.runtime_id.clone())),
            ("terminal".into(), Value::from(true)),
            ("incarnation_id".into(), Value::from(incarnation)),
        ]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(store.apply_runtime_start_retirement(&operation.id, incarnation, &request.id).unwrap());
    let receipt = store.observations_for(&desired.subject, "runtime.action.succeeded").unwrap()
        .into_iter().find(|claim| claim.body["fields"]["operation_status"] == "retired").unwrap();
    assert!(receipt.body["fields"].get("source_host").is_none());
    assert_eq!(receipt.origin, "node");
    assert_eq!(receipt.body["evidence"][0], request.id);
    let mut binding = LocalSession {
        subject: desired.subject.clone(), runtime_id: member.runtime_id.clone(),
        incarnation: incarnation.into(), native_incarnation: incarnation.into(),
        terminal: true, driver: None, lifecycle: member.lifecycle.clone(),
        custody: SessionCustody::Owned, start_operation: Some("another-operation".into()), invocation: None,
    };
    store.bind_local_session(&binding).unwrap();
    let observation = RuntimeObservation {
        runtime_id: member.runtime_id.clone(), terminal: true, status: "stopped".into(),
        incarnation_id: Some(incarnation.into()), exit_code: None,
    };
    let runtime = Arc::new(super::tests::FakeRuntime::default());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
    assert!(reconciler.reconcile_runtime_stop(&desired.subject, &member.runtime_id, true,
        Some(incarnation), 0, Some(&observation)).unwrap());
    assert!(store.has_current_local_session(&desired.subject).unwrap(),
        "a receipt from another start operation cannot close custody");
    binding.start_operation = Some(operation.id);
    store.bind_local_session(&binding).unwrap();
    drop(reconciler);
    drop(store);
    let store = Arc::new(Store::open(&database, "node").unwrap());
    let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
    for _ in 0..2 {
        assert!(reconciler.reconcile_runtime_stop(&desired.subject, &member.runtime_id, true,
            Some(incarnation), 0, Some(&observation)).unwrap());
        assert!(!store.has_current_local_session(&desired.subject).unwrap());
    }
    assert_eq!(store.observations_for(&desired.subject, "runtime.action.succeeded").unwrap().len(), 1);
}

#[tokio::test]
async fn ended_native_observations_recover_only_receipted_custody_without_stopping_again() {
    let environment = std::env::vars().collect::<BTreeMap<_, _>>();
    let Ok(binary) = fractal_runtime::resolve_executable("pty", &environment) else {
        assert_ne!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"), "hosted custody fixtures require pty");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("custody.sqlite");
    let runtime = Arc::new(NativeRuntime::new(root.path(), None, &binary));
    let runtime_id = "receipt-recovery";
    runtime.pty().unwrap().spawn(runtime_id, &fractal_runtime::Launch::Shell("sleep 600 & wait".into()),
        root.path(), &environment, None, &BTreeMap::new(), None).unwrap();
    struct Cleanup<'a>(&'a NativeRuntime);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) { let _ = self.0.pty().and_then(|pty| pty.stop("receipt-recovery")); }
    }
    let _cleanup = Cleanup(&runtime);
    let mut ended = runtime.snapshot_ptys().unwrap().into_iter()
        .find(|o| o.runtime_id == runtime_id).unwrap();
    let incarnation = ended.incarnation_id.clone().unwrap();
    let subject = "agent/example/receipt-recovery";
    let binding = LocalSession {
        subject: subject.into(), runtime_id: runtime_id.into(),
        incarnation: incarnation.clone(), native_incarnation: incarnation.clone(),
        terminal: true, driver: None, lifecycle: MemberLifecycle::Service,
        custody: SessionCustody::Owned, start_operation: None, invocation: None,
    };
    let store = Arc::new(Store::open(&database, "executor").unwrap());
    store.bind_local_session(&binding).unwrap();
    assert!(runtime.stop_with_retirement_proof(runtime_id, true, &incarnation).unwrap());
    ended.status = "stopped".into();
    // The runtime ID is now reused. Replaying its older, genuinely stopped generation
    // must be observational: another generation-fenced stop would reject this live replacement.
    runtime.pty().unwrap().spawn(runtime_id, &fractal_runtime::Launch::Shell("sleep 600 & wait".into()),
        root.path(), &environment, None, &BTreeMap::new(), None).unwrap();
    let replacement = runtime.snapshot_ptys().unwrap().into_iter()
        .find(|o| o.runtime_id == runtime_id).unwrap().incarnation_id.unwrap();
    assert_ne!(replacement, incarnation);
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "executor".into(), Arc::new(Notify::new()));
    for _ in 0..3 {
        assert!(reconciler.reconcile_runtime_stop(subject, runtime_id, true, Some(&incarnation), 0, Some(&ended)).unwrap());
        assert!(store.has_current_local_session(subject).unwrap(), "an ended projection alone is not a retirement receipt");
    }
    // Simulate a crash after committing the real native retirement receipt but before
    // settling custody. A receipt for another generation cannot release this binding.
    let receipt = |generation: &str| ClaimInput {
        subject: subject.into(), kind: "runtime.action.succeeded".into(), actor: None,
        fields: BTreeMap::from([
            ("action".into(), Value::from("terminate")),
            ("runtime_id".into(), Value::from(runtime_id)),
            ("terminal".into(), Value::from(true)),
            ("incarnation_id".into(), Value::from(generation)),
            ("source_host".into(), Value::from("executor")),
            ("operation_status".into(), Value::from("retired")),
        ]),
        evidence: Vec::new(), expected_subject: None,
        idempotency_key: Some(format!("local-session-stop:{subject}:{generation}")),
    };
    store.append_claim(&receipt(&replacement)).unwrap();
    assert!(reconciler.reconcile_runtime_stop(subject, runtime_id, true, Some(&incarnation), 0, Some(&ended)).unwrap());
    assert!(store.has_current_local_session(subject).unwrap());
    store.append_claim(&receipt(&incarnation)).unwrap();
    drop(reconciler);
    drop(store);
    let store = Arc::new(Store::open(&database, "executor").unwrap());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "executor".into(), Arc::new(Notify::new()));
    for _ in 0..3 {
        assert!(reconciler.reconcile_runtime_stop(subject, runtime_id, true, Some(&incarnation), 0, Some(&ended)).unwrap());
        assert!(!store.has_current_local_session(subject).unwrap());
    }
    assert_eq!(store.observations_for(subject, "runtime.action.succeeded").unwrap().len(), 2);
    assert!(runtime.snapshot_ptys().unwrap().iter().any(|o| o.runtime_id == runtime_id
        && o.status == "running" && o.incarnation_id.as_deref() == Some(replacement.as_str())));
}

#[test]
fn offline_owned_stop_proves_native_retirement_but_waits_for_a_durable_receipt_to_settle() {
    let environment = std::env::vars().collect::<BTreeMap<_, _>>();
    let Ok(binary) = fractal_runtime::resolve_executable("pty", &environment) else {
        assert_ne!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"), "hosted custody fixtures require pty");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let runtime = NativeRuntime::new(root.path(), None, &binary);
    runtime.pty().unwrap().spawn("owned-proof", &fractal_runtime::Launch::Shell("sleep 600 & wait".into()),
        root.path(), &environment, None, &BTreeMap::new(), None).unwrap();
    struct Cleanup<'a>(&'a NativeRuntime);
    impl Drop for Cleanup<'_> { fn drop(&mut self) { let _ = self.0.pty().and_then(|pty| pty.stop("owned-proof")); } }
    let _cleanup = Cleanup(&runtime);
    let observed = runtime.snapshot_ptys().unwrap().into_iter().find(|o| o.runtime_id == "owned-proof").unwrap();
    let incarnation = observed.incarnation_id.unwrap();
    let store = Store::open_memory("executor").unwrap();
    let subject = "agent/example/owned-proof";
    store.bind_local_session(&LocalSession {
        subject: subject.into(), runtime_id: "owned-proof".into(),
        incarnation: incarnation.clone(), native_incarnation: incarnation.clone(),
        terminal: true, driver: None, lifecycle: MemberLifecycle::Service,
        custody: SessionCustody::Owned, start_operation: None, invocation: None,
    }).unwrap();
    without_graph(&store);
    assert!(stop_local_session_with_retirement_proof(&store, &runtime, subject, &incarnation, "offline-owned-stop").unwrap());
    assert!(local_session(&store, &runtime, subject, Some(&incarnation)).is_err());
    assert!(store.has_current_local_session(subject).unwrap(),
        "native proof alone must not settle custody before the caller's durable completion receipt");
}

#[test]
fn borrowed_terminal_detaches_offline_survives_restart_and_receipt_is_consumed_once() {
    let environment = std::env::vars().collect::<BTreeMap<_, _>>();
    let Ok(binary) = fractal_runtime::resolve_executable("pty", &environment) else {
        assert_ne!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"), "hosted custody fixtures require pty");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("custody.sqlite");
    let pty_root = root.path().join("pty");
    let runtime = Arc::new(NativeRuntime::new(root.path(), Some(&pty_root), &binary));
    runtime.pty().unwrap().spawn("borrowed-shell", &fractal_runtime::Launch::Shell("sleep 600 & wait".into()),
        root.path(), &environment, None, &BTreeMap::new(), None).unwrap();
    struct Cleanup<'a>(&'a NativeRuntime);
    impl Drop for Cleanup<'_> { fn drop(&mut self) { let _ = self.0.pty().and_then(|pty| pty.stop("borrowed-shell")); } }
    let _cleanup = Cleanup(&runtime);
    let observed = runtime.snapshot_ptys().unwrap().into_iter().find(|o| o.runtime_id == "borrowed-shell").unwrap();
    let native = observed.incarnation_id.unwrap();
    let incarnation = format!("{native}:bound:019a0000-0000-7000-8000-000000000001");
    let binding = LocalSession {
        subject: "agent/example/borrowed".into(), runtime_id: "borrowed-shell".into(),
        incarnation: incarnation.clone(), native_incarnation: native.clone(), terminal: true,
        driver: Some("claude".into()), lifecycle: MemberLifecycle::TerminalBound, custody: SessionCustody::Borrowed,
        start_operation: None, invocation: Some(incarnation.clone()),
    };
    {
        let store = Store::open(&database, "executor").unwrap();
        store.bind_local_session(&binding).unwrap();
    }
    {
        let store = Store::open(&database, "executor").unwrap();
        without_graph(&store);
        assert_eq!(local_session(&store, runtime.as_ref(), &binding.subject, Some(&incarnation)).unwrap(), binding);
        runtime.pty().unwrap().screen("borrowed-shell").unwrap();
        let _attached = std::os::unix::net::UnixStream::connect(pty_core::registry::socket_path_in(&pty_root, "borrowed-shell")).unwrap();
        assert!(local_session(&store, runtime.as_ref(), &binding.subject, Some("stale-invocation")).is_err());
        stop_local_session(&store, runtime.as_ref(), &binding.subject, &incarnation, "detach-once").unwrap();
        stop_local_session(&store, runtime.as_ref(), &binding.subject, &incarnation, "detach-once").unwrap();
        assert!(local_session(&store, runtime.as_ref(), &binding.subject, None).is_err());
        assert_eq!(store.pending_local_session_detaches().unwrap(), vec![(binding.clone(), "detach-once".into())]);
        // Return the graph schema before reopening: the detach itself needed neither table.
        store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-hd58a35e0479d owner=fractal_reconcile::reconcile::local_session_tests TODO=prepare-body-and-predecessor-values */ "ALTER TABLE unavailable_claims RENAME TO claims;
            ALTER TABLE unavailable_desired RENAME TO desired;").unwrap();
    }
    let store = Arc::new(Store::open(&database, "executor").unwrap());
    assert!(store.local_session_detached(&binding.subject, &incarnation).unwrap());
    store.bind_local_session(&binding).unwrap();
    assert!(store.local_session(&binding.subject).unwrap().is_none(), "the invocation must not be re-adopted");
    assert_eq!(store.pending_local_session_detaches().unwrap().len(), 1);
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "executor".into(), Arc::new(Notify::new()));
    reconciler.replay_local_session_detaches().unwrap();
    reconciler.replay_local_session_detaches().unwrap();
    assert_eq!(store.observations_for(&binding.subject, "runtime.action.succeeded").unwrap().len(), 1);
    assert!(store.pending_local_session_detaches().unwrap().is_empty());
    assert!(runtime.snapshot_ptys().unwrap().iter().any(|o| o.runtime_id == "borrowed-shell"
        && o.status == "running" && o.incarnation_id.as_deref() == Some(native.as_str())),
        "detach must leave the real shell and its harness child alive");
}
