use super::*;
use crate::fleet::MemberKey;
use crate::store::fleet_admission_tests::{fleet, key, sync};

fn bind(store: &Store, member: &str, key: &str) {
    let intent = crate::parse_intent(&format!(
        "version 2\nnotice-owner {{ member \"{member}\"; member-key \"{key}\"; always-on }}"
    ), &store.origin).unwrap();
    let token = store.selected_desired_token(crate::notice_owner::SUBJECT).unwrap();
    store.apply_as(&intent, &BTreeMap::from([(crate::notice_owner::SUBJECT.into(), token.into_iter().collect())]),
        &format!("bind-{member}"), Some("person/example")).unwrap();
}

fn setup() -> (Store, Store, Arc<MemberKey>) {
    let (owner_key, other_key) = (key(), key());
    let (_, _, mut stores) = fleet(&[("alder", &owner_key), ("birch", &other_key)]);
    let other = stores.pop().unwrap();
    let owner = stores.pop().unwrap();
    bind(&owner, "alder", owner_key.public());
    sync(&owner, &other);
    (owner, other, other_key)
}

fn seed_person(store: &Store, name: &str, person: &str) -> (ClaimRecord, ClaimRecord) {
    store.connection.batched(|tx| -> Result<_> {
        let ask = append_claim_tx(tx, &store.origin, &format!("step-run/digest-{name}"), "work.person-asked",
            Some("agent/example/asker"), &json!({"fields":{"person":person,"title":format!("Choose {name}")}}), &[], None)?;
        let applied = append_claim_tx(tx, &store.origin, &ask.subject, "work.person-done", Some("daemon/runtime"),
            &json!({"fields":{"episode":ask.id,"status":"completed","resolved_by":"deadline",
                "summary":"Default applied","key":format!("person-deadline:{}",ask.id),
                "answer":{"id":"hold","label":"Hold"}}}), std::slice::from_ref(&ask.id), None)?;
        Ok((ask, applied))
    }).unwrap().unwrap()
}

fn seed(store: &Store, name: &str) -> (ClaimRecord, ClaimRecord) {
    seed_person(store, name, "person/example")
}

fn ms(at: &str) -> u128 {
    u128::try_from(chrono::DateTime::parse_from_rfc3339(at).unwrap().timestamp_millis()).unwrap()
}

fn seed_at(store: &Store, name: &str, at: &str) -> (ClaimRecord, ClaimRecord) {
    store.graph.set_write_clock_at(ms(at)).unwrap();
    seed(store, name)
}

fn next_digest(applied: &ClaimRecord) -> u128 {
    ask_digest_window(applied.accepted_at_unix_ms, "UTC", "09:00").unwrap().next_deadline
}

