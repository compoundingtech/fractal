//! Hosted-only controls. These fixtures do not sample or mutate the host daemon.
use super::*;
use crate::store::fleet_admission_tests::{admit, key, node, sync};
use st3_schema::slo_breaches::ProcessDelta;
use std::time::{Duration, Instant};

fn source() -> Store {
    let identity = key();
    let store = node("alder", Some(&identity), Some(&identity));
    admit(&store, "alder", &identity, "anchor", None);
    store.project_replication_backlog().unwrap();
    store.fleet_membership().unwrap();
    store
}

fn declare_owner(store: &Store) {
    let source = "version 2\nagent \"fleet/smalltalk/speed\" { workspace \"/tmp\"; command \"true\"; restart \"never\" }";
    let intent = crate::graph::parse_test_intent(source, "alder").unwrap();
    let planned = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &planned.subject_tokens, "declare-owner").unwrap();
}

fn reading(start: u64, value: Option<f64>) -> Observation {
    Observation {
        member: "alder".into(), epoch: "fixture/1".into(), sequence: start/3_600_000+1,
        predecessor_cut: (start>0).then(|| format!("cut/{}", start-3_600_000)),
        version: "person-read/v1".into(), start_ms: start, end_ms: start + 3_600_000,
        observed_at_ms: start + 3_600_000, window_ms: 3_600_000, covered_ms: 3_600_000,
        complete: true, population: "all eligible requests".into(), source_cut: format!("cut/{start}"),
        checks: vec![Check { unit: "ms".into(), value, target: 100.0, minimum: false }],
        counters: BTreeMap::new(),
    }
}

