use super::*;
use crate::store::custom::RegistrationRequest;

const SOURCE: &str = "custom/attention/source/01900000-0000-7000-8000-000000000002";
const SNAPSHOT: &str = "custom/attention/snapshot/01900000-0000-7000-8000-000000000001";
const CONTROL: &str = "custom/attention/control/01900000-0000-7000-8000-000000000003";
const RUNTIME: &str = "agent/example/runtime";

fn designate(store: &Store, name: &str) -> Result<ApplyResponse, FractalError> {
    let intent = crate::parse_intent(&format!("version 2\nagent \"{name}\" {{ command \"true\"; notice-runtime; }}"), "example").unwrap();
    let expected = intent.subjects.keys().map(|s| (s.clone(), Vec::new())).collect();
    store.apply_as(&intent, &expected, name, Some("person/example"))
}

fn setup() -> (Store, ClaimRecord) {
    let store = Store::open_memory("example").unwrap();
    designate(&store, "example/runtime").unwrap();
    for manifest in notice::manifests() {
        store.register_custom_kind(&RegistrationRequest { manifest: manifest.clone(), actor: "person/example".into() }).unwrap();
    }
    let fixture: Value = serde_json::from_str(include_str!("../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
    let source = store.append_claim(&input(SOURCE, "custom.attention.notice-created", "agent/example/producer", fixture["snapshot"]["input"].clone())).unwrap();
    (store, source)
}

fn input(subject: &str, kind: &str, actor: &str, fields: Value) -> ClaimInput {
    ClaimInput { subject: subject.into(), kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(), ..custom::tests_shared_delivery::empty_input() }
}

fn linked(source: &ClaimRecord, control: bool) -> Value {
    let mut fields = json!({"source_id":SOURCE,"creation_claim":source.id,"producer":"agent/example/producer","recipient":"person/example","runtime":RUNTIME});
    if control {
        for field in ["seen_at", "dismissed_at", "snoozed_until"] { fields[field] = Value::Null; }
    } else { fields["policy"] = json!({"disposition":"deliver"}); fields["delivery"] = json!([]); }
    fields
}

fn linked_store() -> Store {
    let (store, source) = setup();
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&source, false))).unwrap();
    store.append_claim(&input(CONTROL, "custom.attention.control-created", "person/example", linked(&source, true))).unwrap();
    for subject in [SNAPSHOT, CONTROL] {
        assert_eq!(custom::source(&store.readers.get(), subject).unwrap().unwrap()["state"], "ready");
    }
    store
}

fn assert_links_fail_closed_like_rebuild(tx: &Transaction<'_>) -> Result<()> {
    let incremental = [SNAPSHOT, CONTROL].map(|subject| custom::source(tx, subject).unwrap().unwrap());
    for row in &incremental { assert_ne!(row["state"], "ready"); assert!(row["error"].is_string()); }
    custom::rebuild(tx)?;
    for (subject, before) in [SNAPSHOT, CONTROL].iter().zip(incremental) {
        assert_eq!(custom::source(tx, subject)?.unwrap(), before);
    }
    Ok(())
}

#[test]
fn notice_first_creator_forgery_and_forged_runtime_are_refused() {
    let (store, source) = setup();
    for actor in ["agent/example/producer", "person/example"] {
        assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", actor, linked(&source, false))).is_err());
    }
    let mut fields = linked(&source, false); fields["runtime"] = json!("agent/example/forged");
    assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, fields)).is_err());
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&source, false))).unwrap();
}

#[test]
fn notice_agent_and_runtime_cannot_write_person_controls() {
    let (store, source) = setup();
    for actor in [RUNTIME, "agent/example/producer", "person/other"] {
        assert!(store.append_claim(&input(CONTROL, "custom.attention.control-created", actor, linked(&source, true))).is_err());
    }
    store.append_claim(&input(CONTROL, "custom.attention.control-created", "person/example", linked(&source, true))).unwrap();
    assert!(store.append_claim(&input(CONTROL, "custom.attention.control-updated", RUNTIME, linked(&source, true))).is_err());
}

