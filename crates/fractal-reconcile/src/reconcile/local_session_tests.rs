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
