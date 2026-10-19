#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::{FLEET, advertise, claim, declare, enable, store};
use super::*;
use crate::fleet::MemberKey;
use rusqlite::StatementStatus;
use st3_store::store::directive_notes::tests_shared_delivery::*;
use st3_store::store::directive_notes::tests_shared_delivery::{sync};
use st3_store::store::fleet_admission_tests_shared_delivery::*;
use st3_store::store::fleet_admission_tests_shared_delivery::{fleet};


#[test]
fn directive_note_lifecycle_author_time_bounds_expiry_and_tombstones() {
    let store = store();
    assert!(store.directive_notes("person/avery").unwrap().is_empty());
    let first = store.set_directive_note("person/avery", "person/avery", Some("Prioritize the release"), None).unwrap().unwrap();
    assert_eq!(first.person, "person/avery");
    assert_eq!(first.author, "person/avery");
    let record = store.claim_by_id(&first.revision).unwrap().unwrap();
    assert_eq!(chrono::DateTime::parse_from_rfc3339(&first.time).unwrap().timestamp_millis() as u128, record.accepted_at_unix_ms);
    assert_eq!(store.directive_notes("person/avery").unwrap(), vec![first.clone()]);
    let replacement = store.set_directive_note("person/avery", "person/avery", Some(&"é".repeat(2048)), Some("2099-01-01T00:00:00+00:00")).unwrap().unwrap();
    assert_ne!(replacement.revision, first.revision);
    assert_eq!(replacement.text.len(), 4096);
    for text in [" \n\t".to_owned(), "é".repeat(2049)] {
        assert!(store.set_directive_note("person/avery", "person/avery", Some(&text), None).is_err());
    }
    for at in ["2099-02-30T00:00:00Z", "2099-01-01T00:00:00+01:00"] {
        assert!(store.set_directive_note("person/avery", "person/avery", Some("Context"), Some(at)).is_err());
    }
    assert_eq!(store.directive_notes("person/avery").unwrap(), vec![replacement]);
    assert!(store.set_directive_note("person/avery", "person/avery", None, None).unwrap().is_none());
    assert!(store.directive_notes("person/avery").unwrap().is_empty());
    let expiring = store.set_directive_note("person/avery", "person/avery", Some("Temporary focus"), Some("2099-01-01T00:00:00.500Z")).unwrap().unwrap();
    let expiry = st3_schema::directive_notes::expiry(expiring.expires_at.as_deref().unwrap()).unwrap().timestamp_millis() as u128;
    assert!(current(&store.readers.get(), "person/avery", expiry - 1).unwrap().is_some());
    assert!(current(&store.readers.get(), "person/avery", expiry).unwrap().is_none());
    assert!(current(&store.readers.get(), "person/avery", u128::MAX).unwrap().is_none());
    let precise = store.set_directive_note("person/avery", "person/avery", Some("Short focus"), Some("2099-01-01T00:00:00.000500Z")).unwrap().unwrap();
    let floor = st3_schema::directive_notes::expiry(precise.expires_at.as_deref().unwrap()).unwrap().timestamp_millis() as u128;
    assert!(current(&store.readers.get(), "person/avery", floor).unwrap().is_some());
    assert!(current(&store.readers.get(), "person/avery", floor + 1).unwrap().is_none());
    assert!(store.set_directive_note("person/avery", "person/avery", Some("Already expired"), Some("2000-01-01T00:00:00Z")).unwrap().is_none());
    assert!(store.directive_notes("person/avery").unwrap().is_empty());
    let rows: usize = store.readers.get().query_row("SELECT COUNT(*) FROM person_directive_notes WHERE person='person/avery'", [], |row| row.get(0)).unwrap();
    assert_eq!(rows, 1, "clear and expired revisions must remain current tombstones");
    store.replay_replication_graph().unwrap();
    assert!(store.directive_notes("person/avery").unwrap().is_empty());
}

