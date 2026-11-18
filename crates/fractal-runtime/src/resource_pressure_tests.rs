use super::*;

fn readings(now: u64, memory: u32, io: u32) -> [Reading; 2] {
    [
        Reading { signal: Signal::MemoryFull, window: Window::Avg60,
            millipercent: memory, observed_at_ms: now },
        Reading { signal: Signal::IoSome, window: Window::Avg60,
            millipercent: io, observed_at_ms: now },
    ]
}

#[test]
fn initial_healthy_reading_opens_without_a_startup_sleep() {
    let mut brake = Brake::new(Policy::default()).unwrap();
    assert_eq!(brake.observe(&readings(0, 0, 0), 0), Decision { blocked: false, reason: None });
}

#[test]
fn sustained_trip_and_both_signal_recovery_have_separate_hysteresis() {
    let mut brake = Brake::new(Policy::default()).unwrap();
    for now in (0..60000).step_by(5000) {
        assert!(!brake.observe(&readings(now, 1000, 0), now).blocked);
    }
    assert_eq!(brake.observe(&readings(60000, 1000, 0), 60000),
        Decision { blocked: true, reason: Some(Reason::Pressure(Signal::MemoryFull)) });
    for now in (65000..=185000).step_by(5000) {
        // Memory recovered, but IO remains between its resume and trip thresholds.
        assert!(brake.observe(&readings(now, 0, 11000), now).blocked);
    }
    for now in (190000..310000).step_by(5000) {
        assert!(brake.observe(&readings(now, 0, 0), now).blocked);
    }
    assert!(!brake.observe(&readings(310000, 0, 0), 310000).blocked);
}

#[test]
fn short_spikes_and_alternating_signals_do_not_accumulate_trip_time() {
    let mut brake = Brake::new(Policy::default()).unwrap();
    for now in (0..=180000).step_by(5000) {
        let memory = if now % 10000 == 0 { 1000 } else { 0 };
        let io = if now % 10000 == 0 { 0 } else { 25000 };
        assert!(!brake.observe(&readings(now, memory, io), now).blocked);
    }
}

#[test]
fn missing_stale_future_duplicate_and_out_of_range_readings_never_mean_healthy() {
    let cases = [
        Vec::new(),
        readings(0, 0, 0).to_vec(), // stale at now=20000
        readings(30000, 0, 0).to_vec(), // future
        vec![readings(20000, 0, 0)[0]; 2],
        readings(20000, 100001, 0).to_vec(),
    ];
    for values in cases {
        let mut brake = Brake::new(Policy::default()).unwrap();
        assert_eq!(brake.observe(&values, 20000),
            Decision { blocked: true, reason: Some(Reason::UnknownObservation) });
        for now in (25000..145000).step_by(5000) {
            assert!(brake.observe(&readings(now, 0, 0), now).blocked);
        }
        assert!(!brake.observe(&readings(145000, 0, 0), 145000).blocked);
    }
}

#[test]
fn a_sampling_gap_or_backwards_clock_resets_continuous_evidence() {
    let mut brake = Brake::new(Policy::default()).unwrap();
    assert!(!brake.observe(&readings(0, 0, 0), 0).blocked);
    assert_eq!(brake.observe(&readings(20000, 0, 0), 20000).reason,
        Some(Reason::UnknownObservation));
    assert_eq!(brake.observe(&readings(10000, 0, 0), 10000).reason,
        Some(Reason::UnknownObservation));
}

#[test]
fn configured_signal_window_threshold_and_timings_are_used() {
    let policy = Policy {
        rules: vec![Rule { signal: Signal::IoFull, window: Window::Avg10,
            threshold_millipercent: 6000, resume_millipercent: 2000 }],
        sustained_ms: 10, resume_ms: 20, freshness_ms: 100,
    };
    let mut brake = Brake::new(policy).unwrap();
    let input = |now, pressure| [Reading { signal: Signal::IoFull, window: Window::Avg10,
        millipercent: pressure, observed_at_ms: now }];
    assert!(!brake.observe(&input(0, 6000), 0).blocked);
    assert!(brake.observe(&input(10, 6000), 10).blocked);
    assert!(brake.observe(&input(11, 0), 11).blocked);
    assert!(!brake.observe(&input(31, 0), 31).blocked);
}

#[test]
fn invalid_hysteresis_and_duplicate_rules_are_refused() {
    let mut policy = Policy::default();
    policy.rules[0].resume_millipercent = policy.rules[0].threshold_millipercent;
    assert!(Brake::new(policy).is_err());
    let mut policy = Policy::default();
    policy.rules.push(policy.rules[0]);
    assert!(Brake::new(policy).is_err());
    assert!(Brake::new(Policy { rules: vec![], ..Policy::default() }).is_err());
}

#[test]
fn kernel_decimal_pressure_parser_preserves_threshold_precision() {
    let line = "full avg10=4.07 avg60=3.14 avg300=1.28 total=4654995392";
    assert_eq!(parse_millipercent(line, Window::Avg60).unwrap(), 3140);
    assert_eq!(parse_millipercent("some avg60=0.999", Window::Avg60).unwrap(), 999);
    assert_eq!(parse_millipercent("some avg60=1", Window::Avg60).unwrap(), 1000);
    for invalid in ["some avg60=NaN", "some avg60=-1.0", "some avg60=100.01",
        "some avg60=1.0001", "some avg10=1.00", "some avg60=1 avg60=2"] {
        assert!(parse_millipercent(invalid, Window::Avg60).is_err());
    }
}

#[test]
fn host_reader_parses_real_files_without_substituting_missing_or_stale_data() {
    let directory = tempfile::tempdir().unwrap();
    let memory = directory.path().join("memory");
    let io = directory.path().join("io");
    std::fs::write(&memory,
        "some avg10=6.81 avg60=4.90 avg300=1.86 total=4781940413\nfull avg10=4.07 avg60=3.14 avg300=1.28 total=4654995392\n").unwrap();
    std::fs::write(&io,
        "some avg10=2.07 avg60=1.42 avg300=3.52 total=36086872246\nfull avg10=0.00 avg60=0.13 avg300=2.89 total=34379897790\n").unwrap();
    let policy = Policy::default();
    let mut reader = Reader::default();
    let observed = reader.read_paths(&policy, 123, &memory, &io).unwrap();
    assert_eq!(observed, &readings(123, 3140, 1420));
    std::fs::remove_file(&io).unwrap();
    assert!(reader.read_paths(&policy, 124, &memory, &io).is_err());
    std::fs::write(&io, "full avg60=0.00\n").unwrap();
    assert!(reader.read_paths(&policy, 125, &memory, &io).is_err());
    std::fs::write(&io, "some avg60=1.00\nsome avg60=2.00\n").unwrap();
    assert!(reader.read_paths(&policy, 126, &memory, &io).is_err());
}

#[test]
fn omitted_policy_durations_use_declared_defaults_without_ignoring_unknown_fields() {
    let policy: Policy = serde_json::from_str("{\"sustainedMs\":15000}").unwrap();
    assert_eq!(policy.sustained_ms, 15000);
    assert_eq!(policy.resume_ms, 120000);
    assert_eq!(policy.rules, Policy::default().rules);
    assert!(serde_json::from_str::<Policy>("{\"tripWhenever\":true}").is_err());
}
