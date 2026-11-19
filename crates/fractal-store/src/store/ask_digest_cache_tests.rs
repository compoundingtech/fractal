use super::*;
use crate::store::fleet_admission_tests::{fleet, key, sync};
use crate::store::ask_digest::AskDigestPass;

fn setup() -> (Store, Store, ClaimRecord) {
    let (owner_key, other_key) = (key(), key());
    let (_, _, mut stores) = fleet(&[("alder", &owner_key), ("birch", &other_key)]);
    let other = stores.pop().unwrap();
    let owner = stores.pop().unwrap();
    let intent = crate::parse_intent(&format!(
        "version 2\nnotice-owner {{ member \"alder\"; member-key \"{}\"; always-on }}",
        owner_key.public()), &owner.origin).unwrap();
    owner.apply_as(&intent, &BTreeMap::from([(crate::notice_owner::SUBJECT.into(), Vec::new())]),
        "bind-cache-owner", Some("person/example")).unwrap();
    sync(&owner, &other);
    // The same owner binding, but a distinct successor declaration on the other writer.
    // Its claim is deliberately withheld from the digest writer until after cache discovery.
    let declaration = other.connection.batched(|tx| -> Result<_> {
        let selected = current_desired_row_tx(tx, crate::notice_owner::SUBJECT)?.unwrap();
        let prior = claim_by_id_tx(tx, &selected.claim_id)?.unwrap();
        append_claim_tx(tx, &other.origin, &prior.subject, &prior.kind, Some("person/example"),
            &prior.body, &[prior.id], None)
    }).unwrap().unwrap();
    (owner, other, declaration)
}

fn source(tx: &Transaction<'_>, origin: &str, owner: &str, person: &str, resolution: &str) -> ClaimRecord {
    append_claim_tx(tx, origin, &format!("{}{person}/2027-11-15", notice::DIGEST_SOURCE_PREFIX),
        "custom.attention.notice-created", Some(notice::RUNTIME_ACTOR), &json!({"fields":{
            "_registration":notice::HASHES[0], "schema_version":notice::PAYLOAD_VERSION,
            "dedupe_key":format!("digest/{person}/2027-11-15"), "resource":"resource/attention",
            "recipient":person, "intent":"fyi", "urgency":"info", "requested":false,
            "summary":"One ask auto-resolved", "links":[], "created_at":"2027-11-15T09:00:00Z",
            "source":{"owner_claim":owner,"host":format!("host/{origin}"),
                "covered_through":"2027-11-15T09:00:00Z", "digest":[{
                    "resolution_id":resolution,"title":"Choose", "requester_id":"agent/example/asker",
                    "resolved_at":"2027-11-15T08:00:00Z", "target_id":"step-run/cache-arrival", "episode":"episode-1"
                }]}
        }}), &[], None).unwrap()
}

fn settle_cache(store: &Store) {
    store.connection.batched(|tx| -> Result<()> {
        initialize(tx)?;
        for _ in 0..1000 {
            if !ingest(tx)? { return Ok(()); }
        }
        panic!("cache discovery did not settle");
    }).unwrap().unwrap();
}

