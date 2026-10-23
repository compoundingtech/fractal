//! Hosted-only controls. These fixtures do not sample or mutate the host daemon.
use super::*;
use crate::store::fleet_admission_tests::{admit, key, node, sync};
use fractal_schema::slo_breaches::ProcessDelta;
use std::time::{Duration, Instant};

const SOURCE_START: u64 = 1_600_000_000_000;

fn source() -> Store {
    let identity = key();
    smallclaims::store::set_thread_clock(Some(u128::from(SOURCE_START)));
    let store = node("alder", Some(&identity), Some(&identity));
    smallclaims::store::set_thread_clock(None);
    admit(&store, "alder", &identity, "anchor", None);
    store.project_replication_backlog().unwrap();
    store.fleet_membership().unwrap();
    store
}

fn reading(store: &Store, start: u64, value: Option<f64>) -> Observation {
    let (epoch, base) = store.slo_source_epoch();
    let source_cut = format!("{epoch}/{start}");
    let predecessor_cut = (start>0).then(|| format!("{epoch}/{}",start-3_600_000));
    Observation {
        member: store.origin.clone(), epoch, epoch_started_ms: base, sequence: start/3_600_000+1,
        predecessor_cut,
        version: "person-read/v1".into(), start_ms: base + start, end_ms: base + start + 3_600_000,
        observed_at_ms: base + start + 3_600_000, window_ms: 3_600_000, covered_ms: 3_600_000,
        complete: true, population: "all eligible requests".into(), source_cut,
        checks: vec![Check { unit: "ms".into(), value, target: 100.0, minimum: false }],
        counters: BTreeMap::new(),
    }
}

fn miss(store: &Store, objective: &str) -> String {
    // The in-memory test store shares one cache between its connections, so a reader of the
    // fleet generation can find it locked by a concurrent writer; a file store (WAL) cannot.
    loop {
        match store.record_slo_miss(objective, None, reading(store, 0, Some(120.0)), None) {
            Err(error) if format!("{error:#}").contains("is locked") => std::thread::sleep(Duration::from_millis(2)),
            result => return result.unwrap().unwrap(),
        }
    }
}

