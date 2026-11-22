use super::*;

fn readings(now: u64, memory: u32, io: u32) -> [Reading; 2] {
    [
        Reading { signal: Signal::MemoryFull, window: Window::Avg60,
            millipercent: memory, observed_at_ms: now, memory_shortage: false, memory_available_bytes: None, cgroup: None },
        Reading { signal: Signal::IoSome, window: Window::Avg60,
            millipercent: io, observed_at_ms: now, memory_shortage: false, memory_available_bytes: None, cgroup: None },
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
        Decision { blocked: true, reason: Some(Reason::Pressure {
            signal: Signal::MemoryFull, value: 1000, threshold: 1000, cgroup: None,
        }) });
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
        vec![readings(20000, 0, 0)[0].clone(); 2],
        readings(20000, 100001, 0).to_vec(),
    ];
    for values in cases {
        let mut brake = Brake::new(Policy::default()).unwrap();
        assert_eq!(brake.observe(&values, 20000),
            Decision { blocked: true, reason: Some(Reason::UnknownObservation {
                cause: ObservationCause::MissingOrInvalid,
            }) });
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
        Some(Reason::UnknownObservation { cause: ObservationCause::ObservationGap }));
    assert_eq!(brake.observe(&readings(10000, 0, 0), 10000).reason,
        Some(Reason::UnknownObservation { cause: ObservationCause::ObservationGap }));
}

#[test]
fn independent_samples_release_while_reconcile_passes_are_17_to_27_seconds_apart() {
    let policy = Policy::default();
    let period = u64::try_from(policy.sample_interval().as_millis()).unwrap();
    let mut brake = Brake::new(policy).unwrap();
    // Missing data closes admission. Only the independent healthy samples recover it.
    assert_eq!(brake.observe(&[], 0).reason,
        Some(Reason::UnknownObservation { cause: ObservationCause::MissingOrInvalid }));
    let reconcile_times = [17000, 44000, 61000, 88000, 105000, 132000];
    let mut sampled = 0;
    let mut decision = Decision { blocked: true, reason: None };
    for now in reconcile_times {
        while sampled + period <= now {
            sampled += period;
            decision = brake.observe(&readings(sampled, 0, 0), sampled);
        }
        // Admission reads the last sampler decision without adding observations.
        assert_eq!(decision.blocked, now < 125000, "reconcile at {now}ms");
        if decision.blocked {
            assert_eq!(decision.reason, Some(Reason::Recovering { remaining: 125000 - sampled }));
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
        Some(Reason::Recovering { remaining: 120000 }));
    assert_eq!(brake.observe(&readings(65000, 1000, 0), 65000).reason,
        Some(Reason::Pressure { signal: Signal::MemoryFull, value: 1000, threshold: 1000, cgroup: None }));
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
        millipercent: pressure, observed_at_ms: now, memory_shortage: false, memory_available_bytes: None, cgroup: None }];
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
fn omitted_policy_durations_use_declared_defaults_without_ignoring_unknown_fields() {
    let policy: Policy = serde_json::from_str("{\"sustainedMs\":15000}").unwrap();
    assert_eq!(policy.sustained_ms, 15000);
    assert_eq!(policy.resume_ms, 120000);
    assert_eq!(policy.memory_available_floor_bytes, 2 << 30);
    assert_eq!(policy.sample_interval(), std::time::Duration::from_secs(5));
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
        std::fs::create_dir_all(root.path().join("proc/self")).unwrap();
        let fixture = Self { root, reader: Reader::default(),
            brake: Brake::new(Policy::default()).unwrap(), policy: Policy::default() };
        fixture.write("proc/self/cgroup", "0::/system.slice/fractal.service\n");
        fixture.available(8 << 30);
        for path in ["system.slice", "system.slice/fractal.service", "app.slice",
            "interactive.slice", "app.slice/editor.service", "app.slice/app-lanes.slice",
            "app.slice/gate.slice"]
        {
            fixture.cgroup(path, "0", "0");
        }
        fixture
    }

    fn write(&self, path: &str, value: &str) {
        std::fs::write(self.root.path().join(path), value).unwrap();
    }

    fn cgroup(&self, path: &str, memory: &str, io: &str) {
        let path = self.root.path().join("cgroup").join(path);
        std::fs::create_dir_all(&path).unwrap();
        for (file, value) in [("memory.pressure", memory), ("io.pressure", io)] {
            std::fs::write(path.join(file),
                format!("some avg10={value} avg60={value} avg300={value}\nfull avg10={value} avg60={value} avg300={value}\n")).unwrap();
        }
    }

    fn available(&self, bytes: u64) {
        self.write("proc/meminfo", &format!("MemTotal: 16777216 kB\nMemAvailable: {} kB\n", bytes / 1024));
    }

    fn observe(&mut self, now: u64) -> Decision {
        let root = self.root.path();
        match self.reader.read_layout(&self.policy, now, &root.join("proc/self/cgroup"),
            &root.join("cgroup"), &root.join("proc/meminfo"))
        {
            Ok(readings) => self.brake.observe(readings, now),
            Err(error) => {
                self.brake.last_observation = Some(now);
                self.brake.unknown(ObservationCause::from_error(&error))
            }
        }
    }

    fn observer(&self) -> Result<Observer> {
        Observer::with_paths(self.policy.clone(), self.root.path().join("proc/self/cgroup"),
            self.root.path().join("cgroup"), self.root.path().join("proc/meminfo"))
    }
}

