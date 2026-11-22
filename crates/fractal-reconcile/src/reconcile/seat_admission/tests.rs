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
async fn admission_refresh_does_not_schedule_pressure_polling() {
    let reconciler = reconciler();
    reconciler.refresh_seat_admission_at(u64::try_from(now_ms() / 1000).unwrap());
    assert!(reconciler.seat_admission.pressure.lock().unwrap().is_none());
    assert!(!reconciler.step_deadlines.lock().unwrap().contains_key("resource-pressure"));
}


#[tokio::test]
async fn daemon_sampler_observes_without_a_pass_or_native_lanes() {
    let reconciler = Arc::new(reconciler());
    assert!(reconciler.seat_admission.lanes.is_none());
    assert!(reconciler.seat_admission.config.is_none());
    assert!(reconciler.seat_admission.pressure.lock().unwrap().is_none());
    let events = reconciler.event_notify.subscribe();
    let sampler = reconciler.sample_resource_pressure();
    tokio::pin!(sampler);
    if cfg!(target_os = "linux") {
        tokio::select! {
            () = &mut sampler => panic!("sampler must remain scoped to the daemon run"),
            () = reconciler.notify.notified() => {},
            () = tokio::time::sleep(Duration::from_secs(10)) => panic!("sampler did not observe independently"),
        }
        assert!(reconciler.seat_admission.pressure.lock().unwrap().is_some());
        assert!(!reconciler.step_deadlines.lock().unwrap().contains_key("resource-pressure"));
        assert!(!events.has_changed().unwrap(), "sampling must not publish a graph mutation");
    } else {
        tokio::select! {
            () = &mut sampler => panic!("unsupported platforms keep the sampler dormant"),
            () = std::future::ready(()) => {},
        }
        assert!(reconciler.seat_admission.pressure.lock().unwrap().is_none());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn sampler_recovers_while_incremental_observe_holds_its_lock_past_freshness() {
    use crate::incremental::{ChangeFeed, ChangeSource};
    use fractal_runtime::resource_pressure::{Policy, Reason, Rule, Signal, Window};
    use std::time::Instant;

    struct StalledFeed {
        entered: tokio::sync::mpsc::UnboundedSender<Instant>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl ChangeSource for StalledFeed {
        fn changes_since(&self, _: u64, _: i64) -> Result<ChangeFeed> {
            self.entered.send(Instant::now()).unwrap();
            self.release.recv_timeout(Duration::from_secs(30)).unwrap();
            Ok(ChangeFeed::default())
        }
    }

    let mut reconciler = reconciler();
    // Inject kernel fixtures: this cadence regression must not depend on host pressure.
    let root = tempfile::tempdir().unwrap();
    let memory = root.path().join("memory.pressure");
    let io = root.path().join("io");
    let meminfo = root.path().join("meminfo");
    let root_memory = root.path().join("root-memory");
    let vmstat = root.path().join("vmstat");
    std::fs::write(&memory, "full avg10=0.00\n").unwrap();
    std::fs::write(&io, "some avg10=0.00\n").unwrap();
    std::fs::write(&meminfo, "MemTotal: 16777216 kB\nMemAvailable: 8388608 kB\n").unwrap();
    std::fs::write(&root_memory, "full avg10=0.00\n").unwrap();
    std::fs::write(&vmstat, "allocstall_normal 0\n").unwrap();
    reconciler.seat_admission.finite_policy.psi_brake = Policy {
        rules: vec![Rule { signal: Signal::MemoryFull, window: Window::Avg10,
            threshold_millipercent: 100000, resume_millipercent: 99999 }],
        sustained_ms: 1, resume_ms: 2000, freshness_ms: 1000,
        ..Policy::default()
    };
    *reconciler.seat_admission.pressure.lock().unwrap() =
        Some(fractal_runtime::resource_pressure::Observer::with_paths(
            reconciler.seat_admission.finite_policy.psi_brake.clone(),
            [memory, io, meminfo, root_memory, vmstat]).unwrap());
    assert!(!reconciler.seat_admission.pressure_decision().unwrap().blocked);
    let (samples, mut sampled) = tokio::sync::mpsc::unbounded_channel();
    reconciler.seat_admission.pressure_samples.set(samples).unwrap();
    let reconciler = Arc::new(reconciler);
    let incremental = reconciler.incremental.clone();
    let (entered, mut entering) = tokio::sync::mpsc::unbounded_channel();
    let (release, stalled) = std::sync::mpsc::channel();
    let observer = std::thread::spawn(move || {
        incremental.observe(&StalledFeed { entered, release: stalled }).unwrap();
    });
    let held_since = entering.recv().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let sampler = reconciler.sample_resource_pressure();
    tokio::pin!(sampler);
    let mut previous = None;
    let mut count = 0;
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let (at, decision) = tokio::select! {
                () = &mut sampler => panic!("sampler ended while the feed was stalled"),
                sample = sampled.recv() => sample.unwrap(),
            };
            if let Some(previous) = previous {
                assert!(at.duration_since(previous) <= Duration::from_millis(1000),
                    "incremental observation stalled the independent sampling cadence");
            }
            previous = Some(at);
            count += 1;
            if count == 1 {
                assert_eq!(decision.reason, Some(Reason::UnknownObservation));
            } else if decision.blocked {
                assert_eq!(decision.reason, Some(Reason::Recovering));
            } else {
                break;
            }
        }
    }).await.expect("the real Brake did not recover while changes_since held the incremental lock");
    assert!(count >= 10, "recovery requires continued periodic healthy observations");
    assert!(held_since.elapsed() > Duration::from_millis(1000));
    assert!(reconciler.seat_admission.pressure_dirty.load(Ordering::Acquire));

    release.send(()).unwrap();
    observer.join().unwrap();
    // Only the next pass drains the signal and takes incremental's lock.
    reconciler.refresh_seat_admission_at(0);
    assert!(!reconciler.seat_admission.pressure_dirty.load(Ordering::Acquire));
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
