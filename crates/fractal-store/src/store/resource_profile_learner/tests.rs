use super::*;

fn ready() -> HostReadiness {
    HostReadiness { nix_per_build_cgroups: true, io_accounting: true, disk_measurement: true }
}

fn sample(id: u64, peak: u64) -> RetirementSample {
    RetirementSample {
        host: "host".into(), key: "repo/install".into(), semantic_revision: "scope-v1".into(),
        execution: format!("execution-{id}"), generation: "generation-1".into(), retirement_confirmed: true,
        pre_retirement_populated: Some(false),
        end_reason: EndReason::Natural, coverage: Coverage::Full, memory_peak_bytes: Some(peak),
        memory_events: Some(MemoryEvents { max: 0, high: 0, oom: 0, oom_kill: 0, oom_group_kill: 0 }),
        disk_growth_bytes: Some(10),
    }
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    create_schema(&connection).unwrap();
    connection
}

fn record(connection: &mut Connection, sample: &RetirementSample) -> RecordOutcome {
    let transaction = connection.transaction().unwrap();
    let result = record_retirement_tx(&transaction, sample).unwrap();
    transaction.commit().unwrap();
    result
}

fn reading(connection: &Connection, readiness: HostReadiness) -> LearnedEstimate {
    estimate(connection, "host", "repo/install", "scope-v1",
        SeedLimits { memory_high_bytes: 1000, memory_max_bytes: 1200, disk_bytes: 100 },
        readiness).unwrap()
}

#[test]
fn remains_at_seed_until_twenty_confirmed_samples_then_uses_nearest_rank() {
    let mut connection = database();
    for id in 1..20 {
        assert_eq!(record(&mut connection, &sample(id, id)), RecordOutcome::Recorded);
    }
    assert_eq!(reading(&connection, ready()).memory_high_bytes, 1000);
    record(&mut connection, &sample(20, 20));
    let result = reading(&connection, ready());
    assert_eq!(result.memory_high_bytes, 19);
    assert_eq!(result.memory_max_bytes, 23);
    assert_eq!(result.disk_bytes, 10);
    assert_eq!(result.memory_samples, 20);
    assert!(result.memory_learned && result.disk_learned);
}

#[test]
fn each_host_readiness_requirement_independently_prevents_below_seed_estimates() {
    let mut connection = database();
    for id in 0..20 { record(&mut connection, &sample(id, 10)); }
    for readiness in [
        HostReadiness { nix_per_build_cgroups: false, ..ready() },
        HostReadiness { io_accounting: false, ..ready() },
        HostReadiness { disk_measurement: false, ..ready() },
    ] {
        let result = reading(&connection, readiness);
        assert_eq!((result.memory_high_bytes, result.disk_bytes), (1000, 100));
        assert!(!result.below_seed_ready);
    }
    assert_eq!(reading(&connection, ready()).memory_high_bytes, 10);
}

#[test]
fn incomplete_readiness_does_not_prevent_safe_upward_learning() {
    let mut connection = database();
    for id in 0..20 { record(&mut connection, &sample(id, 2000)); }
    assert_eq!(reading(&connection, HostReadiness::default()).memory_high_bytes, 2000);
}

#[test]
fn retained_256_mib_oom_fixture_is_censored_not_a_low_training_peak() {
    let mut connection = database();
    let mut fixture = sample(1, 268435456);
    fixture.memory_events = Some(MemoryEvents { max: 49, high: 0, oom: 1, oom_kill: 1, oom_group_kill: 0 });
    assert_eq!(record(&mut connection, &fixture),
        RecordOutcome::Rejected(Rejection::MemoryLimitEvent));
    assert_eq!(reading(&connection, ready()).memory_samples, 0);
}