#[test]
fn admitted_high_24_5_job_does_not_contaminate_protected_memory_or_io() {
    let mut fixture = PressureFixture::new();
    fixture.cgroup("app.slice/app-lanes.slice", "24.5", "98");
    fixture.cgroup("app.slice/gate.slice", "80", "90");
    // Ancestor PSI aggregates the jobs and must never be used for either signal.
    fixture.cgroup("app.slice", "24.5", "98");
    fixture.write("cgroup/app.slice/app-lanes.slice/memory.high", "1073741824\n");
    fixture.write("cgroup/app.slice/app-lanes.slice/memory.events", "high 100\n");
    for now in (0..=180000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
        assert!(fixture.reader.readings.iter().all(|reading| reading.millipercent == 0));
    }
}

#[test]
fn nested_heavy_job_parent_with_polluted_ancestors_stays_quiet_through_removal_and_restart() {
    let mut fixture = PressureFixture::new();
    // dev5 shape: jobs nest under app.slice/app-capped.slice/app-capped-heavy.slice,
    // polluting every ancestor aggregate along the configured lexical path.
    let heavy = "app.slice/app-capped.slice/app-capped-heavy.slice";
    fixture.cgroup(heavy, "24.5", "90");
    fixture.cgroup("app.slice/app-capped.slice", "24.5", "90");
    fixture.cgroup("app.slice", "24.5", "90");
    fixture.cgroup("app.slice/app-capped.slice/editor.service", "0", "0");
    // The whole job-parent subtree is excluded, scope jobs included.
    fixture.cgroup(&format!("{heavy}/gate-slot-heavy.scope"), "24.5", "90");
    for now in (0..60000).step_by(2500) {
        assert!(!fixture.observe(now).blocked, "polluted ancestors stayed quiet at {now}ms");
    }
    assert!(fixture.reader.readings.iter().all(|reading| reading.millipercent == 0));
    // Removing the endpoint never falls back to the contaminated ancestor aggregates.
    std::fs::remove_dir_all(fixture.root.path().join(format!("cgroup/{heavy}"))).unwrap();
    for now in (60000..120000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    // A fresh Reader without any observed-child cache agrees after the removal.
    fixture.reader = Reader::default();
    assert!(!fixture.observe(120000).blocked);
    // Recovery remains reachable for genuinely protected siblings.
    fixture.cgroup("app.slice/editor.service", "3.14", "0");
    for now in (122500..182500).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    assert_eq!(fixture.observe(182500).reason, Some(Reason::Pressure {
        signal: Signal::MemoryFull, value: 3140, threshold: 1000,
        cgroup: Some("/app.slice/editor.service".into()),
    }));
    fixture.cgroup("app.slice/editor.service", "0", "0");
    for now in (185000..302500).step_by(2500) {
        assert!(fixture.observe(now).blocked);
    }
    assert!(!fixture.observe(305000).blocked);
}

#[test]
fn empty_job_parent_with_high_aggregate_emits_zero_without_failing_closed() {
    let mut fixture = PressureFixture::new();
    for child in ["app-lanes.slice", "editor.service", "gate.slice"] {
        std::fs::remove_dir_all(fixture.root.path().join(format!("cgroup/app.slice/{child}"))).unwrap();
    }
    // No configured job-parent endpoint exists under the high protected ancestor.
    fixture.cgroup("app.slice", "24.5", "90");
    assert!(!fixture.observe(0).blocked);
    assert!(fixture.reader.readings.iter().all(|reading|
        reading.millipercent == 0 && reading.cgroup.is_none()));
    // An existing but empty job parent with high pressure emits nothing either.
    fixture.cgroup("app.slice/app-capped.slice", "24.5", "90");
    assert!(!fixture.observe(2500).blocked);
    assert!(fixture.reader.readings.iter().all(|reading| reading.millipercent == 0));
}

#[test]
fn discovered_interactive_slice_pressure_is_sampled_when_present() {
    let mut fixture = PressureFixture::new();
    fixture.cgroup("interactive.slice", "3.14", "0");
    for now in (0..60000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    assert_eq!(fixture.observe(60000).reason, Some(Reason::Pressure {
        signal: Signal::MemoryFull, value: 3140, threshold: 1000,
        cgroup: Some("/interactive.slice".into()),
    }));
}

#[test]
fn protected_pressure_trips_with_maximum_value_and_source_for_each_signal() {
    for (signal, memory, io, value, threshold) in [
        (Signal::MemoryFull, "3.14", "0", 3140, 1000),
        (Signal::IoSome, "0", "31.4", 31400, 25000),
    ] {
        let mut fixture = PressureFixture::new();
        fixture.cgroup("app.slice/editor.service", memory, io);
        for now in (0..60000).step_by(2500) {
            assert!(!fixture.observe(now).blocked);
        }
        assert_eq!(fixture.observe(60000).reason, Some(Reason::Pressure {
            signal, value, threshold, cgroup: Some("/app.slice/editor.service".into()),
        }));
        fixture.cgroup("app.slice/editor.service", "0", "0");
        for now in (62500..182500).step_by(2500) {
            assert!(fixture.observe(now).blocked);
        }
        assert!(!fixture.observe(182500).blocked);
    }
}

#[test]
fn protected_values_are_maxima_not_sums_for_every_configured_window() {
    let mut fixture = PressureFixture::new();
    fixture.policy.rules = [Window::Avg10, Window::Avg60, Window::Avg300].into_iter()
        .flat_map(|window| [
            Rule { signal: Signal::MemoryFull, window, threshold_millipercent: 1000, resume_millipercent: 500 },
            Rule { signal: Signal::IoSome, window, threshold_millipercent: 25000, resume_millipercent: 10000 },
        ]).collect();
    fixture.brake = Brake::new(fixture.policy.clone()).unwrap();
    fixture.cgroup("system.slice", "0.7", "18");
    fixture.cgroup("system.slice/fractal.service", "0.6", "16");
    fixture.cgroup("app.slice/editor.service", "0.8", "19");
    for now in (0..=65000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    for reading in &fixture.reader.readings {
        assert_eq!(reading.millipercent, if reading.signal == Signal::MemoryFull { 800 } else { 19000 });
        assert_eq!(reading.cgroup.as_deref(), Some("/app.slice/editor.service"));
    }
}

#[test]
fn existing_unreadable_or_missing_protected_psi_fails_closed_with_path() {
    for file in ["memory.pressure", "io.pressure"] {
        for unreadable in [false, true] {
            let mut fixture = PressureFixture::new();
            let path = fixture.root.path().join("cgroup/app.slice/editor.service").join(file);
            std::fs::remove_file(&path).unwrap();
            if unreadable { std::fs::create_dir(&path).unwrap(); }
            assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation {
                cause: ObservationCause::Io {
                    os_error: Some(if unreadable { libc::EISDIR } else { libc::ENOENT }),
                    path: Some(path.to_string_lossy().into()),
                },
            }));
        }
    }
}

#[test]
fn malformed_protected_psi_identifies_the_file() {
    let mut fixture = PressureFixture::new();
    let file = "cgroup/app.slice/editor.service/io.pressure";
    fixture.write(file, "some avg60=NaN\n");
    assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation {
        cause: ObservationCause::InvalidPressure {
            path: fixture.root.path().join(file).to_string_lossy().into(),
        },
    }));
}

