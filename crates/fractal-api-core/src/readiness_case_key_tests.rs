use super::*;
use super::migration_tests::{fixture, reading};

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
    let early = phase_receipt(&store, &opening, "early").unwrap().unwrap();
    assert_eq!(early.kind, "operational.failure");
    drain_notices(&mut admission, &journal, &store, None).unwrap();
    let weekly_miss = reading(end + MINUTE_MS, 11);
    admission.weekly(&journal, &policy.config, &store, &weekly_miss, weekly_miss.end_ms,
        |opening, action| record_weekly(&store, opening, action)).unwrap();
    let weekly = phase_receipt(&store, &opening, "weekly").unwrap().unwrap();
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
    occupy_chain(&store, &format!("{}:early:{backup}", opening.episode), 1);
    drain_notices(&mut admission, &journal, &store, Some(backup)).unwrap();
    assert_eq!(store.messages(Some(backup), true).unwrap().len(), 1);
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
    let early = phase_receipt(&store, &opening, "early").unwrap().unwrap();
    let weekly = phase_receipt(&store, &opening, "weekly").unwrap().unwrap();
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
