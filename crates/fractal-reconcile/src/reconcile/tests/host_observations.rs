use super::*;

fn observed() -> RuntimeObservation {
    RuntimeObservation {
        runtime_id: "example.terminal".into(), terminal: true,
        status: "running".into(), exit_code: None,
        incarnation_id: Some("123:birth-one".into()),
    }
}

fn fixture() -> (tempfile::TempDir, Arc<AtomicUsize>, Reconciler<FakeRuntime>) {
    let root = tempfile::tempdir().unwrap();
    let evaluations = Arc::new(AtomicUsize::new(0));
    let count = evaluations.clone();
    let runtime = FakeRuntime {
        observation_paths: Some(vec![root.path().join("physical-registry")]),
        before_snapshot: Mutex::new(Some(Box::new(move || { count.fetch_add(1, Ordering::Relaxed); }))),
        ptys: Mutex::new(vec![observed()]),
        ..Default::default()
    };
    let store = Arc::new(Store::open(&root.path().join("graph.sqlite3"), "node").unwrap());
    crate::store::messages_shard::open(&store, root.path()).unwrap();
    let mut reconciler = Reconciler::new(store, Arc::new(runtime), "node".into(), Arc::new(Notify::new()));
    reconciler.skip_unneeded = true;
    reconciler.incremental.evaluated("example-process-reader",
        BTreeSet::from(["exec:example.process".into()]), None);
    reconciler.reconcile_once().unwrap();
    assert_eq!(evaluations.load(Ordering::Relaxed), 1);
    assert_eq!(reconciler.runtime.physical_exec_observations.load(Ordering::Relaxed), 1);
    (root, evaluations, reconciler)
}

#[test]
fn unrelated_fleet_writes_do_not_evaluate_physical_executor() {
    let (_root, evaluations, reconciler) = fixture();
    for index in 0..8 {
        reconciler.store.append_claim(&ClaimInput {
            subject: format!("resource/unrelated-{index}"), kind: "resource.observed".into(),
            actor: None, fields: BTreeMap::from([("status".into(), Value::from("ready"))]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        reconciler.reconcile_once().unwrap();
        reconciler.evaluate_mission_runs().unwrap();
    }
    assert_eq!(evaluations.load(Ordering::Relaxed), 1,
        "counts real backend snapshots, not calls to the retained observation adapter");
    assert_eq!(reconciler.runtime.physical_exec_observations.load(Ordering::Relaxed), 1,
        "unrelated writes also cause zero backend process observations");
}

#[test]
fn unchanged_physical_evidence_does_not_dirty_downstream_readers() {
    let (_root, evaluations, reconciler) = fixture();
    reconciler.incremental.evaluated("example-reader",
        BTreeSet::from(["pty:example.terminal".into()]), None);
    reconciler.host_observations.invalidate();
    reconciler.reconcile_once().unwrap();
    assert_eq!(evaluations.load(Ordering::Relaxed), 2);
    assert!(!reconciler.incremental.needs("example-reader", now_ms()),
        "an actual no-op physical resample causes zero downstream evaluations");

    reconciler.runtime.ptys.lock().unwrap()[0].incarnation_id = Some("123:birth-two".into());
    reconciler.host_observations.invalidate();
    reconciler.reconcile_once().unwrap();
    assert_eq!(evaluations.load(Ordering::Relaxed), 3);
    assert!(reconciler.incremental.needs("example-reader", now_ms()),
        "PID reuse changes the physical incarnation even when status is unchanged");
}

#[test]
fn explicit_safety_pass_resamples_without_a_registry_event() {
    let (_root, evaluations, mut reconciler) = fixture();
    // The existing full-pass safety path remains able to detect a process that died
    // without updating its native registry. Ordinary fleet changes do not take it.
    reconciler.skip_unneeded = false;
    reconciler.reconcile_once().unwrap();
    assert_eq!(evaluations.load(Ordering::Relaxed), 2);
}

#[test]
fn failed_physical_sample_never_reuses_pre_failure_evidence() {
    let (_root, evaluations, reconciler) = fixture();
    *reconciler.runtime.snapshot_error.lock().unwrap() = true;
    reconciler.host_observations.invalidate();
    reconciler.reconcile_once().unwrap();
    assert_eq!(evaluations.load(Ordering::Relaxed), 2);
    *reconciler.runtime.snapshot_error.lock().unwrap() = false;
    // No additional registry event is required to recover from unknown evidence.
    reconciler.reconcile_once().unwrap();
    assert_eq!(evaluations.load(Ordering::Relaxed), 3);
}

#[test]
fn restarted_adapter_resamples_the_existing_physical_incarnation() {
    let (_root, evaluations, reconciler) = fixture();
    let restarted = Reconciler::new(
        reconciler.store.clone(), reconciler.runtime.clone(),
        "node".into(), Arc::new(Notify::new())).skipping_unneeded(true);
    restarted.reconcile_once().unwrap();
    assert_eq!(evaluations.load(Ordering::Relaxed), 2);
    let snapshot = restarted.host_pty_snapshot().unwrap();
    assert_eq!(snapshot.observations[0].incarnation_id.as_deref(), Some("123:birth-one"));
    assert_eq!(evaluations.load(Ordering::Relaxed), 2);
}
