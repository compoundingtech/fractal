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
fn independent_samples_release_while_reconcile_passes_are_17_to_27_seconds_apart() {
    let policy = Policy::default();
    let period = u64::try_from(policy.sample_interval().as_millis()).unwrap();
    let mut brake = Brake::new(policy).unwrap();
    // Missing data closes admission. Only the independent healthy samples recover it.
    assert_eq!(brake.observe(&[], 0).reason, Some(Reason::UnknownObservation));
    let reconcile_times = [17000, 44000, 61000, 88000, 105000, 132000];
    let mut sampled = 0;
    for now in reconcile_times {
        while sampled + period <= now {
            sampled += period;
            brake.observe(&readings(sampled, 0, 0), sampled);
        }
        // Admission may also consult the shared observer between periodic samples.
        let decision = brake.observe(&readings(sampled, 0, 0), now);
        assert_eq!(decision.blocked, now < 122500, "reconcile at {now}ms");
        if decision.blocked {
            assert_eq!(decision.reason, Some(Reason::Recovering));
        }
    }
}

#[test]
fn healthy_recovery_is_distinct_from_the_original_pressure_signal() {
    let mut brake = Brake::new(Policy::default()).unwrap();
    for now in (0..=60000).step_by(2500) {
        brake.observe(&readings(now, 1000, 0), now);
    }
    assert_eq!(brake.observe(&readings(62500, 0, 0), 62500).reason,
        Some(Reason::Recovering));
    assert_eq!(brake.observe(&readings(65000, 1000, 0), 65000).reason,
        Some(Reason::Pressure(Signal::MemoryFull)));
}

#[test]
fn configured_signal_window_threshold_and_timings_are_used() {
    let policy = Policy {
        rules: vec![Rule { signal: Signal::IoFull, window: Window::Avg10,
            threshold_millipercent: 6000, resume_millipercent: 2000 }],
        sustained_ms: 10, resume_ms: 20, freshness_ms: 100,
        ..Policy::default()
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
    let meminfo = directory.path().join("meminfo");
    std::fs::write(&meminfo, "MemAvailable: 8388608 kB\n").unwrap();
    std::fs::write(&memory,
        "some avg10=6.81 avg60=4.90 avg300=1.86 total=4781940413\nfull avg10=4.07 avg60=3.14 avg300=1.28 total=4654995392\n").unwrap();
    std::fs::write(&io,
        "some avg10=2.07 avg60=1.42 avg300=3.52 total=36086872246\nfull avg10=0.00 avg60=0.13 avg300=2.89 total=34379897790\n").unwrap();
    let policy = Policy::default();
    let mut reader = Reader::default();
    let observed = reader.read_paths(&policy, 123, &memory, &io, &meminfo).unwrap();
    assert_eq!(observed, &readings(123, 3140, 1420));
    std::fs::remove_file(&io).unwrap();
    assert!(reader.read_paths(&policy, 124, &memory, &io, &meminfo).is_err());
    std::fs::write(&io, "full avg60=0.00\n").unwrap();
    assert!(reader.read_paths(&policy, 125, &memory, &io, &meminfo).is_err());
    std::fs::write(&io, "some avg60=1.00\nsome avg60=2.00\n").unwrap();
    assert!(reader.read_paths(&policy, 126, &memory, &io, &meminfo).is_err());
}

#[test]
fn omitted_policy_durations_use_declared_defaults_without_ignoring_unknown_fields() {
    let policy: Policy = serde_json::from_str("{\"sustainedMs\":15000}").unwrap();
    assert_eq!(policy.sustained_ms, 15000);
    assert_eq!(policy.resume_ms, 120000);
    assert_eq!(policy.memory_available_floor_bytes, 2 << 30);
    assert_eq!(policy.rules, Policy::default().rules);
    assert!(serde_json::from_str::<Policy>("{\"tripWhenever\":true}").is_err());
}

struct PressureFixture {
    root: tempfile::TempDir,
    reader: Reader,
    brake: Brake,
    policy: Policy,
}

impl PressureFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for path in ["proc/pressure", "sys/fs/cgroup/app.slice/app-lanes.slice",
            "sys/fs/cgroup/system.slice/capped.service", "sys/fs/cgroup/quiet.slice"]
        {
            std::fs::create_dir_all(root.path().join(path)).unwrap();
        }
        let mut fixture = Self { root, reader: Reader::default(),
            brake: Brake::new(Policy::default()).unwrap(), policy: Policy::default() };
        fixture.write("proc/pressure/memory", "some avg60=5.0\nfull avg60=3.0\n");
        fixture.write("proc/pressure/io", "some avg60=0.0\n");
        fixture.write("sys/fs/cgroup/system.slice/capped.service/memory.high", "1073741824\n");
        fixture.write("sys/fs/cgroup/system.slice/capped.service/memory.events", "high 100\n");
        fixture.write("sys/fs/cgroup/system.slice/capped.service/memory.pressure",
            "some avg60=35.0\nfull avg60=29.0\n");
        fixture.write("sys/fs/cgroup/quiet.slice/memory.pressure", "full avg60=0.0\n");
        fixture.write("sys/fs/cgroup/app.slice/app-lanes.slice/memory.events", "high 0\n");
        fixture.lane("0.0");
        fixture.available(8 << 30);
        fixture
    }

    fn write(&self, path: &str, value: &str) {
        std::fs::write(self.root.path().join(path), value).unwrap();
    }

    fn lane(&mut self, pressure: &str) {
        self.write("sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure",
            &format!("some avg60={pressure}\nfull avg60={pressure}\n"));
    }

    fn available(&mut self, bytes: u64) {
        self.write("proc/meminfo", &format!("MemAvailable: {} kB\n", bytes / 1024));
    }

    fn observe(&mut self, now: u64) -> Decision {
        let root = self.root.path();
        match self.reader.read_paths(&self.policy, now,
            &root.join("sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure"),
            &root.join("proc/pressure/io"), &root.join("proc/meminfo"))
        {
            Ok(readings) => self.brake.observe(readings, now),
            Err(_) => self.brake.observe(&[], now),
        }
    }
}

