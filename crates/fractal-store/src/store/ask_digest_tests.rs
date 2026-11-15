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

fn seed(store: &Store, name: &str) -> (ClaimRecord, ClaimRecord) {
    store.connection.batched(|tx| -> Result<_> {
        let ask = append_claim_tx(tx, &store.origin, &format!("step-run/digest-{name}"), "work.person-asked",
            Some("agent/example/asker"), &json!({"fields":{"person":"person/example","title":format!("Choose {name}")}}), &[], None)?;
        let applied = append_claim_tx(tx, &store.origin, &ask.subject, "work.person-done", Some("daemon/runtime"),
            &json!({"fields":{"episode":ask.id,"status":"completed","resolved_by":"deadline",
                "summary":"Default applied","key":format!("person-deadline:{}",ask.id),
                "answer":{"id":"hold","label":"Hold"}}}), std::slice::from_ref(&ask.id), None)?;
        Ok((ask, applied))
    }).unwrap().unwrap()
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

#[test]
fn daily_digest_lists_defaults_and_retry_is_idempotent() {
    let (store, _, _) = setup();
    let (first, applied) = seed(&store, "first");
    let (second, _) = seed(&store, "second");
    let now = next_digest(&applied);
    let pass = store.reconcile_ask_digests_at(now).unwrap();
    assert!(pass.changed);
    assert!(pass.next_deadline.unwrap() > now);
    assert_eq!(counts(&store), (1, 1));
    let fields: String = store.readers.get().query_row(
        "SELECT body FROM claims WHERE kind='custom.attention.notice-created'", [], |row| row.get(0)).unwrap();
    let fields: Value = serde_json::from_str(&fields).unwrap();
    let rows = fields["fields"]["source"]["digest"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["title"], "Choose first");
    assert_eq!(rows[0]["requester_id"], "agent/example/asker");
    assert_eq!(rows[0]["default_answer"], json!({"id":"hold","label":"Hold"}));
    assert_eq!(rows[0]["resolved_at"], timestamp(applied.accepted_at_unix_ms));
    assert_eq!(rows[0]["action_parameters"]["work.override"], json!({"target_id":first.subject,"episode":first.id}));
    assert_eq!(rows[1]["target_id"], second.subject);
    assert!(!store.reconcile_ask_digests_at(now).unwrap().changed);
    assert_eq!(counts(&store), (1, 1));
}

#[test]
fn empty_days_and_nonowners_write_nothing() {
    let (owner, other, _) = setup();
    let now = ask_digest_window(now_ms(), "UTC", "09:00").unwrap().next_deadline;
    assert!(!owner.reconcile_ask_digests_at(now).unwrap().changed);
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
    assert!(owner.reconcile_ask_digests_at(now).unwrap().changed);
    sync(&owner, &other);
    bind(&other, "birch", other_key.public());
    assert!(!other.reconcile_ask_digests_at(now).unwrap().changed);
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
    assert!(!store.reconcile_ask_digests_at(next_digest(&applied)).unwrap().changed);
    assert_eq!(counts(&store), (0, 0));
}

#[test]
fn expired_override_action_is_omitted_and_window_is_half_open() {
    let (store, _, _) = setup();
    let (_, applied) = seed(&store, "expired");
    let at = applied.accepted_at_unix_ms;
    let connection = store.readers.get();
    let rows = resolved_rows(&connection, "person/example", at, at + 1, at + OVERRIDE_WINDOW_MS + 1).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("action_parameters").is_none());
    assert!(resolved_rows(&connection, "person/example", at - 1, at, at).unwrap().is_empty());
}
