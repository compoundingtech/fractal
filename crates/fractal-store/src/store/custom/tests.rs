#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_delivery::{DECIDER, FLEET, RAW_KINDS, SEAT, TREE, decision_tree, empty_input, example, legacy_sync, registration};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_delivery::{request, signed_pair, signed_sync, tree_claim, tree_document, tree_status};
use super::*;

#[test]
fn decision_inputs_reject_unregistered_person_answer() {
    let store = Store::open_memory("example").unwrap();
    tree_claim(&store, TREE, "answered", DECIDER, json!({
        "question":"ship", "answer":"yes"
    })).unwrap();
    assert!(store.decision_inputs(TREE).unwrap_err().to_string().contains("registration is unavailable"));
}

#[test]
fn decision_inputs_require_recipient_authority_in_immutable_registration() {
    let store = Store::open_memory("example").unwrap();
    let mut manifest = decision_tree();
    // Keep attention's recipient mapping and a schema-valid recipient-only reply,
    // while weakening the distinct answer claim consumed by the decision reader.
    let reply_kind = "custom.decision.tree.v1.reply";
    manifest.claims.insert(
        reply_kind.into(),
        manifest.claims["custom.decision.tree.v1.answered"].clone(),
    );
    manifest.attention.as_mut().unwrap().reply.kind = reply_kind.into();
    manifest.claims.get_mut("custom.decision.tree.v1.answered").unwrap().authority = Authority::Owner;
    store.register_custom_kind(&RegistrationRequest {
        manifest, actor: SEAT.into(),
    }).unwrap();
    tree_claim(&store, TREE, "opened", SEAT, json!({
        "seat":SEAT, "recipient":DECIDER
    })).unwrap();
    assert!(store.decision_inputs(TREE).unwrap_err().to_string()
        .contains("only from the addressed recipient"));
}

fn decision_inputs_limit_fixture() -> (Store, ClaimRecord, u64) {
    let store = Store::open_memory("example").unwrap();
    store.register_custom_kind(&RegistrationRequest {
        manifest: decision_tree(), actor: SEAT.into(),
    }).unwrap();
    let opened = tree_claim(&store, TREE, "opened", SEAT, json!({
        "seat":SEAT, "recipient":DECIDER
    })).unwrap();
    let bytes = store.readers.get().query_row(
        "SELECT length(CAST(body AS BLOB)) FROM claims WHERE id=?1",
        [&opened.id], |row| row.get(0),
    ).unwrap();
    (store, opened, bytes)
}

#[test]
fn decision_inputs_claim_limit_refuses_instead_of_truncating() {
    let (store, opened, bytes) = decision_inputs_limit_fixture();
    let error = store.decision_inputs_with_limits(TREE, 0, bytes).unwrap_err();
    assert_eq!(error.downcast_ref::<FractalError>().unwrap().code, "decision-inputs-too-large");
    let snapshot = store.decision_inputs_with_limits(TREE, 1, bytes).unwrap();
    assert_eq!(snapshot.claims.len(), 1);
    assert_eq!(snapshot.claims[0].id, opened.id);
}

#[test]
fn decision_inputs_body_byte_limit_refuses_instead_of_truncating() {
    let (store, opened, bytes) = decision_inputs_limit_fixture();
    let error = store.decision_inputs_with_limits(TREE, 1, bytes - 1).unwrap_err();
    assert_eq!(error.downcast_ref::<FractalError>().unwrap().code, "decision-inputs-too-large");
    let snapshot = store.decision_inputs_with_limits(TREE, 1, bytes).unwrap();
    assert_eq!(snapshot.claims.len(), 1);
    assert_eq!(snapshot.claims[0].id, opened.id);
}