#[test]
fn directive_note_local_and_replicated_admission_require_subject_person() {
    let store = store();
    for actor in ["person/robin", "agent/worker"] {
        for text in [Some("Approved"), None] {
            assert_eq!(store.set_directive_note("person/avery", actor, text, None).unwrap_err().code, "directive-note-forbidden");
        }
    }
    let note = store.set_directive_note("person/avery", "person/avery", Some("Approved"), None).unwrap().unwrap();
    let original = store.claim_by_id(&note.revision).unwrap().unwrap();
    for actor in [None, Some("person/robin"), Some("agent/worker")] {
        let mut record = original.clone();
        record.actor = actor.map(str::to_owned);
        assert_eq!(classify_replicated_claim_with_registry(&record, st3_schema::registry()).err().unwrap().code, "directive-note-forbidden");
    }
    for text in [json!(""), json!("é".repeat(2049)), json!(42)] {
        let mut record = original.clone();
        record.body["fields"]["text"] = text;
        assert_eq!(classify_replicated_claim_with_registry(&record, st3_schema::registry()).err().unwrap().code, "invalid-replicated-claim");
    }
    let mut record = original.clone();
    record.body["fields"]["expires_at"] = json!("2099-01-01T00:00:00+01:00");
    assert_eq!(classify_replicated_claim_with_registry(&record, st3_schema::registry()).err().unwrap().code, "invalid-replicated-claim");
    assert!(matches!(classify_replicated_claim_with_registry(&original, st3_schema::registry()).unwrap(), ReplicatedClaimAdmission::Valid));
    let mut legacy = st3_schema::registry().clone();
    legacy.claims.remove(KIND);
    assert!(matches!(classify_replicated_claim_with_registry(&original, &legacy).unwrap(), ReplicatedClaimAdmission::UnknownKind));
}

#[test]
fn directive_note_visibility_fails_closed_on_unknown_cycles_and_depth_limit() {
    let store = store();
    store.set_directive_note("person/avery", "person/avery", Some("Context"), None).unwrap();
    declare(&store, "version 2\nagent \"note/cycle-a\" { workspace \"/tmp\"; command \"true\"; }", "agent/note/cycle-b", "cycle-a");
    declare(&store, "version 2\nagent \"note/cycle-b\" { workspace \"/tmp\"; command \"true\"; }", "agent/note/cycle-a", "cycle-b");
    declare(&store, "version 2\nagent \"note/missing\" { workspace \"/tmp\"; command \"true\"; }", "agent/note/absent", "missing");
    for actor in ["agent/note/cycle-a", "agent/note/cycle-b", "agent/note/missing"] {
        assert!(store.directive_notes(actor).unwrap().is_empty());
    }
    for index in (0..17).rev() {
        let author = if index == 16 { "person/avery".into() } else { format!("agent/note/hop-{}", index + 1) };
        declare(&store, &format!("version 2\nagent \"note/hop-{index}\" {{ workspace \"/tmp\"; command \"true\"; }}"), &author, &format!("hop-{index}"));
    }
    assert!(store.directive_notes("agent/note/hop-0").unwrap().is_empty());
    assert_eq!(store.directive_notes("agent/note/hop-1").unwrap().len(), 1);
}

#[test]
fn directive_note_feature_barrier_covers_generic_append_active_peers_and_legacy_hole() {
    let store = Store::open_memory("alder").unwrap();
    let input = ClaimInput { subject: "person/avery".into(), kind: KIND.into(), actor: Some("person/avery".into()),
        fields: BTreeMap::from([("text".into(), json!("Context"))]), evidence: Vec::new(), expected_subject: None, idempotency_key: None };
    assert_eq!(store.append_client_claim(&input).unwrap_err().code, "directive-note-feature-barrier");
    let anchor = enable(&store);
    advertise(&store, false);
    assert_eq!(store.append_claim(&input).unwrap_err().code, "directive-note-feature-barrier");
    advertise(&store, true);
    let peer_key = Arc::new(MemberKey::generate().unwrap().0);
    claim(&store, "host/birch", "fleet.member-admitted", None, json!({"fleet_id":FLEET,"member_key":peer_key.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}));
    assert_eq!(store.append_client_claim(&input).unwrap_err().code, "directive-note-feature-barrier");
    store.replication_snapshot().unwrap();
    assert_eq!(store.append_client_claim(&input).unwrap_err().code, "directive-note-feature-barrier");
    // Another origin cannot advertise on a missing peer's behalf.
    claim(&store, "daemon/birch", "daemon.started", None, json!({"status":"running","features":{"person_directive_note":1}}));
    assert_eq!(store.append_client_claim(&input).unwrap_err().code, "directive-note-feature-barrier");
    let peer = Store::open_memory("birch").unwrap();
    peer.bind_fleet(FLEET).unwrap();
    peer.pin_fleet_anchor(anchor.public()).unwrap();
    peer.set_member_key(Some(peer_key.clone())).unwrap();
    sync(&store, &peer);
    advertise(&peer, true);
    sync(&peer, &store);
    store.append_client_claim(&input).unwrap();
    advertise(&peer, false);
    sync(&peer, &store);
    assert_eq!(store.set_directive_note("person/avery", "person/avery", None, None).unwrap_err().code, "directive-note-feature-barrier");
    let high_water = writer_high_water(&peer.readers.get(), "birch").unwrap().unwrap();
    claim(&store, "host/birch", "fleet.member-removed", None, json!({"member_key":peer_key.public(),"high_water":high_water,"reason":"Unsupported peer fenced"}));
    store.replication_snapshot().unwrap();
    store.append_client_claim(&input).unwrap();
    // Membership advertisements cannot account for a legacy writer outside the membership fold.
    let legacy = Store::open_memory("legacy").unwrap();
    legacy.bind_fleet(FLEET).unwrap();
    advertise(&legacy, true);
    sync(&legacy, &store);
    let refusal = store.append_client_claim(&input).unwrap_err();
    assert_eq!(refusal.code, "directive-note-feature-barrier");
    assert!(refusal.message.contains("legacy") && refusal.message.contains("unfenced"));
    let legacy_high_water = writer_high_water(&legacy.readers.get(), "legacy").unwrap().unwrap();
    claim(&store, "host/legacy", "fleet.member-removed", None, json!({"high_water":legacy_high_water,"reason":"Legacy replication fenced"}));
    store.replication_snapshot().unwrap();
    store.append_client_claim(&input).unwrap();
    assert!(matches!(store.fleet_membership().unwrap().window("legacy", legacy_high_water + 1), crate::fleet::Window::Fenced));
}