#[test]
fn retirement_filter_rejects_unknown_counters_coverage_and_all_truncations() {
    let mut connection = database();
    let mut rejected = sample(1, 100);
    rejected.retirement_confirmed = false;
    assert_eq!(record(&mut connection, &rejected),
        RecordOutcome::Rejected(Rejection::UnconfirmedRetirement));
    for end_reason in [EndReason::Cancelled, EndReason::TimedOut, EndReason::MaxHold, EndReason::Signalled, EndReason::Unknown] {
        rejected = sample(1, 100);
        rejected.end_reason = end_reason;
        assert_eq!(record(&mut connection, &rejected), RecordOutcome::Rejected(Rejection::Truncated));
    }
    for coverage in [Coverage::ClientOnly, Coverage::Unknown] {
        rejected = sample(1, 100);
        rejected.coverage = coverage;
        assert_eq!(record(&mut connection, &rejected),
            RecordOutcome::Rejected(Rejection::IncompleteCoverage));
    }
    rejected = sample(1, 100);
    rejected.memory_events = None;
    assert_eq!(record(&mut connection, &rejected),
        RecordOutcome::Rejected(Rejection::MissingMemoryEvents));
    for peak in [None, Some(0)] {
        rejected = sample(1, 100);
        rejected.memory_peak_bytes = peak;
        assert_eq!(record(&mut connection, &rejected),
            RecordOutcome::Rejected(Rejection::MissingMemoryPeak));
    }
    assert_eq!(reading(&connection, ready()).memory_samples, 0);
}

#[test]
fn each_limit_counter_independently_rejects_an_observation() {
    for events in [
        MemoryEvents { max: 1, high: 0, oom: 0, oom_kill: 0, oom_group_kill: 0 },
        MemoryEvents { max: 0, high: 0, oom: 1, oom_kill: 0, oom_group_kill: 0 },
        MemoryEvents { max: 0, high: 0, oom: 0, oom_kill: 1, oom_group_kill: 0 },
        MemoryEvents { max: 0, high: 0, oom: 0, oom_kill: 0, oom_group_kill: 1 },
    ] {
        let mut rejected = sample(1, 100);
        rejected.memory_events = Some(events);
        assert_eq!(rejected.rejection(), Some(Rejection::MemoryLimitEvent));
    }
}

#[test]
fn absent_disk_measurements_never_train_zero_growth() {
    let mut connection = database();
    for id in 0..20 {
        let mut value = sample(id, 100);
        value.disk_growth_bytes = None;
        record(&mut connection, &value);
    }
    let result = reading(&connection, ready());
    assert!(result.memory_learned);
    assert!(!result.disk_learned);
    assert_eq!((result.disk_samples, result.disk_bytes), (0, 100));
}

#[test]
fn host_key_semantic_revision_and_generation_partition_history() {
    let mut connection = database();
    let original = sample(1, 100);
    record(&mut connection, &original);
    let mut changed = original.clone();
    changed.host = "another-host".into();
    record(&mut connection, &changed);
    changed = sample(2, 100);
    changed.key = "another-operation".into();
    record(&mut connection, &changed);
    changed = sample(3, 100);
    changed.semantic_revision = "scope-v2".into();
    record(&mut connection, &changed);
    changed = original.clone();
    changed.generation = "generation-2".into();
    record(&mut connection, &changed);
    assert_eq!(reading(&connection, ready()).memory_samples, 2);
    let before = reading(&connection, ready());
    assert_eq!(record(&mut connection, &original), RecordOutcome::Duplicate);
    assert_eq!(reading(&connection, ready()), before);
}

#[test]
fn trailing_window_is_bounded_without_forgetting_old_execution_dedupe() {
    let mut connection = database();
    let old = sample(0, 10000);
    record(&mut connection, &old);
    for id in 1..=256 { record(&mut connection, &sample(id, 10)); }
    let result = reading(&connection, ready());
    assert_eq!(result.memory_samples, 256);
    assert_eq!(result.memory_high_bytes, 10);
    assert_eq!(record(&mut connection, &old), RecordOutcome::Duplicate);
    assert_eq!(reading(&connection, ready()), result);
}

#[test]
fn persistence_reopen_and_terminal_transaction_rollback_preserve_contract() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.sqlite3");
    {
        let mut connection = Connection::open(&path).unwrap();
        create_schema(&connection).unwrap();
        for id in 0..20 { record(&mut connection, &sample(id, 100)); }
        let transaction = connection.transaction().unwrap();
        record_retirement_tx(&transaction, &sample(999, 9000)).unwrap();
        transaction.rollback().unwrap();
    }
    let mut connection = Connection::open(path).unwrap();
    create_schema(&connection).unwrap();
    assert_eq!(reading(&connection, ready()).memory_samples, 20);
    assert_eq!(reading(&connection, ready()).memory_high_bytes, 100);
    assert_eq!(record(&mut connection, &sample(0, 100)), RecordOutcome::Duplicate);
}