#[test]
fn decision_inputs_limits_exclude_repaired_history() {
    let (store, opened, bytes) = decision_inputs_limit_fixture();
    let raw = tree_document(&store, "doc/example/limit-repair", b"unreadable source");
    let repaired = tree_claim(&store, TREE, "damaged", SEAT, json!({
        "raw":raw, "records":1, "imported":0, "malformed":1
    })).unwrap();
    store.seal_local_batches().unwrap();
    store.connection.batched(|tx| -> Result<()> {
        let changed = tx.execute(
            /* sqlite-large-value: allow-for-now lv-h7ddc25a3b2d6 owner=fractal_store::store::custom::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1",
            [&repaired.id],
        )?;
        assert!(changed > 0);
        Ok(())
    }).unwrap().unwrap();
    let snapshot = store.decision_inputs_with_limits(TREE, 1, bytes).unwrap();
    assert_eq!(snapshot.claims.len(), 1);
    assert_eq!(snapshot.claims[0].id, opened.id);
}

#[test]
fn decision_inputs_limits_count_all_claims_and_sum_utf8_body_bytes() {
    let (store, _, opened_bytes) = decision_inputs_limit_fixture();
    let raw = tree_document(&store, "doc/example/limit-bytes", b"unreadable source");
    let answered = tree_claim(&store, TREE, "damaged", SEAT, json!({
        "raw":raw, "records":1, "imported":0, "malformed":1, "detail":"✓"
    })).unwrap();
    let (answer_bytes, answer_chars): (u64, u64) = store.readers.get().query_row(
        "SELECT length(CAST(body AS BLOB)),length(body) FROM claims WHERE id=?1",
        [&answered.id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert!(answer_bytes > answer_chars);
    let total = opened_bytes + answer_bytes;
    let count_error = store.decision_inputs_with_limits(TREE, 1, total).unwrap_err();
    assert_eq!(count_error.downcast_ref::<FractalError>().unwrap().code, "decision-inputs-too-large");
    let byte_error = store.decision_inputs_with_limits(TREE, 2, total - 1).unwrap_err();
    assert_eq!(byte_error.downcast_ref::<FractalError>().unwrap().code, "decision-inputs-too-large");
    assert_eq!(store.decision_inputs_with_limits(TREE, 2, total).unwrap().claims.len(), 2);
}

fn decision_preflight_work_fixture(count: u64, repaired: bool) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    // Only the preflight's authoritative tables/indexes are needed. Avoid projection
    // maintenance so the measured work is the production preflight, not fixture setup.
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h749cf1015bd0 owner=fractal_store::store::custom::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE batches(id TEXT PRIMARY KEY);
         CREATE TABLE claims(id TEXT PRIMARY KEY,batch_id TEXT,subject TEXT,body TEXT);
         CREATE INDEX claims_subject ON claims(subject);
         CREATE TABLE replica_records(claim_id TEXT,state TEXT);
         CREATE INDEX replica_records_claim ON replica_records(claim_id);
         CREATE INDEX replica_records_repaired_claim ON replica_records(claim_id) WHERE state='repaired';
         INSERT INTO batches VALUES('batch');",
    ).unwrap();
    {
        let tx = connection.unchecked_transaction().unwrap();
        let mut insert = tx.prepare(
            /* sqlite-large-value: allow-for-now lv-h9d4209b9ce0c owner=fractal_store::store::custom::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES(?1,'batch',?2,'{\"fields\":{\"detail\":\"✓\"}}')",
        ).unwrap();
        let mut repair = tx.prepare(
            /* sqlite-large-value: allow-for-now lv-h7b32036bdf19 owner=fractal_store::store::custom::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records VALUES(?1,'repaired')",
        ).unwrap();
        for index in 0..count {
            insert.execute(/* sqlite-large-value: allow-for-now lv-h51660fd08a1f owner=fractal_store::store::custom::tests TODO=trace-statement-and-value-representation */ params![index.to_string(), TREE]).unwrap();
            if repaired {
                repair.execute(/* sqlite-large-value: allow-for-now lv-h09f141e001e0 owner=fractal_store::store::custom::tests TODO=trace-statement-and-value-representation */ [index.to_string()]).unwrap();
            }
        }
        drop(insert);
        drop(repair);
        tx.commit().unwrap();
    }
    connection
}

fn decision_preflight_refusal_steps(
    connection: &Connection,
    max_claims: u64,
    max_body_bytes: u64,
) -> usize {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let steps = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&steps);
    connection.progress_handler(1, Some(move || {
        counter.fetch_add(1, Ordering::Relaxed);
        false
    }));
    let result = decision_inputs_preflight(connection, TREE, max_claims, max_body_bytes);
    connection.progress_handler(0, None::<fn() -> bool>);
    assert_eq!(
        result.unwrap_err().downcast_ref::<FractalError>().unwrap().code,
        "decision-inputs-too-large",
    );
    steps.load(Ordering::Relaxed)
}

#[test]
fn decision_inputs_claim_preflight_stops_sqlite_before_oversized_tail() {
    let costs = [256, 8192].map(|count| {
        let connection = decision_preflight_work_fixture(count, false);
        decision_preflight_refusal_steps(&connection, 1, u64::MAX)
    });
    assert!(costs[0] < 1000, "claim cap must stop after two rows: {costs:?}");
    assert!(costs[1] <= costs[0] + 100, "tail must not add SQLite work: {costs:?}");
}

