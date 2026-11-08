use super::*;
use super::migration_tests::{fixture, reading};

fn recorded_phase(store: &Store, opening: &Opening, phase: &str) -> crate::model::ClaimRecord {
    match phase_receipt(store, opening, phase).unwrap() {
        keys::Lookup::Recorded(receipt) => *receipt,
        keys::Lookup::Vacant(_) => panic!("phase is vacant"),
        keys::Lookup::Blocked => panic!("phase lookup is blocked"),
    }
}

fn occupy(store: &Store, key: &str) -> crate::model::ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: "custom/fixture/record".into(), kind: "custom.fixture.record".into(),
        actor: Some("agent/fixture.owner".into()), fields: BTreeMap::new(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: Some(key.into()),
    }).unwrap()
}

fn occupy_chain(store: &Store, root: &str, count: usize) {
    let mut key = root.to_owned();
    for _ in 0..count {
        let receipt = occupy(store, &key);
        key = format!("{root}:conflict:{}", receipt.id);
    }
}

fn plan(admission: &mut Admission, policy: &Policy, journal: &Path, end: u64) -> Opening {
    assert!(admission.plan(&policy.config, ShortSummary { end_ms: end, known: 5, failed: 3 }));
    admission.persist(journal).unwrap();
    admission.case.as_ref().unwrap().clone()
}

fn continued_lifecycle(conflicts: usize) {
    let (dir, policy, store) = fixture();
    let journal = dir.path().join("case.json");
    let end = (MINUTES + 10) * MINUTE_MS;
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, end);
    for suffix in ["early", "weekly", "notice:early", "notice:weekly"] {
        occupy_chain(&store, &format!("{}:{suffix}", opening.episode), conflicts);
    }
    admission.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
    assert!(admission.case.as_ref().unwrap().recorded);
    let early = recorded_phase(&store, &opening, "early");
    assert_eq!(early.kind, "operational.failure");
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    let weekly_miss = reading(end + MINUTE_MS, 11);
    admission.weekly(&journal, &policy.config, &store, &weekly_miss, weekly_miss.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    let weekly = recorded_phase(&store, &opening, "weekly");
    assert_eq!(weekly.body["evidence"], json!([early.id]));
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    let messages = store.messages(Some(&policy.config.reviewer), true).unwrap();
    assert_eq!(messages.len(), 2);
    for message in &messages {
        assert!(message.from == "daemon/runtime");
    }
    // Recovery receipts have the same ownership and continuation rules.
    for receipt in [&early.id, &weekly.id] {
        occupy_chain(&store, &format!("{receipt}:recovered"), conflicts);
    }
    let met = reading(end + 2 * MINUTE_MS, 0);
    admission.weekly(&journal, &policy.config, &store, &met, met.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert!(admission.weekly.recovered);
    assert_eq!(store.operational_failure(&opening.episode).unwrap().unwrap().status, "resolved");
    assert!(store.operational_failures().unwrap().iter()
        .any(|fault| fault.subject.starts_with("readiness-operation-conflict/")));
    let index = store.index().unwrap();
    // Lose local receipt acknowledgments, not the stable case: native continuations replay.
    admission.case.as_mut().unwrap().recorded = false;
    for notice in &mut admission.notices { notice.recorded = false; }
    admission.persist(&journal).unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("graph.db"), "fixture").unwrap();
    let mut restarted = Admission::load(&journal, &policy.config).unwrap();
    restarted.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
    drain_notices(&mut restarted, &journal, &store, None).unwrap();
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 2);
    assert_eq!(store.index().unwrap(), index);
}

#[test]
fn occupied_phase_notice_and_recovery_keys_continue_and_replay_once() {
    continued_lifecycle(1);
}

#[test]
fn four_conflicting_occupants_per_key_continue_without_new_state() {
    continued_lifecycle(4);
}