fn claims(store: &Store) -> Vec<ClaimRecord> {
    store.readers.get().prepare("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims ORDER BY store_index")
        .unwrap().query_map([], claim_from_row).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

fn fact_state(store: &Store, episode: &str) -> String {
    store.latest_actual_value(&format!("resource/slo-breach/{episode}")).unwrap()
        .and_then(|value| value["facts"]["status"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

#[test]
fn unknown_partial_and_nonfinite_readings_cannot_open_a_breach() {
    let store = source();
    for (index, mut observation) in [reading(&store, 0, None), reading(&store, 0, Some(80.0))].into_iter().enumerate() {
        assert!(store.record_slo_miss(&format!("fixture/{index}"), None, observation.clone(), None).unwrap().is_none());
        observation.complete = false;
        observation.checks[0].value = Some(120.0);
        assert!(store.record_slo_miss(&format!("partial/{index}"), None, observation, None).unwrap().is_none());
    }
    assert!(store.record_slo_miss("person-read", None, reading(&store, 0, Some(f64::NAN)), None).is_err());
    assert!(store.prepare_slo_missions("/tmp").unwrap().is_empty());
}

#[test]
fn one_onset_survives_preparation_replay_and_credential_rotation() {
    let store = source();
    let episode = miss(&store, "person-read");
    let count = claims(&store).len();
    assert_eq!(miss(&store, "person-read"), episode);
    assert_eq!(claims(&store).len(), count, "repeated misses are not sample claims");
    assert_eq!(store.prepare_slo_missions("/tmp").unwrap().len(), 1);
    store.connection.batched(|tx| rebuild(tx)).unwrap().unwrap();
    assert_eq!(miss(&store, "person-read"), episode);
    let prepared = store.prepare_slo_missions("/tmp").unwrap().remove(0);
    store.set_member_key(Some(key())).unwrap();
    assert!(store.dispatch_slo_mission(prepared).is_err(), "stale preparation cannot cross key rotation");
    assert_eq!(head(&store.readers.get(), &store.bound_fleet().unwrap().unwrap(), "alder", "person-read").unwrap().unwrap().0,
        episode, "a key is not episode identity");
}

#[test]
fn activation_frontier_is_durable_bounded_and_rate_limited() {
    let store = source();
    for objective in 0..MAX_OBJECTIVES { miss(&store, &format!("objective/{objective}")); }
    assert!(store.record_slo_miss("objective/65", None, reading(&store, 0, Some(120.0)), None).is_err());
    let prepared = store.prepare_slo_missions("/tmp").unwrap();
    assert_eq!(prepared.len(), 2);
    for pending in prepared { store.dispatch_slo_mission(pending).unwrap(); }
    let pending = store.prepare_slo_missions("/tmp").unwrap().remove(0);
    assert!(store.dispatch_slo_mission(pending).is_err(), "third opening in the minute is retained pending");
    store.connection.batched(|tx| rebuild(tx)).unwrap().unwrap();
    assert_eq!(store.prepare_slo_missions("/tmp").unwrap().len(), 2);
    let count: usize = store.readers.get().query_row("SELECT COUNT(*) FROM slo_pending WHERE mission_run IS NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 62, "restart/replay cannot lose the later obligations");
}

#[test]
fn a_new_breach_requires_the_previous_recovery_and_one_mission() {
    let store = source();
    let first = miss(&store, "person-read");
    store.recover_slo_breach(&first, reading(&store, 3_600_000, Some(80.0)), None).unwrap();
    let second = store.record_slo_miss("person-read", None, reading(&store, 7_200_000, Some(120.0)), None).unwrap().unwrap();
    assert_eq!(store.prepare_slo_missions("/tmp").unwrap().len(), 2, "both episode obligations stay durable");
    let old = store.prepare_slo_missions("/tmp").unwrap().remove(0);
    assert_eq!(old.record.episode, first);
    store.dispatch_slo_mission(old).unwrap();
    assert_ne!(first, second);
    assert!(store.recover_slo_breach(&first, reading(&store, 10_800_000, Some(80.0)), None).is_ok(), "duplicate recovery is a no-op");
    assert_eq!(fact_state(&store, &second), "pending");
}

fn cpu_witness(reading: &Observation, cores: f64) -> HealthyWitness {
    let hour = 3_600_000_000_000u64;
    let start = (reading.start_ms-reading.epoch_started_ms)*1_000_000;
    HealthyWitness { source_cut: reading.source_cut.clone(), start_ms: reading.start_ms, end_ms: reading.end_ms, expected_workers: 1,
        monotonic_start_ns: start, monotonic_end_ns: start+hour, boundary_uncertainty_ns: 0,
        complete_cohort: true, positively_idle: true, processes: vec![
            ProcessDelta { role: "daemon".into(), pid: 10, birth: 1, cpu_start_ns: 0, cpu_end_ns: (cores * hour as f64) as u64 },
            ProcessDelta { role: "replication-worker".into(), pid: 11, birth: 2, cpu_start_ns: 0, cpu_end_ns: 0 },
        ] }
}

#[test]
fn idle_cpu_requires_full_eligible_cohort_and_consistent_counters() {
    let store = source();
    let mut observation = reading(&store, 0, Some(0.6));
    observation.checks[0].unit = "cores".into(); observation.checks[0].target = 0.5;
    observation.version = "combined-idle/v1".into();
    assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), None).is_err());
    let mut missing = cpu_witness(&observation, 0.6); missing.processes.pop(); missing.complete_cohort = false;
    let mut duplicate = cpu_witness(&observation, 0.6); duplicate.processes[1] = duplicate.processes[0].clone();
    let mut birth_unknown = cpu_witness(&observation, 0.6); birth_unknown.processes[1].birth = 0;
    let mut short = cpu_witness(&observation, 0.6); short.monotonic_end_ns -= 1;
    for witness in [missing, duplicate, birth_unknown, short] {
        assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), Some(witness)).is_err());
    }
    let mut busy = cpu_witness(&observation, 0.6); busy.positively_idle = false;
    assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), Some(busy)).is_err());
    let mut reset = cpu_witness(&observation, 0.6); reset.processes[1].cpu_start_ns = 1;
    assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), Some(reset)).is_err());
    let episode = store.record_slo_miss("cpu/idle", None, observation.clone(), Some(cpu_witness(&observation, 0.6))).unwrap().unwrap();
    observation.start_ms = SOURCE_START+3_600_000; observation.end_ms = SOURCE_START+7_200_000; observation.observed_at_ms = observation.end_ms;
    observation.sequence = 2; observation.predecessor_cut = Some(observation.source_cut.clone());
    observation.source_cut = "cut/healthy".into(); observation.checks[0].value = Some(0.4);
    let mut healthy = cpu_witness(&observation, 0.4);
    healthy.processes[0].cpu_start_ns = cpu_witness(&observation, 0.6).processes[0].cpu_end_ns;
    healthy.processes[0].cpu_end_ns += healthy.processes[0].cpu_start_ns;
    let mut partial = healthy.clone(); partial.boundary_uncertainty_ns = 1;
    assert!(store.recover_slo_breach(&episode, observation.clone(), Some(partial)).is_err());
    let reset = cpu_witness(&observation, 0.4);
    assert!(store.recover_slo_breach(&episode, observation.clone(), Some(reset)).is_err());
    store.recover_slo_breach(&episode, observation, Some(healthy)).unwrap();
}