#[test]
fn decision_inputs_byte_preflight_stops_sqlite_before_oversized_tail() {
    let costs = [256, 8192].map(|count| {
        let connection = decision_preflight_work_fixture(count, false);
        // Disable the effective count cap: only the running byte cap can stop this read.
        decision_preflight_refusal_steps(&connection, u64::MAX, 1)
    });
    assert!(costs[0] < 1000, "byte cap must stop after one row: {costs:?}");
    assert!(costs[1] <= costs[0] + 100, "tail must not add SQLite work: {costs:?}");
}

#[test]
fn decision_inputs_preflight_bounds_repaired_history_work() {
    let small = decision_preflight_work_fixture(2, true);
    decision_inputs_preflight(&small, TREE, 0, 0).unwrap();
    let costs = [
        DECISION_INPUTS_MAX_SCANNED_CLAIMS + 1,
        4 * DECISION_INPUTS_MAX_SCANNED_CLAIMS,
    ].map(|count| {
        let connection = decision_preflight_work_fixture(count, true);
        decision_preflight_refusal_steps(&connection, 0, 0)
    });
    assert!(
        costs[1] <= costs[0] + 100,
        "repaired-only history must stop at the scanned-row budget: {costs:?}",
    );
}

fn audit_custom_dirty(store: &Store) {
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h4094ac3a489b owner=fractal_store::store::custom::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE custom_dirty_audit(subject TEXT);
         CREATE TRIGGER custom_dirty_audit_insert AFTER INSERT ON local_custom_dirty BEGIN
         INSERT INTO custom_dirty_audit VALUES(NEW.subject); END;",
    ).unwrap();
}

#[test]
fn notice_upgrade_revalidates_quiet_legacy_registration_like_replay() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "example").unwrap();
    let reviewed = fractal_schema::notice::manifests()[0].clone();
    let mut substitute = reviewed.clone();
    substitute.claims.get_mut(&substitute.creation_kind).unwrap().additional_fields = true;
    let bytes = serde_json::to_vec(&substitute).unwrap();
    let hash = hex::encode(Sha256::digest(&bytes));
    let name = format!("doc/custom-kinds/{hash}");
    store.put_document_as(&name, &bytes, &None, "legacy-manifest", Some("person/example")).unwrap();
    let claim = store.connection.batched(|tx| append_claim_tx(
        tx, store.origin(), &substitute.subject(), schema::REGISTERED, Some("person/example"),
        &json!({"fields":{"document":format!("{name}@{hash}")}}), &[], None,
    )).unwrap().unwrap();
    // Emulate a quiet pre-reservation projection, including its old version marker.
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-hb426ee0c141e owner=fractal_store::store::custom::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO custom_registrations VALUES(?1,?2,?3,?4,?5,?6,?7,'ready',NULL)",
            params![substitute.subject(),substitute.kind,substitute.version,substitute.subject_prefix,
                hash,canonical_serialized_json_text(&substitute)?,claim.id])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h105a7bf3df39 owner=fractal_store::store::custom::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='1' WHERE key='custom_projections_version'", [])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h946810538d0c owner=fractal_store::store::custom::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_custom_dirty", [])?;
        Ok(())
    }).unwrap().unwrap();
    drop(store);
    let upgraded = Store::open(&path, "example").unwrap();
    upgraded.connection.batched(|tx| -> Result<()> {
        let incremental = registration_rows(tx)?;
        assert!(!incremental.contains_key(&reviewed.subject()));
        rebuild(tx)?;
        assert_eq!(registration_rows(tx)?, incremental);
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(upgraded.register_custom_kind(&RegistrationRequest {
        manifest: reviewed, actor: "person/example".into(),
    }).unwrap()["state"], "ready");
}

#[test]
fn custom_ordinary_claim_append_and_seal_never_enqueue_dirty_subjects() {
    let store = Store::open_memory("example").unwrap();
    audit_custom_dirty(&store);
    for subject in ["agent/example-a", "agent/example-b"] {
        store.append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.observed".into(),
            ..empty_input()
        }).unwrap();
    }
    store.seal_local_batches().unwrap();
    let count: i64 = store.connection.write().query_row(
        "SELECT count(*) FROM custom_dirty_audit", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0, "flush clearing the queue must not hide transient inserts");
}