#[test]
fn vanished_snapshot_child_is_skipped_only_when_its_cgroup_is_gone() {
    let mut fixture = PressureFixture::new();
    assert!(!fixture.observe(0).blocked);
    let root = fixture.root.path().join("cgroup");
    let children = Reader::children(&root.join("app.slice"), &mut Enumeration::default()).unwrap();
    let child = children.into_iter().find(|path| path.ends_with("editor.service")).unwrap();
    std::fs::remove_dir_all(&child).unwrap();
    fixture.reader.sample_cgroup(&fixture.policy, &root, &child, true).unwrap();
    std::fs::create_dir(&child).unwrap();
    let error = fixture.reader.sample_cgroup(&fixture.policy, &root, &child, true).unwrap_err();
    assert_eq!(ObservationCause::from_error(&error), ObservationCause::Io {
        os_error: Some(libc::ENOENT),
        path: Some(child.join("memory.pressure").to_string_lossy().into()),
    });
}

#[test]
fn every_sample_rediscovers_new_children_and_exclusions() {
    let mut fixture = PressureFixture::new();
    std::fs::remove_dir_all(fixture.root.path().join("cgroup/app.slice/app-lanes.slice")).unwrap();
    std::fs::remove_dir_all(fixture.root.path().join("cgroup/app.slice/gate.slice")).unwrap();
    assert!(!fixture.observe(0).blocked);
    fixture.cgroup("app.slice/app-lanes.slice", "24.5", "90");
    fixture.cgroup("app.slice", "24.5", "90");
    assert!(!fixture.observe(2500).blocked);
    fixture.cgroup("app.slice/new.service", "3", "40");
    fixture.observe(5000);
    assert_eq!(fixture.reader.readings[0].cgroup.as_deref(), Some("/app.slice/new.service"));
    assert_eq!(fixture.reader.readings[1].millipercent, 40000);
}

