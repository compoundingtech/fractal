use super::*;

fn readings(now: u64, memory: u32, io: u32) -> [Reading; 2] {
    [
        Reading { signal: Signal::MemoryFull, window: Window::Avg60,
            millipercent: memory, observed_at_ms: now, memory_shortage: false },
        Reading { signal: Signal::IoSome, window: Window::Avg60,
            millipercent: io, observed_at_ms: now, memory_shortage: false },
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
        millipercent: pressure, observed_at_ms: now, memory_shortage: false }];
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
    std::fs::write(&meminfo, "MemTotal: 16777216 kB\nMemAvailable: 8388608 kB\n").unwrap();
    let vmstat = directory.path().join("vmstat");
    std::fs::write(&vmstat, "allocstall_normal 0\npgscan_kswapd 0\n").unwrap();
    std::fs::write(&memory,
        "some avg10=6.81 avg60=4.90 avg300=1.86 total=4781940413\nfull avg10=4.07 avg60=3.14 avg300=1.28 total=4654995392\n").unwrap();
    std::fs::write(&io,
        "some avg10=2.07 avg60=1.42 avg300=3.52 total=36086872246\nfull avg10=0.00 avg60=0.13 avg300=2.89 total=34379897790\n").unwrap();
    let policy = Policy::default();
    let mut reader = Reader::default();
    let paths = KernelPaths { memory_path: Some(&memory), io_path: &io,
        meminfo_path: &meminfo, root_memory_path: &memory, vmstat_path: &vmstat };
    let observed = reader.read_paths(&policy, 123, paths).unwrap();
    assert_eq!(observed, &readings(123, 3140, 1420));
    std::fs::remove_file(&io).unwrap();
    assert!(reader.read_paths(&policy, 124, paths).is_err());
    std::fs::write(&io, "full avg60=0.00\n").unwrap();
    assert!(reader.read_paths(&policy, 125, paths).is_err());
    std::fs::write(&io, "some avg60=1.00\nsome avg60=2.00\n").unwrap();
    assert!(reader.read_paths(&policy, 126, paths).is_err());
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
        fixture.write("proc/vmstat", "allocstall_dma 0\nallocstall_normal 100\npgscan_kswapd 0\npgscan_direct 0\n");
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
        self.write("proc/meminfo", &format!("MemTotal: 16777216 kB\nMemAvailable: {} kB\n", bytes / 1024));
    }

    fn observe(&mut self, now: u64) -> Decision {
        let root = self.root.path();
        match self.reader.read_paths(&self.policy, now, KernelPaths {
            memory_path: Some(&root.join("sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure")),
            io_path: &root.join("proc/pressure/io"), meminfo_path: &root.join("proc/meminfo"),
            root_memory_path: &root.join("proc/pressure/memory"), vmstat_path: &root.join("proc/vmstat"),
        }) {
            Ok(readings) => self.brake.observe(readings, now),
            Err(_) => self.brake.observe(&[], now),
        }
    }

    fn observe_layout(&mut self, now: u64, placement: &str) -> Decision {
        let root = self.root.path();
        std::fs::create_dir_all(root.join("proc/self")).unwrap();
        self.write("proc/self/cgroup", placement);
        match self.reader.read_layout(&self.policy, now, &root.join("proc/self/cgroup"),
            &root.join("sys/fs/cgroup"), KernelPaths {
                memory_path: None, io_path: &root.join("proc/pressure/io"),
                meminfo_path: &root.join("proc/meminfo"),
                root_memory_path: &root.join("proc/pressure/memory"), vmstat_path: &root.join("proc/vmstat"),
            })
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
        // Background kswapd activity and memcg direct scans must not qualify root PSI.
        fixture.write("proc/vmstat", &format!(
            "allocstall_dma 0\nallocstall_normal 100\npgscan_kswapd {}\npgscan_direct {}\n",
            now / 2500, now));
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
    for path in ["proc/meminfo", "proc/pressure/io", "proc/pressure/memory", "proc/vmstat",
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
        fixture.write("proc/meminfo", &format!("MemTotal: 16777216 kB\n{meminfo}"));
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

#[test]
fn root_pressure_with_allocator_stalls_trips_even_when_lanes_are_idle_or_absent() {
    for absent in [false, true] {
        let mut fixture = PressureFixture::new();
        if absent {
            std::fs::remove_dir_all(fixture.root.path()
                .join("sys/fs/cgroup/app.slice/app-lanes.slice")).unwrap();
        }
        assert!(!fixture.observe(0).blocked); // First counter sample is a baseline.
        for now in (2500..62500).step_by(2500) {
            fixture.write("proc/vmstat", &format!("allocstall_dma {}\nallocstall_normal 100\n", now));
            assert!(!fixture.observe(now).blocked);
        }
        fixture.write("proc/vmstat", "allocstall_dma 62500\nallocstall_normal 100\n");
        assert_eq!(fixture.observe(62500).reason, Some(Reason::Pressure(Signal::MemoryFull)));
    }
}

#[test]
fn allocator_counter_reset_is_not_pressure_and_malformed_vmstat_is_unknown() {
    let mut fixture = PressureFixture::new();
    assert!(!fixture.observe(0).blocked);
    fixture.write("proc/vmstat", "allocstall_normal 0\n");
    for now in (2500..=65000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    for value in ["pgscan_kswapd 1\n", "allocstall_normal NaN\n", "allocstall_normal 1 extra\n",
        "allocstall_normal 1\nallocstall_normal 2\n", "allocstall_normal\n",
        "allocstall_normal 1", "allocstall_ 1\n"] {
        fixture.write("proc/vmstat", value);
        assert_eq!(fixture.observe(67500).reason, Some(Reason::UnknownObservation));
    }
}

#[test]
fn allocator_stalls_without_root_pressure_do_not_trip() {
    let mut fixture = PressureFixture::new();
    fixture.write("proc/pressure/memory", "full avg60=0.0\n");
    for now in (0..=65000).step_by(2500) {
        fixture.write("proc/vmstat", &format!("allocstall_normal {}\n", now));
        assert!(!fixture.observe(now).blocked);
    }
}

#[test]
fn io_only_policy_keeps_the_memory_floor_independent_and_zero_disables_it() {
    let mut fixture = PressureFixture::new();
    fixture.policy.rules.retain(|rule| rule.signal == Signal::IoSome);
    fixture.brake = Brake::new(fixture.policy.clone()).unwrap();
    fixture.available(1 << 30);
    for now in (0..60000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    assert_eq!(fixture.observe(60000).reason, Some(Reason::Pressure(Signal::MemoryFull)));
    std::fs::remove_file(fixture.root.path().join("proc/meminfo")).unwrap();
    assert_eq!(fixture.observe(62500).reason, Some(Reason::UnknownObservation));
    fixture.policy.memory_available_floor_bytes = 0;
    fixture.brake = Brake::new(fixture.policy.clone()).unwrap();
    assert!(!fixture.observe(65000).blocked);
}

#[test]
fn floor_at_or_above_total_memory_is_an_explicit_configuration_error() {
    let fixture = PressureFixture::new();
    for floor in [16 << 30, 17 << 30] {
        let mut policy = fixture.policy.clone();
        policy.memory_available_floor_bytes = floor;
        let root = fixture.root.path();
        let error = Observer::with_paths(policy.clone(), [
            root.join("sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure"),
            root.join("proc/pressure/io"), root.join("proc/meminfo"),
            root.join("proc/pressure/memory"), root.join("proc/vmstat"),
        ]).err().expect("invalid floor must be rejected at construction").to_string();
        assert!(error.contains("memoryAvailableFloorBytes") && error.contains("must be below MemTotal"));
    }
    let root = fixture.root.path();
    let mut observer = Observer::with_paths(fixture.policy.clone(), [
        root.join("sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure"),
        root.join("proc/pressure/io"), root.join("proc/meminfo"),
        root.join("proc/pressure/memory"), root.join("proc/vmstat"),
    ]).unwrap();
    fixture.write("proc/meminfo", "MemTotal: 2097152 kB\nMemAvailable: 1048576 kB\n");
    for _ in 0..2 {
        let error = observer.observe(&fixture.policy).unwrap_err().to_string();
        assert!(error.contains("memoryAvailableFloorBytes") && error.contains("must be below MemTotal"));
    }
}

#[test]
fn bursty_allocator_events_qualify_root_pressure_through_the_sustained_interval() {
    for cadence_ms in [5000, 20000] {
        let mut fixture = PressureFixture::new();
        assert!(!fixture.observe(0).blocked);
        for now in (2500..=cadence_ms + 60000).step_by(2500) {
            fixture.write("proc/vmstat", &format!("allocstall_normal {}\n", 100 + now / cadence_ms));
            let decision = fixture.observe(now);
            assert_eq!(decision.blocked, now >= cadence_ms + 60000);
        }
    }
}

#[test]
fn one_allocator_event_qualifies_a_long_stall_for_exactly_the_psi_window() {
    for window in [Window::Avg10, Window::Avg60, Window::Avg300] {
        let mut fixture = PressureFixture::new();
        fixture.policy.rules[0].window = window;
        fixture.brake = Brake::new(fixture.policy.clone()).unwrap();
        fixture.write("proc/pressure/memory", "full avg10=3.0 avg60=3.0 avg300=3.0\n");
        fixture.write("sys/fs/cgroup/app.slice/app-lanes.slice/memory.pressure",
            "full avg10=0.0 avg60=0.0 avg300=0.0\n");
        assert!(!fixture.observe(0).blocked); // No earlier increment is inferred.
        assert_eq!(fixture.reader.readings[0].millipercent, 0);
        fixture.write("proc/vmstat", "allocstall_normal 101\n");
        for now in (2500..=2500 + window.duration_ms()).step_by(2500) {
            fixture.observe(now);
            assert_eq!(fixture.reader.readings[0].millipercent, 3000);
        }
        fixture.observe(5000 + window.duration_ms());
        assert_eq!(fixture.reader.readings[0].millipercent, 0);
        // A reset clears even unexpired evidence, rather than qualifying the root.
        fixture.write("proc/vmstat", "allocstall_normal 102\n");
        fixture.observe(7500 + window.duration_ms());
        assert_eq!(fixture.reader.readings[0].millipercent, 3000);
        fixture.write("proc/vmstat", "allocstall_normal 0\n");
        fixture.observe(10000 + window.duration_ms());
        assert_eq!(fixture.reader.readings[0].millipercent, 0);
    }
}

struct WarningLog(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl tracing::Subscriber for WarningLog {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool { *metadata.level() == tracing::Level::WARN }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id { tracing::span::Id::from_u64(1) }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                write!(&mut self.0, "{}={value:?} ", field.name()).unwrap();
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
}

#[test]
fn unrecognized_layout_uses_host_signals_and_warns_only_once_with_the_placement() {
    let mut fixture = PressureFixture::new();
    let warnings = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let placement = "0::/system.slice/example-daemon.service\n";
    tracing::subscriber::with_default(WarningLog(warnings.clone()), || {
        for now in (0..=65000).step_by(2500) {
            assert!(!fixture.observe_layout(now, placement).blocked);
        }
    });
    let warnings = warnings.lock().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("/system.slice/example-daemon.service"));
    assert!(warnings[0].contains("not applicable"));
    fixture.available(1 << 30);
    for now in (67500..127500).step_by(2500) {
        assert!(!fixture.observe_layout(now, placement).blocked);
    }
    assert_eq!(fixture.observe_layout(127500, placement).reason,
        Some(Reason::Pressure(Signal::MemoryFull)));
}

#[test]
fn recognized_layout_with_unreadable_or_missing_lane_psi_fails_closed() {
    let mut fixture = PressureFixture::new();
    let placement = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/example.service\n";
    let pressure = fixture.root.path().join(
        "sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice/app-lanes.slice/memory.pressure");
    // A directory cannot be read as a PSI file, even when tests run as root.
    std::fs::create_dir_all(&pressure).unwrap();
    assert_eq!(fixture.observe_layout(0, placement).reason, Some(Reason::UnknownObservation));
    assert!(!fixture.reader.warned_lane_layout);
    std::fs::remove_dir(&pressure).unwrap();
    assert_eq!(fixture.observe_layout(2500, placement).reason, Some(Reason::UnknownObservation));
    std::fs::write(&pressure, "full avg60=NaN\n").unwrap();
    assert_eq!(fixture.observe_layout(5000, placement).reason, Some(Reason::UnknownObservation));
}