fn externally_mark_custom_dirty(store: &Store, path: &std::path::Path, subject: &str) {
    assert!(!transaction_work::custom_pending(&store.connection.write()));
    let external = Connection::open(path).unwrap();
    configure_projection_writer(&external).unwrap();
    // A non-Store writer has the durable projection functions, but none of the
    // connection-local custom work notifiers installed by custom::create_schema.
    let notifiers: i64 = external.query_row(
        "SELECT count(*) FROM sqlite_temp_master WHERE type='trigger' AND name GLOB 'custom_work_*'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(notifiers, 0);
    assert_eq!(external.execute(
        /* sqlite-large-value: allow-for-now lv-hade739ecf70b owner=fractal_store::store::custom::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_custom_dirty(subject) VALUES(?1)", [subject],
    ).unwrap(), 1);
    assert!(external.is_autocommit());
    drop(external);

    let connection = store.connection.write();
    assert!(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject=?1)",
        [subject], |row| row.get::<_, bool>(0),
    ).unwrap(), "the external marker must be committed and visible to the Store writer");
    assert!(!transaction_work::custom_pending(&connection),
        "another connection's durable marker must not set this writer's hint");
}

#[test]
fn custom_claim_append_refreshes_after_external_dirty_marker_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "example").unwrap();
    registration(&store);
    let subject = "custom/garden/review/v1/external-dirty-append";
    let requested = request(&store, subject);
    let before = store.custom_subject(subject).unwrap().unwrap();
    assert_eq!(before["state"], "ready");
    assert!(before["fields"]["selection"].is_null());
    assert_eq!(store.attention_items(Some("person/lichen")).unwrap().len(), 1);
    externally_mark_custom_dirty(&store, &path, subject);

    store.append_claim(&ClaimInput {
        subject: subject.into(),
        kind: "custom.garden.review.v1.answered".into(),
        actor: Some("person/lichen".into()),
        fields: serde_json::from_value(json!({
            "request": requested.id, "selection": "keep"
        })).unwrap(),
        ..empty_input()
    }).unwrap();

    let after = store.custom_subject(subject).unwrap().unwrap();
    assert_eq!(after["state"], "ready");
    assert_eq!(after["fields"]["selection"], "keep");
    assert_ne!(after["revision"], before["revision"]);
    assert!(store.attention_items(Some("person/lichen")).unwrap().is_empty());
    assert!(!store.connection.write().query_row(
        "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject=?1)",
        [subject], |row| row.get::<_, bool>(0),
    ).unwrap());
}

#[test]
fn custom_replicated_repair_refreshes_after_external_dirty_marker_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let source = Store::open_memory("alder").unwrap();
    let store = Store::open(&path, "birch").unwrap();
    // Use the signed_pair admission pattern with a file-backed receiver so an
    // independent SQLite writer can leave a durable dirty marker there.
    let keys = (0..2)
        .map(|_| Arc::new(crate::fleet::MemberKey::generate().unwrap().0))
        .collect::<Vec<_>>();
    for (index, member) in [&source, &store].into_iter().enumerate() {
        member.bind_fleet(FLEET).unwrap();
        member.pin_fleet_anchor(keys[0].public()).unwrap();
        member.set_member_key(Some(keys[index].clone())).unwrap();
        let mut fields = json!({
            "fleet_id": FLEET, "member_key": keys[index].public(),
            "via": if index == 0 { "anchor" } else { "invite" }, "mode": "listening"
        });
        if index != 0 {
            fields["sponsor"] = json!("host/alder");
        }
        source.append_claim(&ClaimInput {
            subject: format!("host/{}", member.origin()),
            kind: "fleet.member-admitted".into(),
            fields: serde_json::from_value(fields).unwrap(),
            ..empty_input()
        }).unwrap();
    }
    registration(&source);
    let subject = "custom/garden/review/v1/external-dirty-repair";
    let requested = request(&source, subject);
    let answered = source.append_claim(&ClaimInput {
        subject: subject.into(),
        kind: "custom.garden.review.v1.answered".into(),
        actor: Some("person/lichen".into()),
        fields: serde_json::from_value(json!({
            "request": requested.id, "selection": "keep"
        })).unwrap(),
        ..empty_input()
    }).unwrap();
    signed_sync(&source, &store);
    let before = store.custom_subject(subject).unwrap().unwrap();
    assert_eq!(before["state"], "ready");
    assert_eq!(before["fields"]["selection"], "keep");
    assert!(store.attention_items(Some("person/lichen")).unwrap().is_empty());
    externally_mark_custom_dirty(&store, &path, subject);

    // Isolate the canonical repair projection from receiving new claims: an
    // unrelated INSERT could set the hint and hide a lost conflicting dirty mark.
    // Validate the raw seam against the actually replicated record and the same
    // repaired-filtered canonical history consumed by the custom projection.
    store.connection.batched(|tx| -> Result<()> {
        assert!(!transaction_work::custom_pending(tx));
        assert_eq!(tx.query_row(
            "SELECT count(*) FROM replica_records WHERE claim_id=?1",
            [&answered.id], |row| row.get::<_, i64>(0),
        )?, 1);
        assert_eq!(tx.query_row(
            "SELECT kind_hint FROM replica_records WHERE claim_id=?1",
            [&answered.id], |row| row.get::<_, Option<String>>(0),
        )?.as_deref(), Some("custom.garden.review.v1.answered"),
            "a NULL hint would exercise the generic notifier instead of custom-kind repair");
        assert!(claims(tx, subject)?.iter().any(|claim| claim.id == answered.id));
        assert_eq!(tx.execute(
            /* sqlite-large-value: allow-for-now lv-h72c162193663 owner=fractal_store::store::custom::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1 AND state<>'repaired'",
            [&answered.id],
        )?, 1);
        let canonical = claims(tx, subject)?;
        assert_eq!(canonical.len(), 1);
        assert_eq!(canonical[0].id, requested.id);
        assert!(!canonical.iter().any(|claim| claim.id == answered.id));
        flush(tx)
    }).unwrap().unwrap();

    let after = store.custom_subject(subject).unwrap().unwrap();
    assert_eq!(after["state"], "ready");
    assert!(after["fields"]["selection"].is_null());
    assert_ne!(after["revision"], before["revision"]);
    let attention = store.attention_items(Some("person/lichen")).unwrap();
    assert_eq!(attention.len(), 1);
    assert!(!store.connection.write().query_row(
        "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject=?1)",
        [subject], |row| row.get::<_, bool>(0),
    ).unwrap());
}

#[test]
fn custom_non_custom_dependency_canonical_change_refreshes_consumer() {
    let store = Store::open_memory("example").unwrap();
    let mut manifest = example();
    let reference: schema::Field =
        serde_json::from_value(json!({"value_type":"string","claim":true})).unwrap();
    manifest.claims.get_mut(&manifest.creation_kind.clone()).unwrap()
        .fields.insert("input".into(), reference);
    store.register_custom_kind(&RegistrationRequest {
        manifest: manifest.clone(), actor: "agent/example-a".into(),
    }).unwrap();
    let dependency = "agent/example-a";
    let input = store.append_claim(&ClaimInput {
        subject: dependency.into(), kind: "runtime.observed".into(), ..empty_input()
    }).unwrap();
    let consumer = "custom/garden/review/v1/example";
    store.append_claim(&ClaimInput {
        subject: consumer.into(), kind: manifest.creation_kind,
        actor: Some("agent/example-a".into()),
        fields: serde_json::from_value(json!({
            "title":"Review", "detail":"Evidence", "recipient":"person/robin", "input": &input.id
        })).unwrap(), ..empty_input()
    }).unwrap();
    assert_eq!(store.custom_subject(consumer).unwrap().unwrap()["state"], "ready");
    store.seal_local_batches().unwrap();
    audit_custom_dirty(&store);
    // Synthetic canonical repair isolates record invalidation from claim insertion.
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h59a9d79d4e2d owner=fractal_store::store::custom::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1", [&input.id])?;
        let queued: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject=?1)",
            [consumer], |row| row.get(0),
        )?;
        assert!(queued);
        flush(tx)
    }).unwrap().unwrap();
    // Claim references remain available after canonical repair; the queue assertion
    // above proves invalidation without relying on a basis restricted to custom kinds.
    assert_eq!(store.custom_subject(consumer).unwrap().unwrap()["state"], "ready");
}

#[test]
fn custom_noop_updates_preserve_dirty_queue_and_schema_reopen_is_stable() {
    let store = Store::open_memory("example").unwrap();
    store.append_claim(&ClaimInput {
        subject: "custom/example/fact".into(), kind: "custom.example.recorded".into(),
        ..empty_input()
    }).unwrap();
    store.put_document_as(
        "doc/example-a", b"example", &None, "example-document", Some("agent/example-a"),
    ).unwrap();
    store.seal_local_batches().unwrap();
    audit_custom_dirty(&store);
    let connection = store.connection.write();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h77cbd3b06e97 owner=fractal_store::store::custom::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE claims SET subject=subject,body=body;
         UPDATE documents SET name=name,hash=hash;
         UPDATE batches SET origin=origin,replica_sequence=replica_sequence;
         UPDATE replica_records SET position=position,claim_id=claim_id,state=state;",
    ).unwrap();
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM custom_dirty_audit", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0);
    let cookie: i64 = connection.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
    create_schema(&connection).unwrap();
    assert_eq!(connection.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0)).unwrap(), cookie);
}

#[test]
fn custom_registration_document_record_change_reaches_registry_expansion() {
    let store = Store::open_memory("example").unwrap();
    store.register_custom_kind(&RegistrationRequest {
        manifest: example(), actor: "agent/example-a".into(),
    }).unwrap();
    store.seal_local_batches().unwrap();
    store.connection.batched(|tx| -> Result<()> {
        let (binding, registry): (String, String) = tx.query_row(
            "SELECT binding.id,registration.subject FROM claims AS binding
             JOIN claims AS registration ON registration.kind='custom.st3-kinds.registered'
             AND substr(json_extract(registration.body,'$.fields.document'),1,length(binding.subject)+1)=binding.subject||'@'
             WHERE binding.kind='doc.bound' LIMIT 1", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-hbe9ab32f43d1 owner=fractal_store::store::custom::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1", [binding])?;
        assert!(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject=?1)",
            [registry], |row| row.get::<_, bool>(0),
        )?);
        flush(tx)
    }).unwrap().unwrap();
}

fn invalidations_view(connection: &Connection) -> String {
    connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type='view' AND name='custom_claim_invalidations'",
        [], |row| row.get(0),
    ).unwrap()
}

#[test]
fn custom_batch_suffix_invalidations_look_claims_up_by_id_not_every_document_binding() {
    let store = Store::open_memory("example").unwrap();
    let connection = store.connection.write();
    // The statement custom_claim_insert runs for every claim it appends. Without
    // sqlite_stat1 the planner guesses, as it does on a member that never ran ANALYZE.
    let plan: Vec<String> = connection.prepare(
        "EXPLAIN QUERY PLAN SELECT subject FROM custom_claim_invalidations WHERE id IN
         (SELECT id FROM claims WHERE batch_id=?1 AND store_index>?2
          AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id))",
    ).unwrap().query_map(params!["batch/example", 0], |row| row.get(3)).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    let binding: Vec<&String> = plan.iter().filter(|line| line.contains("binding")).collect();
    assert!(!binding.is_empty(), "{plan:#?}");
    assert!(
        binding.iter().all(|line| !line.contains("claims_kind_index")),
        "a lookup by claim ID must not walk every doc.bound claim: {plan:#?}",
    );
}

#[test]
fn custom_schema_replaces_an_installed_older_invalidations_view_once() {
    let store = Store::open_memory("example").unwrap();
    let connection = store.connection.write();
    let current = invalidations_view(&connection);
    let older = current.replace("+binding.kind", "binding.kind");
    assert_ne!(older, current);
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h5d05266b3a85 owner=fractal_store::store::custom::tests TODO=bound-or-reference-values-and-reclamation */ "DROP VIEW custom_claim_invalidations").unwrap();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h285b42617459 owner=fractal_store::store::custom::tests TODO=trace-statement-and-value-representation */ &older).unwrap();
    create_schema(&connection).unwrap();
    assert_eq!(invalidations_view(&connection), current);
    let cookie: i64 = connection.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
    create_schema(&connection).unwrap();
    assert_eq!(connection.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0)).unwrap(), cookie);
}