#[test]
fn notice_altered_links_recipient_producer_and_manifest_are_refused() {
    let (store, source) = setup();
    for (field, value) in [("source_id", json!("custom/attention/source/other")), ("creation_claim", json!("0".repeat(64))),
        ("recipient", json!("person/other")), ("producer", json!(RUNTIME)), ("_registration", json!("0".repeat(64)))] {
        let mut fields = linked(&source, false); fields[field] = value;
        assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, fields)).is_err(), "{field}");
    }
    let mut fields = custom::fields(&source); fields.remove("_registration"); fields.insert("links".into(), json!([]));
    assert!(store.append_claim(&ClaimInput { fields, ..input(SOURCE, &source.kind, "agent/example/producer", json!({})) }).is_err());
}

#[test]
fn notice_second_designated_holder_is_refused_and_missing_binding_fails_closed() {
    let (store, _) = setup();
    assert_eq!(designate(&store, "example/other").unwrap_err().code, "notices-unavailable");
    let empty = Store::open_memory("example").unwrap();
    assert!(runtime(&empty.readers.get()).is_err());
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("DELETE FROM desired WHERE subject=?1", [RUNTIME])?;
        custom::rebuild(tx)?;
        let row = custom::source(tx, SOURCE)?.unwrap();
        assert_ne!(row["state"], "ready");
        assert!(row["error"].as_str().unwrap().contains("missing notice-runtime"));
        Ok(())
    }).unwrap().unwrap();
}

#[test]
fn notice_reserved_prefix_squat_and_alternate_claim_prefix_are_refused() {
    let store = Store::open_memory("example").unwrap();
    let mut manifest = notice::manifests()[0].clone(); manifest.kind = "attention.squat".into();
    assert!(store.register_custom_kind(&RegistrationRequest { manifest, actor: "person/example".into() }).is_err());
    assert!(store.append_claim(&input("custom/other/fact", "custom.attention.snapshot-created", RUNTIME, json!({}))).is_err());
    assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, json!({}))).is_err());
}

#[test]
fn notice_generic_reads_never_expose_reserved_raw_rows() {
    let (store, source) = setup();
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&source, false))).unwrap();
    for subject in [SOURCE, SNAPSHOT, CONTROL] { assert!(store.custom_subject(subject).is_err()); }
    for kind in [None, Some("attention.notice-source"), Some("attention.notice-snapshot")] {
        assert!(store.custom_subjects(kind, None, None, 10).unwrap().is_empty());
    }
}

#[test]
fn notice_source_payload_cannot_forge_authenticated_producer() {
    let (store, source) = setup();
    let mut fields = custom::fields(&source); fields.insert("producer".into(), json!("agent/example/forged"));
    assert!(store.append_claim(&ClaimInput { subject: format!("{}other", notice::SOURCE_PREFIX), fields,
        ..input(SOURCE, &source.kind, "agent/example/producer", json!({})) }).is_err());
}

