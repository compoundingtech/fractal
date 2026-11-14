use super::*;
use crate::fleet::MemberKey;
use crate::store::custom::RegistrationRequest;
use crate::store::fleet_admission_tests::{append, fleet, key, sync};

const SOURCE: &str = "custom/attention/source/01900000-0000-7000-8000-000000000002";
const SNAPSHOT: &str = "custom/attention/snapshot/01900000-0000-7000-8000-000000000001";
const RUNTIME: &str = notice::RUNTIME_ACTOR;

/// An anchor `a` and two members: `alder`, the bound notice owner, and `birch`, which is not.
struct Fleet {
    a: Store,
    alder: Store,
    birch: Store,
    alder_key: Arc<MemberKey>,
    source: ClaimRecord,
}

fn declare(store: &Store, member: &str, key: &str) -> Result<ApplyResponse, FractalError> {
    let intent = crate::parse_intent(&format!(
        "version 2\nnotice-owner {{ member \"{member}\"; member-key \"{key}\"; always-on }}"
    ), &store.origin).unwrap();
    let token = store.selected_desired_token(notice_owner::SUBJECT).unwrap();
    let expected = BTreeMap::from([(notice_owner::SUBJECT.to_owned(), token.into_iter().collect())]);
    store.apply_as(&intent, &expected, &format!("notice-owner-{member}-{key}"), Some("person/example"))
}

fn setup() -> Fleet {
    let (alder_key, birch_key) = (key(), key());
    let (_, a, mut stores) = fleet(&[("alder", &alder_key), ("birch", &birch_key)]);
    let birch = stores.pop().unwrap();
    let alder = stores.pop().unwrap();
    declare(&alder, "alder", alder_key.public()).unwrap();
    for manifest in notice::manifests() {
        alder.register_custom_kind(&RegistrationRequest { manifest: manifest.clone(), actor: "person/example".into() }).unwrap();
    }
    let fixture: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h99da4f16cb38 owner=fractal_store::store::notice_authority_tests TODO=trace-statement-and-value-representation */ include_str!("../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
    let source = alder.append_claim(&input(SOURCE, "custom.attention.notice-created", "agent/example/producer", fixture["snapshot"]["input"].clone())).unwrap();
    for to in [&a, &birch] { sync(&alder, to); }
    Fleet { a, alder, birch, alder_key, source }
}

fn input(subject: &str, kind: &str, actor: &str, fields: Value) -> ClaimInput {
    ClaimInput { subject: subject.into(), kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(), ..custom::tests_shared_delivery::empty_input() }
}

fn snapshot(source: &ClaimRecord) -> Value {
    json!({"source_id":SOURCE,"creation_claim":source.id,"producer":"agent/example/producer","recipient":"person/example",
        "runtime":"host/alder","policy":{"disposition":"deliver"},"delivery":[]})
}

fn state(store: &Store, subject: &str) -> Value {
    custom::source(&store.readers.get(), subject).unwrap().unwrap()
}

fn written() -> Fleet {
    let fleet = setup();
    fleet.alder.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, snapshot(&fleet.source))).unwrap();
    assert_eq!(state(&fleet.alder, SNAPSHOT)["state"], "ready");
    fleet
}

fn assert_fails_closed_like_rebuild(tx: &Transaction<'_>, reason: &str) -> Result<()> {
    let row = custom::source(tx, SNAPSHOT)?.unwrap();
    assert_ne!(row["state"], "ready");
    assert!(row["error"].as_str().unwrap().contains(reason), "{row}");
    assert_matches_rebuild(tx)
}

fn assert_matches_rebuild(tx: &Transaction<'_>) -> Result<()> {
    let incremental = custom::source(tx, SNAPSHOT)?.unwrap();
    custom::rebuild(tx)?;
    assert_eq!(custom::source(tx, SNAPSHOT)?.unwrap(), incremental);
    Ok(())
}

#[test]
fn notice_owner_is_one_root_declaration_with_a_fixed_subject() {
    let source = "version 2\nnotice-owner { member \"alder\"; member-key \"k\"; always-on }";
    let intent = crate::parse_intent(source, "alder").unwrap();
    let desired = &intent.subjects[notice_owner::SUBJECT];
    assert_eq!((desired.kind.as_str(), desired.member.is_none()), (notice_owner::KIND, true));
    assert_eq!(crate::parse_intent(&format!("{source}\n{}", &source[10..]), "alder").unwrap_err().code, "duplicate-subject");
    assert_eq!(crate::graph::parse_execution_intent(source, "alder", "run-1").unwrap_err().code, "notice-owner-inside-mission");
    assert_eq!(crate::parse_intent(&source.replace("; always-on", ""), "alder").unwrap_err().code, "invalid-notice-owner");
    let agent = "version 2\nagent \"example/runtime\" { command \"true\"; notice-runtime; }";
    assert!(crate::parse_intent(agent, "alder").is_err(), "the agent designation is gone");
}