#[test]
fn replica_admission_rejects_relabelled_origin_or_non_runtime_actor() {
    let store = source();
    miss(&store, "person-read");
    let claim = claims(&store).into_iter().find(|claim| claim.kind == KIND).unwrap();
    assert!(matches!(classify_replicated_claim_with_registry(&claim, fractal_schema::registry()).unwrap(), ReplicatedClaimAdmission::Valid));
    for forged in [true, false] {
        let mut bad = claim.clone();
        if forged { bad.origin = "birch".into(); } else { bad.actor = Some(DEFAULT_OWNER.into()); }
        assert!(classify_replicated_claim_with_registry(&bad, fractal_schema::registry()).is_err());
    }
}

// A test-scoped process counter, not a daemon collector or an hour-coverage claim.
fn process_cpu_ns() -> u64 {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: time is a writable, valid timespec for the duration of the call.
    assert_eq!(unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time) }, 0);
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

#[test]
fn hosted_cost_control_reports_full_writer_and_replica_holds() {
    let source = source();
    let identity = key();
    let replica = node("birch", Some(&identity), None);
    let begin = Instant::now();
    let cpu_before = process_cpu_ns();
    let ((episode, result), writer_hold) = smallclaims::windows::holds::longest_during(|| {
        let episode = miss(&source, "person-read");
        let prepared = source.prepare_slo_missions("/tmp").unwrap().remove(0);
        (episode, source.dispatch_slo_mission(prepared))
    });
    result.unwrap();
    let source_cpu_ns = process_cpu_ns().saturating_sub(cpu_before);
    let cpu_before = process_cpu_ns();
    let (admission, replica_hold) = smallclaims::windows::holds::longest_during(|| sync(&source, &replica));
    let replica_cpu_ns = process_cpu_ns().saturating_sub(cpu_before);
    assert_eq!(admission.invalid, 0);
    let exchange = source.export_replication_exchange(&source.bound_fleet().unwrap().unwrap(),
        &smallclaims::replication::ReplicationInventory::default()).unwrap();
    for envelope in &exchange.envelopes {
        assert!(serde_json::to_vec(envelope).unwrap().len() <= MAX_GROUP_BYTES);
    }
    eprintln!("SLO_COST objective=person-read episode={episode} full_dispatch_elapsed_ms={} writer_hold_ms={} replica_hold_ms={} source_process_cpu_ms={} replica_process_cpu_ms={} writer_hold_reference=st2/3080 qualification=HOSTED_CONTROL_ONLY isolation=nextest-process",
        begin.elapsed().as_secs_f64()*1000.0, writer_hold.as_secs_f64()*1000.0, replica_hold.as_secs_f64()*1000.0,
        source_cpu_ns as f64/1_000_000.0, replica_cpu_ns as f64/1_000_000.0);
    // Debug-runner ceiling only; meeting production budgets still requires Intake's
    // measured comparison with the current writer-hold baseline before activation.
    assert!(writer_hold <= Duration::from_millis(500));
    assert!(replica_hold <= Duration::from_millis(500));
}