#[test]
fn directive_note_replication_out_of_order_clear_replay_and_reopen_agree() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.db");
    let source = Store::open_memory("alder").unwrap();
    let anchor = enable(&source);
    let peer_key = Arc::new(MemberKey::generate().unwrap().0);
    claim(&source, "host/birch", "fleet.member-admitted", None, json!({"fleet_id":FLEET,"member_key":peer_key.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}));
    let peer = Store::open_memory("birch").unwrap();
    peer.bind_fleet(FLEET).unwrap();
    peer.pin_fleet_anchor(anchor.public()).unwrap();
    peer.set_member_key(Some(peer_key)).unwrap();
    sync(&source, &peer);
    advertise(&peer, true);
    sync(&peer, &source);
    let set = source.set_directive_note("person/avery", "person/avery", Some("Old context"), None).unwrap().unwrap();
    peer.set_directive_note("person/avery", "person/avery", None, None).unwrap();
    let target = Store::open(&path, "cedar").unwrap();
    target.bind_fleet(FLEET).unwrap();
    target.pin_fleet_anchor(anchor.public()).unwrap();
    sync(&peer, &target); // Clear arrives before the older set.
    assert!(target.directive_notes("person/avery").unwrap().is_empty());
    sync(&source, &target);
    assert!(target.claim_by_id(&set.revision).unwrap().is_some());
    assert!(target.directive_notes("person/avery").unwrap().is_empty());
    let expected = target.readers.get().query_row("SELECT claim_id,head_key FROM person_directive_notes WHERE person='person/avery'", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))).unwrap();
    target.replay_replication_graph().unwrap();
    assert_eq!(target.readers.get().query_row("SELECT claim_id,head_key FROM person_directive_notes WHERE person='person/avery'", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))).unwrap(), expected);
    sync(&peer, &source);
    let replacement = source.set_directive_note("person/avery", "person/avery", Some("New context"), None).unwrap().unwrap();
    sync(&source, &target);
    assert_eq!(target.directive_notes("person/avery").unwrap(), vec![replacement.clone()]);
    drop(target);
    let reopened = Store::open(&path, "cedar").unwrap();
    assert_eq!(reopened.directive_notes("person/avery").unwrap(), vec![replacement]);
    reopened.rebuild_claim_projections().unwrap();
    assert_eq!(reopened.directive_notes("person/avery").unwrap()[0].text, "New context");
}

