use super::*;
use tests_shared_client_v0_tests::{folded_at_once, mission, refresh_until_published};

/// Admit older run changes behind a local claim, so the chunk loop queues its rebuild.
fn backlog() -> (tempfile::TempDir, Arc<Store>, String, String) {
    let root = tempfile::tempdir().unwrap();
    let writer = crate::test_support::fleet_member(&root.path().join("writer"), "writer");
    let reader = crate::test_support::fleet_member(&root.path().join("reader"), "reader");
    mission(&writer, "drain-publication");
    let run = writer.create_mission_run(&crate::model::MissionRunRequest {
        mission: "drain-publication".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/operator".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "drain-publication-run".into(),
    }).unwrap();
    let step = run.steps.iter().find(|step| step.step == "plant").unwrap().subject.clone();
    // Creating a run leaves its steps pending. Work publications deliberately omit pending
    // steps; establish the visible old ready row before replication, as the work-list fixtures do.
    writer.set_step_state(&step, "ready", None).unwrap();
    crate::test_support::replicate(&writer, &reader);
    assert!(reader.work(None, false).unwrap().iter()
        .any(|view| view.subject == step && view.status == "ready"));
    reader.set_write_clock_at(now_ms() + 1_000_000).unwrap();
    reader.append_claim(&crate::model::ClaimInput {
        subject: "custom/drain/order".into(), kind: "custom.drain.order".into(), actor: None,
        fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    reader.project_replication_backlog().unwrap();
    for index in 0..129 {
        writer.set_step_state(&step, if index % 2 == 0 { "blocked" } else { "ready" }, Some("backlog")).unwrap();
    }
    let exchange = writer.export_replication_exchange(
        crate::test_support::PAUSE_FLEET, &reader.replication_inventory().unwrap(),
    ).unwrap();
    assert!(exchange.envelopes.len() >= 129);
    reader.receive_replication_exchange("writer", crate::test_support::PAUSE_FLEET, &exchange).unwrap();
    let admission = reader.validate_replication_backlog().unwrap();
    assert_eq!(admission.invalid, 0);
    assert_eq!(admission.unknown, 0);
    reader.apply_replication_repairs().unwrap();
    (root, reader, run.subject, step)
}

fn warm(store: &Store) {
    store.published_missions_list().start();
    store.published_work_list().start();
    refresh_until_published(store, |store| store.published_missions_list(), fold_missions);
    refresh_until_published(store, |store| store.published_work_list(), fold_work);
}

fn assert_fresh(store: &Store, step: &str, cancelled: bool, before: (u64, u64)) {
    // A stale fold must not be accepted merely because its cut still equals the target.
    // base_and_pending consumes the forget, so inspect generations and let warm take it first.
    assert!(store.published_work_list().generation() > before.0, "drain must forget the work list");
    assert!(store.published_missions_list().generation() > before.1, "drain must forget the missions list");
    // Both folds stamp card as_of from the clock; compare them at one instant.
    smallclaims::store::set_thread_clock(Some(now_ms() as u128));
    warm(store);
    let work = store.published_work_list().newest().unwrap();
    let missions = store.published_missions_list().newest().unwrap();
    let (fresh_work, _) = folded_at_once(fold_work(store, None, None));
    let (fresh_missions, _) = folded_at_once(fold_missions(store, None, None));
    smallclaims::store::set_thread_clock(None);
    assert_eq!(work_window(&work.rows, None, 200), work_window(&fresh_work.rows, None, 200));
    assert_eq!(mission_window(&missions.rows, 200).unwrap(), mission_window(&fresh_missions.rows, 200).unwrap());
    if cancelled {
        assert!(work_window(&work.rows, None, 200).0.is_empty(), "cancelled run must publish no active work");
    } else {
        assert_eq!(work.rows.rows.get(step).unwrap().view.status, "blocked");
    }
}

#[test]
fn receive_final_drain_forgets_lists_cached_after_last_chunk_and_rejects_inflight_fold() {
    let (_root, reader, _run, step) = backlog();
    let mut cached = None;
    reader.project_replication_backlog_with_yield(|| {
        if reader.projection_frontier().unwrap() == reader.index().unwrap()
            && reader.run_projection_pending(&step).unwrap()
        {
            assert!(cached.is_none());
            warm(&reader);
            let work = reader.published_work_list().newest().unwrap();
            assert_eq!(work.cut, reader.index().unwrap());
            assert_eq!(work.rows.rows.get(&step).unwrap().view.status, "ready");
            // Hold folds that began before the drain; forgetting must fence their publication.
            let (_, _, work_generation) = reader.published_work_list().base_and_pending();
            let (_, _, mission_generation) = reader.published_missions_list().base_and_pending();
            let (work_fold, _) = folded_at_once(fold_work(&reader, None, None));
            let (mission_fold, _) = folded_at_once(fold_missions(&reader, None, None));
            cached = Some((work_generation, mission_generation, work_fold, mission_fold));
        }
    }).unwrap();
    let (work_generation, mission_generation, work_fold, mission_fold) = cached.expect("refresher ran between last chunk and drain");
    assert!(!reader.run_projection_pending(&step).unwrap());
    assert!(reader.published_work_list().publish(work_fold, work_generation).is_none());
    assert!(reader.published_missions_list().publish(mission_fold, mission_generation).is_none());
    assert_fresh(&reader, &step, false, (work_generation, mission_generation));
}

#[test]
fn caller_drains_forget_both_lists_with_and_without_a_cancellation_claim() {
    for cancel in [false, true] {
        let (_root, reader, run, step) = backlog();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reader.project_replication_backlog_with_yield(|| {
                if reader.projection_frontier().unwrap() == reader.index().unwrap()
                    && reader.run_projection_pending(&step).unwrap()
                {
                    panic!("interrupt before final drain");
                }
            }).unwrap();
        }));
        assert!(interrupted.is_err());
        assert!(reader.run_projection_pending(&step).unwrap());
        warm(&reader);
        assert_eq!(reader.published_work_list().newest().unwrap().rows.rows.get(&step).unwrap().view.status, "ready");
        let before = (reader.published_work_list().generation(), reader.published_missions_list().generation());
        let cut = reader.index().unwrap();
        if cancel {
            assert!(reader.request_mission_run_cancellation(&run, "cancel while projection pending").unwrap());
        } else {
            // Use the shared transactional helper directly: no per-caller cache callback and
            // no new replicated claim may be required to invalidate the published lists.
            let mut connection = reader.connection.write();
            let transaction = connection.transaction().unwrap();
            assert!(crate::store::finish_run_projection_tx(&transaction, &step).unwrap());
            transaction.commit().unwrap();
            drop(connection);
            assert_eq!(reader.index().unwrap(), cut);
        }
        assert!(!reader.run_projection_pending(&step).unwrap());
        assert_fresh(&reader, &step, cancel, before);
    }
}