#[test]
fn custom_bad_projection_isolated_from_other_sources_and_core_reads() {
    let s = Store::open_memory("alder").unwrap();
    registration(&s);
    request(&s, "custom/garden/review/v1/good");
    let mut m = example();
    m.version = 2;
    m.subject_prefix = "custom/garden/review/v2/".into();
    m.attention.as_mut().unwrap().title = Expr::Constant { value: json!(17) };
    s.register_custom_kind(&RegistrationRequest {
        manifest: m.clone(),
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let id = "custom/garden/review/v2/bad";
    request(&s, id);
    assert_eq!(s.custom_subject(id).unwrap().unwrap()["state"], "invalid");
    assert_eq!(s.attention_items(Some("person/lichen")).unwrap().len(), 1);
    s.status(None).unwrap();
    s.append_claim(&ClaimInput {
        subject: "custom/legacy/fact".into(),
        kind: "custom.legacy.recorded".into(),
        fields: BTreeMap::from([("any".into(), json!({"open":true}))]),
        ..empty_input()
    })
    .unwrap();
    assert!(s.custom_subject("custom/legacy/fact").unwrap().is_none());
    m.slots.insert(
        "wrong".into(),
        schema::Slot {
            kind: "unknown".into(),
            select: Select::First,
        },
    );
    assert!(
        s.register_custom_kind(&RegistrationRequest {
            manifest: m,
            actor: "agent/garden/seed".into()
        })
        .is_err()
    );
}
#[test]
fn custom_registration_immutable_and_cannot_capture_legacy_prefix() {
    let s = Store::open_memory("alder").unwrap();
    let first = registration(&s);
    assert_eq!(registration(&s), first);
    let mut m = example();
    m.fields.insert(
        "extra".into(),
        Expr::Constant {
            value: json!("new"),
        },
    );
    assert!(
        s.register_custom_kind(&RegistrationRequest {
            manifest: m,
            actor: "agent/garden/seed".into()
        })
        .is_err()
    );
    let s = Store::open_memory("birch").unwrap();
    s.append_claim(&ClaimInput {
        subject: "custom/garden/review/v1/legacy".into(),
        kind: "custom.garden.legacy".into(),
        ..empty_input()
    })
    .unwrap();
    assert!(
        s.register_custom_kind(&RegistrationRequest {
            manifest: example(),
            actor: "agent/garden/seed".into()
        })
        .is_err()
    );
}
#[test]
fn custom_offline_registration_conflicts_and_indexed_reads() {
    let (a, b) = signed_pair();
    registration(&a);
    let mut m = example();
    m.fields.insert(
        "variant".into(),
        Expr::Constant {
            value: json!("second"),
        },
    );
    b.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let id = "custom/garden/review/v1/conflicted";
    request(&a, id);
    signed_sync(&a, &b);
    signed_sync(&b, &a);
    assert_eq!(a.custom_subject(id).unwrap().unwrap()["state"], "conflict");
    assert!(a.attention_items(Some("person/lichen")).unwrap().is_empty());
    assert_eq!(
        a.custom_registrations().unwrap(),
        b.custom_registrations().unwrap()
    );
    for sql in [
        "EXPLAIN QUERY PLAN SELECT body FROM custom_sources WHERE kind='garden.review' AND subject>'a' ORDER BY subject LIMIT 51",
        "EXPLAIN QUERY PLAN SELECT body FROM custom_sources WHERE active=1 AND person='person/lichen' ORDER BY subject",
    ] {
        let plan = a
            .readers
            .get()
            .prepare(/* sqlite-large-value: allow-for-now lv-h842c39f15807 owner=fractal_store::store::custom::tests TODO=trace-statement-and-value-representation */ sql)
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join(" ");
        assert!(plan.contains("SEARCH custom_sources"), "{plan}");
        assert!(!plan.contains("claims"), "{plan}");
    }
}

#[test]
fn custom_projection_row_bounds_and_invalid_replicated_facts_leave_core_healthy() {
    let a = Store::open_memory("alder").unwrap();
    registration(&a);
    request(&a, "custom/garden/review/v1/healthy");
    let bad = "custom/garden/review/v1/malformed";
    request(&a, bad);
    a.connection.batched(|tx| append_claim_tx(tx,&a.origin,bad,"custom.garden.review.v1.answered",Some("person/lichen"),&json!({"fields":{"_registration":registration_hash(tx,bad),"request":a_creation(tx,bad),"selection":17}}),&[],None)).unwrap().unwrap();
    assert_eq!(a.custom_subject(bad).unwrap().unwrap()["state"], "invalid");
    let mut m = example();
    m.version = 2;
    m.subject_prefix = "custom/garden/review/v2/".into();
    m.fields = (0..64)
        .map(|n| {
            (
                format!("detail_{n}"),
                Expr::Field {
                    slot: "request".into(),
                    field: "detail".into(),
                },
            )
        })
        .collect();
    a.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let big = "custom/garden/review/v2/large";
    a.append_claim(&ClaimInput {
        subject: big.into(),
        kind: example().creation_kind,
        actor: Some("agent/garden/seed".into()),
        fields: serde_json::from_value(
            json!({"title":"Large read","detail":"x".repeat(8000),"recipient":"person/lichen"}),
        )
        .unwrap(),
        ..empty_input()
    })
    .unwrap();
    assert_eq!(a.custom_subject(big).unwrap().unwrap()["state"], "invalid");
    assert_eq!(a.attention_items(Some("person/lichen")).unwrap().len(), 1);
    a.status(None).unwrap();
    let b = Store::open_memory("birch").unwrap();
    legacy_sync(&a, &b);
    assert_eq!(
        a.custom_subject(bad).unwrap(),
        b.custom_subject(bad).unwrap()
    );
    assert_eq!(
        a.custom_subject(big).unwrap(),
        b.custom_subject(big).unwrap()
    );
    assert_eq!(
        projection_digest::oracle(&a.readers.get()).unwrap(),
        projection_digest::oracle(&b.readers.get()).unwrap()
    );
}
fn registration_hash(tx: &Transaction<'_>, id: &str) -> String {
    super::registration(tx, id).unwrap().unwrap().0
}
fn a_creation(tx: &Transaction<'_>, id: &str) -> String {
    claims(tx, id).unwrap()[0].id.clone()
}

#[test]
fn custom_late_document_recovers_only_its_source_and_cycles_are_invalid() {
    let s = Store::open_memory("alder").unwrap();
    let mut m = example();
    let context: schema::Field =
        serde_json::from_value(json!({"value_type":"string","document":true})).unwrap();
    m.claims
        .get_mut(&m.creation_kind.clone())
        .unwrap()
        .fields
        .insert("context".into(), context);
    let derived = "custom.garden.review.v1.derived";
    m.claims.insert(
        derived.into(),
        schema::ClaimSchema {
            authority: Authority::Owner,
            fields: BTreeMap::new(),
            additional_fields: false,
        },
    );
    m.slots.insert(
        "derived".into(),
        schema::Slot {
            kind: derived.into(),
            select: Select::Last,
        },
    );
    s.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let id = "custom/garden/review/v1/context";
    let content = b"Immutable garden context";
    let hash = hex::encode(Sha256::digest(content));
    let registration = s.custom_registrations().unwrap()[0]["registration"]
        .as_str()
        .unwrap()
        .to_owned();
    s.connection.batched(|tx|append_claim_tx(tx,&s.origin,id,&example().creation_kind,Some("agent/garden/seed"),&json!({"fields":{"_registration":registration,"title":"Review context","detail":"Read the immutable evidence.","recipient":"person/lichen","context":format!("doc/garden/context@{hash}")}}),&[],None)).unwrap().unwrap();
    assert_eq!(
        s.custom_subject(id).unwrap().unwrap()["state"],
        "pending-dependencies"
    );
    assert!(s.attention_items(Some("person/lichen")).unwrap().is_empty());
    s.put_document_as(
        "doc/garden/context",
        content,
        &None,
        "garden-context-001",
        Some("agent/garden/seed"),
    )
    .unwrap();
    assert_eq!(s.custom_subject(id).unwrap().unwrap()["state"], "ready");
    assert_eq!(s.attention_items(Some("person/lichen")).unwrap().len(), 1);
    let a = "custom/garden/review/v1/cycle-a";
    let b = "custom/garden/review/v1/cycle-b";
    request(&s, a);
    request(&s, b);
    let kinds = vec![example().creation_kind];
    for (source, dependency) in [(a, b), (b, a)] {
        let basis = s.custom_basis_revision(dependency, &kinds).unwrap();
        s.append_claim(&ClaimInput {
            subject: source.into(),
            kind: derived.into(),
            actor: Some("agent/garden/seed".into()),
            fields: serde_json::from_value(
                json!({"_basis":[{"subject":dependency,"kinds":kinds,"revision":basis}]}),
            )
            .unwrap(),
            ..empty_input()
        })
        .unwrap();
    }
    assert_eq!(s.custom_subject(a).unwrap().unwrap()["state"], "invalid");
    assert_eq!(s.custom_subject(b).unwrap().unwrap()["state"], "invalid");
    let before = projection_digest::oracle(&s.readers.get()).unwrap();
    s.connection
        .batched(replay_graph_from_nothing_tx)
        .unwrap()
        .unwrap();
    assert_eq!(before, projection_digest::oracle(&s.readers.get()).unwrap());
}


#[test]
fn notice_reserved_roots_and_legacy_attention_preserve_unrelated_items() {
    let store = Store::open_memory("example").unwrap();
    registration(&store);
    let ordinary = "custom/garden/review/v1/ordinary";
    request(&store, ordinary);
    store.connection.batched(|tx| -> Result<()> {
        for subject in ["custom", "custom/attention", "custom/attention/source",
            "custom/attention/snapshot", "custom/attention/control", "custom/attention/source/legacy"] {
            tx.execute(/* sqlite-large-value: allow-for-now lv-he9d908465c99 owner=fractal_store::store::custom::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO custom_sources SELECT ?1,registration,kind,revision,state,
                json_set(body,'$.id',?1),person,episode,active,requested_at FROM custom_sources WHERE subject=?2",
                params![subject, ordinary])?;
        }
        Ok(())
    }).unwrap().unwrap();
    for kind in [None, Some("garden.review")] {
        for version in [None, Some(1)] {
            let page = store.custom_subjects(kind, version, None, 1).unwrap();
            assert_eq!(page.len(), 1);
            assert_eq!(page[0]["id"], ordinary);
        }
    }
    for person in [None, Some("person/lichen")] {
        let items = store.custom_attention_items(person).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].subject, ordinary);
    }
}