#[test]
fn directive_note_repaired_original_is_excluded_after_round_trip_and_replay() {
    let source = Store::open_memory("alder").unwrap();
    let anchor = enable(&source);
    let peer_key = Arc::new(MemberKey::generate().unwrap().0);
    claim(&source, "host/birch", "fleet.member-admitted", None, json!({"fleet_id":FLEET,"member_key":peer_key.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}));
    let peer = Store::open_memory("birch").unwrap();
    peer.bind_fleet(FLEET).unwrap();
    peer.pin_fleet_anchor(anchor.public()).unwrap();
    peer.set_member_key(Some(peer_key)).unwrap();
    sync(&source, &peer);
    advertise(&peer, true);
    sync(&peer, &source);
    let fallback = source.set_directive_note("person/avery", "person/avery", Some("Retained context"), None).unwrap().unwrap();
    let original = source.set_directive_note("person/avery", "person/avery", Some("Rejected context"), None).unwrap().unwrap();
    sync(&source, &peer);
    assert_eq!(peer.directive_notes("person/avery").unwrap(), vec![original.clone()]);
    // An upgraded receiver rejects an already-admitted record, then repairs it to an
    // older valid claim. Other peers retain the original row when the repair arrives.
    let record_ref: String = peer.readers.get().query_row(
        "SELECT record_ref FROM replica_records WHERE claim_id=?1",
        [&original.revision], |row| row.get(0),
    ).unwrap();
    peer.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-ha3a62ff8fd3d owner=st3_delivery::store::directive_notes::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='invalid' WHERE record_ref=?1", [&record_ref])?;
        Ok(())
    }).unwrap().unwrap();
    peer.repair_replica_record(&record_ref, &fallback.revision,
        "Receiver rejects the original after an upgrade", "person/operator", "repair-note").unwrap();
    sync(&peer, &source);
    for node in [&source, &peer] {
        assert!(node.claim_by_id(&original.revision).unwrap().is_some());
        assert_eq!(node.replica_record(&record_ref).unwrap().unwrap().state, "repaired");
        assert_eq!(node.directive_notes("person/avery").unwrap(), vec![fallback.clone()]);
        node.rebuild_claim_projections().unwrap();
        assert_eq!(node.directive_notes("person/avery").unwrap(), vec![fallback.clone()]);
        node.replay_replication_graph().unwrap();
        assert_eq!(node.directive_notes("person/avery").unwrap(), vec![fallback.clone()]);
    }
    // Exercise the incremental dirty selector without the repair-triggered full replay.
    source.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h86fc8255624d owner=st3_delivery::store::directive_notes::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='valid',replacement_claim_id=NULL WHERE record_ref=?1", [&record_ref])?;
        flush(tx)?;
        assert_eq!(current(tx, "person/avery", now_ms()).unwrap(), Some(original.clone()));
        tx.execute(/* sqlite-large-value: allow-for-now lv-h3046ff7110fa owner=st3_delivery::store::directive_notes::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired',replacement_claim_id=?2 WHERE record_ref=?1",
            params![record_ref, fallback.revision])?;
        flush(tx)?;
        assert_eq!(current(tx, "person/avery", now_ms()).unwrap(), Some(fallback.clone()));
        tx.execute(/* sqlite-large-value: allow-for-now lv-h4d5006271587 owner=st3_delivery::store::directive_notes::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET replacement_claim_id=NULL WHERE record_ref=?1", [&record_ref])?;
        let dirty: usize = tx.query_row("SELECT COUNT(*) FROM local_directive_note_dirty WHERE subject='person/avery'", [], |row| row.get(0))?;
        assert_eq!(dirty, 1, "replacement-only changes must invalidate the person's note");
        tx.execute(/* sqlite-large-value: allow-for-now lv-h4d33e3b464fe owner=st3_delivery::store::directive_notes::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET replacement_claim_id=?2 WHERE record_ref=?1",
            params![record_ref, fallback.revision])?;
        flush(tx)?;
        assert_eq!(current(tx, "person/avery", now_ms()).unwrap(), Some(fallback.clone()));
        Ok(())
    }).unwrap().unwrap();
}

#[test]
fn directive_note_projection_rollback_and_canonical_metadata_corrections() {
    let store = store();
    let first = store.set_directive_note("person/avery", "person/avery", Some("First"), None).unwrap().unwrap();
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        append_claim_tx(&tx, "alder", "person/avery", KIND, Some("person/avery"), &json!({"fields":{"text":"Rolled back"}}), &[], None).unwrap();
        assert_eq!(current(&tx, "person/avery", now_ms()).unwrap().unwrap().text, "Rolled back");
    }
    assert_eq!(store.directive_notes("person/avery").unwrap(), vec![first.clone()]);
    let second = store.set_directive_note("person/avery", "person/avery", Some("Second"), None).unwrap().unwrap();
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h032e0c3fe4b3 owner=st3_delivery::store::directive_notes::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=accepted_at_unix_ms+100000 WHERE id=?1", [&first.revision])?;
        flush(tx)?;
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(store.directive_notes("person/avery").unwrap()[0].revision, first.revision);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-hbee5a86c2688 owner=st3_delivery::store::directive_notes::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&first.revision])?;
        flush(tx)?;
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(store.directive_notes("person/avery").unwrap()[0].revision, second.revision);
}

#[test]
fn directive_note_keyed_read_work_and_statements_ignore_history_size() {
    let store = store();
    let mut costs = Vec::new();
    for count in [16, 256] {
        for index in 0..count {
            store.set_directive_note("person/avery", "person/avery", Some(&format!("Context {count}/{index}")), None).unwrap();
            store.set_directive_note("person/other", "person/other", Some("Unrelated context"), None).unwrap();
        }
        store.directive_notes("person/avery").unwrap(); // Warm the pooled connection.
        let before = STATEMENTS_RUN.with(std::cell::Cell::get);
        assert_eq!(store.directive_notes("person/avery").unwrap().len(), 1);
        let statements = STATEMENTS_RUN.with(std::cell::Cell::get) - before;
        let connection = store.readers.get();
        let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-h8f1c8289710b owner=st3_delivery::store::directive_notes::tests TODO=trace-statement-and-value-representation */ CURRENT_QUERY).unwrap();
        query.query_row(/* sqlite-large-value: allow-for-now lv-h0c2d6d7a8196 owner=st3_delivery::store::directive_notes::tests TODO=trace-statement-and-value-representation */ ["person/avery"], claim_from_row).unwrap();
        let vm = query.get_status(StatementStatus::VmStep);
        assert_eq!(query.get_status(StatementStatus::FullscanStep), 0);
        assert!(vm < 100, "one person and one claim lookup must stay keyed: {vm}");
        costs.push((statements, vm));
    }
    assert_eq!(costs[0], costs[1]);
}