#[test]
fn notice_only_the_bound_owners_daemon_writes_snapshots() {
    let fleet = setup();
    for actor in ["agent/example/producer", "person/example", "daemon/alder"] {
        assert!(fleet.alder.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", actor, snapshot(&fleet.source))).is_err(), "{actor}");
    }
    let error = fleet.birch.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, snapshot(&fleet.source))).unwrap_err();
    assert!(error.message.contains("only the bound notice owner's daemon"), "{error:?}");
    let mut forged = snapshot(&fleet.source); forged["runtime"] = json!("host/birch");
    assert!(fleet.alder.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, forged)).is_err());
    fleet.alder.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, snapshot(&fleet.source))).unwrap();
    sync(&fleet.alder, &fleet.birch);
    assert_eq!(state(&fleet.birch, SNAPSHOT)["state"], "ready");
}

#[test]
fn notice_a_replicated_snapshot_from_another_members_daemon_is_invalid() {
    let fleet = setup();
    // Model a mixed-version member that admits its own daemon's snapshot without the owner check.
    let fields = json!({"fields": snapshot(&fleet.source)});
    fleet.birch.connection.batched(|tx| {
        let mut fields = fields.clone();
        let registration = custom::registration(tx, SNAPSHOT)?.unwrap().0;
        fields["fields"]["_registration"] = json!(registration);
        append_claim_tx(tx, &fleet.birch.origin, SNAPSHOT, "custom.attention.snapshot-created", Some(RUNTIME), &fields, &[], None)
    }).unwrap().unwrap();
    sync(&fleet.birch, &fleet.alder);
    fleet.alder.connection.batched(|tx| assert_fails_closed_like_rebuild(tx, "only the bound notice owner's daemon"))
        .unwrap().unwrap();
}

#[test]
fn notice_missing_or_unbound_owner_fails_closed() {
    let (_, _, stores) = fleet(&[("alder", &key())]);
    let alder = &stores[0];
    alder.connection.batched(|tx| {
        assert!(owner(tx).unwrap_err().message.contains("missing notice-owner declaration"));
        Ok::<_, anyhow::Error>(())
    }).unwrap().unwrap();
    for (member, key, reason) in [
        ("alder", key(), "bound incarnation has ended"),
        ("elm", key(), "not an admitted fleet member"),
    ] {
        declare(alder, member, key.public()).unwrap();
        let error = alder.connection.batched(|tx| Ok::<_, anyhow::Error>(owner(tx).unwrap_err())).unwrap().unwrap();
        assert!(error.message.contains(reason), "{member}: {error:?}");
    }
}

#[test]
fn notice_handover_and_owner_removal_revalidate_snapshots_like_rebuild() {
    let fleet = written();
    let birch_key = fleet.birch.member_public_key().unwrap();
    declare(&fleet.alder, "birch", &birch_key).unwrap();
    fleet.alder.connection.batched(|tx| assert_fails_closed_like_rebuild(tx, "forged runtime")).unwrap().unwrap();

    let fleet = written();
    for to in [&fleet.a, &fleet.birch] { sync(&fleet.alder, to); }
    assert_eq!(state(&fleet.birch, SNAPSHOT)["state"], "ready");
    let high_water = fleet.a.readers.get().query_row(
        "SELECT MAX(replica_sequence) FROM batches WHERE origin='alder'", [], |row| row.get::<_, u64>(0),
    ).unwrap();
    append(&fleet.a, "fleet.member-removed", "host/alder",
        json!({"high_water": high_water, "reason": "test", "member_key": fleet.alder_key.public()}));
    // Only the membership changes at birch: the fleet generation revalidates the snapshot.
    sync(&fleet.a, &fleet.birch);
    fleet.birch.connection.batched(|tx| assert_fails_closed_like_rebuild(tx, "bound incarnation has ended")).unwrap().unwrap();
}

