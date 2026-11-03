use super::*;
use crate::reconcile::tests::FakeRuntime;

fn reconciler() -> Reconciler<FakeRuntime> {
    Reconciler::new(
        Arc::new(Store::open_memory("node").unwrap()),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    )
}

#[test]
fn cached_pressure_expires_without_a_dirty_event_or_file_read() {
    let mut reconciler = reconciler();
    // No file exists: opening it would replace the stale reason with an open error.
    let root = tempfile::tempdir().unwrap();
    reconciler.seat_admission = Controller {
        config: Some(Config { state_file: root.path().join("missing"), batch_paused_min_secs: 30 }),
        checked: Mutex::new(Some(1110)),
        ..Controller::default()
    };
    let status = &reconciler.seat_admission.status;
    status.state().decision = Decision {
        mode: "enforce".into(), reason: "envelope exceeded and batch pause reached minimum age".into(),
        queue: true, observed_at: Some(1000),
    };
    let mut events = reconciler.event_notify.subscribe();

    reconciler.refresh_seat_admission_at(1120);
    assert!(status.state().decision.queue, "age 120 is still fresh");
    assert!(!events.has_changed().unwrap());

    reconciler.refresh_seat_admission_at(1121);
    assert_eq!(status.state().decision, Decision {
        mode: "fail-open".into(), reason: "admission state is stale".into(),
        queue: false, observed_at: None,
    });
    assert_eq!(*reconciler.seat_admission.checked.lock().unwrap(), Some(1110));
    assert!(!reconciler.seat_admission.dirty.load(Ordering::Acquire));
    assert!(events.has_changed().unwrap(), "expiry wakes incremental reconciliation");
    events.borrow_and_update();
    assert!(status.admit("seat", None, None, false, 1121));

    reconciler.refresh_seat_admission_at(1122);
    assert!(!events.has_changed().unwrap(), "unchanged decisions do not wake again");
    assert_eq!(*reconciler.seat_admission.checked.lock().unwrap(), Some(1110));

    reconciler.refresh_seat_admission_at(1140);
    assert_eq!(*reconciler.seat_admission.checked.lock().unwrap(), Some(1140));
    #[cfg(target_os = "linux")]
    assert!(status.state().decision.reason.contains("open admission state"),
        "periodic observation still reads at the original 30-second deadline");
    #[cfg(not(target_os = "linux"))]
    assert_eq!(status.state().decision.mode, "off");
}

#[cfg(target_os = "linux")]
fn enforce_fixture(now: u64) -> (tempfile::TempDir, Config) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state");
    let state = serde_json::json!({
        "schema": "interactive-envelope.state/v1", "host": local_host().unwrap(),
        "observedAt": now, "envelopeCgroup": "/interactive", "envelopeBytes": 100,
        "usageBytes": 101, "exceeded": true, "exceededSince": now - 60,
        "batchPaused": true, "batchPausedSince": now - 30, "mode": "enforce",
    });
    std::fs::write(&path, state.to_string()).unwrap();
    (root, Config { state_file: path, batch_paused_min_secs: 30 })
}

#[cfg(target_os = "linux")]
#[test]
fn builder_and_requests_admit_until_explicit_observation_without_waiting() {
    let now = u64::try_from(now_ms() / 1000).unwrap();
    let (_root, config) = enforce_fixture(now);
    let status = Arc::new(Status::default());
    let reconciler = reconciler().with_seat_admission(Some(config), status.clone());

    assert_eq!(status.state().decision, Decision {
        mode: "fail-open".into(), reason: "seat admission initializing; admitting until first observation".into(),
        queue: false, observed_at: None,
    });
    assert!(reconciler.seat_admission.checked.lock().unwrap().is_none());
    assert!(reconciler.seat_admission.dirty.load(Ordering::Acquire));
    // Requests return their cached decision directly; they neither read nor wait
    // for the valid enforce fixture, even while initial observation is pending.
    for subject in ["first", "second"] {
        assert!(status.admit(subject, None, None, false, now));
    }
    assert!(reconciler.seat_admission.checked.lock().unwrap().is_none());
    assert_eq!(status.snapshot(now)["pendingCount"], 0);

    reconciler.refresh_seat_admission_at(now);
    assert!(status.state().decision.queue);
    assert_eq!(*reconciler.seat_admission.checked.lock().unwrap(), Some(now));
    assert!(!status.admit("third", None, None, false, now));
    assert_eq!(status.snapshot(now)["pendingCount"], 1);
    assert_eq!(*reconciler.seat_admission.checked.lock().unwrap(), Some(now));
}

#[cfg(target_os = "linux")]
#[test]
fn dirty_event_refreshes_before_the_periodic_read_is_due() {
    let (_root, config) = enforce_fixture(1000);
    let status = Arc::new(Status::default());
    let reconciler = reconciler().with_seat_admission(Some(config), status.clone());
    *reconciler.seat_admission.checked.lock().unwrap() = Some(1000);
    // The initial dirty event alone must trigger the observation, not the timer.
    assert!(reconciler.seat_admission.dirty.load(Ordering::Acquire));
    reconciler.refresh_seat_admission_at(1001);
    assert!(status.state().decision.queue);
    assert_eq!(status.state().decision.observed_at, Some(1000));
    assert_eq!(*reconciler.seat_admission.checked.lock().unwrap(), Some(1001));
    assert!(!reconciler.seat_admission.dirty.load(Ordering::Acquire));
}