#[test]
fn conflicting_events_converge_and_never_publish_ready() {
    let source = source();
    let episode = miss(&source, "person-read");
    let a = claims(&source).into_iter().find(|claim| claim.kind == KIND).unwrap();
    let mut b = a.clone();
    b.id = "a-conflicting-claim".into();
    b.body["fields"]["record"]["owner"] = json!("agent/another-owner");
    let mut digests = Vec::new();
    for order in [[&a, &b], [&b, &a]] {
        let target = Store::open_memory("projection").unwrap();
        target.connection.batched(|tx| {
            for claim in order { project(tx, claim)?; }
            Ok::<_,anyhow::Error>(())
        }).unwrap().unwrap();
        assert_eq!(fact_state(&target, &episode), "unknown");
        let tables = projection_digest::tables(&target.readers.get()).unwrap();
        digests.push([tables["slo_pending"].clone(), tables["slo_episode_heads"].clone(), tables["slo_episode_events"].clone()]);
    }
    assert_eq!(digests[0], digests[1]);
}

#[test]
fn relabelled_remote_readings_and_wrong_health_sequences_are_rejected() {
    let source = source();
    let mut remote = reading(&source, 0, Some(120.0)); remote.member = "birch".into();
    assert!(source.record_slo_miss("person-read", None, remote, None).is_err());
    let episode = miss(&source, "person-read");
    for mode in 0..3 {
        let mut healthy = reading(&source, 3_600_000, Some(80.0));
        match mode { 0 => healthy.member = "birch".into(), 1 => healthy.sequence = 1,
            _ => healthy.predecessor_cut = Some("unrelated-cut".into()) }
        assert!(source.recover_slo_breach(&episode, healthy, None).is_err());
    }
}

#[test]
fn database_recovery_needs_its_full_day_not_an_hour() {
    let source = source();
    let day = 86_400_000;
    let mut onset = reading(&source, 0, Some(120.0));
    onset.window_ms = day; onset.covered_ms = day; onset.end_ms = SOURCE_START+day; onset.observed_at_ms = SOURCE_START+day;
    let episode = source.record_slo_miss("database", None, onset, None).unwrap().unwrap();
    let mut healthy = reading(&source, day, Some(80.0)); healthy.predecessor_cut = Some(reading(&source, 0, Some(120.0)).source_cut);
    assert!(source.recover_slo_breach(&episode, healthy.clone(), None).is_err());
    healthy.window_ms = day; healthy.covered_ms = day; healthy.end_ms = SOURCE_START+2*day; healthy.observed_at_ms = SOURCE_START+2*day;
    source.recover_slo_breach(&episode, healthy, None).unwrap();
}

#[test]
fn oversized_publication_rolls_back_without_losing_pending_intake() {
    let source = source();
    let episode = miss(&source, "person-read");
    let before = claims(&source).len();
    let result = source.connection.batched(|tx| {
        let batch = open_batch(tx, "alder")?;
        for index in 0..17 {
            append_claim_tx(tx, "alder", "daemon/alder", "daemon.diagnostic", Some("daemon/runtime"),
                &json!({"fields":{"severity":"warning","code":"group-bound","reason":index.to_string()}}), &[], Some(&batch))?;
        }
        check_group(tx, &batch)
    }).unwrap();
    assert!(result.is_err());
    assert_eq!(claims(&source).len(), before);
    assert_eq!(fact_state(&source, &episode), "pending");
}