fn miss(store: &Store, objective: &str) -> String {
    store.record_slo_miss(objective, None, reading(0, Some(120.0)), None).unwrap().unwrap()
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

fn request(key: &str) -> WorkRequest {
    WorkRequest { actor: Some(DEFAULT_OWNER.into()), incarnation: Some("prefix-control".into()),
        summary: None, reason: None, evidence: vec![], wait_ms: None, idempotency_key: key.into() }
}

#[test]
fn unknown_partial_and_nonfinite_readings_cannot_open_a_breach() {
    let store = source();
    for mut observation in [reading(0, None), reading(0, Some(80.0))] {
        assert!(store.record_slo_miss("person-read", None, observation.clone(), None).unwrap().is_none());
        observation.complete = false;
        observation.checks[0].value = Some(120.0);
        assert!(store.record_slo_miss("person-read", None, observation, None).unwrap().is_none());
    }
    assert!(store.record_slo_miss("person-read", None, reading(0, Some(f64::NAN)), None).is_err());
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
    assert!(store.record_slo_miss("objective/65", None, reading(0, Some(120.0)), None).is_err());
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
fn recovery_uses_the_objectives_actual_window_and_preserves_the_fix() {
    let store = source();
    declare_owner(&store);
    let episode = miss(&store, "person-read");
    let run = store.dispatch_slo_mission(store.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    let fix = store.work(None, true).unwrap().into_iter().find(|step| step.step == "fix").unwrap();
    store.set_step_state(&fix.subject, "ready", None).unwrap();
    store.work_action(&fix.subject, "claim", &request("active-fix")).unwrap();
    assert_eq!(fact_state(&store, &episode), "ready");
    let before = store.work(None, true).unwrap();
    let mut stale = reading(0, Some(80.0));
    assert!(store.recover_slo_breach(&episode, stale.clone(), None).is_err());
    stale = reading(3_600_000, Some(80.0));
    store.recover_slo_breach(&episode, stale, None).unwrap();
    assert_eq!(fact_state(&store, &episode), "recovered");
    assert!(execution_admitted(&store.readers.get(), &run).unwrap(), "an admitted fix remains admitted");
    let after = store.work(None, true).unwrap();
    assert_eq!(before.iter().map(|step| (&step.subject, &step.status, &step.claimant)).collect::<Vec<_>>(),
        after.iter().map(|step| (&step.subject, &step.status, &step.claimant)).collect::<Vec<_>>());
    store.work_action(&fix.subject, "renew", &request("renew-active-after-health")).unwrap();
    assert!(store.record_slo_miss("person-read", None, reading(0, Some(120.0)), None).is_err(), "a stale miss must not start a later episode");
}

#[test]
fn a_new_breach_requires_the_previous_recovery_and_one_mission() {
    let store = source();
    let first = miss(&store, "person-read");
    store.recover_slo_breach(&first, reading(3_600_000, Some(80.0)), None).unwrap();
    assert_eq!(store.record_slo_miss("person-read", None, reading(7_200_000, Some(120.0)), None).unwrap().unwrap(), first,
        "a recovered pending episode still owes its mission");
    store.dispatch_slo_mission(store.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    let second = store.record_slo_miss("person-read", None, reading(7_200_000, Some(120.0)), None).unwrap().unwrap();
    assert_ne!(first, second);
    assert!(store.recover_slo_breach(&first, reading(10_800_000, Some(80.0)), None).is_ok(), "duplicate recovery is a no-op");
    assert_eq!(fact_state(&store, &second), "pending");
}

#[test]
fn owner_absence_is_named_once_without_fallback_or_sample_writes() {
    let store = source();
    let episode = miss(&store, "person-read");
    store.dispatch_slo_mission(store.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    assert_eq!(fact_state(&store, &episode), "owner_unavailable");
    let (_, blocked) = event(&store.readers.get(), &episode, Event::OwnerBlocked).unwrap().unwrap();
    assert_eq!(blocked.owner, DEFAULT_OWNER);
    assert!(store.unblock_slo_owner(&episode).is_err());
    let count = claims(&store).len();
    assert!(store.prepare_slo_missions("/tmp").unwrap().is_empty());
    assert_eq!(claims(&store).len(), count);
    assert!(store.work(None, true).unwrap().iter().filter(|step| step.step == "fix")
        .all(|step| step.assigned_to.as_deref() == Some(DEFAULT_OWNER)));
}

fn cpu_witness(cores: f64) -> HealthyWitness {
    let hour = 3_600_000_000_000u64;
    HealthyWitness { monotonic_start_ns: 0, monotonic_end_ns: hour, boundary_uncertainty_ns: 0,
        complete_cohort: true, positively_idle: true, processes: vec![
            ProcessDelta { role: "daemon".into(), pid: 10, birth: 1, cpu_start_ns: 0, cpu_end_ns: (cores * hour as f64) as u64 },
            ProcessDelta { role: "replication-worker".into(), pid: 11, birth: 2, cpu_start_ns: 0, cpu_end_ns: 0 },
        ] }
}

#[test]
fn idle_cpu_requires_full_eligible_cohort_and_consistent_counters() {
    let store = source();
    let mut observation = reading(0, Some(0.6));
    observation.checks[0].unit = "cores".into(); observation.checks[0].target = 0.5;
    observation.version = "combined-idle/v1".into();
    assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), None).is_err());
    let mut missing = cpu_witness(0.6); missing.processes.pop(); missing.complete_cohort = false;
    let mut duplicate = cpu_witness(0.6); duplicate.processes[1] = duplicate.processes[0].clone();
    let mut birth_unknown = cpu_witness(0.6); birth_unknown.processes[1].birth = 0;
    let mut short = cpu_witness(0.6); short.monotonic_end_ns -= 1;
    for witness in [missing, duplicate, birth_unknown, short] {
        assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), Some(witness)).is_err());
    }
    let mut busy = cpu_witness(0.6); busy.positively_idle = false;
    assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), Some(busy)).is_err());
    let mut reset = cpu_witness(0.6); reset.processes[1].cpu_start_ns = 1;
    assert!(store.record_slo_miss("cpu/idle", None, observation.clone(), Some(reset)).is_err());
    let episode = store.record_slo_miss("cpu/idle", None, observation.clone(), Some(cpu_witness(0.6))).unwrap().unwrap();
    observation.start_ms = 3_600_000; observation.end_ms = 7_200_000; observation.observed_at_ms = observation.end_ms;
    observation.sequence = 2; observation.predecessor_cut = Some(observation.source_cut.clone());
    observation.source_cut = "cut/healthy".into(); observation.checks[0].value = Some(0.4);
    let mut healthy = cpu_witness(0.4);
    healthy.monotonic_start_ns = 3_600_000_000_000;
    healthy.monotonic_end_ns += healthy.monotonic_start_ns;
    healthy.processes[0].cpu_start_ns = cpu_witness(0.6).processes[0].cpu_end_ns;
    healthy.processes[0].cpu_end_ns += healthy.processes[0].cpu_start_ns;
    let mut partial = healthy.clone(); partial.boundary_uncertainty_ns = 1;
    assert!(store.recover_slo_breach(&episode, observation.clone(), Some(partial)).is_err());
    assert!(store.recover_slo_breach(&episode, observation.clone(), Some(cpu_witness(0.4))).is_err());
    store.recover_slo_breach(&episode, observation, Some(healthy)).unwrap();
}