fn count(store: &Store, table: &str) -> u64 {
    store.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-h91316fa0b99c owner=fractal_store::store::ask_digest_cache_tests TODO=trace-statement-and-value-representation */ &format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

#[test]
fn source_before_owner_declaration_is_retried_and_fences_its_date() {
    let (owner, other, declaration) = setup();
    let creation = owner.connection.batched(|tx| -> Result<_> {
        Ok(source(tx, &owner.origin, &declaration.id, "person/example", "reported-resolution"))
    }).unwrap().unwrap();
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_deferred"), 1);
    assert_eq!(count(&owner, "local_ask_digest_sources"), 0);
    assert_eq!(count(&owner, "local_ask_digest_retry"), 0);
    // An idle pass has no retry work and does not advance or discard the sleeping source.
    owner.connection.batched(|tx| -> Result<()> { assert!(!ingest(tx)?); Ok(()) }).unwrap().unwrap();
    sync(&other, &owner);
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_deferred"), 0);
    assert_eq!(count(&owner, "local_ask_digest_sources"), 1);
    owner.connection.batched(|tx| -> Result<()> {
        let fence: (String,String) = tx.query_row(
            "SELECT source_id,date FROM local_ask_digest_sources", [], |r| Ok((r.get(0)?,r.get(1)?)))?;
        assert_eq!(fence, (creation.subject, "2027-11-15".into()));
        assert!(index_source_page(tx)?);
        assert!(!index_source_page(tx)?);
        assert!(tx.query_row("SELECT EXISTS(SELECT 1 FROM local_ask_digest_emitted WHERE resolution_id='reported-resolution')",
            [], |r| r.get::<_,bool>(0))?);
        Ok(())
    }).unwrap().unwrap();
    // A late, unreported resolution must wait for tomorrow, not produce a second notice for
    // the already emitted local date. Tomorrow's native emission remains available.
    let at = |text: &str| u128::try_from(chrono::DateTime::parse_from_rfc3339(text).unwrap().timestamp_millis()).unwrap();
    owner.graph.set_write_clock_at(at("2027-11-15T08:30:00Z")).unwrap();
    owner.connection.batched(|tx| -> Result<()> {
        let ask = append_claim_tx(tx, &owner.origin, "step-run/cache-late", "work.person-asked",
            Some("agent/example/asker"), &json!({"fields":{"person":"person/example","title":"Choose late"}}), &[], None)?;
        append_claim_tx(tx, &owner.origin, &ask.subject, "work.person-done", Some(notice::RUNTIME_ACTOR),
            &json!({"fields":{"episode":ask.id,"status":"completed","resolved_by":"deadline",
                "summary":"Default applied","key":format!("person-deadline:{}",ask.id),
                "answer":{"id":"hold","label":"Hold"}}}), &[ask.id], None)?;
        Ok(())
    }).unwrap().unwrap();
    let source_count = || owner.readers.get().query_row(
        "SELECT count(*) FROM claims WHERE kind='custom.attention.notice-created' AND subject LIKE 'custom/attention/source/digest/%'",
        [], |r| r.get::<_,u64>(0)).unwrap();
    for (now, expected) in [("2027-11-15T09:00:00Z", 1), ("2027-11-16T09:00:00Z", 2)] {
        let now = at(now);
        let mut settled = false;
        for _ in 0..100 {
            let pass = owner.reconcile_ask_digests_at(now).unwrap();
            if pass.next_deadline.is_none_or(|next| next > now) { settled = true; break; }
        }
        assert!(settled, "native digest work did not settle");
        assert_eq!(source_count(), expected);
    }
}

#[test]
fn declaration_arrival_retries_only_one_bounded_dependency_page_per_pass() {
    let (owner, other, declaration) = setup();
    owner.connection.batched(|tx| -> Result<()> {
        for index in 0..PAGE + 3 {
            source(tx, &owner.origin, &declaration.id, "person/example", &format!("resolution-{index}"));
        }
        Ok(())
    }).unwrap().unwrap();
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_deferred"), (PAGE + 3) as u64);
    assert_eq!(count(&owner, "local_ask_digest_retry"), 0);
    sync(&other, &owner);
    owner.connection.batched(|tx| -> Result<()> { assert!(ingest(tx)?); Ok(()) }).unwrap().unwrap();
    assert_eq!(count(&owner, "local_ask_digest_source_history"), PAGE as u64);
    assert_eq!(count(&owner, "local_ask_digest_deferred"), 3);
    assert_eq!(count(&owner, "local_ask_digest_retry"), 1);
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_source_history"), (PAGE + 3) as u64);
    assert_eq!(count(&owner, "local_ask_digest_deferred"), 0);
    assert_eq!(count(&owner, "local_ask_digest_retry"), 0);
}

#[test]
fn known_nonowner_source_neither_fences_nor_queues_retries() {
    let (owner, other, _) = setup();
    other.connection.batched(|tx| -> Result<()> {
        let selected = current_desired_row_tx(tx, crate::notice_owner::SUBJECT)?.unwrap();
        source(tx, &other.origin, &selected.claim_id, "person/example", "forged-resolution");
        Ok(())
    }).unwrap().unwrap();
    settle_cache(&other);
    assert_eq!(count(&other, "local_ask_digest_deferred"), 0);
    assert_eq!(count(&other, "local_ask_digest_sources"), 0);
    assert_eq!(count(&other, "local_ask_digest_source_history"), 0);
    assert_eq!(other.reconcile_ask_digests_at(0).unwrap(), AskDigestPass::default());
    assert_eq!(Store::open_memory("unbound").unwrap().reconcile_ask_digests_at(0).unwrap(), AskDigestPass::default());
    // The actual owner still admits its legitimate source for the identical person and date.
    owner.connection.batched(|tx| -> Result<()> {
        let selected = current_desired_row_tx(tx, crate::notice_owner::SUBJECT)?.unwrap();
        source(tx, &owner.origin, &selected.claim_id, "person/example", "legitimate-resolution");
        Ok(())
    }).unwrap().unwrap();
    sync(&other, &owner);
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_sources"), 1);
    assert_eq!(count(&owner, "local_ask_digest_source_history"), 1);
}

#[test]
fn duplicate_authenticated_date_indexes_every_emissions_reported_ids() {
    let (owner, _, _) = setup();
    owner.connection.batched(|tx| -> Result<()> {
        let selected = current_desired_row_tx(tx, crate::notice_owner::SUBJECT)?.unwrap();
        source(tx, &owner.origin, &selected.claim_id, "person/example", "first-emission-resolution");
        source(tx, &owner.origin, &selected.claim_id, "person/example", "second-emission-resolution");
        Ok(())
    }).unwrap().unwrap();
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_sources"), 1);
    assert_eq!(count(&owner, "local_ask_digest_source_history"), 2);
    owner.connection.batched(|tx| -> Result<()> {
        assert!(index_source_page(tx)?);
        assert!(index_source_page(tx)?);
        assert!(!index_source_page(tx)?);
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(count(&owner, "local_ask_digest_emitted"), 2);
}

#[test]
fn missing_bound_incarnation_sleeps_even_when_an_older_key_is_known() {
    let (owner, other, _) = setup();
    sync(&other, &owner);
    let next_key = key();
    let creation = owner.connection.batched(|tx| -> Result<_> {
        let selected = current_desired_row_tx(tx, crate::notice_owner::SUBJECT)?.unwrap();
        let prior = claim_by_id_tx(tx, &selected.claim_id)?.unwrap();
        let mut body = prior.body;
        let children = body["desired"]["children"].as_array_mut().unwrap();
        children.iter_mut().find(|child| child["name"] == "member-key").unwrap()["arguments"] = json!([next_key.public()]);
        let declaration = append_claim_tx(tx, &owner.origin, crate::notice_owner::SUBJECT,
            "intent.desired", Some("person/example"), &body, &[prior.id], None)?;
        Ok(source(tx, &owner.origin, &declaration.id, "person/example", "new-incarnation-resolution"))
    }).unwrap().unwrap();
    let membership = owner.fleet_membership().unwrap();
    assert!(membership.incarnations().any(|incarnation| incarnation.name == owner.origin));
    assert!(!membership.incarnations().any(|incarnation| incarnation.member_key == next_key.public()));
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_deferred"), 1);
    assert_eq!(count(&owner, "local_ask_digest_sources"), 0);
    owner.connection.batched(|tx| -> Result<()> {
        assert!(!ingest(tx)?);
        Ok(())
    }).unwrap().unwrap();
    // The missing admission arrives independently; fleet generation wakes the
    // sleeping source despite another incarnation having been known throughout.
    crate::store::fleet_admission_tests::admit(&other, &owner.origin, &next_key, "invite", None);
    sync(&other, &owner);
    settle_cache(&owner);
    assert_eq!(count(&owner, "local_ask_digest_deferred"), 0);
    assert_eq!(count(&owner, "local_ask_digest_sources"), 1);
    assert_eq!(count(&owner, "local_ask_digest_source_history"), 1);
    owner.connection.batched(|tx| -> Result<()> {
        assert!(notice_authority::authenticated_digest_source(tx, &creation)?);
        assert!(index_source_page(tx)?);
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(count(&owner, "local_ask_digest_emitted"), 1);
}
