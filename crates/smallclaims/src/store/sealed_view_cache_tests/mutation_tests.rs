use super::*;

fn admission_envelope(store: &Store) -> (String, u64, String) {
    store.readers.get().query_row(
        "SELECT e.writer,e.sequence,e.envelope_hash FROM replica_envelopes e
         JOIN claims c ON c.batch_id=e.batch_id WHERE c.kind='fleet.member-admitted'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap()
}

#[test]
fn sealed_view_cache_invalidates_on_claim_subject_update() {
    let (store, _) = fixture(1);
    store.fleet_view_sealed().unwrap();
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-ha3756d1815b0 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET subject='host/different-anchor' WHERE kind='fleet.member-admitted'",
        [],
    ).unwrap(), 1);
    assert!(refolded(&store).members.is_empty());
}

#[test]
fn sealed_view_cache_invalidates_on_claim_id_update() {
    let (store, _) = fixture(1);
    let old_id: String = store.readers.get().query_row(
        "SELECT id FROM claims WHERE kind='fleet.member-admitted'", [], |row| row.get(0),
    ).unwrap();
    assert!(fleet_membership_tx(&store.readers.get()).unwrap().counts(&old_id));
    let first = store.fleet_view_sealed().unwrap();
    let error = store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h4935436a2ebd owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET id='renamed-anchor-admission' WHERE id=?1", [&old_id],
    ).unwrap_err();
    assert!(error.to_string().contains("claim source identity is immutable"));
    assert_eq!(first, store.fleet_view_sealed().unwrap());
    // A legitimate ID-column UPDATE still exercises the Store's cache trigger.
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hbe0e76e2c510 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET id=id WHERE id=?1", [&old_id],
    ).unwrap(), 1);
    assert_eq!(refolded(&store), first);

    // Store protects persisted source identities. Exercise an actual identity
    // change separately on the real core schema and generation triggers, copying
    // admitted fixture rows rather than bypassing any Store protection.
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hd9f612610d78 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=trace-statement-and-value-representation */ SCHEMA).unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    initialize_fleet_generation(&connection).unwrap();
    {
        let reader = store.readers.get();
        for table in ["meta", "batches", "claims", "replica_envelopes", "replica_envelope_signatures"] {
            let mut source = reader.prepare(/* sqlite-large-value: allow-for-now lv-hb77719269e65 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=trace-statement-and-value-representation */ &format!("SELECT * FROM {table}")).unwrap();
            let column_count = source.column_count();
            let placeholders = vec!["?"; column_count].join(",");
            let mut insert = connection.prepare(
                &format!(/* sqlite-large-value: allow-for-now lv-hfe1d9e9e6d19 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO {table} VALUES({placeholders})"),
            ).unwrap();
            let rows = source.query_map([], |row| {
                (0..column_count).map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            }).unwrap();
            for row in rows {
                insert.execute(/* sqlite-large-value: allow-for-now lv-h61152c78e9ef owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=trace-statement-and-value-representation */ rusqlite::params_from_iter(row.unwrap())).unwrap();
            }
        }
    }
    let generation_before = fleet_generation(&connection).unwrap();
    assert!(fleet_membership_tx(&connection).unwrap().counts(&old_id));
    assert_eq!(connection.execute(
        /* sqlite-large-value: allow-for-now lv-hc46b9fec4615 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET id='renamed-anchor-admission' WHERE id=?1", [&old_id],
    ).unwrap(), 1);
    assert!(fleet_generation(&connection).unwrap() > generation_before);
    let membership = fleet_membership_tx(&connection).unwrap();
    assert!(!membership.counts(&old_id));
    assert!(membership.counts("renamed-anchor-admission"));
    // FleetView intentionally omits claim identities.
    assert_eq!(FleetView::from_membership(&membership), oracle(&store));
}

#[test]
fn sealed_view_cache_invalidates_on_batch_id_update() {
    let (store, _) = fixture(1);
    let batch: String = store.readers.get().query_row(
        "SELECT batch_id FROM claims WHERE kind='fleet.member-endpoints'", [], |row| row.get(0),
    ).unwrap();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h6f9328cfbacb owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,previous_hash,hash,accepted_at_unix_ms)
         SELECT 'replacement-batch','different-writer',replica_sequence,previous_hash,hash,accepted_at_unix_ms
         FROM batches WHERE id=?1", [&batch],
    ).unwrap();
    let first = store.fleet_view_sealed().unwrap();
    assert!(!first.members[0].endpoints.is_empty());
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        tx.execute_batch("PRAGMA defer_foreign_keys=ON;").unwrap();
        // Swap existing parent IDs without touching claims or envelopes: their triggers
        // must not conceal a missing batches.id invalidation. All FKs hold at commit.
        assert_eq!(tx.execute(/* sqlite-large-value: allow-for-now lv-hbaf37261e5a0 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE batches SET id='temporary-batch' WHERE id=?1", [&batch]).unwrap(), 1);
        assert_eq!(tx.execute(/* sqlite-large-value: allow-for-now lv-h42d016d4e93f owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE batches SET id=?1 WHERE id='replacement-batch'", [&batch]).unwrap(), 1);
        assert_eq!(tx.execute(/* sqlite-large-value: allow-for-now lv-h792f6b885157 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE batches SET id='replacement-batch' WHERE id='temporary-batch'", []).unwrap(), 1);
        tx.commit().unwrap();
    }
    let changed = refolded(&store);
    assert_eq!(changed.members.len(), 1);
    assert!(changed.members[0].endpoints.is_empty());
}

#[test]
fn sealed_view_cache_invalidates_on_batch_origin_update() {
    let (store, _) = fixture(1);
    store.fleet_view_sealed().unwrap();
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h4eeb70e425dd owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE batches SET origin='different-writer'
         WHERE id IN (SELECT batch_id FROM claims WHERE kind='fleet.member-admitted')", [],
    ).unwrap(), 1);
    assert!(refolded(&store).members.is_empty());
}

#[test]
fn sealed_view_cache_invalidates_on_batch_replica_sequence_update() {
    let (store, _) = fixture(1);
    store.fleet_view_sealed().unwrap();
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h5f9bab21337c owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=0
         WHERE id IN (SELECT batch_id FROM claims WHERE kind='fleet.member-admitted')", [],
    ).unwrap(), 1);
    assert!(refolded(&store).members.is_empty());
}

#[test]
fn sealed_view_cache_invalidates_on_envelope_hash_update() {
    let (store, _) = fixture(1);
    let (writer, sequence, hash) = admission_envelope(&store);
    store.fleet_view_sealed().unwrap();
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h94e698592810 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET envelope_hash='different-envelope-hash'
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![writer, sequence, hash],
    ).unwrap(), 1);
    assert!(refolded(&store).members.is_empty());
}

fn signature_identity_update(sql: &str) {
    let (store, _) = fixture(1);
    let (writer, sequence, hash) = admission_envelope(&store);
    store.fleet_view_sealed().unwrap();
    assert_eq!(store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h02e0fef789eb owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=trace-statement-and-value-representation */ sql, params![writer, sequence, hash]).unwrap(), 1);
    assert!(refolded(&store).members.is_empty());
}

