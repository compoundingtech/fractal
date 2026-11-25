use super::*;

#[tokio::test]
async fn ordinary_stop_declaration_physically_stops_remote_start_offline_and_recovers_its_receipt() {
    let mut pair = NativePair::for_start().await;
    let authorization = pair.start_authorization("printf x >> ordinary-stop-launches; exec sleep 600");
    let start = pair.home.enqueue(pair.draft("ordinary-stop-native-start",
        Operation::Start { authorization: Box::new(authorization) })).unwrap();
    pair.connect();
    pair.settled(&start).await;
    let local = pair.store.local_session(&start.binding.subject).unwrap().unwrap();
    assert_eq!(local.start_operation.as_deref(), Some(start.execution_id.as_str()));
    assert_eq!(native_file(&pair.root.path().join("ordinary-stop-launches")).await, b"x");
    pair.disconnect().await;

    // Enable the ordinary local declaration reader while the signed session is
    // installed, without making home reachable or adopting a fullnode launch.
    pair.store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h6348dfbc44a9 owner=fractal_reconcile::reconcile::home_session_stop_tests TODO=prepare-body-and-predecessor-values */ "ALTER TABLE unavailable_claims RENAME TO claims; ALTER TABLE unavailable_desired RENAME TO desired;"
    ).unwrap();
    crate::reconcile::tests::apply_source(&pair.store,
        "version 2\nstop \"agent/example/worker\"", "ordinary-stop-declaration");
    let stop = pair.store.desired_subjects().unwrap().into_iter()
        .find(|subject| subject.subject == start.binding.subject).unwrap();
    assert_eq!(stop.kind, "stop");
    let mut errors = Vec::new();
    pair.executor.reconcile_stop_item(&stop, "ordinary-stop-native", false, None,
        &BTreeSet::new(), &mut errors);
    assert!(errors.is_empty(), "ordinary stop reader failed: {errors:?}");
    assert!(pair.store.local_session_retirement_verified(&local).unwrap(),
        "ordinary Stop must physically retire the exact native scope while home is offline");
    assert!(!pair.runtime.snapshot_ptys().unwrap().iter().any(|entry|
        entry.runtime_id == local.runtime_id && entry.incarnation_id.as_deref() == Some(local.native_incarnation.as_str())));
    let pending = pair.store.runtime_start_for(&stop.subject).unwrap().unwrap();
    assert_eq!(pending.id, start.execution_id);
    assert!(pending.revoked_by.is_some());
    assert!(!pending.cleanup_verified && !pending.applied,
        "physical retirement alone cannot bypass session finalization");
    assert!(!pair.executor.stop_is_settled(&stop).unwrap());
    assert!(pair.seat.checkpoint("seat", pair.selection.epoch, start.sequence).is_err());
    assert!(pair.store.observations_for(&stop.subject, "runtime.action.succeeded").unwrap().is_empty(),
        "remote Start must never acquire graph-backed completion adoption");
    assert!(!pair.executor.apply_start_result(&start.execution_id, &stop.subject).unwrap());
    pair.executor.reconcile_start_jobs().unwrap();

    // Exit between native closure and receipt completion. Reopened native
    // recovery consumes the exact durable withdrawal and retirement witness.
    pair.reopen_native();
    let recovery = pair.seat.recoverable().unwrap().into_iter()
        .find(|recovery| recovery.command.execution_id == start.execution_id).unwrap();
    assert!(recovery.cancellation_reason.is_some());
    let outcome = pair.driver.recover(recovery).await.unwrap();
    assert!(matches!(&outcome, Outcome::Stopped { incarnation } if incarnation == &local.incarnation));
    pair.seat.complete(&start.execution_id, outcome).unwrap();
    assert!(pair.store.remote_runtime_start(&start.execution_id).unwrap().unwrap().cleanup_verified);
    assert!(pair.store.runtime_start_for(&stop.subject).unwrap().is_none());
    // Terminal result custody is retained until the disconnected home ACKs it.
    assert!(pair.seat.checkpoint("seat", pair.selection.epoch, start.sequence).is_err());
    pair.connect();
    pair.stopped(&start).await;
    pair.disconnect().await;
    assert_eq!(std::fs::read(pair.root.path().join("ordinary-stop-launches")).unwrap(), b"x",
        "ordinary stop and durable recovery must never relaunch the native producer");
}