fn counts(store: &Store) -> (u64, u64) {
    store.readers.get().query_row(
        "SELECT (SELECT count(*) FROM claims WHERE subject LIKE 'custom/attention/source/digest/%'),
                (SELECT count(*) FROM claims WHERE subject LIKE 'custom/attention/snapshot/digest/%')",
        [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
}

fn drain(store: &Store, now: u128) -> bool {
    let before = counts(store);
    for _ in 0..10_000 {
        let pass = store.reconcile_ask_digests_at(now).unwrap();
        if pass.next_deadline.is_none_or(|next| next > now) { return counts(store) != before; }
    }
    panic!("digest work did not settle");
}

fn digests(store: &Store) -> Vec<Value> {
    store.readers.get().prepare("SELECT body FROM claims WHERE kind='custom.attention.notice-created' ORDER BY subject").unwrap()
        .query_map([], |r| r.get::<_,String>(0)).unwrap()
        .map(|r| serde_json::from_str::<Value>(&r.unwrap()).unwrap()["fields"].clone()).collect()
}

fn rows(store: &Store, source: &Value) -> Vec<Value> {
    let connection = store.readers.get();
    let mut rows = Vec::new();
    ask_digest_payload::visit_rows(&connection, &source["source"], |row| { rows.push(row); Ok(()) }).unwrap();
    rows
}

#[test]
fn daily_digest_lists_defaults_and_retry_is_idempotent() {
    let (store, _, _) = setup();
    let (first, applied) = seed(&store, "first");
    let (second, _) = seed(&store, "second");
    let now = next_digest(&applied);
    assert!(drain(&store, now));
    assert_eq!(counts(&store), (1, 1));
    let all = rows(&store, &digests(&store)[0]);
    assert_eq!(all.len(), 2);
    let row = all.iter().find(|row| row["episode"] == first.id).unwrap();
    assert_eq!(row["title"], "Choose first");
    assert_eq!(row["requester_id"], "agent/example/asker");
    assert_eq!(row["default_answer"], json!({"id":"hold","label":"Hold"}));
    assert_eq!(row["resolved_at"], timestamp(applied.accepted_at_unix_ms));
    assert_eq!(row["action_parameters"]["work.override"], json!({"target_id":first.subject,"episode":first.id}));
    assert!(all.iter().any(|row| row["target_id"] == second.subject));
    assert!(!drain(&store, now));
    assert_eq!(counts(&store), (1, 1));
}

#[test]
fn empty_days_and_nonowners_write_nothing() {
    let (owner, other, _) = setup();
    let now = ask_digest_window(now_ms(), "UTC", "09:00").unwrap().next_deadline;
    assert!(!drain(&owner, now));
    assert_eq!(counts(&owner), (0, 0));
    let (_, applied) = seed(&owner, "owner-only");
    sync(&owner, &other);
    assert_eq!(other.reconcile_ask_digests_at(next_digest(&applied)).unwrap(), AskDigestPass::default());
    assert_eq!(counts(&other), (0, 0));
    let unbound = Store::open_memory("unbound").unwrap();
    seed(&unbound, "no-owner");
    assert_eq!(unbound.reconcile_ask_digests_at(now).unwrap(), AskDigestPass::default());
    assert_eq!(counts(&unbound), (0, 0));
}

#[test]
fn owner_handover_does_not_duplicate_daily_notice() {
    let (owner, other, other_key) = setup();
    let (_, applied) = seed(&owner, "handover");
    let now = next_digest(&applied);
    assert!(drain(&owner, now));
    sync(&owner, &other);
    bind(&other, "birch", other_key.public());
    assert!(!drain(&other, now));
    assert_eq!(counts(&other), (1, 1));
    sync(&other, &owner);
    assert_eq!(owner.reconcile_ask_digests_at(now).unwrap(), AskDigestPass::default());
}

#[test]
fn overridden_asks_are_excluded_from_daily_digest() {
    let (store, _, _) = setup();
    let (ask, applied) = seed(&store, "overridden");
    store.connection.batched(|tx| append_claim_tx(tx, &store.origin, &ask.subject,
        "work.person-override", Some("person/example"), &json!({"fields":{
            "episode":ask.id,"overrides":applied.id,"answer":{"id":"go","label":"Go"},
            "summary":"Go","key":format!("person-override:{}:go",ask.id)}}), &[ask.id.clone(),applied.id.clone()], None))
        .unwrap().unwrap();
    assert!(!drain(&store, next_digest(&applied)));
    assert_eq!(counts(&store), (0, 0));
}

#[test]
fn expired_override_action_is_omitted() {
    let (store, _, _) = setup();
    let (ask, applied) = seed(&store, "expired");
    let row = resolution_row(&store.readers.get(), &applied.id, &ask.id,
        applied.accepted_at_unix_ms + OVERRIDE_WINDOW_MS + 1).unwrap().unwrap();
    assert!(row.get("action_parameters").is_none());
}

#[test]
fn policy_reload_keeps_replicated_coverage_without_repeats_or_gaps() {
    for timezone in ["Europe/Berlin", "America/New_York"] {
        let (store, _, _) = setup();
        let (_, first) = seed_at(&store, "before-policy", "2027-11-15T08:30:00Z");
        assert!(drain(&store, ms("2027-11-15T09:00:00Z")));
        let (_, gap) = seed_at(&store, "between-boundaries", "2027-11-15T09:30:00Z");
        store.set_ask_digest_policy(&crate::config::AsksConfig {
            digest_timezone: timezone.into(), ..crate::config::AsksConfig::default()
        }).unwrap();
        assert!(drain(&store, ms("2027-11-17T18:00:00Z")));
        let all = digests(&store);
        assert_eq!(all.len(), 2);
        let ids: Vec<_> = all.iter().flat_map(|source| rows(&store, source)).map(|row| row["resolution_id"].clone()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&json!(first.id)) && ids.contains(&json!(gap.id)));
        assert_eq!(all[0]["source"]["covered_through"], "2027-11-15T09:00:00.000Z");
        assert!(!drain(&store, ms("2027-11-17T18:00:00Z")));
    }
}

#[test]
fn outage_before_first_wake_catches_up_before_todays_wake() {
    let (store, _, _) = setup();
    seed_at(&store, "missed-day", "2027-11-15T08:30:00Z");
    // The owner resumes at 08:00 the following day, before that day's 09:00 wake.
    assert!(drain(&store, ms("2027-11-16T08:00:00Z")));
    let all = digests(&store);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["created_at"], "2027-11-15T09:00:00.000Z");
    assert_eq!(rows(&store, &all[0]).len(), 1);
}