#[test]
fn sealed_view_cache_invalidates_on_signature_writer_update() {
    signature_identity_update(
        /* sqlite-large-value: allow-for-now lv-hceb7e47091f1 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET writer='different-writer'
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
    );
}

#[test]
fn sealed_view_cache_invalidates_on_signature_sequence_update() {
    signature_identity_update(
        /* sqlite-large-value: allow-for-now lv-hb26521be96c8 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET sequence=0
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
    );
}

#[test]
fn sealed_view_cache_invalidates_on_signature_envelope_hash_update() {
    signature_identity_update(
        /* sqlite-large-value: allow-for-now lv-h4673cf2c0886 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET envelope_hash='different-envelope-hash'
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
    );
}

#[test]
fn sealed_view_cache_invalidates_on_anchor_delete() {
    let (store, _) = fixture(1);
    store.fleet_view_sealed().unwrap();
    assert_eq!(store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-ha03803a4ab00 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='fleet_anchor_key'", []).unwrap(), 1);
    let changed = refolded(&store);
    assert_eq!(changed, FleetView::default());
}

#[test]
fn sealed_view_cache_invalidates_on_anchor_insert() {
    let (store, key) = fixture(1);
    let expected = oracle(&store);
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hbd133317b6ba owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='fleet_anchor_key'", []).unwrap();
    let first = store.fleet_view_sealed().unwrap();
    assert_eq!(first, FleetView::default());
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h4fa6d9897a14 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('fleet_anchor_key',?1)", [&key],
    ).unwrap(), 1);
    assert_eq!(refolded(&store), expected);
}

#[test]
fn sealed_view_cache_reopen_migrates_old_narrow_generation_triggers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let old_claim_trigger;
    let old_envelope_trigger;
    {
        let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
        configure(&store, 1);
        let writer = store.connection.write();
        // Recreate the previous trigger surface on a populated real database. Keep
        // its INSERT/DELETE, signature, and anchor triggers rather than erasing it.
        writer.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h9e081facd1ca owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE fleet_generation SET trigger_version=0 WHERE id=1;
             DROP TRIGGER fleet_generation_claim_update;
             DROP TRIGGER fleet_generation_envelope_update;
             DROP TRIGGER IF EXISTS fleet_generation_batch_insert;
             DROP TRIGGER IF EXISTS fleet_generation_batch_delete;
             DROP TRIGGER IF EXISTS fleet_generation_batch_update;
             CREATE TRIGGER fleet_generation_claim_update
             AFTER UPDATE OF kind,batch_id,body ON claims
             WHEN NEW.kind IN ({FLEET_CLAIM_KINDS}) OR OLD.kind IN ({FLEET_CLAIM_KINDS})
             BEGIN UPDATE fleet_generation SET value=value+1 WHERE id=1; END;
             CREATE TRIGGER fleet_generation_envelope_update
             AFTER UPDATE OF batch_id ON replica_envelopes
             WHEN NEW.batch_id IS NOT NULL AND EXISTS(
                 SELECT 1 FROM claims WHERE batch_id=NEW.batch_id
                 AND kind IN ({FLEET_CLAIM_KINDS}))
             BEGIN UPDATE fleet_generation SET value=value+1 WHERE id=1; END;
             CREATE TRIGGER IF NOT EXISTS fleet_generation_envelope_detach
             AFTER UPDATE OF batch_id ON replica_envelopes
             WHEN OLD.batch_id IS NOT NEW.batch_id AND EXISTS(
                 SELECT 1 FROM claims WHERE batch_id=OLD.batch_id
                 AND kind IN ({FLEET_CLAIM_KINDS}))
             BEGIN UPDATE fleet_generation SET value=value+1 WHERE id=1; END;"
        )).unwrap();
        old_claim_trigger = writer.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='fleet_generation_claim_update'",
            [], |row| row.get::<_, String>(0),
        ).unwrap();
        old_envelope_trigger = writer.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='fleet_generation_envelope_update'",
            [], |row| row.get::<_, String>(0),
        ).unwrap();
    }
    let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
    {
        let reader = store.readers.get();
        let claim_trigger: String = reader.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='fleet_generation_claim_update'",
            [], |row| row.get(0),
        ).unwrap();
        let envelope_trigger: String = reader.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='fleet_generation_envelope_update'",
            [], |row| row.get(0),
        ).unwrap();
        assert_ne!(claim_trigger, old_claim_trigger);
        assert_ne!(envelope_trigger, old_envelope_trigger);
        let batch_triggers: i64 = reader.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger'
             AND name IN ('fleet_generation_batch_insert','fleet_generation_batch_delete','fleet_generation_batch_update')",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(batch_triggers, 3);
        let obsolete_detach: bool = reader.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='fleet_generation_envelope_detach')",
            [], |row| row.get(0),
        ).unwrap();
        assert!(!obsolete_detach);
    }
    let first = store.fleet_view_sealed().unwrap();
    assert_eq!(first.members.len(), 1);
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h60330cf01093 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET subject='host/different-anchor' WHERE kind='fleet.member-admitted'", [],
    ).unwrap();
    let renamed = refolded(&store);
    assert!(renamed.members.is_empty());
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h852431c8435e owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET subject='host/anchor' WHERE kind='fleet.member-admitted'", [],
    ).unwrap();
    let restored = refolded(&store);
    assert_eq!(restored, first);
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hbe8e048efc23 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE batches SET origin='different-writer'
         WHERE id IN (SELECT batch_id FROM claims WHERE kind='fleet.member-endpoints')", [],
    ).unwrap();
    let changed_batch = refolded(&store);
    assert_eq!(changed_batch.members.len(), 1);
    assert!(changed_batch.members[0].endpoints.is_empty());
    let (writer, sequence, hash) = admission_envelope(&store);
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h018f7b59a395 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET envelope_hash='different-envelope-hash'
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![writer, sequence, hash],
    ).unwrap();
    assert!(refolded(&store).members.is_empty());
}

#[test]
fn sealed_view_cache_signature_writer_update_after_batch_origin_mutation() {
    let (store, _) = fixture(1);
    let (writer, sequence, hash) = admission_envelope(&store);
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h5fc5efbdc56c owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE batches SET origin='renamed-anchor'
             WHERE id IN (SELECT batch_id FROM claims WHERE kind='fleet.member-admitted')", [],
        ).unwrap();
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h4a2329426cad owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET subject='host/renamed-anchor' WHERE kind='fleet.member-admitted'", [],
        ).unwrap();
        tx.commit().unwrap();
    }
    let first = store.fleet_view_sealed().unwrap();
    assert_eq!(first, oracle(&store));
    assert!(first.members.is_empty());
    // The fold looks up the batch's writer, not the unchanged envelope's writer.
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-he3af63a63e79 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET writer='renamed-anchor'
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![writer, sequence, hash],
    ).unwrap(), 1);
    let changed = refolded(&store);
    assert_eq!(changed.members.len(), 1);
    assert_eq!(changed.members[0].name, "renamed-anchor");
}

#[test]
fn sealed_view_cache_signature_sequence_update_after_batch_sequence_mutation() {
    let (store, _) = fixture(1);
    let (writer, sequence, hash) = admission_envelope(&store);
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-ha2fa34523595 owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=replica_sequence+1000
         WHERE id IN (SELECT batch_id FROM claims WHERE kind='fleet.member-admitted')", [],
    ).unwrap(), 1);
    let first = store.fleet_view_sealed().unwrap();
    assert_eq!(first, oracle(&store));
    assert!(first.members.is_empty());
    // Signature identity must follow the mutated batch sequence even though the
    // envelope keeps its original sequence.
    assert_eq!(store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h70d93219f92b owner=smallclaims::store::sealed_view_cache_tests::mutation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET sequence=sequence+1000
         WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![writer, sequence, hash],
    ).unwrap(), 1);
    let changed = refolded(&store);
    assert_eq!(changed.members.len(), 1);
    assert_eq!(changed.members[0].name, "anchor");
    assert!(!changed.members[0].endpoints.is_empty());
}