#[test]
fn cgroup_enumeration_is_bounded_and_names_the_limit() {
    let mut fixture = PressureFixture::new();
    fixture.policy.protected_cgroups = vec!["bounded.slice".into()];
    // Only the job-parent frontier enumerates children; keep the depth-one bound meaningful.
    fixture.policy.job_parent_cgroups = vec!["/bounded.slice/jobs".into()];
    for index in 0..=MAX_CGROUPS {
        std::fs::create_dir_all(fixture.root.path().join(format!("cgroup/bounded.slice/child{index}"))).unwrap();
    }
    assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation {
        cause: ObservationCause::TooManyCgroups {
            path: fixture.root.path().join("cgroup/bounded.slice").to_string_lossy().into(),
            limit: 256,
        },
    }));
}

#[test]
fn frontier_enumeration_is_bounded_by_raw_entries_too() {
    let mut fixture = PressureFixture::new();
    fixture.policy.protected_cgroups = vec!["bounded.entries.slice".into()];
    fixture.policy.job_parent_cgroups = vec!["/bounded.entries.slice/jobs".into()];
    let root = fixture.root.path().join("cgroup/bounded.entries.slice");
    std::fs::create_dir_all(&root).unwrap();
    // Control files count toward the raw-entry budget, not the directory budget.
    for index in 0..=MAX_CGROUP_ENTRIES {
        std::fs::write(root.join(format!("entry{index}")), "").unwrap();
    }
    assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation {
        cause: ObservationCause::TooManyCgroupEntries {
            path: root.to_string_lossy().into(), limit: 4096,
        },
    }));
}