#[test]
fn directive_note_signed_wire_admission_rejects_wrong_writer_bounds_and_expiry() {
    for mutation in ["wrong-person", "agent", "missing-actor", "blank", "oversized", "non-utc"] {
        let source = Store::open_memory("alder").unwrap();
        let key = enable(&source);
        let target = Store::open_memory("birch").unwrap();
        target.bind_fleet(FLEET).unwrap();
        target.pin_fleet_anchor(key.public()).unwrap();
        sync(&source, &target);
        source.set_directive_note("person/avery", "person/avery", Some("Context"), None).unwrap();
        let mut exchange = source.export_replication_exchange(FLEET, &target.replication_inventory().unwrap()).unwrap();
        exchange.envelopes.retain(|envelope| {
            let payload: ReplicaEnvelopePayload = ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
            payload.batch.claims.iter().any(|claim| claim.kind == KIND)
        });
        assert_eq!(exchange.envelopes.len(), 1);
        let envelope = &mut exchange.envelopes[0];
        let mut payload: ReplicaEnvelopePayload = ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
        let record = &mut payload.batch.claims[0];
        match mutation {
            "wrong-person" => record.actor = Some("person/robin".into()),
            "agent" => record.actor = Some("agent/worker".into()),
            "missing-actor" => record.actor = None,
            "blank" => record.body["fields"]["text"] = json!(" \n\t"),
            "oversized" => record.body["fields"]["text"] = json!("é".repeat(2049)),
            "non-utc" => record.body["fields"]["expires_at"] = json!("2099-01-01T00:00:00+01:00"),
            _ => unreachable!(),
        }
        record.id = claim_hash(&record.batch_id, &record.subject, &record.kind, &record.origin,
            record.actor.as_deref(), &record.body, &record.predecessors).unwrap();
        let id = record.id.clone();
        let mut bytes = Vec::new();
        ciborium::into_writer(&payload, &mut bytes).unwrap();
        envelope.hash = replica_envelope_hash(&envelope.writer, envelope.sequence,
            envelope.previous_hash.as_deref(), envelope.accepted_at_unix_ms, &bytes);
        envelope.payload = bytes.into();
        envelope.member_key = Some(key.public().into());
        envelope.signature = Some(key.sign(&crate::fleet::envelope_signature_message(
            FLEET, &envelope.writer, envelope.sequence, &envelope.hash)));
        exchange.inventory = ReplicationInventory {
            envelopes: vec![ReplicaEnvelopeId { writer: envelope.writer.clone(), sequence: envelope.sequence, hash: envelope.hash.clone() }],
            ..ReplicationInventory::default()
        };
        target.receive_replication_exchange("alder", FLEET, &exchange).unwrap();
        let admission = target.validate_replication_backlog().unwrap();
        assert_eq!(admission.invalid, 1, "{mutation}: a validly signed, rehashed record must fail semantic admission");
        assert_eq!(admission.valid, 0);
        assert!(target.project_replication_backlog().unwrap());
        assert!(target.claim_by_id(&id).unwrap().is_none());
        assert!(target.directive_notes("person/avery").unwrap().is_empty());
    }
}
