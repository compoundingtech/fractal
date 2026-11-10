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
    let fixture: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h99da4f16cb38 owner=fractal_store::store::notice_authority_tests TODO=trace-statement-and-value-representation */ include_str!("../../../../docs/st3/client-v0/fixtures/notice-conformance.json")).unwrap();
    let source = store.append_claim(&input(SOURCE, "custom.attention.notice-created", "agent/example/producer", fixture["snapshot"]["input"].clone())).unwrap();
    (store, source)
}

fn input(subject: &str, kind: &str, actor: &str, fields: Value) -> ClaimInput {
    ClaimInput { subject: subject.into(), kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(), ..custom::tests_shared_delivery::empty_input() }
}

fn linked(store: &Store, source: &ClaimRecord, control: bool) -> Value {
    let mut fields = json!({"source_id":SOURCE,"creation_claim":source.id,"producer":"agent/example/producer","recipient":"person/example","runtime":RUNTIME});
    if control {
        fields.as_object_mut().unwrap().remove("runtime");
        for field in ["seen_at", "dismissed_at", "snoozed_until"] { fields[field] = Value::Null; }
    } else {
        fields["runtime_binding"] = json!(binding(&store.readers.get()).unwrap().1);
        fields["policy"] = json!({"disposition":"deliver"});
        fields["delivery"] = json!([]);
    }
    fields
}

fn linked_store() -> Store {
    let (store, source) = setup();
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&store, &source, false))).unwrap();
    store.append_claim(&input(CONTROL, "custom.attention.control-created", "person/example", linked(&store, &source, true))).unwrap();
    for subject in [SNAPSHOT, CONTROL] {
        assert_eq!(custom::source(&store.readers.get(), subject).unwrap().unwrap()["state"], "ready");
    }
    store
}

fn assert_links_fail_closed_like_rebuild(tx: &Transaction<'_>) -> Result<()> {
    for subject in [SNAPSHOT, CONTROL] {
        let row = custom::source(tx, subject)?.unwrap();
        assert_ne!(row["state"], "ready"); assert!(row["error"].is_string());
    }
    assert_links_match_rebuild(tx)
}

fn assert_links_match_rebuild(tx: &Transaction<'_>) -> Result<()> {
    let incremental = [SNAPSHOT, CONTROL].map(|subject| custom::source(tx, subject).unwrap().unwrap());
    custom::rebuild(tx)?;
    for (subject, before) in [SNAPSHOT, CONTROL].iter().zip(incremental) {
        assert_eq!(custom::source(tx, subject)?.unwrap(), before);
    }
    Ok(())
}

fn assert_runtime_fenced_like_rebuild(tx: &Transaction<'_>) -> Result<()> {
    let before = [SOURCE, SNAPSHOT, CONTROL].map(|subject| custom::source(tx, subject).unwrap().unwrap());
    assert_eq!(before[0]["state"], "ready");
    assert_eq!(before[2]["state"], "ready");
    assert_ne!(before[1]["state"], "ready");
    assert!(before[1]["error"].is_string());
    custom::rebuild(tx)?;
    for (subject, row) in [SOURCE, SNAPSHOT, CONTROL].iter().zip(before) {
        assert_eq!(custom::source(tx, subject)?.unwrap(), row);
    }
    Ok(())
}

#[test]
fn notice_first_creator_forgery_and_forged_runtime_are_refused() {
    let (store, source) = setup();
    for actor in ["agent/example/producer", "person/example"] {
        assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", actor, linked(&store, &source, false))).is_err());
    }
    let mut fields = linked(&store, &source, false); fields["runtime"] = json!("agent/example/forged");
    assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, fields)).is_err());
    let mut fields = linked(&store, &source, false); fields["runtime_binding"] = json!("0".repeat(64));
    assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, fields)).is_err());
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&store, &source, false))).unwrap();
}

#[test]
fn notice_agent_and_runtime_cannot_write_person_controls() {
    let (store, source) = setup();
    for actor in [RUNTIME, "agent/example/producer", "person/other"] {
        assert!(store.append_claim(&input(CONTROL, "custom.attention.control-created", actor, linked(&store, &source, true))).is_err());
    }
    store.append_claim(&input(CONTROL, "custom.attention.control-created", "person/example", linked(&store, &source, true))).unwrap();
    assert!(store.append_claim(&input(CONTROL, "custom.attention.control-updated", RUNTIME, linked(&store, &source, true))).is_err());
}