#[test]
fn defaults_discover_service_system_and_interactive_app_slice() {
    let mut fixture = PressureFixture::new();
    let mount = fixture.root.path().join("cgroup");
    let placement = fixture.root.path().join("proc/self/cgroup");
    assert_eq!(fixture.reader.protected(&fixture.policy, &placement, &mount).unwrap(), vec![
        mount.join("app.slice"), mount.join("interactive.slice"), mount.join("system.slice"),
        mount.join("system.slice/fractal.service"),
    ]);
    fixture.write("proc/self/cgroup",
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/fractal.service/worker\n");
    fixture.cgroup("user.slice/user-1000.slice/user@1000.service/interactive.slice", "0", "0");
    let found = fixture.reader.protected(&fixture.policy, &placement, &mount).unwrap();
    assert!(found.contains(&mount.join("user.slice/user-1000.slice/user@1000.service/app.slice")));
    assert!(found.contains(&mount.join(
        "user.slice/user-1000.slice/user@1000.service/app.slice/fractal.service")));
    assert!(found.contains(&mount.join(
        "user.slice/user-1000.slice/user@1000.service/interactive.slice")));
    assert_eq!(fixture.policy.job_parent_cgroups, vec![
        "gate.slice".into(), "app-lanes.slice".into(), "app-capped.slice".into(),
        "app-capped-heavy.slice".into(), "app-batch.slice".into(), "app-batch-heavy.slice".into(),
    ]);
    assert_eq!(fixture.policy.excluded_cgroups, vec![PathBuf::from("app-lanes.slice"), PathBuf::from("gate.slice")]);
}

#[test]
fn configured_protection_and_absolute_job_parents_use_mount_relative_paths() {
    let mut fixture = PressureFixture::new();
    fixture.policy.protected_cgroups = vec!["/custom.slice".into()];
    // Absolute job parents are mount-relative exact paths of arbitrary depth.
    fixture.policy.job_parent_cgroups = vec!["/custom.slice/nested/jobs".into()];
    fixture.cgroup("custom.slice", "99", "99");
    fixture.cgroup("custom.slice/nested/jobs", "99", "99");
    fixture.cgroup("custom.slice/editor", "0.1", "0.2");
    fixture.write("proc/self/cgroup", "not needed for explicit protection");
    assert!(!fixture.observe(0).blocked);
    // The polluted ancestor is never sampled; only the quiet sibling leaf is.
    assert_eq!(fixture.reader.readings[0].millipercent, 100);
    assert_eq!(fixture.reader.readings[1].millipercent, 200);
    assert_eq!(fixture.reader.readings[0].cgroup.as_deref(), Some("/custom.slice/editor"));
}

#[test]
fn deep_absolute_exclusion_is_honored_by_descent_without_failing_closed() {
    let mut fixture = PressureFixture::new();
    fixture.policy.protected_cgroups = vec!["/custom.slice".into()];
    fixture.policy.excluded_cgroups = vec!["/custom.slice/nested/jobs".into()];
    fixture.cgroup("custom.slice", "99", "99");
    fixture.cgroup("custom.slice/nested/jobs", "99", "99");
    fixture.cgroup("custom.slice/editor", "0.1", "0.2");
    fixture.write("proc/self/cgroup", "not needed for explicit protection");
    assert!(!fixture.observe(0).blocked);
    // Exclusions drive splits like job parents: nested is recursed, jobs skipped whole.
    assert_eq!(fixture.reader.readings[0].millipercent, 100);
    assert_eq!(fixture.reader.readings[1].millipercent, 200);
    assert_eq!(fixture.reader.readings[0].cgroup.as_deref(), Some("/custom.slice/editor"));
}

#[test]
fn malformed_or_non_unified_placement_fails_closed() {
    for placement in ["", "garbage\n", "1:memory:/system.slice/example.service\n",
        "0::relative/path\n", "0::/../system.slice/example.service\n",
        "0::/system.slice/one.service\n0::/system.slice/two.service\n"]
    {
        let mut fixture = PressureFixture::new();
        fixture.write("proc/self/cgroup", placement);
        assert_eq!(fixture.observe(0).reason,
            Some(Reason::UnknownObservation { cause: ObservationCause::MissingOrInvalid }));
    }
}

#[test]
fn independent_memory_floor_survives_io_only_policy_and_zero_disables_it() {
    let mut fixture = PressureFixture::new();
    fixture.policy.rules.retain(|rule| rule.signal == Signal::IoSome);
    fixture.brake = Brake::new(fixture.policy.clone()).unwrap();
    fixture.available(1 << 30);
    for now in (0..60000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    assert_eq!(fixture.observe(60000).reason, Some(Reason::Pressure {
        signal: Signal::MemoryAvailable, value: 1 << 30, threshold: 2 << 30, cgroup: None,
    }));
    fixture.policy.memory_available_floor_bytes = 0;
    fixture.brake = Brake::new(fixture.policy.clone()).unwrap();
    std::fs::remove_file(fixture.root.path().join("proc/meminfo")).unwrap();
    assert!(!fixture.observe(62500).blocked);
}

#[test]
fn floor_at_or_above_total_memory_is_an_explicit_configuration_error() {
    let mut fixture = PressureFixture::new();
    for floor in [16 << 30, 17 << 30] {
        fixture.policy.memory_available_floor_bytes = floor;
        let error = fixture.observer().err().unwrap().to_string();
        assert!(error.contains("memoryAvailableFloorBytes") && error.contains("must be below MemTotal"));
    }
    fixture.policy = Policy::default();
    let mut observer = fixture.observer().unwrap();
    fixture.write("proc/meminfo", "MemTotal: 2097152 kB\nMemAvailable: 1048576 kB\n");
    assert!(observer.observe(&fixture.policy).is_err());
}

#[tokio::test(start_paused = true)]
async fn stopped_sampler_expires_cached_decision_without_reading_or_advancing_brake() {
    let fixture = PressureFixture::new();
    let mut observer = fixture.observer().unwrap();
    assert_eq!(observer.decision().reason,
        Some(Reason::UnknownObservation { cause: ObservationCause::NotYetSampled }));
    assert!(!observer.observe(&fixture.policy).unwrap().blocked);
    std::fs::remove_file(fixture.root.path().join("cgroup/app.slice/editor.service/io.pressure")).unwrap();
    tokio::time::advance(std::time::Duration::from_millis(fixture.policy.freshness_ms)).await;
    assert!(!observer.decision().blocked);
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    assert_eq!(observer.decision().reason,
        Some(Reason::UnknownObservation { cause: ObservationCause::StaleDecision }));
    assert!(!observer.brake.tripped, "reading stale decisions must not mutate the brake");
}

#[tokio::test(start_paused = true)]
async fn every_observe_reads_files_even_at_jittered_interval_and_preserves_error_cause() {
    let fixture = PressureFixture::new();
    let mut observer = fixture.observer().unwrap();
    assert!(!observer.observe(&fixture.policy).unwrap().blocked);
    let file = fixture.root.path().join("cgroup/app.slice/editor.service/io.pressure");
    std::fs::remove_file(&file).unwrap();
    tokio::time::advance(std::time::Duration::from_millis(4999)).await;
    let missing = observer.observe(&fixture.policy).unwrap();
    assert_eq!(missing.reason, Some(Reason::UnknownObservation {
        cause: ObservationCause::Io { os_error: Some(libc::ENOENT), path: Some(file.to_string_lossy().into()) },
    }));
    assert_eq!(observer.decision(), missing);
    fixture.cgroup("app.slice/editor.service", "0", "0");
    assert_eq!(observer.observe(&fixture.policy).unwrap().reason,
        Some(Reason::Recovering { remaining: fixture.policy.resume_ms }));
    for _ in 0..25 {
        tokio::time::advance(std::time::Duration::from_millis(4999)).await;
        observer.observe(&fixture.policy).unwrap();
    }
    assert!(!observer.decision().blocked);
}


#[test]
fn pressure_reasons_round_trip_with_structured_values_and_units() {
    for reason in [
        Reason::Pressure { signal: Signal::MemoryFull, value: 3140, threshold: 1000, cgroup: Some("/protected".into()) },
        Reason::Pressure { signal: Signal::MemoryAvailable, value: 1 << 30, threshold: 2 << 30, cgroup: None },
        Reason::Recovering { remaining: 120000 },
        Reason::UnknownObservation { cause: ObservationCause::Io { os_error: Some(libc::ENOENT), path: Some("/file".into()) } },
    ] {
        let serialized = serde_json::to_value(&reason).unwrap();
        assert!(serialized.get("kind").is_some());
        assert_eq!(serde_json::from_value::<Reason>(serialized).unwrap(), reason);
    }
}

#[test]
fn diagnostic_memory_available_signal_cannot_be_used_as_a_psi_rule() {
    let mut policy = Policy::default();
    policy.rules[0].signal = Signal::MemoryAvailable;
    assert!(Brake::new(policy).is_err());
}

#[test]
fn system_service_discovers_interactive_app_slices_from_mount() {
    let mut fixture = PressureFixture::new();
    let app = "user.slice/user-1000.slice/user@1000.service/app.slice";
    fixture.cgroup(app, "99", "99");
    fixture.cgroup(&format!("{app}/app-lanes.slice"), "99", "99");
    fixture.cgroup(&format!("{app}/editor.service"), "0", "0");
    fixture.cgroup("user.slice/user-1000.slice/user@1000.service/interactive.slice", "0", "0");
    assert!(!fixture.observe(0).blocked);
    fixture.cgroup(&format!("{app}/editor.service"), "3", "40");
    fixture.observe(2500);
    assert_eq!(fixture.reader.readings[0].cgroup.as_deref(),
        Some("/user.slice/user-1000.slice/user@1000.service/app.slice/editor.service"));
    assert_eq!(fixture.reader.readings[1].millipercent, 40000);
}

#[test]
fn unavailable_default_layout_fails_closed_but_absent_interactive_slice_is_optional() {
    let mut fixture = PressureFixture::new();
    let app = fixture.root.path().join("cgroup/app.slice");
    std::fs::remove_dir_all(&app).unwrap();
    assert_eq!(fixture.observe(0).reason, Some(Reason::UnknownObservation {
        cause: ObservationCause::Io {
            os_error: Some(libc::ENOENT), path: Some(app.to_string_lossy().into()),
        },
    }));
    // A discovered interactive.slice is optional: absent means skipped, rediscovered
    // every sample, and never a fail-close.
    let mut fixture = PressureFixture::new();
    std::fs::remove_dir_all(fixture.root.path().join("cgroup/interactive.slice")).unwrap();
    assert!(!fixture.observe(0).blocked);
    assert!(!fixture.observe(2500).blocked);
}

#[test]
fn normal_service_with_job_descendants_is_sampled_directly_and_never_fails_closed() {
    let mut fixture = PressureFixture::new();
    // The daemon service is not a configured job parent: a descendant sharing an
    // excluded basename does not disqualify it, and descendants never fail-close it.
    fixture.cgroup("system.slice/fractal.service", "3.14", "0");
    fixture.cgroup("system.slice/fractal.service/app-lanes.slice", "24.5", "99");
    fixture.cgroup("system.slice/fractal.service/interactive", "0", "0");
    for now in (0..60000).step_by(2500) {
        assert!(!fixture.observe(now).blocked);
    }
    assert_eq!(fixture.observe(60000).reason, Some(Reason::Pressure {
        signal: Signal::MemoryFull, value: 3140, threshold: 1000,
        cgroup: Some("/system.slice/fractal.service".into()),
    }));
}

#[test]
fn unsupported_job_parent_shapes_are_rejected_at_load_time_not_discovered_while_sampling() {
    // A relative multi-component path cannot represent a canonical job parent.
    let policy = Policy {
        job_parent_cgroups: vec!["custom.slice/nested/jobs".into()],
        ..Policy::default()
    };
    assert!(policy.validate().is_err());
    // Rejection is lexical and precedes any cgroup enumeration or sampling.
    let mut fixture = PressureFixture::new();
    fixture.policy = policy;
    fixture.cgroup("custom.slice/nested/jobs", "99", "99");
    assert!(fixture.observer().is_err());
}

#[test]
fn legacy_pressure_and_io_payloads_default_optional_paths_and_omit_none() {
    let reason: Reason = serde_json::from_value(serde_json::json!({
        "kind": "pressure", "signal": "memory-full", "value": 1000, "threshold": 1000,
    })).unwrap();
    assert_eq!(reason, Reason::Pressure {
        signal: Signal::MemoryFull, value: 1000, threshold: 1000, cgroup: None,
    });
    assert!(serde_json::to_value(reason).unwrap().get("cgroup").is_none());
    let cause: ObservationCause = serde_json::from_value(serde_json::json!({
        "kind": "io", "os_error": 2,
    })).unwrap();
    assert_eq!(cause, ObservationCause::Io { os_error: Some(2), path: None });
    assert!(serde_json::to_value(cause).unwrap().get("path").is_none());
}

#[test]
fn policy_rejects_paths_outside_mount_and_non_basename_relative_exclusions() {
    for path in ["../outside", "/../outside"] {
        let policy = Policy { protected_cgroups: vec![path.into()], ..Policy::default() };
        assert!(policy.validate().is_err());
    }
    let policy = Policy { excluded_cgroups: vec!["relative/nested".into()], ..Policy::default() };
    assert!(policy.validate().is_err());
}

#[test]
fn job_parent_cgroups_default_to_launcher_lane_slices_and_validate_lexically() {
    assert_eq!(Policy::default().job_parent_cgroups, vec![
        "gate.slice".into(), "app-lanes.slice".into(), "app-capped.slice".into(),
        "app-capped-heavy.slice".into(), "app-batch.slice".into(), "app-batch-heavy.slice".into(),
    ]);
    // Basenames are systemd slice unit names; absolutes are exact mount-relative paths.
    let policy = Policy {
        job_parent_cgroups: vec!["app-capped-heavy.slice".into(), "/custom.slice/nested/jobs".into()],
        ..Policy::default()
    };
    assert!(policy.validate().is_ok());
    let camel_case: Policy = serde_json::from_str("{\"jobParentCgroups\":[\"gate.slice\"]}").unwrap();
    assert_eq!(camel_case.job_parent_cgroups, vec![PathBuf::from("gate.slice")]);
    for invalid in ["nested/parent.slice", "editor.service", "", "../outside", "/../outside"] {
        let policy = Policy { job_parent_cgroups: vec![invalid.into()], ..Policy::default() };
        assert!(policy.validate().is_err(), "jobParentCgroups entry {invalid:?} must be refused");
    }
}

#[test]
fn policy_bounds_configured_path_counts_depths_and_slice_stem_segments() {
    // Many lane slices remain configurable; the combined configured count is bounded.
    let lanes: Vec<PathBuf> = (0..200).map(|index| format!("lane-{index}.slice").into()).collect();
    let policy = Policy { job_parent_cgroups: lanes, ..Policy::default() };
    assert!(policy.validate().is_ok());
    let oversubscribed: Vec<PathBuf> =
        (0..300).map(|index| format!("lane-{index}.slice").into()).collect();
    let policy = Policy { job_parent_cgroups: oversubscribed, ..Policy::default() };
    assert!(policy.validate().is_err());
    // Absolute entries are depth-bounded and slice stems segment-bounded.
    let deep = PathBuf::from(format!("/{}", vec!["a"; 300].join("/")));
    let policy = Policy { job_parent_cgroups: vec![deep], ..Policy::default() };
    assert!(policy.validate().is_err());
    let fragmented = format!("{}.slice", vec!["s"; 300].join("-"));
    let policy = Policy { job_parent_cgroups: vec![fragmented.into()], ..Policy::default() };
    assert!(policy.validate().is_err());
}

#[tokio::test(start_paused = true)]
async fn jittered_sampler_ticks_do_not_skip_observations_or_reset_recovery() {
    let fixture = PressureFixture::new();
    let mut observer = fixture.observer().unwrap();
    let file = fixture.root.path().join("cgroup/app.slice/editor.service/io.pressure");
    std::fs::remove_file(&file).unwrap();
    assert!(observer.observe(&fixture.policy).unwrap().blocked);
    fixture.cgroup("app.slice/editor.service", "0", "0");
    tokio::time::advance(std::time::Duration::from_millis(5001)).await;
    assert_eq!(observer.observe(&fixture.policy).unwrap().reason,
        Some(Reason::Recovering { remaining: fixture.policy.resume_ms }));
    let mut elapsed = 0;
    for tick in 0..24 {
        let interval = if tick % 2 == 0 { 4999 } else { 5001 };
        elapsed += interval;
        tokio::time::advance(std::time::Duration::from_millis(interval)).await;
        let decision = observer.observe(&fixture.policy).unwrap();
        assert_eq!(observer.sampled_at, Some(5001 + elapsed),
            "every timer tick must supply a fresh observation");
        if elapsed < fixture.policy.resume_ms {
            assert_eq!(decision.reason, Some(Reason::Recovering {
                remaining: fixture.policy.resume_ms - elapsed,
            }));
        } else {
            assert!(!decision.blocked);
        }
    }
}