#[test]
fn late_replicated_resolution_is_carried_into_next_digest_once() {
    let (owner, other, _) = setup();
    let (_, first) = seed_at(&owner, "local-first", "2027-11-15T08:00:00Z");
    let (_, late) = seed_at(&other, "late-remote", "2027-11-15T08:59:00Z");
    assert!(drain(&owner, ms("2027-11-15T09:00:00Z")));
    sync(&other, &owner);
    assert!(!drain(&owner, ms("2027-11-15T09:01:00Z")));
    assert!(drain(&owner, ms("2027-11-16T09:00:00Z")));
    let ids: Vec<_> = digests(&owner).iter().flat_map(|source| rows(&owner, source))
        .map(|row| row["resolution_id"].clone()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&json!(first.id)) && ids.contains(&json!(late.id)));
    assert!(!drain(&owner, ms("2027-11-17T09:00:00Z")));
}

#[test]
fn busy_day_is_lossless_bounded_and_does_not_block_another_person() {
    let (store, _, _) = setup();
    store.graph.set_write_clock_at(ms("2027-11-15T08:00:00Z")).unwrap();
    for index in 0..300 { seed(&store, &format!("busy-{index}")); }
    seed_person(&store, "another-person", "person/avery");
    let now = ms("2027-11-15T09:00:00Z");
    store.reconcile_ask_digests_at(now).unwrap();
    let discovered: usize = store.readers.get().query_row("SELECT count(*) FROM local_ask_digest_pending", [], |r| r.get(0)).unwrap();
    assert!(discovered <= ask_digest_cache::PAGE);
    assert!(drain(&store, now));
    assert_eq!(counts(&store), (2, 2));
    let all = digests(&store);
    let main = all.iter().find(|source| source["recipient"] == "person/example").unwrap();
    assert!(main["source"].get("digest_document").is_some());
    assert_eq!(rows(&store, main).len(), 300);
    for source in all { assert!(serde_json::to_vec(&source).unwrap().len() < 65_536); }
    let connection = store.readers.get();
    let plan = connection.prepare("EXPLAIN QUERY PLAN SELECT resolution_id FROM local_ask_digest_pending INDEXED BY local_ask_digest_due
        WHERE person=?1 AND staged_source IS NULL AND resolved_at<?2 ORDER BY resolved_at,resolution_id LIMIT 128").unwrap()
        .query_map(params!["person/example",i64::try_from(now).unwrap()], |r| r.get::<_,String>(3)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("; ");
    assert!(plan.contains("person=? AND resolved_at<?"), "{plan}");
    assert!(!plan.contains("SCAN"), "{plan}");
}