#[test]
fn a_signed_replica_reduces_but_does_not_originate_work() {
    let source = source();
    let episode = miss(&source, "person-read");
    let identity = key();
    admit(&source, "birch", &identity, "invite", None);
    let replica = node("birch", Some(&identity), None);
    replica.pin_fleet_anchor(&source.member_public_key().unwrap()).unwrap();
    sync(&source, &replica);
    replica.fleet_membership().unwrap();
    assert_eq!(fact_state(&replica, &episode), "pending");
    assert!(replica.prepare_slo_missions("/tmp").unwrap().is_empty());
    assert!(replica.work(None, true).unwrap().is_empty());
    let source_count = claims(&source).len();
    sync(&replica, &source);
    assert_eq!(claims(&source).len(), source_count);
}

#[test]
fn replica_admission_rejects_relabelled_origin_or_non_runtime_actor() {
    let store = source();
    miss(&store, "person-read");
    let claim = claims(&store).into_iter().find(|claim| claim.kind == KIND).unwrap();
    assert!(matches!(classify_replicated_claim_with_registry(&claim, st3_schema::registry()).unwrap(), ReplicatedClaimAdmission::Valid));
    for forged in [true, false] {
        let mut bad = claim.clone();
        if forged { bad.origin = "birch".into(); } else { bad.actor = Some(DEFAULT_OWNER.into()); }
        assert!(classify_replicated_claim_with_registry(&bad, st3_schema::registry()).is_err());
    }
}

