use super::*;
use std::time::Duration;

fn number(snapshot: &Value, field: &str) -> u64 {
    snapshot[field].as_u64().unwrap_or_else(|| panic!("missing integer {field}"))
}

#[test]
fn fixed_work_labels_are_unique_and_exhaustive() {
    for (index, work) in Work::ALL.into_iter().enumerate() {
        assert_eq!(work as usize, index);
        assert!(!work.name().is_empty());
        assert!(Work::ALL[..index].iter().all(|other| other.name() != work.name()));
    }
}

#[test]
fn integer_nanosecond_conversion_never_wraps() {
    for seconds in [0, 1, 7, u32::MAX as u64, u64::MAX / 1_000_000_000] {
        for nanos in [0, 1, 999, 999_999_999] {
            let duration = Duration::new(seconds, nanos);
            let expected = u64::try_from(duration.as_nanos());
            match (duration_ns(duration, "test"), expected) {
                (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
                (Err(Error::CounterOverflow { counter: "test" }), Err(_)) => {}
                (actual, expected) => panic!("conversion mismatch: {actual:?} vs {expected:?}"),
            }
        }
    }
    assert!(matches!(duration_ns(Duration::MAX, "test"), Err(Error::CounterOverflow { .. })));
}

#[test]
fn counters_saturate_and_never_wrap() {
    for initial in [0, 1, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
        for increment in [0, 1, 2, u64::MAX] {
            let counter = AtomicU64::new(initial);
            add(&counter, increment);
            assert_eq!(counter.load(Ordering::Relaxed), initial.saturating_add(increment));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn invalid_clock_values_and_syscall_failures_are_errors() {
    for (seconds, nanos) in [(-1, 0), (0, -1), (0, 1_000_000_000), (1, i64::MAX)] {
        assert!(matches!(timespec_ns(seconds, nanos, "test"), Err(Error::InvalidClockValue { .. })));
    }
    assert!(matches!(timespec_ns(i64::MAX, 0, "test"), Err(Error::CounterOverflow { .. })));
    assert_eq!(timespec_ns(7, 123, "test").unwrap(), 7_000_000_123);
    assert!(matches!(clock_ns(libc::clockid_t::MAX, "invalid-test-clock"), Err(Error::ClockRead { .. })));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn snapshots_preserve_identity_baseline_and_counter_units() {
    let meter = Meter::new().unwrap();
    let first = meter.snapshot().unwrap();
    for work in Work::ALL {
        meter.record(work);
    }
    let second = meter.snapshot().unwrap();
    assert_eq!(first["process_instance_id"], second["process_instance_id"]);
    assert_eq!(first["accounting_started_unix_ns"], second["accounting_started_unix_ns"]);
    assert_ne!(first["process_instance_id"], Meter::new().unwrap().snapshot().unwrap()["process_instance_id"]);
    assert_eq!(first["pid"], std::process::id());
    assert_eq!(first["process_cpu_baseline_ns"], second["process_cpu_baseline_ns"]);
    for snapshot in [&first, &second] {
        assert!(number(snapshot, "process_cpu_ns") >= number(snapshot, "process_cpu_baseline_ns"));
        assert!(number(snapshot, "cpu_baseline_uptime_before_ns") <= number(snapshot, "cpu_baseline_uptime_after_ns"));
        assert!(number(snapshot, "cpu_baseline_uptime_after_ns") <= number(snapshot, "cpu_sample_uptime_before_ns"));
        assert!(number(snapshot, "cpu_sample_uptime_before_ns") <= number(snapshot, "cpu_sample_uptime_after_ns"));
        assert_eq!(number(snapshot, "accounting_uptime_ns"), number(snapshot, "cpu_sample_uptime_after_ns"));
    }
    assert!(number(&second, "accounting_uptime_ns") >= number(&first, "accounting_uptime_ns"));
    assert!(number(&second, "process_cpu_ns") >= number(&first, "process_cpu_ns"));
    let rows = second["work"].as_array().unwrap();
    assert_eq!(rows.len(), Work::ALL.len());
    for (row, work) in rows.iter().zip(Work::ALL) {
        assert_eq!(row["kind"], work.name());
        assert_eq!(row["count"], 1);
        assert_eq!(row["thread_cpu_samples"], 0);
        assert_eq!(row["thread_cpu_errors"], 0);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn synchronous_measurement_executes_once_and_preserves_result() {
    let meter = Meter::new().unwrap();
    let mut calls = 0;
    let (result, sample) = meter.measure_sync(Work::NativeObserve, || {
        calls += 1;
        42
    });
    assert_eq!(result, 42);
    assert_eq!(calls, 1);
    sample.unwrap();
    let snapshot = meter.snapshot().unwrap();
    let row = &snapshot["work"][Work::NativeObserve as usize];
    assert_eq!(row["count"], 1);
    assert_eq!(row["thread_cpu_samples"], 1);
    assert_eq!(row["thread_cpu_errors"], 0);
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_platform_fails_explicitly() {
    assert!(matches!(Meter::new(), Err(Error::UnsupportedPlatform)));
    // No meter can be constructed on unsupported platforms; no zero-valued
    // physical counter is available to the runtime.
    assert!(matches!(process_cpu_ns(), Err(Error::UnsupportedPlatform)));
    assert!(matches!(thread_cpu_ns(), Err(Error::UnsupportedPlatform)));
}

/// Isolate the physical control from other tests' CPU. No benchmark budget or
/// production idle/600-second gate is asserted by this short sensor test.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn own_process_busy_idle_positive_control() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["isolated_own_process_busy_idle_probe", "--ignored", "--test-threads=1", "--nocapture"])
        .output()
        .unwrap();
    assert!(output.status.success(), "positive control failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("OWN_PROCESS_CPU_POSITIVE_CONTROL_OK"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
#[ignore = "isolated helper, launched by own_process_busy_idle_positive_control"]
fn isolated_own_process_busy_idle_probe() {
    let meter = Meter::new().unwrap();
    let before_idle = meter.snapshot().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let after_idle = meter.snapshot().unwrap();
    // Burn CPU on a different thread: the process clock must include it while
    // the querying thread waits. Stop by physical thread CPU, not loop count.
    let measured_thread_cpu = std::thread::spawn(|| {
        let started = thread_cpu_ns().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            for value in 0..1024_u64 {
                std::hint::black_box(value.wrapping_mul(value));
            }
            let consumed = thread_cpu_ns().unwrap() - started;
            if consumed >= 50_000_000 {
                return consumed;
            }
            assert!(Instant::now() < deadline, "busy positive control could not get scheduled");
        }
    }).join().unwrap();
    let after_busy = meter.snapshot().unwrap();
    let idle_cpu = number(&after_idle, "process_cpu_ns") - number(&before_idle, "process_cpu_ns");
    let busy_cpu = number(&after_busy, "process_cpu_ns") - number(&after_idle, "process_cpu_ns");
    assert!(busy_cpu >= measured_thread_cpu, "process clock missed worker CPU");
    assert!(busy_cpu > idle_cpu, "physical process counter did not distinguish busy and idle");
    assert!(number(&after_idle, "accounting_uptime_ns") > number(&before_idle, "accounting_uptime_ns"));
    println!("OWN_PROCESS_CPU_POSITIVE_CONTROL_OK idle_cpu_ns={idle_cpu} busy_cpu_ns={busy_cpu}");
}