#[test]
fn capped_early_phase_is_not_accepted_and_weekly_work_still_progresses() {
    let (dir, policy, store) = fixture();
    let journal = dir.path().join("case.json");
    let end = (MINUTES + 10) * MINUTE_MS;
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, end);
    occupy_chain(&store, &format!("{}:early", opening.episode), 5);
    admission.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
    assert!(!admission.case.as_ref().unwrap().recorded);
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    assert!(admission.notices.is_empty(), "an unrelated receipt is never failure evidence");
    let missed = reading(end + MINUTE_MS, 11);
    admission.weekly(&journal, &policy.config, &store, &missed, missed.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert!(admission.weekly.escalated);
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 1);
    assert!(store.fault_delivery_snapshot(u128::from(now_ms()) + 1).unwrap().iter()
        .any(|fault| fault.item.title == "Readiness operation conflict"));
}

#[test]
fn capped_notice_does_not_block_other_notices_or_recovery() {
    let (dir, policy, store) = fixture();
    let journal = dir.path().join("case.json");
    let end = (MINUTES + 10) * MINUTE_MS;
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, end);
    assert!(record(&store, &policy.config, &opening).unwrap());
    occupy_chain(&store, &format!("{}:notice:early", opening.episode), 5);
    let missed = reading(end + MINUTE_MS, 11);
    admission.weekly(&journal, &policy.config, &store, &missed, missed.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    assert!(admission.notices.iter().any(|notice| notice.phase == "early" && !notice.recorded));
    assert!(admission.notices.iter().any(|notice| notice.phase == "weekly" && notice.recorded));
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 1);
    let met = reading(end + 2 * MINUTE_MS, 0);
    admission.weekly(&journal, &policy.config, &store, &met, met.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert!(admission.weekly.recovered);
    assert_eq!(store.operational_failure(&opening.episode).unwrap().unwrap().status, "resolved");
    let index = store.index().unwrap();
    let mut restarted = Admission::load(&journal, &policy.config).unwrap();
    drain_notices(&mut restarted, &journal, &store, None).unwrap();
    assert_eq!(store.index().unwrap(), index);
}

#[test]
fn held_notice_continues_and_replays_without_a_second_message() {
    let (dir, policy, store) = fixture();
    let declarations = "version 2\nagent \"fixture.owner\" { command \"true\" }\nagent \"fixture.backup\" { command \"true\" }\n";
    let intent = crate::graph::parse_intent(declarations, "fixture").unwrap();
    let preview = store.mission(&intent, IntentInput {
        kdl: declarations.into(), source_name: None,
    }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "key-ownership-backup").unwrap();
    let journal = dir.path().join("case.json");
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, (MINUTES + 10) * MINUTE_MS);
    assert!(record(&store, &policy.config, &opening).unwrap());
    let backup = "agent/fixture.backup";
    let root = format!("{}:early:{backup}", opening.episode);
    occupy_chain(&store, &root, 1);
    let remote = Store::open_memory("remote").unwrap();
    merge_different_occupant(&store, &remote, &root);
    drain_notices(&mut admission, &journal, &store, Some(backup)).unwrap();
    assert_eq!(store.messages(Some(backup), true).unwrap().len(), 1);
    let (occupants, _) = store.operation_claims(&root).unwrap();
    merge_different_occupant(&store, &remote,
        &format!("{root}:conflict:{}", occupants[0].id));
    drain_notices(&mut admission, &journal, &store, Some(backup)).unwrap();
    let index = store.index().unwrap();
    for notice in &mut admission.notices { notice.recorded = false; }
    admission.persist(&journal).unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("graph.db"), "fixture").unwrap();
    let mut restarted = Admission::load(&journal, &policy.config).unwrap();
    drain_notices(&mut restarted, &journal, &store, Some(backup)).unwrap();
    assert_eq!(store.messages(Some(backup), true).unwrap().len(), 1);
    assert_eq!(store.index().unwrap(), index);
}