#[test]
fn notice_conflicting_owner_declarations_fail_closed() {
    let fleet = written();
    let birch_key = fleet.birch.member_public_key().unwrap();
    assert!(fleet.birch.selected_desired_token(notice_owner::SUBJECT).unwrap().is_some(), "birch has alder's declaration");
    let intent = crate::parse_intent(&format!(
        "version 2\nnotice-owner {{ member \"birch\"; member-key \"{birch_key}\"; always-on }}"
    ), "birch").unwrap();
    let rival = BTreeMap::from([(notice_owner::SUBJECT.to_owned(), Vec::new())]);
    assert!(fleet.birch.apply_as(&intent, &rival, "rival", Some("person/example")).is_err(),
        "a local apply cannot fork the declaration it has seen");
    fleet.alder.connection.batched(|tx| {
        let head: String = tx.query_row("SELECT claim_id FROM desired WHERE subject=?1", [notice_owner::SUBJECT], |r| r.get(0))?;
        let mut body = serde_json::from_str::<Value>(&tx.query_row(
            "SELECT body FROM claims WHERE id=?1", [&head], |r| r.get::<_, String>(0))?)?;
        body["fields"]["revision"] = json!("rival");
        append_claim_tx(tx, "alder", notice_owner::SUBJECT, "intent.desired", Some("person/example"), &body, &[], None)
    }).unwrap().unwrap();
    fleet.alder.connection.batched(|tx| assert_fails_closed_like_rebuild(tx, "conflicting notice-owner declarations"))
        .unwrap().unwrap();
}

#[test]
fn notice_required_fields_cannot_be_omitted_independently() {
    let fleet = setup();
    let source = &fleet.source;
    for (subject, kind, actor, fields, required) in [
        ("custom/attention/source/01900000-0000-7000-8000-000000000004",
            source.kind.as_str(), "agent/example/producer", serde_json::to_value(custom::fields(source)).unwrap(),
            &["schema_version", "dedupe_key", "resource", "recipient", "intent", "urgency",
                "requested", "summary", "links", "source", "created_at"][..]),
        (SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, snapshot(source),
            &["source_id", "creation_claim", "producer", "recipient", "runtime", "policy", "delivery"][..]),
    ] {
        for field in required {
            let mut omitted = fields.clone();
            assert!(omitted.as_object_mut().unwrap().remove(*field).is_some());
            assert!(fleet.alder.append_claim(&input(subject, kind, actor, omitted)).is_err(), "{kind}: {field}");
        }
        assert!(custom::source(&fleet.alder.readers.get(), subject).unwrap().is_none(), "{kind}");
        fleet.alder.append_claim(&input(subject, kind, actor, fields)).unwrap();
    }
}

#[test]
fn notice_sources_are_producer_input_without_runtime_or_daemon_authors() {
    let fleet = setup();
    let mut fields = custom::fields(&fleet.source);
    fields.remove("_registration");
    let other = format!("{}other", notice::SOURCE_PREFIX);
    for (actor, field, value) in [
        (RUNTIME, None, None),
        ("agent/example/producer", Some("producer"), Some(json!("agent/example/forged"))),
        ("agent/example/producer", Some("runtime"), Some(json!("host/alder"))),
    ] {
        let mut fields = fields.clone();
        if let (Some(field), Some(value)) = (field, value) { fields.insert(field.into(), value); }
        assert!(fleet.alder.append_claim(&ClaimInput { subject: other.clone(), fields,
            ..input(&other, &fleet.source.kind, actor, json!({})) }).is_err(), "{actor} {field:?}");
    }
}

