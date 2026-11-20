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
#[tokio::test]
async fn idle_host_without_finite_custody_does_not_poll_pressure() {
    let reconciler = reconciler();
    reconciler.refresh_seat_admission_at(u64::try_from(now_ms() / 1000).unwrap());
    assert!(reconciler.seat_admission.pressure.lock().unwrap().is_none());
    assert!(!reconciler.step_deadlines.lock().unwrap().contains_key("resource-pressure"));
}


#[tokio::test]
async fn logical_admission_keeps_pressure_refresh_platform_explicit_without_native_lanes() {
    let reconciler = reconciler();
    assert!(reconciler.seat_admission.lanes.is_none());
    assert!(reconciler.seat_admission.config.is_none());
    use crate::store::resource_membership::{Admission, Fit, LaneRequest};
    let request: LaneRequest = serde_json::from_value(serde_json::json!({
        "id":"pressure-owner", "name":"checks", "host":"node", "maxMembers":1,
        "capacity":{"cores":1,"memory":1024}, "coverage":"logical",
        "owner":{"run":"run","step":"step","attempt":1}, "class":"check",
    })).unwrap();
    reconciler.store.start_resource_authority("node", "pressure-authority").unwrap();
    let fit = Fit { room: request.capacity, ..Fit::default() };
    let Admission::Admitted(grant) = reconciler.store.request_resource_membership(&request, fit.clone(), 1).unwrap()
        else { panic!("finite fixture must admit"); };
    let mut queued = request.clone();
    queued.id = "pressure-queued".into();
    assert!(matches!(reconciler.store.request_resource_membership(&queued, fit, 2).unwrap(), Admission::Queued { .. }));
    let now = u64::try_from(now_ms() / 1000).unwrap();
    reconciler.refresh_seat_admission_at(now);
    let deadlines = reconciler.step_deadlines.lock().unwrap();
    if cfg!(target_os = "linux") {
        assert!(reconciler.seat_admission.pressure.lock().unwrap().is_some());
        assert!(deadlines.get("resource-pressure").is_some_and(|deadline|
            *deadline >= u128::from(now + 5) * 1000));
    } else {
        assert!(reconciler.seat_admission.pressure.lock().unwrap().is_none());
        assert!(!deadlines.contains_key("resource-pressure"));
    }
    drop(deadlines);
    reconciler.store.cancel_resource_membership(&grant, &request.owner, 3).unwrap();
    let runtime = fractal_runtime::ExecRuntime::new(PathBuf::new(), PathBuf::new());
    assert!(reconciler.store.retire_resource_membership(&grant, &request.owner, &runtime, 3).unwrap());
    assert!(reconciler.store.has_live_finite_resource_memberships("node").unwrap(),
        "queued custody alone keeps pressure recovery observable");
    reconciler.store.withdraw_resource_membership(&queued.id, &queued.owner, 4).unwrap();
    assert!(!reconciler.store.has_live_finite_resource_memberships("node").unwrap(),
        "terminal custody no longer needs pressure observations");
}

#[test]
fn changed_host_memory_observation_wakes_waiters_without_an_envelope_file() {
    use fractal_runtime::host::HostMemoryObservation;
    fn outside() -> Option<HostMemoryObservation> {
        Some(HostMemoryObservation { outside_bytes: 42, psi_blocked: false })
    }
    let mut reconciler = reconciler();
    reconciler.seat_admission.lanes = Some(LaneHost {
        room: fractal_runtime::Capacity { cores: 8, memory: 1024 }, outside, burst_lane: None,
    });
    let mut events = reconciler.event_notify.subscribe();
    reconciler.refresh_seat_admission_at(1000);
    assert!(events.has_changed().unwrap(), "first host observation wakes fit evaluation");
    events.borrow_and_update();
    reconciler.refresh_seat_admission_at(1001);
    assert!(!events.has_changed().unwrap(), "unchanged memory does not wake again");
    *reconciler.seat_admission.outside_observation.lock().unwrap() =
        Some(Some(HostMemoryObservation { outside_bytes: 100, psi_blocked: false }));
    reconciler.refresh_seat_admission_at(1002);
    assert!(events.has_changed().unwrap(), "changed outside bytes wake incremental fit evaluation");
    assert_eq!(*reconciler.seat_admission.outside_observation.lock().unwrap(), Some(outside()));
    events.borrow_and_update();
    *reconciler.seat_admission.outside_observation.lock().unwrap() =
        Some(Some(HostMemoryObservation { outside_bytes: 42, psi_blocked: true }));
    reconciler.refresh_seat_admission_at(1003);
    assert!(events.has_changed().unwrap(), "changed PSI attribution also wakes the same observation dependency");
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