#[test]
fn checkpoint_proof_trim_and_canonical_renumber_preserve_episode_fences() {
    let source = source();
    let episode = miss(&source, "person-read");
    let (_, pending) = event(&source.readers.get(), &episode, Event::Pending).unwrap().unwrap();
    for _ in 0..2 {
        source.connection.batched(|tx| { let batch = open_batch(tx, "alder")?; append(tx, "alder", &pending, &batch) })
            .unwrap().unwrap();
    }
    source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
    let rows = claims(&source).into_iter().filter(|claim| claim.kind == KIND).collect::<Vec<_>>();
    let latest = rows.iter().map(|claim| claim.accepted_at_unix_ms).max().unwrap();
    let young = latest + 86_400_000;
    let cut = latest + 8*86_400_000;
    let references = rows.iter().collect::<Vec<_>>();
    assert_eq!(checkpoint_rules::rule_keep(checkpoint_rules::Rule::SloEventFences, rows.len(), &references,
        false, &BTreeMap::new(), young).len(), rows.len());
    let scratch = tempfile::tempdir().unwrap();
    let (_, _, proof) = source.plan_checkpoint_through(cut, None, scratch.path()).unwrap();
    assert!(proof.passed, "canonical proof/renumber: {:?}", proof.mismatches);
    let sealed = source.checkpoint_sealed_set(cut).unwrap();
    let plan = checkpoint::plan_drops(&sealed);
    assert!(!plan.claims.is_empty(), "exercise an actual redundant receipt drop");
    let before = projection_digest::oracle(&source.readers.get()).unwrap();
    let mut connection = source.connection.write();
    let tx = connection.transaction().unwrap();
    checkpoint::record_checkpoint_tombstones_tx(&tx, &checkpoint::checkpoint_name(cut), &plan.envelopes, &plan.claims).unwrap();
    checkpoint::delete_dropped_rows_tx(&tx, &plan.envelopes, &plan.claims).unwrap();
    replay_graph_from_nothing_tx(&tx).unwrap();
    tx.commit().unwrap();
    drop(connection);
    let after = projection_digest::oracle(&source.readers.get()).unwrap();
    for table in ["slo_pending", "slo_episode_heads", "slo_episode_events"] {
        assert_eq!(before[table], after[table], "{table} changed after real trim/replay");
    }
    assert_eq!(fact_state(&source, &episode), "recovered");
}

#[test]
fn an_unknown_conflicting_objective_does_not_starve_other_pending_obligations() {
    let source = source();
    let first = miss(&source, "objective/a");
    let (_, mut conflicting) = event(&source.readers.get(), &first, Event::Pending).unwrap().unwrap();
    conflicting.owner = "agent/another-owner".into();
    source.connection.batched(|tx| { let batch = open_batch(tx, "alder")?; append(tx, "alder", &conflicting, &batch) }).unwrap().unwrap();
    let next = miss(&source, "objective/b");
    let prepared = source.prepare_slo_missions("/tmp").unwrap();
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].record.episode, next);
    assert_eq!(fact_state(&source, &first), "unknown");
}

#[test]
fn an_old_registry_repair_withdraws_closure_and_matches_replay_on_current_members() {
    let source = source();
    let episode = miss(&source, "person-read");
    source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    let (recovered, _) = event(&source.readers.get(), &episode, Event::Recovered).unwrap().unwrap();
    let replacement = claims(&source).into_iter().find(|claim| claim.kind == "fleet.member-admitted").unwrap().id;
    let identity = key();
    admit(&source, "birch", &identity, "invite", None);
    let old = node("birch", Some(&identity), None);
    old.pin_fleet_anchor(&source.member_public_key().unwrap()).unwrap();
    let mut registry = fractal_schema::registry().clone(); registry.claims.remove(KIND).unwrap(); old.set_claim_registry(registry);
    sync(&source, &old);
    let reference: String = old.readers.get().query_row("SELECT record_ref FROM replica_records WHERE claim_id=?1",
        [&recovered], |row| row.get(0)).unwrap();
    old.repair_replica_record(&reference, &replacement, "explicit old-registry repair fixture", "person/avery", "repair-slo-recovery").unwrap();
    sync(&old, &source);
    source.apply_replication_repairs().unwrap();
    assert_ne!(fact_state(&source, &episode), "recovered", "a repaired closure is not live evidence");
    let before = projection_digest::oracle(&source.readers.get()).unwrap();
    source.connection.batched(|tx| rebuild(tx)).unwrap().unwrap();
    let after = projection_digest::oracle(&source.readers.get()).unwrap();
    for table in ["slo_pending", "slo_episode_heads", "slo_episode_events"] { assert_eq!(before[table], after[table], "{table}"); }
}