#[test]
fn integer_margin_preserves_precision_and_rejects_overflow() {
    for (high, max) in [(1, 2), (5, 6), (19, 23), (20, 24), (9007199254740993, 10808639105689192)] {
        assert_eq!(memory_max(high).unwrap(), max);
    }
    assert!(memory_max(0).is_err());
    assert!(memory_max(u64::MAX).is_err());
    assert_eq!(remaining_disk(100, 40),
        DiskRemaining { remaining_bytes: 60, overshot: false });
    assert_eq!(remaining_disk(100, 100),
        DiskRemaining { remaining_bytes: 0, overshot: false });
    assert_eq!(remaining_disk(100, 101),
        DiskRemaining { remaining_bytes: 0, overshot: true });
}

#[test]
fn conflicting_replay_cannot_silently_relabel_or_change_a_sample() {
    let mut connection = database();
    record(&mut connection, &sample(1, 100));
    for changed in [
        RetirementSample { key: "different-key".into(), ..sample(1, 100) },
        RetirementSample { semantic_revision: "different-revision".into(), ..sample(1, 100) },
        RetirementSample { disk_growth_bytes: None, ..sample(1, 100) },
        sample(1, 101),
    ] {
        let transaction = connection.transaction().unwrap();
        assert!(record_retirement_tx(&transaction, &changed).is_err());
    }
    assert_eq!(reading(&connection, ready()).memory_samples, 1);
}

#[test]
fn seed_preserves_explicit_non_twenty_percent_worker_margin_until_learning_is_ready() {
    let mut connection = database();
    let seed = SeedLimits { memory_high_bytes: 3, memory_max_bytes: 4, disk_bytes: 10 };
    let read = |connection: &Connection, readiness|
        estimate(connection, "host", "repo/install", "scope-v1", seed, readiness).unwrap();
    assert_eq!(read(&connection, ready()).memory_max_bytes, 4);
    for id in 0..20 { record(&mut connection, &sample(id, 1)); }
    assert_eq!(read(&connection, HostReadiness::default()).memory_max_bytes, 4);
    assert_eq!(read(&connection, ready()).memory_max_bytes, 2);
}

#[test]
fn memory_high_throttling_alone_is_not_a_censored_peak() {
    let mut observed = sample(1, 100);
    observed.memory_events.as_mut().unwrap().high = 585;
    assert_eq!(observed.rejection(), None);
}

#[test]
fn nearest_rank_is_exact_at_every_trailing_window_length() {
    assert_eq!(p95(&mut []), None);
    for count in 1_u64..=256 {
        let mut values: Vec<_> = (1..=count).rev().collect();
        assert_eq!(p95(&mut values), Some((count * 95).div_ceil(100)));
    }
}

#[test]
fn persisted_memory_and_disk_counters_preserve_the_full_unsigned_domain() {
    let mut connection = database();
    let mut observed = sample(1, u64::MAX);
    observed.disk_growth_bytes = Some(u64::MAX);
    assert_eq!(record(&mut connection, &observed), RecordOutcome::Recorded);
    assert_eq!(record(&mut connection, &observed), RecordOutcome::Duplicate);
    let result = reading(&connection, ready());
    assert_eq!((result.memory_samples, result.disk_samples), (1, 1));
    // With fewer than twenty samples the oversized peak does not replace the seed.
    assert_eq!(result.memory_high_bytes, 1000);
}

#[test]
fn natural_leader_exit_does_not_make_destructively_retired_descendants_trainable() {
    let mut connection = database();
    let mut observed = sample(1, 100);
    observed.pre_retirement_populated = Some(true);
    assert_eq!(record(&mut connection, &observed),
        RecordOutcome::Rejected(Rejection::PopulatedBeforeRetirement));
    assert_eq!(reading(&connection, ready()).memory_samples, 0);
}

#[test]
fn absent_cgroup_population_counter_cannot_certify_untruncated_retirement() {
    let mut observed = sample(1, 100);
    observed.pre_retirement_populated = None;
    assert_eq!(observed.rejection(), Some(Rejection::MissingRetirementEvidence));
}

#[test]
fn partial_readiness_configuration_never_implies_unobserved_capabilities() {
    let readiness: HostReadiness = serde_json::from_str(
        "{\"nixPerBuildCgroups\":true}").unwrap();
    assert!(readiness.nix_per_build_cgroups);
    assert!(!readiness.io_accounting && !readiness.disk_measurement);
    assert!(!readiness.permits_below_seed());
    assert!(serde_json::from_str::<HostReadiness>(
        "{\"pretendReady\":true}").is_err());
}
