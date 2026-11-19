use super::*;
use crate::store::{checkpoint_agreement::SealTerms, runtime::Plain};
use std::sync::Arc;

const NOW: u128 = 10 * DAY_MS;

fn store() -> Store {
    Store::open_memory("status-node", Arc::new(Plain)).unwrap()
}

#[test]
fn cold_status_does_not_walk_or_invent_terms() {
    let store = store();
    SEALED_IDENTITY_WALKS.with(|walks| walks.set(0));
    let pending = store.checkpoint_status(NOW, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.terms, None);
    assert_eq!(pending.local_digest_state, "unavailable");
    assert!(pending.sealed.is_empty());
    assert!(pending.disagreeing.is_empty());
    SEALED_IDENTITY_WALKS.with(|walks| assert_eq!(walks.get(), 0));
}

#[test]
fn warm_status_reuses_the_maintenance_digest_without_walking() {
    let store = store();
    store.register_checkpoint_capture(newest_due_cut(NOW), None).unwrap();
    let sealed = store.checkpoint_sealed_identities(newest_due_cut(NOW), None).unwrap();
    SEALED_IDENTITY_WALKS.with(|walks| walks.set(0));
    let pending = store.checkpoint_status(NOW, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.local_digest_state, "ready");
    let terms = pending.terms.unwrap();
    assert_eq!(terms.sealed_digest, sealed.digest);
    assert_eq!(terms.rules_digest, "plain");
    assert_eq!(terms.cut_unix_ms, newest_due_cut(NOW));
    SEALED_IDENTITY_WALKS.with(|walks| assert_eq!(walks.get(), 0));
}

#[test]
fn cold_status_preserves_our_published_terms_without_claiming_a_ready_digest() {
    let store = store();
    let cut = newest_due_cut(NOW);
    let sealed = store.checkpoint_sealed_identities(cut, None).unwrap();
    let terms = SealTerms {
        cut_unix_ms: cut,
        participants: BTreeSet::from(["status-node".to_owned()]),
        sealed_digest: sealed.digest.clone(),
        rules_digest: "published-rules".into(),
    };
    store.publish_seal(&checkpoint_name(cut), &terms, &sealed, None).unwrap();
    *store.sealed_identity_memo.lock().unwrap() = None;
    SEALED_IDENTITY_WALKS.with(|walks| walks.set(0));
    let pending = store.checkpoint_status(NOW, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.terms, Some(terms));
    assert_eq!(pending.local_digest_state, "unavailable");
    assert!(pending.sealed.contains("status-node"));
    SEALED_IDENTITY_WALKS.with(|walks| assert_eq!(walks.get(), 0));
}

#[test]
fn cold_membership_status_never_seals_pending_batches_or_writes() {
    let store = store();
    store.append_claim(&serde_json::from_value(serde_json::json!({
        "subject":"note/pending", "kind":"note"
    })).unwrap()).unwrap();
    *store.membership_cache.lock().unwrap() = None;
    *store.sealing_recovery.try_lock().unwrap() = true;
    let pending_batches = || store.readers.get().query_row(
        "SELECT COUNT(*) FROM batches WHERE NOT EXISTS (SELECT 1 FROM replica_envelopes WHERE batch_id=batches.id)",
        [], |row| row.get::<_, u64>(0),
    ).unwrap();
    let pending = pending_batches();
    assert!(pending > 0);
    let changes = store.connection.write().total_changes();
    store.checkpoint_status(NOW, &[]).unwrap();
    assert_eq!(store.connection.write().total_changes(), changes);
    assert_eq!(pending_batches(), pending);
    assert!(*store.sealing_recovery.try_lock().unwrap(), "status entered sealing recovery");
}

#[test]
fn worker_diagnostics_preserve_the_failed_phase_without_walking_history() {
    use crate::store::checkpoint_agreement::CheckpointPhase;
    let store = store();
    store.checkpoint_worker_phase(CheckpointPhase::Disabled);
    SEALED_IDENTITY_WALKS.with(|walks| walks.set(0));
    let status = store.checkpoint_status(NOW, &["absent-writer".into()]).unwrap();
    let summary = status.blocking_summary().unwrap();
    assert!(summary.contains("status-node phase=Disabled"));
    assert!(summary.contains("absent-writer phase=seal (no seal)"));
    assert!(summary.contains("disabled by config on 1 node (local); peer config unknown"));
    assert!(summary.contains("newest stable none"));
    store.checkpoint_worker_phase(CheckpointPhase::Capture);
    store.checkpoint_worker_error("scratch space refused".into());
    let status = store.checkpoint_status(NOW, &[]).unwrap();
    let worker = status.worker.as_ref().unwrap();
    assert_eq!(worker.phase, CheckpointPhase::Capture);
    assert_eq!(worker.error.as_deref(), Some("scratch space refused"));
    assert!(status.blocking_summary().unwrap().contains("phase=Capture failed: scratch space refused"));
    assert_eq!(serde_json::from_value::<crate::store::CheckpointStatusView>(
        serde_json::to_value(&status).unwrap()).unwrap(), status);
    store.checkpoint_worker_phase(CheckpointPhase::Seal);
    assert!(store.checkpoint_status(NOW, &[]).unwrap().worker.unwrap().error.is_none());
    SEALED_IDENTITY_WALKS.with(|walks| assert_eq!(walks.get(), 0));
}

#[test]
fn blockers_distinguish_seals_from_verifications_and_accept_older_status() {
    let store = store();
    let mut status = store.checkpoint_status(NOW, &[]).unwrap();
    let pending = status.pending.as_mut().unwrap();
    pending.unsealed.clear();
    assert!(status.blocking_summary().unwrap().contains("status-node phase=verify"));
    status.pending.as_mut().unwrap().verified.insert("status-node".into());
    assert!(!status.blocking_summary().unwrap().contains("phase=verify"));
    let mut json = serde_json::to_value(&status).unwrap();
    json.as_object_mut().unwrap().remove("worker");
    let old: crate::store::CheckpointStatusView = serde_json::from_value(json).unwrap();
    assert!(old.worker.is_none());
}

#[test]
fn worker_failures_and_halted_trims_remain_visible_without_pending_work() {
    use crate::store::checkpoint_agreement::CheckpointPhase;
    let store = store();
    store.checkpoint_worker_phase(CheckpointPhase::ApplyStable);
    store.checkpoint_worker_error("trim commit failed".into());
    let mut status = store.checkpoint_status(NOW, &[]).unwrap();
    status.pending = None;
    assert!(status.blocking_summary().unwrap().contains("phase=ApplyStable failed: trim commit failed"));
    status.worker.as_mut().unwrap().error = None;
    status.halted = true;
    assert!(status.blocking_summary().unwrap().contains("phase=trim (halted"));
    status.halted = false;
    assert!(status.blocking_summary().is_none());
}

#[test]
fn verification_disagreements_name_participants_and_reasons() {
    let store = store();
    let mut status = store.checkpoint_status(NOW, &[]).unwrap();
    let pending = status.pending.as_mut().unwrap();
    pending.unsealed.clear();
    pending.verified.insert("status-node".into());
    pending.verifications_agree = false;
    pending.verifications_differ.insert("status-node".into(), "readers, retained claims".into());
    assert!(status.blocking_summary().unwrap().contains(
        "status-node phase=verify (different readers, retained claims)"));
}