#[test]
fn notice_later_replicated_source_conflict_invalidates_links_like_rebuild() {
    let (store, source) = setup();
    for (subject, kind, actor, control) in [
        (SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, false),
        (CONTROL, "custom.attention.control-created", "person/example", true),
    ] {
        store.append_claim(&input(subject, kind, actor, linked(&source, control))).unwrap();
        let connection = store.readers.get();
        assert_eq!(custom::source(&connection, subject).unwrap().unwrap()["state"], "ready");
        let edges: i64 = connection.query_row(
            "SELECT count(*) FROM custom_dependencies WHERE source=?1 AND dependency IN (?2,?3)",
            params![subject, SOURCE, source.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(edges, 2);
    }
    let remote = Store::open_memory("remote").unwrap();
    custom::tests_shared_delivery::legacy_sync(&store, &remote);
    // Model a mixed-version member admitting a competing creation in a later batch.
    let mut fields = custom::fields(&source); fields.insert("summary".into(), json!("Competing input"));
    remote.connection.batched(|tx| append_claim_tx(tx, &remote.origin, SOURCE,
        &source.kind, source.actor.as_deref(), &json!({"fields":fields}), &[], None)).unwrap().unwrap();
    custom::tests_shared_delivery::legacy_sync(&remote, &store);
    store.connection.batched(|tx| -> Result<()> {
        let incremental = [SNAPSHOT, CONTROL].map(|subject| custom::source(tx, subject).unwrap().unwrap());
        for row in &incremental {
            assert_eq!(row["state"], "invalid");
            assert!(row["error"].as_str().unwrap().contains("conflicting or ineligible source creation"));
        }
        custom::rebuild(tx)?;
        for (subject, before) in [SNAPSHOT, CONTROL].iter().zip(incremental) {
            assert_eq!(custom::source(tx, subject)?.unwrap(), before);
        }
        Ok(())
    }).unwrap().unwrap();
}

#[test]
fn notice_source_registration_and_document_removal_revalidate_links_like_rebuild() {
    for remove_document in [false, true] {
        let store = linked_store();
        store.seal_local_batches().unwrap();
        store.connection.batched(|tx| -> Result<()> {
            let registration: String = tx.query_row(
                "SELECT claim_id FROM custom_registrations WHERE prefix=?1", [notice::SOURCE_PREFIX], |r| r.get(0),
            )?;
            if remove_document {
                let reference: String = tx.query_row("SELECT json_extract(body,'$.fields.document') FROM claims WHERE id=?1",
                    [&registration], |r| r.get(0))?;
                let (name, hash) = split_document_ref(&reference)?;
                assert_eq!(tx.execute("DELETE FROM documents WHERE name=?1 AND hash=?2", params![name, hash])?, 1);
            } else {
                assert_eq!(tx.execute("UPDATE replica_records SET state='repaired' WHERE claim_id=?1", [&registration])?, 1);
            }
            custom::flush(tx)?;
            assert_links_fail_closed_like_rebuild(tx)
        }).unwrap().unwrap();
    }
}

#[test]
fn notice_local_designation_stop_change_and_retirement_match_rebuild() {
    for mutation in ["stop", "drop-role", "retire"] {
        let store = linked_store();
        let token = store.selected_desired_token(RUNTIME).unwrap().unwrap();
        if mutation == "retire" {
            let retirement = input(RUNTIME, retirements::KIND, "person/example",
                json!({"reason":"Finished","retired_by":"person/example","declaration":token,"owner_host":store.origin}));
            append_claim_with_commit_context(
                &store.graph, &retirement, None, None, None, None,
                ClaimCommitContext { retirement_writer: true, ..Default::default() },
            ).unwrap();
        } else {
            let source = if mutation == "stop" { "version 2\nstop \"agent/example/runtime\"" }
                else { "version 2\nagent \"example/runtime\" { command \"true\"; }" };
            let intent = crate::parse_intent(source, "example").unwrap();
            store.apply_as(&intent, &BTreeMap::from([(RUNTIME.into(), vec![token])]),
                mutation, Some("person/example")).unwrap();
        }
        store.connection.batched(assert_links_fail_closed_like_rebuild).unwrap().unwrap();
    }
}

#[test]
fn notice_owned_runtime_liveness_mutations_match_rebuild() {
    for mutation in ["terminal", "generation", "generation-removal"] {
        let store = linked_store();
        store.connection.batched(|tx| -> Result<()> {
            tx.execute("INSERT INTO mission_runs VALUES('notice-owner','example/notice-owner','revision',
                'notice-generation','revision','notice-owner',NULL,'.','person/example','{}','run','running','normal','0','0')", [])?;
            tx.execute("INSERT INTO run_generations VALUES('notice-generation','notice-owner','revision',
                NULL,'running','person/example','Fixture','0','0')", [])?;
            tx.execute("UPDATE desired SET owner_run='mission-run/notice-owner',
                owner_generation='run-generation/notice-generation' WHERE subject=?1", [RUNTIME])?;
            custom::flush(tx)?;
            for subject in [SNAPSHOT, CONTROL] {
                assert_eq!(custom::source(tx, subject)?.unwrap()["state"], "ready");
            }
            match mutation {
                "terminal" => { tx.execute("UPDATE mission_runs SET status='completed' WHERE id='notice-owner'", [])?; }
                "generation" => { tx.execute("UPDATE mission_runs SET current_generation_id='missing' WHERE id='notice-owner'", [])?; }
                _ => { tx.execute("DELETE FROM run_generations WHERE id='notice-generation'", [])?; }
            }
            custom::flush(tx)?;
            assert_links_fail_closed_like_rebuild(tx)
        }).unwrap().unwrap();
    }
}

#[test]
fn notice_only_runtime_dependency_chain_queues_revalidation() {
    for mutation in ["unrelated", "parent-step", "ancestor-run", "ancestor-generation"] {
        let store = linked_store();
        store.connection.batched(|tx| -> Result<()> {
            for name in ["owner", "ancestor", "unrelated"] {
                tx.execute("INSERT INTO mission_runs VALUES(?1,?1,'revision',?1,'revision',?1,
                    NULL,'.','person/example','{}','run','running','normal','0','0')", [name])?;
                tx.execute("INSERT INTO run_generations VALUES(?1,?1,'revision',NULL,'running',
                    'person/example','Fixture','0','0')", [name])?;
                tx.execute("INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,
                    status,attempt,goals,created_at_unix_ms,updated_at_unix_ms)
                    VALUES('step-run/'||?1,?1,?1,'build','hash','working',1,'[]','0','0')", [name])?;
            }
            tx.execute("UPDATE mission_runs SET parent_step_run='step-run/ancestor',
                root_run_id='ancestor' WHERE id='owner'", [])?;
            tx.execute("UPDATE desired SET owner_run='mission-run/owner',
                owner_generation='run-generation/owner' WHERE subject=?1", [RUNTIME])?;
            tx.execute("INSERT INTO desired(subject,kind,revision,claim_id,body)
                SELECT 'agent/example/unrelated',kind,revision,claim_id,'{}' FROM desired WHERE subject=?1", [RUNTIME])?;
            custom::flush(tx)?;
            assert_eq!(custom::source(tx, SNAPSHOT)?.unwrap()["state"], "ready");
            let queued = |tx: &Transaction<'_>| -> Result<i64> {
                Ok(tx.query_row("SELECT count(*) FROM local_custom_dirty", [], |r| r.get(0))?)
            };
            assert_eq!(queued(tx)?, 0);
            // Observe transient missing-generation projections, then restore referential integrity.
            tx.execute_batch("PRAGMA defer_foreign_keys=ON")?;
            if mutation == "unrelated" {
                for sql in [
                    "UPDATE mission_runs SET status='completed' WHERE id='unrelated'",
                    "UPDATE step_runs SET status='completed' WHERE subject='step-run/unrelated'",
                    "UPDATE run_generations SET id='unrelated-next' WHERE id='unrelated'",
                    "UPDATE run_generations SET id='unrelated' WHERE id='unrelated-next'",
                    "UPDATE desired SET body='{\"children\":[]}' WHERE subject='agent/example/unrelated'",
                ] {
                    tx.execute(sql, [])?;
                    assert_eq!(queued(tx)?, 0, "{sql}");
                }
                return Ok(());
            }
            tx.execute(match mutation {
                "parent-step" => "UPDATE step_runs SET status='completed' WHERE subject='step-run/ancestor'",
                "ancestor-run" => "UPDATE mission_runs SET status='completed' WHERE id='ancestor'",
                _ => "DELETE FROM run_generations WHERE id='ancestor'",
            }, [])?;
            assert!(queued(tx)? > 0);
            custom::flush(tx)?;
            assert_links_fail_closed_like_rebuild(tx)?;
            if mutation == "ancestor-generation" {
                tx.execute("INSERT INTO run_generations VALUES('ancestor','ancestor','revision',NULL,
                    'running','person/example','Fixture','0','0')", [])?;
            }
            Ok(())
        }).unwrap().unwrap();
    }
}