/// Replays the actual ordinary known claims, not a substitute mission state machine.
/// The registry fixture models a supported older build keeping new kinds unknown.
#[test]
fn recovered_before_dispatch_refuses_every_known_claim_prefix() {
    let source = source();
    let episode = miss(&source, "person-read");
    source.recover_slo_breach(&episode, reading(3_600_000, Some(80.0)), None).unwrap();
    let run = source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    assert!(!execution_admitted(&source.readers.get(), &run).unwrap());
    let canonical = claims(&source).into_iter().filter(|claim| !claim.kind.starts_with("fleet.")).collect::<Vec<_>>();
    let mut old_registry = st3_schema::registry().clone();
    old_registry.claims.remove(KIND).unwrap();
    assert!(canonical.len() <= 8, "prefix control must stay bounded");
    let fix_subject = source.work(None, true).unwrap().into_iter().find(|step| step.step == "fix").unwrap().subject;
    // Every possible prefix is a subset of this finite group of known/new claims.
    // Test every subset in forward and reverse arrival order, including omission
    // of cancellation and/or recovery, and both build registries.
    for old in [false, true] {
        for reverse in [false, true] {
            for mask in 0..(1usize << canonical.len()) {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("prefix.sqlite3");
                let target = Arc::new(Store::open(&path, "prefix").unwrap());
                let mut chosen = canonical.iter().enumerate().filter(|(index, _)| mask & (1<<index) != 0)
                    .map(|(_, claim)| claim).collect::<Vec<_>>();
                if reverse { chosen.reverse(); }
                for claim in chosen {
                    if old && !matches!(classify_replicated_claim_with_registry(claim, &old_registry).unwrap(), ReplicatedClaimAdmission::Valid) { continue; }
                    target.connection.batched(|tx| {
                        append_claim_tx(tx, &claim.origin, &claim.subject, &claim.kind, claim.actor.as_deref(), &claim.body, &[], None)?;
                        Ok::<_,anyhow::Error>(())
                    }).unwrap().unwrap();
                }
                // Missing ordinary prerequisites may defer projection; an error must
                // leave the previous projection non-executable as well. Import and
                // projection have separate durability boundaries in the actual store.
                let _projection = target.connection.batched(|tx| replay_graph_from_nothing_tx(tx)).unwrap();
                assert!(!crate::reconcile::tests::slo_breach_tests::gate_passes(target.clone(), &episode),
                    "old={old} reverse={reverse} mask={mask}: actual Field gate passed");
                let selected = target.work(Some(DEFAULT_OWNER), true).unwrap();
                assert!(selected.iter().all(|step| step.step != "fix" || step.status != "ready"));
                let error = target.work_action(&fix_subject, "claim", &request(&format!("prefix/{old}/{reverse}/{mask}"))).unwrap_err();
                assert!(matches!(error.code.as_str(), "missing-step-run" | "work-not-ready" | "terminal-work-owner" | "terminal-step-run"),
                    "the normal claim must refuse for readiness/absence, not an unrelated missing fixture: {error:?}");
                let _checkpoint_projection = target.connection.batched(|tx| checkpoint_rules::replay_from_nothing(tx)).unwrap();
                assert!(!crate::reconcile::tests::slo_breach_tests::gate_passes(target.clone(), &episode));
                drop(target);
                let reopened = Arc::new(Store::open(&path, "prefix").unwrap());
                assert!(!crate::reconcile::tests::slo_breach_tests::gate_passes(reopened.clone(), &episode));
                assert!(reopened.work_for_reconcile(DEFAULT_OWNER).unwrap().iter()
                    .all(|step| step.step != "fix" || step.status != "ready"));
                assert!(reopened.work_action(&fix_subject, "claim", &request(&format!("reopen/{old}/{reverse}/{mask}"))).is_err());
            }
        }
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
    eprintln!("SLO_COST objective=person-read episode={episode} full_dispatch_elapsed_ms={} writer_hold_ms={} replica_hold_ms={} source_process_cpu_ms={} replica_process_cpu_ms={} writer_hold_reference=st2/3080 qualification=HOSTED_CONTROL_ONLY test_threads_required=1",
        begin.elapsed().as_secs_f64()*1000.0, writer_hold.as_secs_f64()*1000.0, replica_hold.as_secs_f64()*1000.0,
        source_cpu_ns as f64/1_000_000.0, replica_cpu_ns as f64/1_000_000.0);
    // Debug-runner ceiling only; meeting production budgets still requires Intake's
    // measured comparison with the current writer-hold baseline before activation.
    assert!(writer_hold <= Duration::from_millis(500));
    assert!(replica_hold <= Duration::from_millis(500));
}

#[test]
fn real_field_gate_accepts_only_a_positive_admitted_episode() {
    let source = Arc::new(source());
    declare_owner(&source);
    let episode = miss(&source, "person-read");
    assert!(!crate::reconcile::tests::slo_breach_tests::gate_passes(source.clone(), &episode));
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    assert!(crate::reconcile::tests::slo_breach_tests::gate_passes(source.clone(), &episode));
    source.recover_slo_breach(&episode, reading(3_600_000, Some(80.0)), None).unwrap();
    assert!(crate::reconcile::tests::slo_breach_tests::gate_passes(source.clone(), &episode),
        "recovery must not withdraw an admitted ordinary gate");
}

#[test]
fn a_supported_old_registry_keeps_signed_episode_claims_unknown_until_upgrade() {
    let source = source();
    let episode = miss(&source, "person-read");
    source.recover_slo_breach(&episode, reading(3_600_000, Some(80.0)), None).unwrap();
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.sqlite3");
    let old = Store::open(&path, "birch").unwrap();
    old.bind_fleet(&source.bound_fleet().unwrap().unwrap()).unwrap();
    let mut registry = st3_schema::registry().clone();
    registry.claims.remove(KIND).unwrap();
    old.set_claim_registry(registry);
    let admission = sync(&source, &old);
    assert_eq!(admission.invalid, 0);
    assert_eq!(admission.unknown, 3);
    assert_eq!(fact_state(&old, &episode), "unknown");
    assert!(old.work(Some(DEFAULT_OWNER), true).unwrap().iter().all(|step| step.status != "ready"));
    drop(old);
    let upgraded = Store::open(&path, "birch").unwrap();
    assert_eq!(upgraded.validate_replication_backlog().unwrap().invalid, 0);
    upgraded.project_replication_backlog().unwrap();
    assert_eq!(fact_state(&upgraded, &episode), "recovered");
    assert!(upgraded.prepare_slo_missions("/tmp").is_err() || upgraded.prepare_slo_missions("/tmp").unwrap().is_empty());
    assert!(upgraded.work(Some(DEFAULT_OWNER), true).unwrap().iter().all(|step| step.status != "ready"));
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
    let mut remote = reading(0, Some(120.0)); remote.member = "birch".into();
    assert!(source.record_slo_miss("person-read", None, remote, None).is_err());
    let episode = miss(&source, "person-read");
    for mode in 0..3 {
        let mut healthy = reading(3_600_000, Some(80.0));
        match mode { 0 => healthy.member = "birch".into(), 1 => healthy.sequence = 1,
            _ => healthy.predecessor_cut = Some("unrelated-cut".into()) }
        assert!(source.recover_slo_breach(&episode, healthy, None).is_err());
    }
}

#[test]
fn database_recovery_needs_its_full_day_not_an_hour() {
    let source = source();
    let day = 86_400_000;
    let mut onset = reading(0, Some(120.0));
    onset.window_ms = day; onset.covered_ms = day; onset.end_ms = day; onset.observed_at_ms = day;
    let episode = source.record_slo_miss("database", None, onset, None).unwrap().unwrap();
    let mut healthy = reading(day, Some(80.0)); healthy.predecessor_cut = Some("cut/0".into());
    assert!(source.recover_slo_breach(&episode, healthy.clone(), None).is_err());
    healthy.window_ms = day; healthy.covered_ms = day; healthy.end_ms = 2*day; healthy.observed_at_ms = 2*day;
    source.recover_slo_breach(&episode, healthy, None).unwrap();
}

#[test]
fn oversized_publication_rolls_back_without_losing_pending_intake() {
    let source = source();
    let episode = miss(&source, "person-read");
    let before = claims(&source).len();
    let batch = Uuid::now_v7().simple().to_string();
    let result = source.connection.batched(|tx| {
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
        source.connection.batched(|tx| append(tx, "alder", &pending, &Uuid::now_v7().simple().to_string()))
            .unwrap().unwrap();
    }
    source.recover_slo_breach(&episode, reading(3_600_000, Some(80.0)), None).unwrap();
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
    source.connection.batched(|tx| append(tx, "alder", &conflicting, &Uuid::now_v7().simple().to_string())).unwrap().unwrap();
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
    source.recover_slo_breach(&episode, reading(3_600_000, Some(80.0)), None).unwrap();
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    let (recovered, _) = event(&source.readers.get(), &episode, Event::Recovered).unwrap().unwrap();
    let replacement = claims(&source).into_iter().find(|claim| claim.kind == "fleet.member-admitted").unwrap().id;
    let identity = key();
    admit(&source, "birch", &identity, "invite", None);
    let old = node("birch", Some(&identity), None);
    old.pin_fleet_anchor(&source.member_public_key().unwrap()).unwrap();
    let mut registry = st3_schema::registry().clone(); registry.claims.remove(KIND).unwrap(); old.set_claim_registry(registry);
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
