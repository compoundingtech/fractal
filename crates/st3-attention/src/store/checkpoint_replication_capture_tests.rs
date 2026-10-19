#![allow(unused_imports)]
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use st3_store::store::checkpoint_replication_capture_tests_shared_attention_tests::{CUT, FLEET, PagePause, ROUNDS, TraceGuard, admit_and_project, epoch, exchange, pause_after_envelope_page, publish_desired};
use st3_store::store::checkpoint_replication_capture_tests_shared_attention_tests::{publish_document};


#[test]
fn replicated_admission_duplicate_reoffers_and_projection_leave_older_capture_unchanged() {
    let scratch = tempfile::tempdir().unwrap();
    let source = Store::open_memory("sender").unwrap();
    source.bind_fleet(FLEET).unwrap();
    let target = Arc::new(Store::open(&scratch.path().join("claims.sqlite3"), "receiver").unwrap());
    target.bind_fleet(FLEET).unwrap();
    source.set_write_clock_at(100).unwrap();
    for index in 0..ROUNDS {
        let claim = source
            .append_claim(&ClaimInput {
                subject: "daemon/sender".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("capture-replication")),
                    ("reason".into(), json!(format!("Invented old diagnostic {index}."))),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("capture-old-{index}")),
            })
            .unwrap();
        assert_eq!(claim.accepted_at_unix_ms, 100);
        source.seal_local_batches().unwrap();
    }
    let old_exchange = exchange(&source, &target);
    assert_eq!(old_exchange.envelopes.len(), ROUNDS);
    admit_and_project(&source, &target, &old_exchange);

    // These projection identities are already inside the frontier but above the cut.
    // Updating them tests OLD as well as NEW references without touching an old claim.
    source.set_write_clock_at(200).unwrap();
    let remote_subject = publish_desired(&source, "remote", 0);
    publish_document(&source, "remote", 0);
    admit_and_project(&source, &target, &exchange(&source, &target));
    target.set_write_clock_at(200).unwrap();
    let local_subject = publish_desired(&target, "local", 0);
    publish_document(&target, "local", 0);
    let expected = target.checkpoint_sealed_set_paged(CUT, None, 1, 1).unwrap();
    assert_eq!(expected.claims.len(), ROUNDS);
    assert_eq!(expected.envelopes.len(), ROUNDS);
    assert!(expected.claims.iter().all(|claim| claim.claim.accepted_at_unix_ms == 100));
    let identities = checkpoint::SealedIdentities::of(&expected);
    let expected_epoch = epoch(&target);

    let (reached, at_page_boundary) = mpsc::sync_channel(1);
    let (resume, continue_capture) = mpsc::sync_channel(1);
    let pause = Arc::new(PagePause {
        envelope_page: AtomicBool::new(false),
        pauses: AtomicUsize::new(0),
        reached,
        resume: Mutex::new(continue_capture),
    });
    let reader = target.clone();
    let reader_pause = pause.clone();
    let capture = std::thread::spawn(move || {
        // Only this worker's request reader has a callback. Other replication reads cannot
        // accidentally pause, and each capture page still starts and ends its own snapshot.
        reader.readers.request_read(|| {
            let connection = reader.readers.get();
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(
                    connection.handle(),
                    (rusqlite::ffi::SQLITE_TRACE_STMT | rusqlite::ffi::SQLITE_TRACE_PROFILE) as u32,
                    Some(pause_after_envelope_page),
                    Arc::as_ptr(&reader_pause).cast_mut().cast(),
                );
            }
            let _trace = TraceGuard(&connection);
            reader.checkpoint_sealed_set_paged(CUT, None, 1, 1)
        })
    });

    for round in 0..ROUNDS {
        assert_eq!(at_page_boundary.recv_timeout(Duration::from_secs(30)).unwrap(), round);
        // PROFILE fires after COMMIT finishes: no capture snapshot remains while these real
        // receive/admission/projection transactions commit, before the next page begins.
        source.set_write_clock_at(201 + round as u128).unwrap();
        publish_desired(&source, "remote", round + 1);
        let remote_hash = publish_document(&source, "remote", round + 1);
        let incoming = exchange(&source, &target);
        assert!(!incoming.envelopes.is_empty());
        assert!(incoming.envelopes.iter().all(|envelope| envelope.accepted_at_unix_ms >= 200));
        let receipt = target.receive_replication_exchange(source.origin(), FLEET, &incoming).unwrap();
        assert_eq!(receipt.received, incoming.envelopes.len());
        assert_eq!(epoch(&target), expected_epoch, "ordinary receive in round {round}");
        let admitted = target.validate_replication_backlog().unwrap();
        assert!(admitted.changed && admitted.valid > 0);
        assert_eq!((admitted.invalid, admitted.unknown, admitted.held), (0, 0, 0));
        assert_eq!(epoch(&target), expected_epoch, "ordinary admission in round {round}");
        assert!(target.project_replication_backlog().unwrap());
        assert_eq!(epoch(&target), expected_epoch, "ordinary projection in round {round}");
        assert_eq!(target.selected_desired_token(&remote_subject).unwrap(), source.selected_desired_token(&remote_subject).unwrap());
        assert_eq!(target.latest_document_hash("doc/capture/remote").unwrap(), Some(remote_hash));

        // A below-cut envelope is re-offered exactly as a real peer does it. OR IGNORE must
        // cause neither an AFTER event nor a content-difference BEFORE invalidation.
        for _ in 0..3 {
            for duplicate in [&old_exchange, &incoming] {
                let receipt = target.receive_replication_exchange(source.origin(), FLEET, duplicate).unwrap();
                assert_eq!(receipt.received, 0);
                assert_eq!(receipt.duplicate, duplicate.envelopes.len());
                let admitted = target.validate_replication_backlog().unwrap();
                assert!(!admitted.changed);
                assert_eq!((admitted.valid, admitted.invalid, admitted.unknown, admitted.held), (0, 0, 0, 0));
                target.project_replication_backlog().unwrap();
                assert_eq!(epoch(&target), expected_epoch, "duplicate re-offer in round {round}");
            }
        }
        target.set_write_clock_at(300 + round as u128).unwrap();
        publish_desired(&target, "local", round + 1);
        let local_hash = publish_document(&target, "local", round + 1);
        target.seal_local_batches().unwrap();
        assert_eq!(epoch(&target), expected_epoch, "local desired/document writes in round {round}");
        assert_eq!(target.latest_document_hash("doc/capture/local").unwrap(), Some(local_hash));
        let token = target.selected_desired_token(&local_subject).unwrap().unwrap();
        assert!(target.claim_by_id(&token).unwrap().unwrap().accepted_at_unix_ms >= 200);
        resume.send(()).unwrap();
    }
    let captured = capture.join().unwrap().unwrap().expect("ordinary replication must not exhaust capture retries");
    assert_eq!(pause.pauses.load(Ordering::Acquire), ROUNDS);
    assert_eq!(epoch(&target), expected_epoch);
    assert_eq!(checkpoint::SealedIdentities::of(&captured), identities);
    assert_eq!(format!("{:?}", captured.claims), format!("{:?}", expected.claims));
    assert!(captured.envelopes.iter().all(|envelope| envelope.accepted_at_unix_ms == 100));
}
