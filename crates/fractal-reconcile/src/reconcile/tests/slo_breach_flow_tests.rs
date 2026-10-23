//! Hosted-only controls that need the work surface or the real reconciler, which sit above the store
//! crate. These fixtures do not sample or mutate the host daemon.
use super::*;
use crate::model::{ClaimRecord, IntentInput, WorkRequest};
use smallclaims::store::ReplicatedClaimAdmission;
use crate::store::{checkpoint, checkpoint_rules, classify_replicated_claim_with_registry, rules_digest};
use sha2::{Digest as _, Sha256};
use crate::store::fleet_admission_tests_shared_seats::{admit, key, node, sync};
use crate::store::{SloCheck as Check, SloObservation as Observation};
use fractal_schema::slo_breaches::{DEFAULT_OWNER, Event, KIND};
use serde_json::json;


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


fn declare_owner(store: &Store) {
    let source = "version 2\nagent \"fleet/fixture-slo/owner\" { workspace \"/tmp\"; command \"true\"; restart \"never\" }";
    let intent = crate::graph::parse_test_intent(source, "alder").unwrap();
    let planned = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &planned.subject_tokens, "declare-owner").unwrap();
    store.append_claim(&ClaimInput { subject: DEFAULT_OWNER.into(), kind: "harness.observed".into(), actor: Some(DEFAULT_OWNER.into()),
        fields: serde_json::from_value(json!({"state":"idle","driver":"codex","incarnation_id":"prefix-control"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None }).unwrap();
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
    store.record_slo_miss(objective, None, reading(store, 0, Some(120.0)), None).unwrap().unwrap()
}


fn claims(store: &Store) -> Vec<ClaimRecord> {
    store.fixture_claims()
}

fn admit_fixture_claim(source: &Store, target: &Store, claim: &ClaimRecord) {
    target.fixture_admit_claim(source, claim);
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
    let mut stale = reading(&store, 0, Some(80.0));
    assert!(store.recover_slo_breach(&episode, stale.clone(), None).is_err());
    stale = reading(&store, 3_600_000, Some(80.0));
    store.recover_slo_breach(&episode, stale, None).unwrap();
    assert_eq!(fact_state(&store, &episode), "recovered");
    assert!(store.fixture_execution_admitted(&run).unwrap(), "an admitted fix remains admitted");
    let after = store.work(None, true).unwrap();
    assert_eq!(before.iter().map(|step| (&step.subject, &step.status, &step.claimant)).collect::<Vec<_>>(),
        after.iter().map(|step| (&step.subject, &step.status, &step.claimant)).collect::<Vec<_>>());
    store.work_action(&fix.subject, "renew", &request("renew-active-after-health")).unwrap();
    assert!(store.record_slo_miss("person-read", None, reading(&store, 0, Some(120.0)), None).is_err(), "a stale miss must not start a later episode");
}


#[test]
fn owner_absence_is_named_once_without_fallback_or_sample_writes() {
    let store = source();
    let episode = miss(&store, "person-read");
    store.dispatch_slo_mission(store.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    assert_eq!(fact_state(&store, &episode), "owner_unavailable");
    let (_, blocked) = store.fixture_event(&episode, Event::OwnerBlocked).unwrap().unwrap();
    assert_eq!(blocked.owner, DEFAULT_OWNER);
    assert!(store.unblock_slo_owner(&episode).is_err());
    let count = claims(&store).len();
    assert!(store.prepare_slo_missions("/tmp").unwrap().is_empty());
    assert_eq!(claims(&store).len(), count);
    assert!(store.work(None, true).unwrap().iter().filter(|step| step.step == "fix")
        .all(|step| step.assigned_to.as_deref() == Some(DEFAULT_OWNER)));
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


/// Replays the actual ordinary known claims, not a substitute mission state machine.
/// The registry fixture models a supported older build keeping new kinds unknown.
#[test]
fn recovered_before_dispatch_refuses_every_known_claim_prefix() {
    let source = source();
    declare_owner(&source);
    let baseline = claims(&source).into_iter().map(|claim| claim.id).collect::<BTreeSet<_>>();
    let episode = miss(&source, "person-read");
    source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
    let run = source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    assert!(!source.fixture_execution_admitted(&run).unwrap());
    // Cancellation is intentionally absent: this matrix proves the readiness fence,
    // rather than succeeding because an ordinary terminal run rejects claims.
    let canonical = claims(&source).into_iter().filter(|claim| !baseline.contains(&claim.id)
        && claim.kind != "mission-run.state").collect::<Vec<_>>();
    let mut old_registry = fractal_schema::registry().clone();
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
                let target = Arc::new(Store::open(&path, "alder").unwrap());
                declare_owner(&target);
                let mut chosen = canonical.iter().enumerate().filter(|(index, _)| mask & (1<<index) != 0)
                    .map(|(_, claim)| claim).collect::<Vec<_>>();
                if reverse { chosen.reverse(); }
                for claim in chosen {
                    if old && !matches!(classify_replicated_claim_with_registry(claim, &old_registry).unwrap(), ReplicatedClaimAdmission::Valid) { continue; }
                    admit_fixture_claim(&source,&target,claim);
                }
                // Missing ordinary prerequisites may defer projection; an error must
                // leave the previous projection non-executable as well. Import and
                // projection have separate durability boundaries in the actual store.
                let _projection = target.fixture_replay_graph().unwrap();
                assert!(!super::slo_breach_tests::gate_passes(target.clone(), &episode),
                    "old={old} reverse={reverse} mask={mask}: actual Field gate passed");
                let raw: Option<String> = target.fixture_step_status(&fix_subject);
                if let Some(raw) = raw { assert_eq!(raw, "pending", "the ordinary prerequisite, before the new ready-only guard, holds the old path"); }
                let selected = target.work(Some(DEFAULT_OWNER), true).unwrap();
                assert!(selected.iter().all(|step| step.step != "fix" || step.status != "ready"));
                let error = target.work_action(&fix_subject, "claim", &request(&format!("prefix/{old}/{reverse}/{mask}"))).unwrap_err();
                assert!(matches!(error.code, "missing-step-run" | "work-not-ready"),
                    "the normal claim must refuse for readiness/absence, not an unrelated missing fixture: {error:?}");
                let _checkpoint_projection = target.fixture_replay_checkpoint().unwrap();
                assert!(!super::slo_breach_tests::gate_passes(target.clone(), &episode));
                drop(target);
                let reopened = Arc::new(Store::open(&path, "alder").unwrap());
                assert!(!super::slo_breach_tests::gate_passes(reopened.clone(), &episode));
                assert!(reopened.work_for_reconcile(DEFAULT_OWNER).unwrap().iter()
                    .all(|step| step.step != "fix" || step.status != "ready"));
                assert!(reopened.work_action(&fix_subject, "claim", &request(&format!("reopen/{old}/{reverse}/{mask}"))).is_err());
            }
        }
    }
}


#[test]
fn real_field_gate_accepts_only_a_positive_admitted_episode() {
    let source = Arc::new(source());
    declare_owner(&source);
    let episode = miss(&source, "person-read");
    assert!(!super::slo_breach_tests::gate_passes(source.clone(), &episode));
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    assert!(super::slo_breach_tests::gate_passes(source.clone(), &episode));
    source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
    assert!(super::slo_breach_tests::gate_passes(source.clone(), &episode),
        "recovery must not withdraw an admitted ordinary gate");
}


#[test]
fn a_supported_old_registry_keeps_signed_episode_claims_unknown_until_upgrade() {
    let source = source();
    let episode = miss(&source, "person-read");
    source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.sqlite3");
    let old = Store::open(&path, "birch").unwrap();
    old.bind_fleet(&source.bound_fleet().unwrap().unwrap()).unwrap();
    let mut registry = fractal_schema::registry().clone();
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
fn initial_owner_block_is_positive_only_after_unblock_and_does_not_flap() {
    let source = Arc::new(source());
    let episode = miss(&source, "person-read");
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    assert!(!super::slo_breach_tests::gate_passes(source.clone(), &episode));
    declare_owner(&source);
    source.unblock_slo_owner(&episode).unwrap();
    assert!(super::slo_breach_tests::gate_passes(source.clone(), &episode));
    let intent = crate::graph::parse_internal_intent("version 2\nstop \"agent/fleet/fixture-slo/owner\"", "alder").unwrap();
    let planned = source.mission(&intent, IntentInput { kdl: "stop owner".into(), source_name: None }).unwrap();
    source.apply(&intent, &planned.subject_tokens, "owner-went-away").unwrap();
    let before = claims(&source).len();
    source.unblock_slo_owner(&episode).unwrap();
    assert_eq!(claims(&source).len(), before, "owner flaps do not create receipt storms");
    assert_eq!(fact_state(&source, &episode), "owner_unavailable");
    assert!(super::slo_breach_tests::gate_passes(source.clone(), &episode),
        "existing execution admission is preserved through ordinary owner liveness changes");
}


#[test]
fn admitted_before_owner_return_recovers_then_runs_the_real_dependency_and_claim_flow() {
    let source = Arc::new(source());
    let episode = miss(&source, "person-read");
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
    declare_owner(&source);
    source.unblock_slo_owner(&episode).unwrap();
    super::slo_breach_tests::drive_ordinary_flow(source.clone(), "alder");
    let fix = source.work(Some(DEFAULT_OWNER), true).unwrap().into_iter().find(|step| step.step == "fix").unwrap();
    assert_eq!(fix.status, "ready", "healthy is not evidence of a substantive repair");
    source.work_action(&fix.subject, "claim", &request("owner-return-after-recovery")).unwrap();
}


#[test]
fn a_positive_dispatch_runs_after_recovery_and_a_later_episode_without_manual_step_state() {
    let source = Arc::new(source());
    declare_owner(&source);
    let episode = miss(&source, "person-read");
    source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    // The agentless dispatch step has not run yet.
    source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
    source.record_slo_miss("person-read", None, reading(&source, 2*3_600_000, Some(120.0)), None).unwrap();
    super::slo_breach_tests::drive_ordinary_flow(source.clone(), "alder");
    let fix = source.work(Some(DEFAULT_OWNER), true).unwrap().into_iter().find(|step| step.step == "fix").unwrap();
    assert_eq!(fix.status, "ready");
    source.work_action(&fix.subject, "claim", &request("positive-after-later-episode")).unwrap();
    source.fixture_replay_graph().unwrap().unwrap();
    assert!(super::slo_breach_tests::gate_passes(source.clone(), &episode));
    source.work_action(&fix.subject, "renew", &request("positive-history-survives-replay")).unwrap();
}


#[test]
fn an_ordinary_mission_step_named_fix_keeps_normal_claim_admission() {
    let source = Arc::new(source()); declare_owner(&source);
    let kdl = format!("version 2\nmission \"ordinary\" state=\"ready\" {{ goal \"ordinary control\"; step \"fix\" {{ assigned-to {DEFAULT_OWNER:?} }} }}");
    let intent = crate::graph::parse_test_intent(&kdl, "alder").unwrap();
    let planned = source.mission(&intent, IntentInput { kdl, source_name: None }).unwrap();
    source.apply(&intent, &planned.subject_tokens, "ordinary-fix").unwrap();
    source.create_mission_run(&MissionRunRequest { mission: "ordinary".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/avery".into()), mode: Some("run".into()), inputs: BTreeMap::new(), idempotency_key: "ordinary-run".into() }).unwrap();
    super::slo_breach_tests::drive_ordinary_flow(source.clone(), "alder");
    let fix = source.work(Some(DEFAULT_OWNER), true).unwrap().into_iter().find(|step| step.step == "fix").unwrap();
    assert_eq!(fix.status, "ready");
    source.work_action(&fix.subject, "claim", &request("ordinary-positive-claim")).unwrap();
}


#[test]
fn different_checkpoint_rules_cannot_form_one_complete_verification_certificate() {
    use smallclaims::store::checkpoint_agreement::{CheckpointClaim, certificates};
    let source = source();
    let cut = 86_400_000u128;
    let checkpoint = checkpoint::checkpoint_name(cut);
    // The source compatibility change has a different identity from the prior rule
    // engine. This control exercises the real certificate reducer for that mismatch.
    let mut legacy = Sha256::new(); legacy.update(b"st3-checkpoint-rules-v1\0");
    legacy.update((crate::store::RULES_VERSION - 1).to_be_bytes());
    let prior_description = checkpoint_rules::rules_description().lines().filter(|line| !line.starts_with(KIND)).collect::<Vec<_>>().join("\n");
    legacy.update(prior_description.as_bytes());
    let legacy = hex::encode(legacy.finalize()); assert_ne!(legacy, rules_digest());
    let verification = |writer: &str, rules: &str| CheckpointClaim { id: format!("verified/{writer}"),
        kind: "checkpoint.verified".into(), subject: checkpoint.clone(), writer: writer.into(), actor: Some("daemon/runtime".into()),
        fields: serde_json::from_value(json!({"cut_unix_ms":cut,"participants":["alder","birch"],
            "sealed_digest":"same-seal","rules_digest":rules,"drop_digest":"same-drop","retained_digest":"same-retained",
            "graph_digest":"same-graph","reader_digest":"same-reader"})).unwrap() };
    let current = rules_digest();
    assert!(certificates(&[verification("alder", &current), verification("birch", &legacy)], &checkpoint).is_empty());
    assert_eq!(certificates(&[verification("alder", &current), verification("birch", &current)], &checkpoint).len(), 1);
    assert!(source.work(None, true).unwrap().is_empty(), "no checkpoint control starts work");
}


#[test]
fn the_same_live_owner_prefix_fixture_can_positively_claim_an_admitted_mission() {
    let original = source(); declare_owner(&original);
    let baseline = claims(&original).into_iter().map(|claim| claim.id).collect::<BTreeSet<_>>();
    let episode = miss(&original, "person-read");
    original.dispatch_slo_mission(original.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
    let onset = original.fixture_event(&episode, Event::Pending).unwrap().unwrap().0;
    let publication = claims(&original).into_iter().find(|claim| claim.kind == "mission.published").unwrap();
    assert!(checkpoint_rules::cited_by(&publication).any(|id| id == onset),
        "the ordinary evidence reference must preserve the mission's immutable gate source");
    let group = claims(&original).into_iter().filter(|claim| !baseline.contains(&claim.id)).collect::<Vec<_>>();
    let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("positive-prefix.sqlite3");
    let target = Arc::new(Store::open(&path, "alder").unwrap()); declare_owner(&target);
    for claim in group {
        admit_fixture_claim(&original,&target,&claim);
    }
    target.fixture_replay_graph().unwrap().unwrap();
    assert!(super::slo_breach_tests::gate_passes(target.clone(), &episode));
    super::slo_breach_tests::drive_ordinary_flow(target.clone(), "alder");
    let fix = target.work(Some(DEFAULT_OWNER), true).unwrap().into_iter().find(|step| step.step == "fix").unwrap();
    assert_eq!(fix.status, "ready");
    target.work_action(&fix.subject, "claim", &request("identical-positive-prefix")).unwrap();
}


#[test]
fn admitted_claim_history_and_gate_survive_both_delivery_orders_reopen_and_owner_loss() {
    for stop_first in [false, true] {
        let source = Arc::new(source()); declare_owner(&source);
        let episode = miss(&source, "person-read");
        source.dispatch_slo_mission(source.prepare_slo_missions("/tmp").unwrap().remove(0)).unwrap();
        super::slo_breach_tests::drive_ordinary_flow(source.clone(), "alder");
        let fix = source.work(Some(DEFAULT_OWNER), true).unwrap().into_iter().find(|step| step.step == "fix").unwrap();
        source.work_action(&fix.subject, "claim", &request("history-positive")).unwrap();
        let stop = || {
            let intent = crate::graph::parse_internal_intent("version 2\nstop \"agent/fleet/fixture-slo/owner\"", "alder").unwrap();
            let planned = source.mission(&intent, IntentInput { kdl: "stop fixture owner".into(), source_name: None }).unwrap();
            source.apply(&intent, &planned.subject_tokens, "history-owner-stop").unwrap();
        };
        if stop_first { stop(); }
        source.recover_slo_breach(&episode, reading(&source, 3_600_000, Some(80.0)), None).unwrap();
        source.record_slo_miss("person-read", None, reading(&source, 2*3_600_000, Some(120.0)), None).unwrap();
        if !stop_first { stop(); }
        let expected_claims = claims(&source).into_iter().filter(|claim| claim.subject == fix.subject)
            .map(|claim| claim.id).collect::<BTreeSet<_>>();
        let fleet = source.bound_fleet().unwrap().unwrap();
        for reverse in [false, true] {
            let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("history.sqlite3");
            let target = Store::open(&path, "birch").unwrap(); target.bind_fleet(&fleet).unwrap();
            target.pin_fleet_anchor(&source.member_public_key().unwrap()).unwrap();
            let mut exchange = source.export_replication_exchange(&fleet, &target.replication_inventory().unwrap()).unwrap();
            if reverse { exchange.envelopes.reverse(); }
            target.receive_replication_exchange("alder", &fleet, &exchange).unwrap();
            target.validate_replication_backlog().unwrap(); target.project_replication_backlog().unwrap();
            target.fixture_replay_checkpoint().unwrap().unwrap();
            drop(target);
            let reopened = Arc::new(Store::open(&path, "birch").unwrap());
            assert!(super::slo_breach_tests::gate_passes(reopened.clone(), &episode));
            let actual = reopened.work(Some(DEFAULT_OWNER), true).unwrap().into_iter().find(|step| step.subject == fix.subject).unwrap();
            assert_eq!(actual.claimant.as_deref(), Some(DEFAULT_OWNER));
            assert_eq!(actual.status, "claimed");
            let actual_claims = claims(&reopened).into_iter().filter(|claim| claim.subject == fix.subject)
                .map(|claim| claim.id).collect::<BTreeSet<_>>();
            assert_eq!(expected_claims, actual_claims);
        }
    }
}