#[test]
fn notice_required_fields_cannot_be_omitted_independently() {
    let (store, source) = setup();
    for (subject, kind, actor, fields, required) in [
        ("custom/attention/source/01900000-0000-7000-8000-000000000004",
            source.kind.as_str(), "agent/example/producer", serde_json::to_value(custom::fields(&source)).unwrap(),
            &["schema_version", "dedupe_key", "resource", "recipient", "intent", "urgency",
                "requested", "summary", "links", "source", "created_at"][..]),
        (SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&source, false),
            &["source_id", "creation_claim", "producer", "recipient", "runtime", "policy", "delivery"][..]),
        (CONTROL, "custom.attention.control-created", "person/example", linked(&source, true),
            &["source_id", "creation_claim", "producer", "recipient", "runtime", "seen_at", "dismissed_at", "snoozed_until"][..]),
    ] {
        for field in required {
            let mut omitted = fields.clone();
            assert!(omitted.as_object_mut().unwrap().remove(*field).is_some());
            assert!(store.append_claim(&input(subject, kind, actor, omitted)).is_err(), "{kind}: {field}");
        }
        assert!(custom::source(&store.readers.get(), subject).unwrap().is_none(), "{kind}");
        store.append_claim(&input(subject, kind, actor, fields)).unwrap();
    }
}

#[test]
fn notice_snapshot_and_person_control_updates_preserve_links_like_rebuild() {
    let store = linked_store();
    let source = custom::claims(&store.readers.get(), SOURCE).unwrap();
    for (subject, kind, actor, control, field, value) in [
        (SNAPSHOT, "custom.attention.snapshot-updated", RUNTIME, false, "policy", json!({"disposition":"deliver","reason":"Updated policy"})),
        (CONTROL, "custom.attention.control-updated", "person/example", true, "seen_at", json!("2026-10-01T10:00:00Z")),
    ] {
        let original = linked(&source[0], control);
        let mut fields = original.clone(); fields[field] = value.clone();
        let updated = store.append_claim(&input(subject, kind, actor, fields)).unwrap();
        assert_eq!(custom::fields(&updated)[field], value);
        for link in ["source_id", "creation_claim", "producer", "recipient", "runtime"] {
            assert_eq!(custom::fields(&updated)[link], original[link], "{link}");
        }
        let row = custom::source(&store.readers.get(), subject).unwrap().unwrap();
        assert_eq!(row["state"], "ready");
        assert_eq!(row["input_count"], 2);
    }
    store.connection.batched(assert_links_match_rebuild).unwrap().unwrap();
}