#[test]
fn concurrent_misses_and_prepared_dispatches_create_one_mission() {
    let source = Arc::new(source());
    let callers = (0..8).map(|_| {
        let source = source.clone();
        std::thread::spawn(move || miss(&source, "person-read"))
    }).collect::<Vec<_>>();
    let episodes = callers.into_iter().map(|thread| thread.join().unwrap()).collect::<BTreeSet<_>>();
    assert_eq!(episodes.len(), 1);
    let prepared = (0..2).map(|_| source.prepare_slo_missions("/tmp").unwrap().remove(0)).collect::<Vec<_>>();
    let dispatches = prepared.into_iter().map(|prepared| {
        let source = source.clone();
        std::thread::spawn(move || source.dispatch_slo_mission(prepared).unwrap())
    }).collect::<Vec<_>>();
    let missions = dispatches.into_iter().map(|thread| thread.join().unwrap()).collect::<BTreeSet<_>>();
    assert_eq!(missions.len(), 1);
    let records = claims(&source);
    assert_eq!(records.iter().filter(|claim| claim.kind == KIND && claim.body["fields"]["record"]["event"] == "pending").count(), 1);
    assert_eq!(records.iter().filter(|claim| claim.kind == "mission-run.created").count(), 1);
}

#[test]
fn a_later_miss_and_unknown_coverage_fence_out_late_healthy_cuts() {
    let source = source();
    let episode = miss(&source, "person-read"); // [0,1h]
    let later = reading(&source, 2*3_600_000, Some(120.0)); // [2h,3h]
    assert_eq!(source.record_slo_miss("person-read", None, later.clone(), None).unwrap().unwrap(), episode);
    let late = reading(&source, 3_600_000, Some(80.0)); // [1h,2h], names onset
    assert!(source.recover_slo_breach(&episode, late, None).is_err());
    assert_ne!(fact_state(&source, &episode), "recovered");
    let mut unknown = reading(&source, 3*3_600_000, None); unknown.complete = false;
    assert!(source.record_slo_miss("person-read", None, unknown, None).unwrap().is_none());
    let mut too_old = reading(&source, 3*3_600_000, Some(80.0)); too_old.predecessor_cut = Some(later.source_cut.clone());
    assert!(source.recover_slo_breach(&episode, too_old, None).is_err());
    let mut healthy = reading(&source, 4*3_600_000, Some(80.0)); healthy.predecessor_cut = Some(later.source_cut);
    source.recover_slo_breach(&episode, healthy, None).unwrap();
    assert_eq!(fact_state(&source, &episode), "recovered");
}

#[test]
fn cpu_cuts_must_match_the_declared_hour_and_required_worker_topology() {
    let source = source();
    let mut observation = reading(&source, 0, Some(0.6));
    observation.version = "combined-idle/v1".into(); observation.checks[0].unit = "cores".into(); observation.checks[0].target = 0.5;
    let mut two_hours = cpu_witness(&observation, 0.6); two_hours.monotonic_end_ns *= 2;
    assert!(source.record_slo_miss("cpu/idle", None, observation.clone(), Some(two_hours)).is_err());
    let mut wrong_cut = cpu_witness(&observation, 0.6); wrong_cut.source_cut = "other-cut".into();
    assert!(source.record_slo_miss("cpu/idle", None, observation.clone(), Some(wrong_cut)).is_err());
    let mut daemon_only = cpu_witness(&observation, 0.6); daemon_only.processes.pop();
    assert!(source.record_slo_miss("cpu/idle", None, observation.clone(), Some(daemon_only)).is_err());
    let mut unproved_zero = cpu_witness(&observation, 0.6); unproved_zero.expected_workers = 0; unproved_zero.processes.pop();
    assert!(source.record_slo_miss("cpu/idle", None, observation.clone(), Some(unproved_zero)).is_err());
    let mut omitted = cpu_witness(&observation, 0.6); omitted.expected_workers = 2;
    assert!(source.record_slo_miss("cpu/idle", None, observation.clone(), Some(omitted.clone())).is_err());
    let mut worker = omitted.processes[1].clone(); worker.pid = 12; worker.birth = 3; omitted.processes.push(worker);
    source.record_slo_miss("cpu/idle", None, observation, Some(omitted)).unwrap().unwrap();
}