#[test]
fn capped_recovery_does_not_prevent_other_failure_receipts_from_closing() {
    let (dir, policy, store) = fixture();
    let journal = dir.path().join("case.json");
    let end = (MINUTES + 10) * MINUTE_MS;
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, end);
    assert!(record(&store, &policy.config, &opening).unwrap());
    let missed = reading(end + MINUTE_MS, 11);
    admission.weekly(&journal, &policy.config, &store, &missed, missed.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    let early = recorded_phase(&store, &opening, "early");
    let weekly = recorded_phase(&store, &opening, "weekly");
    occupy_chain(&store, &format!("{}:recovered", early.id), 5);
    let met = reading(end + 2 * MINUTE_MS, 0);
    admission.weekly(&journal, &policy.config, &store, &met, met.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert!(!admission.weekly.recovered);
    assert!(admission.weekly.pending.is_some());
    assert!(store.operation_claim(&format!("{}:recovered", weekly.id)).unwrap().is_some());
    let index = store.index().unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("graph.db"), "fixture").unwrap();
    let mut restarted = Admission::load(&journal, &policy.config).unwrap();
    restarted.replay_weekly(&journal,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert!(restarted.weekly.pending.is_some());
    assert_eq!(store.index().unwrap(), index);
}

fn merge_different_occupant(store: &Store, remote: &Store, key: &str) {
    remote.append_claim(&ClaimInput {
        subject: "custom/fixture/remote".into(), kind: "custom.fixture.record".into(),
        actor: Some("agent/fixture.owner".into()),
        fields: BTreeMap::from([("member".into(), json!("remote"))]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: Some(key.into()),
    }).unwrap();
    store.import_replication(remote.origin(), &remote.export_replication(0).unwrap()).unwrap();
    assert!(store.operation_claim(key).unwrap().is_none());
    assert_eq!(store.operation_claims(key).unwrap().0.len(), 2);
}

/// The production sampler sequence after root-controlled evidence has been read.
fn sample(admission: &mut Admission, journal: &Path, policy: &Policy, store: &Store, weekly: &Reading) {
    admission.replay(journal, |opening| record(store, &policy.config, opening)).unwrap();
    admission.replay_weekly(journal, |opening, action| record_weekly(store, opening, action)).unwrap();
    drain_notices(admission, journal, store, None).unwrap();
    admission.weekly(journal, &policy.config, store, weekly, weekly.end_ms,
        |opening, action| record_weekly(store, opening, action)).unwrap();
    drain_notices(admission, journal, store, None).unwrap();
}

#[test]
fn replicated_conflicts_continue_each_family_and_preserve_acknowledged_receipts() {
    let (dir, policy, store) = fixture();
    let remote = Store::open_memory("remote").unwrap();
    let journal = dir.path().join("readiness-case.json");
    let end = (MINUTES + 10) * MINUTE_MS;
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, end);
    for suffix in ["early", "weekly", "notice:early", "notice:weekly"] {
        let root = format!("{}:{suffix}", opening.episode);
        let first = occupy(&store, &root);
        merge_different_occupant(&store, &remote, &root);
        let (occupants, _) = store.operation_claims(&root).unwrap();
        // A continuation can itself receive divergent replicated requests.
        let continuation = format!("{root}:conflict:{}", occupants[0].id);
        occupy(&store, &continuation);
        merge_different_occupant(&store, &remote, &continuation);
        assert!(occupants.iter().any(|claim| claim.id == first.id));
    }
    admission.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
    assert!(admission.case.as_ref().unwrap().recorded);
    let early = recorded_phase(&store, &opening, "early");
    let missed = reading(end + MINUTE_MS, 11);
    sample(&mut admission, &journal, &policy, &store, &missed);
    let weekly = recorded_phase(&store, &opening, "weekly");
    assert_eq!(weekly.body["evidence"], json!([early.id]));
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 2);

    // The actual daemon phase and message receipts remain discoverable after a
    // later divergent request changes their operation registry state to conflict.
    let notices = admission.notices.clone();
    for receipt in [&early, &weekly] {
        let root = if receipt.id == early.id {
            format!("{}:early", opening.episode)
        } else {
            format!("{}:weekly", opening.episode)
        };
        let id = receipt.body["_operation"]["id"].as_str().unwrap();
        let (first, _) = store.operation_claims(&root).unwrap();
        let next = format!("{root}:conflict:{}", first[0].id);
        let (second, _) = store.operation_claims(&next).unwrap();
        let actual = format!("{root}:conflict:{}", second[0].id);
        assert_eq!(store.operation_claims(&actual).unwrap().0[0].body["_operation"]["id"], id);
        merge_different_occupant(&store, &remote, &actual);
    }
    for notice in &notices {
        let input = notice_input(notice);
        let root = input.idempotency_key.as_deref().unwrap();
        let (first, _) = store.operation_claims(root).unwrap();
        let next = format!("{root}:conflict:{}", first[0].id);
        let (second, _) = store.operation_claims(&next).unwrap();
        let actual = format!("{root}:conflict:{}", second[0].id);
        merge_different_occupant(&store, &remote, &actual);
    }
    for receipt in [&early, &weekly] {
        let key = format!("{}:recovered", receipt.id);
        occupy(&store, &key);
        merge_different_occupant(&store, &remote, &key);
    }
    let met = reading(end + 2 * MINUTE_MS, 0);
    sample(&mut admission, &journal, &policy, &store, &met);
    assert!(admission.weekly.recovered);
    assert_eq!(store.operational_failure(&opening.episode).unwrap().unwrap().status, "resolved");
    for receipt in [&early, &weekly] {
        let root = format!("{}:recovered", receipt.id);
        let (occupants, _) = store.operation_claims(&root).unwrap();
        merge_different_occupant(&store, &remote,
            &format!("{root}:conflict:{}", occupants[0].id));
    }
    admission.case.as_mut().unwrap().recorded = false;
    for notice in &mut admission.notices { notice.recorded = false; }
    admission.persist(&journal).unwrap();
    // First observation records the late conflicts; subsequent restart is read-only.
    admission.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
    assert!(record_weekly(&store, &opening, &WeeklyAction::Recover { cut: met.end_ms }).unwrap());
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    let index = store.index().unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("graph.db"), "fixture").unwrap();
    let mut restarted = Admission::load(&journal, &policy.config).unwrap();
    restarted.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
    assert!(record_weekly(&store, &opening, &WeeklyAction::Recover { cut: met.end_ms }).unwrap());
    drain_notices(&mut restarted, &journal, &store, None).unwrap();
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 2);
    assert_eq!(store.index().unwrap(), index);
}