#[test]
fn notice_control_update_refreshes_after_external_dirty_marker_conflict() {
    let fixture = linked_store();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "birch").unwrap();
    custom::tests_shared_delivery::legacy_sync(&fixture, &store);
    let before = custom::source(&store.readers.get(), CONTROL).unwrap().unwrap();
    assert_eq!(before["state"], "ready");
    assert_eq!(before["input_count"], 1);
    assert!(!transaction_work::custom_pending(&store.connection.write()));

    let external = Connection::open(&path).unwrap();
    configure_projection_writer(&external).unwrap();
    assert_eq!(external.query_row(
        "SELECT count(*) FROM sqlite_temp_master WHERE type='trigger' AND name GLOB 'custom_work_*'",
        [], |row| row.get::<_, i64>(0),
    ).unwrap(), 0);
    assert_eq!(external.execute(
        /* sqlite-large-value: allow-for-now lv-he5f2d04685a6 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_custom_dirty(subject) VALUES(?1)", [CONTROL],
    ).unwrap(), 1);
    assert!(external.is_autocommit());
    drop(external);
    {
        let connection = store.connection.write();
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject=?1)",
            [CONTROL], |row| row.get::<_, bool>(0),
        ).unwrap(), "the independent writer's durable marker must be visible");
        assert!(!transaction_work::custom_pending(&connection),
            "the external writer must not notify this connection");
    }

    let source = custom::claims(&store.readers.get(), SOURCE).unwrap();
    let mut fields = linked(&source[0], true);
    fields["seen_at"] = json!("2026-10-01T10:00:00Z");
    let updated = store.append_claim(&input(
        CONTROL, "custom.attention.control-updated", "person/example", fields,
    )).unwrap();
    assert_eq!(custom::fields(&updated)["seen_at"], "2026-10-01T10:00:00Z");
    let after = custom::source(&store.readers.get(), CONTROL).unwrap().unwrap();
    assert_eq!(after["state"], "ready");
    assert_eq!(after["input_count"], 2);
    assert_ne!(after["revision"], before["revision"]);
    assert!(!store.connection.write().query_row(
        "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject=?1)",
        [CONTROL], |row| row.get::<_, bool>(0),
    ).unwrap());
    store.connection.batched(assert_links_match_rebuild).unwrap().unwrap();
}

#[test]
fn notice_altered_links_recipient_producer_and_manifest_are_refused() {
    let (store, source) = setup();
    for (field, value) in [("source_id", json!("custom/attention/source/other")), ("creation_claim", json!("0".repeat(64))),
        ("recipient", json!("person/other")), ("producer", json!(RUNTIME)), ("_registration", json!("0".repeat(64)))] {
        let mut fields = linked(&store, &source, false); fields[field] = value;
        assert!(store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, fields)).is_err(), "{field}");
    }
    let mut fields = custom::fields(&source); fields.remove("_registration"); fields.insert("links".into(), json!([]));
    assert!(store.append_claim(&ClaimInput { fields, ..input(SOURCE, &source.kind, "agent/example/producer", json!({})) }).is_err());
}

#[test]
fn notice_second_designated_holder_is_refused_and_missing_binding_fails_closed() {
    let (store, source) = setup();
    assert_eq!(designate(&store, "example/other").unwrap_err().code, "notices-unavailable");
    let empty = Store::open_memory("example").unwrap();
    assert!(runtime(&empty.readers.get()).is_err());
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h4e5df88ef9e3 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject=?1", [RUNTIME])?;
        custom::rebuild(tx)?;
        let row = custom::source(tx, SOURCE)?.unwrap();
        assert_eq!(row["state"], "ready");
        Ok(())
    }).unwrap().unwrap();
    store.append_claim(&input(CONTROL, "custom.attention.control-created", "person/example", linked(&store, &source, true))).unwrap();
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
    store.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&store, &source, false))).unwrap();
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
        store.append_claim(&input(subject, kind, actor, linked(&store, &source, control))).unwrap();
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
                assert_eq!(tx.execute(/* sqlite-large-value: allow-for-now lv-h18dd79b21618 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM documents WHERE name=?1 AND hash=?2", params![name, hash])?, 1);
            } else {
                assert_eq!(tx.execute(/* sqlite-large-value: allow-for-now lv-h20a541ba3f90 owner=fractal_store::store::notice_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1", [&registration])?, 1);
            }
            custom::flush(tx)?;
            assert_links_fail_closed_like_rebuild(tx)
        }).unwrap().unwrap();
    }
}

#[test]
fn notice_local_designation_stop_change_and_retirement_match_rebuild() {
    for mutation in ["stop", "drop-role", "retire", "handover"] {
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
                else if mutation == "handover" {
                    "version 2\nstop \"agent/example/runtime\"\nagent \"example/next\" { command \"true\"; notice-runtime; }"
                } else { "version 2\nagent \"example/runtime\" { command \"true\"; }" };
            let intent = crate::parse_intent(source, "example").unwrap();
            let expected = intent.subjects.keys().map(|subject| (subject.clone(),
                if subject == RUNTIME { vec![token.clone()] } else { Vec::new() })).collect();
            store.apply_as(&intent, &expected, mutation, Some("person/example")).unwrap();
            if mutation == "handover" {
                assert_eq!(runtime(&store.readers.get()).unwrap(), "agent/example/next");
            }
        }
        store.connection.batched(assert_runtime_fenced_like_rebuild).unwrap().unwrap();
    }
}

#[test]
fn notice_owned_runtime_liveness_mutations_match_rebuild() {
    for mutation in ["terminal", "generation", "generation-removal"] {
        let store = linked_store();
        store.connection.batched(|tx| -> Result<()> {
            tx.execute(/* sqlite-large-value: allow-for-now lv-hb0fb45559620 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs VALUES('notice-owner','example/notice-owner','revision',
                'notice-generation','revision','notice-owner',NULL,'.','person/example','{}','run','running','normal','0','0')", [])?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h802ed557558f owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO run_generations VALUES('notice-generation','notice-owner','revision',
                NULL,'running','person/example','Fixture','0','0')", [])?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-haa66dc1a89ad owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_run='mission-run/notice-owner',
                owner_generation='run-generation/notice-generation' WHERE subject=?1", [RUNTIME])?;
            custom::flush(tx)?;
            for subject in [SNAPSHOT, CONTROL] {
                assert_eq!(custom::source(tx, subject)?.unwrap()["state"], "ready");
            }
            match mutation {
                "terminal" => { tx.execute(/* sqlite-large-value: allow-for-now lv-he7c0b1746d3b owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='completed' WHERE id='notice-owner'", [])?; }
                "generation" => { tx.execute(/* sqlite-large-value: allow-for-now lv-ha6074026d9d9 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET current_generation_id='missing' WHERE id='notice-owner'", [])?; }
                _ => { tx.execute(/* sqlite-large-value: allow-for-now lv-h3394ef4bbe97 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM run_generations WHERE id='notice-generation'", [])?; }
            }
            custom::flush(tx)?;
            assert_runtime_fenced_like_rebuild(tx)
        }).unwrap().unwrap();
    }
}

#[test]
fn notice_only_runtime_dependency_chain_queues_revalidation() {
    for mutation in ["unrelated", "parent-step", "ancestor-run", "ancestor-generation"] {
        let store = linked_store();
        store.connection.batched(|tx| -> Result<()> {
            for name in ["owner", "ancestor", "unrelated"] {
                tx.execute(/* sqlite-large-value: allow-for-now lv-h63e65e38bdf8 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs VALUES(?1,?1,'revision',?1,'revision',?1,
                    NULL,'.','person/example','{}','run','running','normal','0','0')", [name])?;
                tx.execute(/* sqlite-large-value: allow-for-now lv-h99362c36c696 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO run_generations VALUES(?1,?1,'revision',NULL,'running',
                    'person/example','Fixture','0','0')", [name])?;
                tx.execute(/* sqlite-large-value: allow-for-now lv-hd355c60374f5 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,
                    status,attempt,goals,created_at_unix_ms,updated_at_unix_ms)
                    VALUES('step-run/'||?1,?1,?1,'build','hash','working',1,'[]','0','0')", [name])?;
            }
            tx.execute(/* sqlite-large-value: allow-for-now lv-h95a6ae5cb3b4 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET parent_step_run='step-run/ancestor',
                root_run_id='ancestor' WHERE id='owner'", [])?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h8610ddeb875c owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_run='mission-run/owner',
                owner_generation='run-generation/owner' WHERE subject=?1", [RUNTIME])?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-he4d3e44fce0b owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired(subject,kind,revision,claim_id,body)
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
                    /* sqlite-large-value: allow-for-now lv-h1f35500980c5 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */
                    "UPDATE mission_runs SET status='completed' WHERE id='unrelated'",
                    /* sqlite-large-value: allow-for-now lv-hfe8fa93ac1c9 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */
                    "UPDATE step_runs SET status='completed' WHERE subject='step-run/unrelated'",
                    /* sqlite-large-value: allow-for-now lv-hd3906354fc71 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */
                    "UPDATE run_generations SET id='unrelated-next' WHERE id='unrelated'",
                    /* sqlite-large-value: allow-for-now lv-hcda036dc4a29 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */
                    "UPDATE run_generations SET id='unrelated' WHERE id='unrelated-next'",
                    /* sqlite-large-value: allow-for-now lv-hca8d43bc7c69 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */
                    "UPDATE desired SET body='{\"children\":[]}' WHERE subject='agent/example/unrelated'",
                ] {
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h21b3f50f7470 owner=fractal_store::store::notice_authority_tests TODO=trace-statement-and-value-representation */ sql, [])?;
                    assert_eq!(queued(tx)?, 0, "{sql}");
                }
                return Ok(());
            }
            tx.execute(match mutation {
                "parent-step" => /* sqlite-large-value: allow-for-now lv-h9e46780a0a9b owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='completed' WHERE subject='step-run/ancestor'",
                "ancestor-run" => /* sqlite-large-value: allow-for-now lv-hcc57e4fcd6e3 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='completed' WHERE id='ancestor'",
                _ => /* sqlite-large-value: allow-for-now lv-ha65e1b1067de owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM run_generations WHERE id='ancestor'",
            }, [])?;
            assert!(queued(tx)? > 0);
            custom::flush(tx)?;
            assert_runtime_fenced_like_rebuild(tx)?;
            if mutation == "ancestor-generation" {
                tx.execute(/* sqlite-large-value: allow-for-now lv-h5f87cb47cc14 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO run_generations VALUES('ancestor','ancestor','revision',NULL,
                    'running','person/example','Fixture','0','0')", [])?;
            }
            Ok(())
        }).unwrap().unwrap();
    }
}

#[test]
fn notice_runtime_identity_fences_match_rebuild_without_declaration_changes() {
    for fence in ["identity-head", "unknown", "invalid"] {
        let store = linked_store();
        store.connection.batched(|tx| -> Result<()> {
            if fence == "identity-head" {
                tx.execute(/* sqlite-large-value: allow-for-now lv-hc25ca9a02029 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO identity_heads VALUES(?1,'agent',?1,NULL,'conflict')
                    ON CONFLICT(identity) DO UPDATE SET unavailable='conflict'", [RUNTIME])?;
                tx.execute(/* sqlite-large-value: allow-for-now lv-hc47322f44af2 owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO identity_names VALUES(?1,?1,NULL)", [RUNTIME])?;
            } else {
                tx.execute(/* sqlite-large-value: allow-for-now lv-hea3a81fd2260 owner=fractal_store::store::notice_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,
                    raw,state,subject_hint,kind_hint,updated_at_unix_ms)
                    VALUES('record/notice-fence','example-peer',1,'pending',0,'{}',?1,?2,'agent.renamed','1')",
                    params![fence,RUNTIME])?;
            }
            assert!(current_desired_row(tx, RUNTIME).is_err());
            Ok(())
        }).unwrap().unwrap();
        // The commit finalizer must discover the fence even with no desired/liveness mutation.
        store.connection.batched(assert_runtime_fenced_like_rebuild).unwrap().unwrap();
        store.connection.batched(|tx| -> Result<()> {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h206fb305ff5c owner=fractal_store::store::notice_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE identity_heads SET unavailable=NULL WHERE identity=?1", [RUNTIME])?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h3a0326f24f3d owner=fractal_store::store::notice_authority_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_records WHERE record_ref='record/notice-fence'", [])?;
            Ok(())
        }).unwrap().unwrap();
        store.connection.batched(|tx| -> Result<()> {
            for subject in [SNAPSHOT, CONTROL] {
                assert_eq!(custom::source(tx, subject)?.unwrap()["state"], "ready");
            }
            Ok(())
        }).unwrap().unwrap();
    }
}

fn notice_replication_pair() -> (Store, Store) {
    use super::fleet_admission_tests_shared_delivery::{fleet, key, sync};
    let (_, _, mut members) = fleet(&[("example", &key()), ("reader", &key())]);
    let reader = members.pop().unwrap();
    let writer = members.pop().unwrap();
    designate(&writer, "example/runtime").unwrap();
    sync(&writer, &reader);
    for manifest in notice::manifests() {
        writer.register_custom_kind(&RegistrationRequest {
            manifest: manifest.clone(), actor: "person/example".into(),
        }).unwrap();
    }
    (writer, reader)
}

fn notice_replication_source(writer: &Store) -> ClaimRecord {
    let (_, fixture) = setup();
    writer.append_claim(&input(SOURCE, &fixture.kind, "agent/example/producer",
        serde_json::to_value(custom::fields(&fixture)).unwrap())).unwrap()
}

fn notice_replication_parity(store: &Store) -> [Option<Value>; 3] {
    for subject in [SOURCE, SNAPSHOT, CONTROL] { assert!(store.custom_subject(subject).is_err()); }
    assert!(store.custom_subjects(None, None, None, 10).unwrap().is_empty());
    store.connection.batched(|tx| -> Result<_> {
        let before = [SOURCE, SNAPSHOT, CONTROL].map(|subject| custom::source(tx, subject).unwrap());
        custom::rebuild(tx)?;
        assert_eq!([SOURCE, SNAPSHOT, CONTROL].map(|subject| custom::source(tx, subject).unwrap()), before);
        Ok(before)
    }).unwrap().unwrap()
}

fn notice_quarantined(store: &Store, claim: &ClaimRecord, state: &str) -> String {
    assert!(store.claim_by_id(&claim.id).unwrap().is_none(), "quarantine is not admission");
    let records = store.replica_records(false).unwrap();
    let record = records.iter().find(|record| record.claim_id.as_deref() == Some(&claim.id)).unwrap();
    assert_eq!(record.state, state);
    if state == "unknown" {
        assert_eq!(record.error_code.as_deref(), Some("missing-claim-dependencies"));
    } else {
        assert_ne!(record.error_code.as_deref(), Some("missing-claim-dependencies"));
    }
    record.record_ref.clone()
}

#[test]
fn notice_reordered_signed_dependencies_quarantine_then_converge_and_replay() {
    use super::fleet_admission_tests_shared_delivery::FLEET;
    let (writer, reader) = notice_replication_pair();
    let source = notice_replication_source(&writer);
    let snapshot = writer.append_claim(&input(SNAPSHOT, "custom.attention.snapshot-created",
        RUNTIME, linked(&writer, &source, false))).unwrap();
    let control = writer.append_claim(&input(CONTROL, "custom.attention.control-created",
        "person/example", linked(&writer, &source, true))).unwrap();
    let exchange = writer.export_replication_exchange(FLEET, &reader.replication_inventory().unwrap()).unwrap();
    let mut groups: [Vec<ReplicaEnvelope>; 6] = Default::default();
    for envelope in &exchange.envelopes {
        assert!(envelope.signature.is_some());
        let payload: ReplicaEnvelopePayload = ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
        let group = if payload.batch.claims.iter().any(|claim| claim.id == control.id) { 1 }
            else if payload.batch.claims.iter().any(|claim| claim.id == snapshot.id) { 2 }
            else if payload.batch.claims.iter().any(|claim| claim.id == source.id) { 5 }
            else if payload.batch.claims.iter().any(|claim| claim.kind == fractal_schema::custom::REGISTERED) { 3 }
            else if payload.batch.claims.iter().any(|claim| claim.kind == "doc.bound") { 4 }
            else { 0 };
        groups[group].push(envelope.clone());
    }
    for claim in [&source, &snapshot, &control] {
        assert!(writer.claim_signature(&claim.id).unwrap().is_some(), "actor signature travels with claim");
        assert!(reader.claim_by_id(&claim.id).unwrap().is_none());
    }
    assert!(notice_replication_parity(&reader).iter().all(Option::is_none));
    for (stage, envelopes) in groups.into_iter().enumerate() {
        assert!(!envelopes.is_empty(), "fixture must exercise delivery stage {stage}");
        let mut delivery = exchange.clone();
        delivery.envelopes = envelopes;
        reader.receive_replication_exchange("example", FLEET, &delivery).unwrap();
        // Dependency admission can require projection between retry passes.
        for _ in 0..4 {
            reader.validate_replication_backlog().unwrap();
            reader.project_replication_backlog().unwrap();
        }
        if stage == 3 {
            let registrations: Vec<_> = reader.replica_records(true).unwrap().into_iter()
                .filter(|record| record.kind.as_deref() == Some(fractal_schema::custom::REGISTERED)).collect();
            assert_eq!(registrations.len(), notice::manifests().len());
            for record in registrations {
                assert_eq!(record.state, "unknown");
                assert_eq!(record.error_code.as_deref(), Some("missing-claim-dependencies"));
                assert!(reader.claim_by_id(record.claim_id.as_deref().unwrap()).unwrap().is_none());
            }
        }
        if (1..5).contains(&stage) {
            notice_quarantined(&reader, &control, "unknown");
            if stage >= 2 { notice_quarantined(&reader, &snapshot, "unknown"); }
            assert!(reader.claim_by_id(&source.id).unwrap().is_none());
            assert!(notice_replication_parity(&reader).iter().all(Option::is_none));
        } else {
            notice_replication_parity(&reader);
        }
    }
    for claim in [&source, &snapshot, &control] {
        assert!(reader.claim_by_id(&claim.id).unwrap().is_some());
        assert_eq!(reader.claim_signature(&claim.id).unwrap(), writer.claim_signature(&claim.id).unwrap());
    }
    let ready = notice_replication_parity(&reader);
    assert!(ready.iter().all(|row| row.as_ref().unwrap()["state"] == "ready"));
    // Duplicate transport and a full canonical replay must preserve the converged projection.
    reader.receive_replication_exchange("example", FLEET, &exchange).unwrap();
    reader.validate_replication_backlog().unwrap();
    reader.project_replication_backlog().unwrap();
    assert_eq!(notice_replication_parity(&reader), ready);
    reader.replay_replication_graph().unwrap();
    assert_eq!(notice_replication_parity(&reader), ready);
}

#[test]
fn notice_signed_wrong_roles_hashes_and_cross_member_forgery_are_invalid_and_repairable() {
    use super::fleet_admission_tests_shared_delivery::{FLEET, sync};
    for forgery in ["snapshot-role", "control-role", "registration-hash", "cross-member"] {
        let (writer, reader) = notice_replication_pair();
        let source = notice_replication_source(&writer);
        sync(&writer, &reader);
        // Registration admission follows manifest projection, then source admission.
        for _ in 0..2 {
            reader.validate_replication_backlog().unwrap();
            reader.project_replication_backlog().unwrap();
        }
        assert!(reader.claim_by_id(&source.id).unwrap().is_some());
        let (subject, kind, actor, mut fields) = match forgery {
            "control-role" => (CONTROL, "custom.attention.control-created", RUNTIME, linked(&writer, &source, true)),
            "snapshot-role" => (SNAPSHOT, "custom.attention.snapshot-created", "agent/example/producer", linked(&writer, &source, false)),
            "cross-member" => (SNAPSHOT, "custom.attention.snapshot-created", "agent/reader/forger", linked(&writer, &source, false)),
            _ => (SNAPSHOT, "custom.attention.snapshot-created", RUNTIME, linked(&writer, &source, false)),
        };
        let replacement = writer.append_claim(&input(subject, kind,
            if subject == CONTROL { "person/example" } else { RUNTIME }, linked(&writer, &source, subject == CONTROL))).unwrap();
        writer.seal_local_batches().unwrap();
        let manifest = notice::manifests().iter().find(|manifest| subject.starts_with(&manifest.subject_prefix)).unwrap();
        fields["_registration"] = json!(notice::pinned_hash(manifest).unwrap());
        let author = if forgery == "cross-member" { &reader } else { &writer };
        let target = if forgery == "cross-member" { &writer } else { &reader };
        if forgery == "registration-hash" { fields["_registration"] = json!("0".repeat(64)); }
        // An older writer can sign a claim that its current local notice policy would refuse.
        // Only the receiving member's real signed admission path is evidence of rejection.
        author.ensure_principal_key(actor).unwrap();
        let forged = author.connection.batched(|tx| append_claim_tx(tx, &author.origin, subject,
            kind, Some(actor), &json!({"fields":fields}), &[], None)).unwrap().unwrap();
        let mut exchange = author.export_replication_exchange(FLEET, &target.replication_inventory().unwrap()).unwrap();
        exchange.envelopes.retain(|envelope| {
            let payload: ReplicaEnvelopePayload = ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
            !payload.batch.claims.iter().any(|claim| claim.id == replacement.id)
        });
        assert!(exchange.envelopes.iter().all(|envelope| envelope.signature.is_some()));
        target.receive_replication_exchange(&author.origin, FLEET, &exchange).unwrap();
        target.validate_replication_backlog().unwrap();
        target.project_replication_backlog().unwrap();
        assert!(author.claim_signature(&forged.id).unwrap().is_some());
        let record = notice_quarantined(target, &forged, "invalid");
        let before = notice_replication_parity(target);
        if forgery != "cross-member" { assert!(before[if subject == CONTROL { 2 } else { 1 }].is_none()); }
        target.replay_replication_graph().unwrap();
        notice_quarantined(target, &forged, "invalid");
        assert_eq!(notice_replication_parity(target), before);
        // Repair points at an independently admitted, correctly authored replacement.
        if forgery != "cross-member" { sync(&writer, &reader); }
        target.repair_replica_record(&record, &replacement.id, "Replace refused notice",
            "person/example", "notice-repair").unwrap();
        target.apply_replication_repairs().unwrap();
        target.project_replication_backlog().unwrap();
        assert_eq!(target.replica_record(&record).unwrap().unwrap().state, "repaired");
        assert!(target.claim_by_id(&forged.id).unwrap().is_none());
        let repaired = notice_replication_parity(target);
        assert_eq!(repaired[if subject == CONTROL { 2 } else { 1 }].as_ref().unwrap()["state"], "ready");
        target.replay_replication_graph().unwrap();
        assert_eq!(notice_replication_parity(target), repaired);
    }
}

#[test]
fn notice_unknown_kind_and_field_remain_schema_quarantine_not_dependency_or_invalid() {
    use super::fleet_admission_tests_shared_delivery::sync;
    for (kind, extra, code) in [
        ("custom.attention.future-created", false, "unknown-claim-kind"),
        ("custom.attention.snapshot-created", true, "unknown-claim-field"),
    ] {
        let (writer, reader) = notice_replication_pair();
        let source = notice_replication_source(&writer);
        sync(&writer, &reader);
        // Establish the known dependencies before testing unknown vocabulary.
        for _ in 0..2 {
            reader.validate_replication_backlog().unwrap();
            reader.project_replication_backlog().unwrap();
        }
        assert!(reader.claim_by_id(&source.id).unwrap().is_some());
        let mut fields = linked(&writer, &source, false);
        let manifest = notice::manifests().iter().find(|manifest| SNAPSHOT.starts_with(&manifest.subject_prefix)).unwrap();
        fields["_registration"] = json!(notice::pinned_hash(manifest).unwrap());
        if extra { fields["future_field"] = json!(true); }
        writer.ensure_principal_key(RUNTIME).unwrap();
        let unknown = writer.connection.batched(|tx| append_claim_tx(tx, &writer.origin, SNAPSHOT,
            kind, Some(RUNTIME), &json!({"fields":fields}), &[], None)).unwrap().unwrap();
        sync(&writer, &reader);
        assert!(reader.claim_by_id(&unknown.id).unwrap().is_none());
        let records = reader.replica_records(true).unwrap();
        let record = records.iter().find(|record| record.claim_id.as_deref() == Some(&unknown.id)).unwrap();
        assert_eq!(record.state, "unknown");
        assert_eq!(record.error_code.as_deref(), Some(code));
        let before = notice_replication_parity(&reader);
        assert!(before[1].is_none());
        reader.replay_replication_graph().unwrap();
        assert_eq!(notice_replication_parity(&reader), before);
        assert_eq!(reader.replica_record(&record.record_ref).unwrap().unwrap().error_code.as_deref(), Some(code));
    }
}

#[test]
fn notice_source_and_person_controls_survive_handover_and_absent_runtime() {
    let store = linked_store();
    let source = custom::claims(&store.readers.get(), SOURCE).unwrap().remove(0);
    let source_bytes = source.body.clone();
    let token = store.selected_desired_token(RUNTIME).unwrap().unwrap();
    let intent = crate::parse_intent("version 2\nstop \"agent/example/runtime\"", "example").unwrap();
    store.apply_as(&intent, &BTreeMap::from([(RUNTIME.into(), vec![token])]),
        "stop-notice-runtime", Some("person/example")).unwrap();
    let control = store.append_claim(&input(CONTROL, "custom.attention.control-updated",
        "person/example", linked(&store, &source, true))).unwrap();
    assert!(control.body["fields"].get("runtime").is_none());
    let fields = custom::fields(&source);
    store.append_claim(&ClaimInput {
        subject: "custom/attention/source/01900000-0000-7000-8000-000000000004".into(),
        fields, ..input(SOURCE, &source.kind, "agent/example/producer", json!({}))
    }).unwrap();
    designate(&store, "example/next-runtime").unwrap();
    assert_eq!(store.claim_by_id(&source.id).unwrap().unwrap().body, source_bytes);
    store.connection.batched(assert_runtime_fenced_like_rebuild).unwrap().unwrap();
    assert!(store.custom_subject(SOURCE).is_err());
    assert!(store.custom_subject(CONTROL).is_err());
}

#[path = "notice_materialization_tests.rs"]
mod materialization;

#[test]
fn notice_untouched_finalizer_is_sql_free_even_with_ready_notices() {
    use rusqlite::hooks::Authorization;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let store = linked_store();
    let mut connection = store.connection.write();
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
    let store = linked_store();
    let mut connection = store.connection.write();
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