#[test]
fn empty_new_tables_change_root_bytes_but_mixed_registry_transport_waits_to_compare() {
    let source = source();
    // The projection comparison is the optional divergence diagnostic, off unless asked for.
    source.capture_divergences(true);
    let identity = key(); admit(&source, "birch", &identity, "invite", None);
    let old = node("birch", Some(&identity), None);
    old.pin_fleet_anchor(&source.member_public_key().unwrap()).unwrap();
    let mut registry = fractal_schema::registry().clone(); registry.claims.remove(KIND).unwrap(); old.set_claim_registry(registry);
    sync(&source, &old); sync(&old, &source);
    let mut legacy = old.export_replication_summary(&source.bound_fleet().unwrap().unwrap()).unwrap();
    let canonical_before = claims(&source).into_iter().map(|claim| claim.id).collect::<BTreeSet<_>>();
    let digest_before = projection_digest::oracle(&source.readers.get()).unwrap();
    let full = legacy.projection_digests.clone();
    for table in ["slo_pending", "slo_episode_heads", "slo_episode_events"] { legacy.projection_digests.remove(table).unwrap(); }
    assert_ne!(projection_digest::root(&full), projection_digest::root(&legacy.projection_digests), "empty table names are hashed");
    legacy.graph_digest = projection_digest::root(&legacy.projection_digests);
    source.receive_replication_exchange("birch", &source.bound_fleet().unwrap().unwrap(), &legacy).unwrap();
    let status = source.replication_status_sealed(true, source.bound_fleet().unwrap().as_deref(), &["birch".into()]).unwrap();
    let peer = status.peers.into_iter().find(|peer| peer.peer == "birch").unwrap();
    assert!(peer.projection_comparison_waiting);
    assert!(peer.projection_comparison_waiting_reason.as_deref().unwrap().contains("builds differ"));
    assert!(peer.projection_graphs_equal.is_none(), "unknown comparison must not be reported as equal");
    assert!(peer.differing_tables.is_empty());
    let sync = peer.sync.unwrap();
    assert!(sync.graph_differs_since_unix_ms.is_none());
    assert!(!sync.diverged);
    assert_eq!(canonical_before, claims(&source).into_iter().map(|claim| claim.id).collect());
    assert_eq!(digest_before, projection_digest::oracle(&source.readers.get()).unwrap(), "metadata comparison must not repair or replay projections");
}

#[test]
fn source_restart_requires_fresh_coverage_and_retains_the_canonical_obligation() {
    let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("restart.sqlite3");
    let identity = key();
    smallclaims::store::set_thread_clock(Some(u128::from(SOURCE_START)));
    let source = Store::open(&path, "alder").unwrap();
    smallclaims::store::set_thread_clock(None);
    source.bind_fleet("7c9e6679-7425-40de-944b-e07fc1f90ae7").unwrap();
    source.pin_fleet_anchor(identity.public()).unwrap(); source.set_member_key(Some(identity.clone())).unwrap();
    admit(&source, "alder", &identity, "anchor", None); source.project_replication_backlog().unwrap(); source.fleet_membership().unwrap();
    let episode = miss(&source, "person-read");
    let onset_cut = reading(&source, 0, Some(120.0)).source_cut;
    let late = reading(&source, 3_600_000, Some(80.0));
    source.record_slo_miss("person-read", None, reading(&source, 2*3_600_000, Some(120.0)), None).unwrap();
    drop(source);
    smallclaims::store::set_thread_clock(Some(u128::from(SOURCE_START+3*3_600_000)));
    let reopened = Store::open(&path, "alder").unwrap();
    smallclaims::store::set_thread_clock(None);
    reopened.set_member_key(Some(identity)).unwrap(); reopened.fleet_membership().unwrap();
    assert!(reopened.recover_slo_breach(&episode, late, None).is_err());
    let mut fresh = reading(&reopened, 0, Some(80.0)); fresh.predecessor_cut = Some(onset_cut);
    reopened.recover_slo_breach(&episode, fresh, None).unwrap();
    assert_eq!(fact_state(&reopened, &episode), "recovered");
}