#[test]
fn capped_service_self_throttling_does_not_trip_quiet_lanes_with_available_memory() {
    let mut fixture = PressureFixture::new();
    for now in (0..=180000).step_by(2500) {
        fixture.write("sys/fs/cgroup/system.slice/capped.service/memory.events",
            &format!("high {}\n", 100 + now));
        assert!(!fixture.observe(now).blocked);
    }
}

#[test]
fn host_shortage_or_lane_pressure_trips_and_requires_sustained_recovery() {
    for lane_pressure in [false, true] {
        let mut fixture = PressureFixture::new();
        if lane_pressure { fixture.lane("1.0"); } else { fixture.available(1 << 30); }
        for now in (0..60000).step_by(2500) {
            assert!(!fixture.observe(now).blocked);
        }
        assert_eq!(fixture.observe(60000).reason, Some(Reason::Pressure(Signal::MemoryFull)));
        fixture.lane("0.49");
        fixture.available(2 << 30); // The absolute floor itself is healthy.
        for now in (62500..182500).step_by(2500) {
            assert!(fixture.observe(now).blocked);
        }
        assert!(!fixture.observe(182500).blocked);
    }
}

#[test]
fn missing_malformed_and_stale_fixture_observations_fail_closed() {
    for path in ["proc/meminfo", "proc/pressure/io",
        "sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure"]
    {
        let mut fixture = PressureFixture::new();
        assert!(!fixture.observe(0).blocked);
        std::fs::remove_file(fixture.root.path().join(path)).unwrap();
        assert_eq!(fixture.observe(2500).reason, Some(Reason::UnknownObservation));
    }
    for meminfo in ["MemFree: 8000000 kB\n", "MemAvailable: NaN kB\n",
        "MemAvailable: 8000000 MB\n", "MemAvailable: 1 kB\nMemAvailable: 8000000 kB\n"]
    {
        let mut fixture = PressureFixture::new();
        fixture.write("proc/meminfo", meminfo);
        assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation));
    }
    let mut fixture = PressureFixture::new();
    assert!(!fixture.observe(0).blocked);
    assert_eq!(fixture.brake.observe(&fixture.reader.readings, 10001).reason,
        Some(Reason::UnknownObservation));
    assert_eq!(fixture.observe(12500).reason, Some(Reason::Recovering));
}

#[test]
fn absent_lane_slice_is_quiet_but_host_memory_and_io_still_protect_admission() {
    for shortage in [None, Some(Signal::MemoryFull), Some(Signal::IoSome)] {
        let mut fixture = PressureFixture::new();
        std::fs::remove_dir_all(fixture.root.path()
            .join("sys/fs/cgroup/app.slice/app-lanes.slice")).unwrap();
        match shortage {
            Some(Signal::MemoryFull) => fixture.available(1 << 30),
            Some(Signal::IoSome) => fixture.write("proc/pressure/io", "some avg60=25.0\n"),
            None => {},
            _ => unreachable!(),
        }
        for now in (0..60000).step_by(2500) {
            assert!(!fixture.observe(now).blocked);
        }
        let decision = fixture.observe(60000);
        assert_eq!(decision.blocked, shortage.is_some());
        assert_eq!(decision.reason, shortage.map(Reason::Pressure));
    }
}

#[test]
fn absent_lane_does_not_substitute_for_missing_host_or_existing_lane_observations() {
    for path in ["proc/meminfo", "proc/pressure/io", "sys/fs/cgroup/app.slice"] {
        let mut fixture = PressureFixture::new();
        std::fs::remove_dir_all(fixture.root.path()
            .join("sys/fs/cgroup/app.slice/app-lanes.slice")).unwrap();
        if path.ends_with("app.slice") {
            std::fs::remove_dir(fixture.root.path().join(path)).unwrap();
        } else {
            std::fs::remove_file(fixture.root.path().join(path)).unwrap();
        }
        assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation));
    }
    let mut fixture = PressureFixture::new();
    fixture.write("sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure",
        "some avg60=0.0\nfull avg60=NaN\n");
    assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation));
}