fn replicated_occupants(store: &Store, key: &str, count: usize, serial: &mut usize) -> Vec<String> {
    let mut ids = Vec::new();
    for _ in 0..count {
        *serial += 1;
        let peer = Store::open_memory(format!("replica-{serial}")).unwrap();
        let receipt = peer.append_claim(&ClaimInput {
            subject: "custom/fixture/record".into(), kind: "custom.fixture.record".into(),
            actor: Some("agent/fixture.owner".into()),
            fields: BTreeMap::from([("request".into(), json!(*serial))]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: Some(key.into()),
        }).unwrap();
        ids.push(receipt.id);
        store.import_replication(peer.origin(), &peer.export_replication(0).unwrap()).unwrap();
    }
    ids
}

#[test]
fn overflowing_occupants_block_lookup_without_stopping_sampler_or_sibling_recovery() {
    let (dir, policy, store) = fixture();
    let journal = dir.path().join("readiness-case.json");
    let end = (MINUTES + 10) * MINUTE_MS;
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, end);
    let mut serial = 0;
    let root = format!("{}:early", opening.episode);
    replicated_occupants(&store, &root, 20, &mut serial);
    let (receipts, overflow) = store.operation_claims(&root).unwrap();
    assert!(overflow);
    assert_eq!(receipts.len(), 16);
    assert!(matches!(phase_receipt(&store, &opening, "early").unwrap(), keys::Lookup::Blocked));
    let notice_root = format!("{}:notice:early", opening.episode);
    replicated_occupants(&store, &notice_root, 20, &mut serial);
    let mut remaining = keys::MAX_VISITS;
    assert!(matches!(keys::resolve(&store, &notice_root, &notice_root,
        &opening.reviewer, 0, &|_, _| false, &mut remaining).unwrap(), keys::Lookup::Blocked));
    assert_eq!(keys::MAX_VISITS - remaining, 17, "one bounded read and sixteen occupants");
    let missed = reading(end + MINUTE_MS, 11);
    sample(&mut admission, &journal, &policy, &store, &missed);
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 1);
    let met = reading(end + 2 * MINUTE_MS, 0);
    sample(&mut admission, &journal, &policy, &store, &met);
    let pending = Admission::load(&journal, &policy.config).unwrap();
    assert!(!pending.weekly.recovered);
    assert!(pending.weekly.pending.is_some());
    assert_eq!(pending.case.as_ref().unwrap().episode, opening.episode);
    assert_eq!(store.operational_failure(&opening.episode).unwrap().unwrap().status, "resolved");
    let index = store.index().unwrap();
    sample(&mut admission, &journal, &policy, &store, &met);
    assert_eq!(store.index().unwrap(), index, "diagnostics and notices are deduplicated");
}