#[test]
fn worst_supported_opening_group_reports_writer_cpu_and_signed_bytes() {
    let source = source();
    let mut observation = reading(&source, 0, Some(0.6));
    observation.version = "v".repeat(90); observation.population = "p".repeat(90); observation.source_cut = "c".repeat(90);
    observation.checks[0].unit = "cores".into(); observation.checks[0].target = 0.5;
    for index in 0..8 { observation.counters.insert(format!("counter-{index}"), u64::MAX); }
    // Fill the admitted summary near its 1 KiB bound without inventing a sample.
    while observation.validate().is_err() {
        if observation.population.len()>1 { observation.population.pop(); }
        else if observation.version.len()>1 { observation.version.pop(); }
        else { panic!("maximum-summary fixture has an invalid non-size field"); }
    }
    while observation.version.len()<128 {
        let mut next = observation.clone(); next.version.push('v');
        if next.validate().is_err() { break; } observation = next;
    }
    observation.validate().unwrap();
    let mut witness = cpu_witness(&observation, 0.6); witness.expected_workers = 8;
    for index in 0..7 { let mut worker = witness.processes[1].clone(); worker.pid = 12+index; worker.birth = 3+u64::from(index); witness.processes.push(worker); }
    let before = process_cpu_ns();
    let (result, hold) = smallclaims::windows::holds::longest_during(|| {
        source.record_slo_miss("cpu/idle", None, observation, Some(witness)).unwrap().unwrap();
        source.dispatch_slo_mission(source.prepare_slo_missions(&format!("/{}", "w".repeat(1000))).unwrap().remove(0))
    });
    result.unwrap();
    let cpu = process_cpu_ns().saturating_sub(before);
    let exchange = source.export_replication_exchange(&source.bound_fleet().unwrap().unwrap(), &smallclaims::replication::ReplicationInventory::default()).unwrap();
    let wire = exchange.envelopes.iter().map(|envelope| serde_json::to_vec(envelope).unwrap().len()).max().unwrap();
    assert!(wire<=MAX_GROUP_BYTES);
    eprintln!("SLO_WORST_GROUP workers=8 source_summary_bound=1024 workspace_bytes=1001 writer_hold_ms={} source_process_cpu_ms={} signed_wire_max_bytes={wire} writer_hold_reference=st2/3080 qualification=HOSTED_CONTROL_ONLY",
        hold.as_secs_f64()*1000.0,cpu as f64/1_000_000.0);
    assert!(hold<=Duration::from_millis(500));
}

#[test]
fn a_two_hour_average_cannot_recover_a_one_hour_idle_reading() {
    let source = source();
    let mut onset = reading(&source, 0, Some(0.6));
    onset.version = "idle/v1".into(); onset.checks[0].unit = "cores".into(); onset.checks[0].target = 0.5;
    let original = cpu_witness(&onset, 0.6);
    let episode = source.record_slo_miss("cpu/idle", None, onset.clone(), Some(original.clone())).unwrap().unwrap();
    let mut healthy = reading(&source, 3_600_000, Some(0.4));
    healthy.version = onset.version; healthy.checks[0].unit = "cores".into(); healthy.checks[0].target = 0.5;
    let mut doubled = cpu_witness(&healthy, 0.4);
    doubled.monotonic_end_ns += 3_600_000_000_000;
    doubled.processes[0].cpu_start_ns = original.processes[0].cpu_end_ns;
    doubled.processes[0].cpu_end_ns = doubled.processes[0].cpu_start_ns + 2*cpu_witness(&healthy, 0.4).processes[0].cpu_end_ns;
    // Numerator / two hours is exactly the declared 0.4: numeric consistency
    // alone passes the old discriminator. Only same-window binding rejects it.
    assert!(source.recover_slo_breach(&episode, healthy.clone(), Some(doubled)).is_err());
    let mut proper = cpu_witness(&healthy, 0.4);
    proper.processes[0].cpu_start_ns = original.processes[0].cpu_end_ns;
    proper.processes[0].cpu_end_ns += proper.processes[0].cpu_start_ns;
    source.recover_slo_breach(&episode, healthy, Some(proper)).unwrap();
}