#[test]
fn notice_snapshot_update_preserves_links_like_rebuild() {
    let fleet = written();
    let mut fields = snapshot(&fleet.source);
    fields["policy"] = json!({"disposition":"deliver","reason":"Updated policy"});
    let updated = fleet.alder.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-updated", RUNTIME, fields)).unwrap();
    assert_eq!(custom::fields(&updated)["policy"]["reason"], "Updated policy");
    let mut moved = snapshot(&fleet.source); moved["recipient"] = json!("person/other");
    assert!(fleet.alder.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-updated", RUNTIME, moved)).is_err());
    let row = state(&fleet.alder, SNAPSHOT);
    assert_eq!((row["state"].as_str(), row["input_count"].as_u64()), (Some("ready"), Some(2)));
    fleet.alder.connection.batched(assert_matches_rebuild).unwrap().unwrap();
}

#[test]
fn notice_later_replicated_source_conflict_invalidates_snapshots_like_rebuild() {
    let fleet = written();
    let mut fields = custom::fields(&fleet.source); fields.insert("summary".into(), json!("Competing input"));
    fleet.birch.connection.batched(|tx| append_claim_tx(tx, &fleet.birch.origin, SOURCE,
        &fleet.source.kind, fleet.source.actor.as_deref(), &json!({"fields":fields}), &[], None)).unwrap().unwrap();
    sync(&fleet.birch, &fleet.alder);
    fleet.alder.connection.batched(|tx| assert_fails_closed_like_rebuild(tx, "conflicting or ineligible source creation"))
        .unwrap().unwrap();
}

#[test]
fn notice_reserved_prefixes_and_the_retired_control_family_are_refused() {
    let fleet = setup();
    let mut manifest = notice::manifests()[0].clone(); manifest.kind = "attention.squat".into();
    assert!(fleet.alder.register_custom_kind(&RegistrationRequest { manifest, actor: "person/example".into() }).is_err());
    for subject in ["custom/other/fact", SNAPSHOT, "custom/attention/control/01900000-0000-7000-8000-000000000003"] {
        assert!(fleet.alder.append_claim(&input(subject, "custom.attention.snapshot-created", RUNTIME, json!({}))).is_err(), "{subject}");
    }
    for subject in [SOURCE, SNAPSHOT] { assert!(fleet.alder.custom_subject(subject).is_err()); }
    for kind in [None, Some("attention.notice-source"), Some("attention.notice-snapshot")] {
        assert!(fleet.alder.custom_subjects(kind, None, None, 10).unwrap().is_empty());
    }
}

#[test]
fn notice_upgrade_drops_the_agent_runtime_triggers_and_binding() {
    let store = Store::open_memory("alder").unwrap();
    store.connection.batched(|tx| -> Result<()> {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h7e2ee67aba06 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER notice_authority_desired_INSERT AFTER INSERT ON desired BEGIN SELECT 1; END;
            INSERT OR REPLACE INTO meta(key,value) VALUES('notice_runtime_binding','{}');")?;
        create_invalidation_triggers(tx)?;
        let left: i64 = tx.query_row("SELECT (SELECT count(*) FROM sqlite_master WHERE name GLOB 'notice_authority_*')
            + (SELECT count(*) FROM meta WHERE key='notice_runtime_binding')", [], |r| r.get(0))?;
        assert_eq!(left, 0);
        let installed: i64 = tx.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name GLOB 'notice_owner_*'", [], |r| r.get(0))?;
        assert_eq!(installed, 6);
        Ok(())
    }).unwrap().unwrap();
}

#[test]
fn notice_untouched_finalizer_is_sql_free_even_with_ready_notices() {
    use rusqlite::hooks::Authorization;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fleet = written();
    let mut connection = fleet.alder.connection.write();
    let tx = connection.transaction().unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h99223601f75d owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value=value WHERE key='custom_projections_version'", []).unwrap();
    assert!(!transaction_work::custom_pending(&tx));
    let statements = Arc::new(AtomicUsize::new(0));
    let counted = statements.clone();
    tx.authorizer(Some(move |_: rusqlite::hooks::AuthContext<'_>| {
        counted.fetch_add(1, Ordering::Relaxed);
        Authorization::Deny
    }));
    let result = custom::flush(&tx);
    tx.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>);
    result.unwrap();
    assert_eq!(statements.load(Ordering::Relaxed), 0);
    tx.rollback().unwrap();
}

#[test]
fn notice_work_signal_preserves_savepoint_work_and_shares_outer_boundary() {
    let fleet = written();
    let mut connection = fleet.alder.connection.write();
    for commit in [false, true] {
        let tx = connection.transaction().unwrap();
        assert!(!transaction_work::custom_pending(&tx));
        assert!(!transaction_work::resources_pending(&tx));
        tx.execute(/* sqlite-large-value: allow-for-now lv-he4c6451c597a owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_custom_dirty VALUES(?1)", [SNAPSHOT]).unwrap();
        assert!(transaction_work::custom_pending(&tx));
        assert!(!transaction_work::resources_pending(&tx));
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-hfece32705544 owner=fractal_store::store::notice_authority_tests TODO=trace-statement-and-value-representation */ "SAVEPOINT notice_flush").unwrap();
        custom::flush(&tx).unwrap();
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-hc35f31df4335 owner=fractal_store::store::notice_authority_tests TODO=trace-statement-and-value-representation */ "ROLLBACK TO notice_flush; RELEASE notice_flush").unwrap();
        assert!(transaction_work::custom_pending(&tx));
        assert!(tx.query_row("SELECT EXISTS(SELECT 1 FROM local_custom_dirty)", [], |r| r.get::<_, bool>(0)).unwrap());
        custom::flush(&tx).unwrap();
        tx.query_row("SELECT resource_preparation_work_pending()", [], |_| Ok(())).unwrap();
        assert!(transaction_work::resources_pending(&tx));
        if commit { tx.commit().unwrap(); } else { tx.rollback().unwrap(); }
        assert!(!transaction_work::custom_pending(&connection));
        assert!(!transaction_work::resources_pending(&connection));
    }
}