#[test]
fn branched_continuations_share_one_total_visit_budget() {
    let (dir, policy, store) = fixture();
    let journal = dir.path().join("case.json");
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, (MINUTES + 10) * MINUTE_MS);
    let root = format!("{}:early", opening.episode);
    let mut serial = 0;
    for first in replicated_occupants(&store, &root, 3, &mut serial) {
        let child = format!("{root}:conflict:{first}");
        for second in replicated_occupants(&store, &child, 3, &mut serial) {
            replicated_occupants(&store, &format!("{root}:conflict:{second}"), 3, &mut serial);
        }
    }
    let mut remaining = keys::MAX_VISITS;
    let result = keys::resolve(&store, &root, &root, &opening.reviewer, 0,
        &|_, _| false, &mut remaining).unwrap();
    assert!(matches!(result, keys::Lookup::Blocked));
    assert_eq!(remaining, 0);
    let index = store.index().unwrap();
    let mut remaining = keys::MAX_VISITS;
    assert!(matches!(keys::resolve(&store, &root, &root, &opening.reviewer, 0,
        &|_, _| false, &mut remaining).unwrap(), keys::Lookup::Blocked));
    assert_eq!(remaining, 0);
    assert_eq!(store.index().unwrap(), index, "replay does not repeat the diagnostic");
    let missed = reading(opening.cut + MINUTE_MS, 11);
    admission.weekly(&journal, &policy.config, &store, &missed, missed.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    assert_eq!(store.messages(Some(&policy.config.reviewer), true).unwrap().len(), 1);
}

#[test]
fn overflowing_ancestor_does_not_turn_a_recorded_continuation_into_absent_failure() {
    let (dir, policy, store) = fixture();
    let journal = dir.path().join("case.json");
    let end = (MINUTES + 10) * MINUTE_MS;
    let mut admission = Admission::load(&journal, &policy.config).unwrap();
    let opening = plan(&mut admission, &policy, &journal, end);
    let root = format!("{}:early", opening.episode);
    occupy_chain(&store, &root, 1);
    admission.replay(&journal, |opening| record(&store, &policy.config, opening)).unwrap();
    let failure = recorded_phase(&store, &opening, "early");
    assert_ne!(store.operation_claim(&root).unwrap().unwrap().id, failure.id);
    let mut serial = 0;
    replicated_occupants(&store, &root, 20, &mut serial);
    assert!(matches!(phase_receipt(&store, &opening, "early").unwrap(), keys::Lookup::Blocked));
    let met = reading(end + MINUTE_MS, 0);
    admission.weekly(&journal, &policy.config, &store, &met, met.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert!(!admission.weekly.recovered);
    assert!(admission.weekly.pending.is_some());
    assert_eq!(admission.case.as_ref().unwrap().episode, opening.episode);
    assert_eq!(store.operational_failure(&opening.episode).unwrap().unwrap().status, "pending");
    assert!(store.claim_by_id(&failure.id).unwrap().is_some());
    assert!(store.fault_delivery_snapshot(u128::from(now_ms()) + 1).unwrap().iter()
        .any(|fault| fault.item.title == "Readiness operation conflict"));
    let next = reading(end + 2 * MINUTE_MS, 0);
    admission.weekly(&journal, &policy.config, &store, &next, next.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert_eq!(admission.case.as_ref().unwrap().episode, opening.episode);
    assert!(!admission.weekly.recovered);
    drop(store);
    let store = Store::open(&dir.path().join("graph.db"), "fixture").unwrap();
    let mut restarted = Admission::load(&journal, &policy.config).unwrap();
    restarted.replay_weekly(&journal,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    assert!(!restarted.weekly.recovered);
    assert!(restarted.weekly.pending.is_some());
    assert_eq!(restarted.case.as_ref().unwrap().episode, opening.episode);
}
