use super::*;

fn append(store: &Store, subject: &str, kind: &str) -> String {
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let claim = append_claim_tx(&transaction, &store.origin, subject, kind, None,
        &json!({"fields":{"status":"running","incarnation_id":"incarnation/test"}}), &[], None).unwrap();
    transaction.commit().unwrap();
    claim.id
}

fn changed(receiver: &mut tokio::sync::watch::Receiver<u64>, previous: u64) -> u64 {
    assert!(receiver.has_changed().unwrap());
    let next = *receiver.borrow_and_update();
    assert!(next > previous);
    next
}

#[test]
fn mail_inputs_fence_a_capture_snapshot_when_a_source_commit_races_its_read() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("store.db"), "local").unwrap();
    let mut receiver = store.subscribe_mail_objective_inputs();
    let generation = *receiver.borrow_and_update();
    store.read_snapshot(|_| {
        let connection = store.readers.get();
        let count = || connection.query_row(
            "SELECT COUNT(*) FROM local_mail_delivery_source_v1", [], |row| row.get::<_, i64>(0),
        ).unwrap();
        assert_eq!(count(), 0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-hf28cb93cb7d0 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mail_delivery_source_v1 VALUES('message/racing','agent/test',1,'{\"sent_clock_valid\":1}')", [],
                ).unwrap();
            }).join().unwrap();
        });
        assert_eq!(count(), 0, "the capture remains on its original SQLite snapshot");
        assert!(*receiver.borrow() > generation, "the retained source cut fences the old read");
        Ok(())
    }).unwrap();
    changed(&mut receiver, generation);
}

#[test]
fn mail_inputs_are_retained_and_ignore_unrelated_claim_and_metadata_commits() {
    let store = Store::open_memory("local").unwrap();
    let mut receiver = store.subscribe_mail_objective_inputs();
    assert!(!store.mail_objective_input_coverage_failed());
    let initial = *receiver.borrow_and_update();
    append(&store, "resource/unrelated", "resource.observed");
    store.seal_local_batches().unwrap();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hc0695b96dfa8 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('unrelated-mail-test','1')", [],
    ).unwrap();
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h4c367a489a4b owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='unrelated-mail-test'", []).unwrap();
    assert_eq!(*receiver.borrow(), initial);
    assert!(!receiver.has_changed().unwrap());
    append(&store, "agent/test", "runtime.observed");
    let runtime = changed(&mut receiver, initial);
    drop(receiver);
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-haaf360d578b3 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mail_acceptance_frontier_v1 VALUES('agent/test','incarnation/test',10,10)", [],
    ).unwrap();
    let receiver = store.subscribe_mail_objective_inputs();
    assert!(*receiver.borrow() > runtime, "generation survives zero subscribers");
}

#[test]
fn mail_inputs_cover_ledger_eligibility_ownership_and_mirror_progress() {
    let store = Store::open_memory("local").unwrap();
    let mut receiver = store.subscribe_mail_objective_inputs();
    let mut generation = *receiver.borrow_and_update();
    for sql in [
        /* sqlite-large-value: allow-for-now lv-ha61927ff357e owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mail_delivery_source_v1 VALUES('message/test','agent/test',1,'{\"sent_clock_valid\":1}')",
        /* sqlite-large-value: allow-for-now lv-h6a1d770cf48f owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mail_delivery_source_v1 SET sent_ms=2 WHERE message='message/test'",
        /* sqlite-large-value: allow-for-now lv-h0c9e11245247 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mail_delivery_steps_v1 VALUES('message/test','agent/test','message.staged',3)",
        /* sqlite-large-value: allow-for-now lv-hb2d270b74795 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mail_delivery_steps_v1 SET at_ms=4 WHERE message='message/test'",
        /* sqlite-large-value: allow-for-now lv-h41972030d6bc owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mail_input_acceptance_v1 VALUES('message/test','agent/test','incarnation/test',5,5)",
        /* sqlite-large-value: allow-for-now lv-hd9028a4fdfb3 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mail_acceptance_frontier_v1 VALUES('agent/test','incarnation/test',6,6)",
        /* sqlite-large-value: allow-for-now lv-h72f9c2a38a39 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mail_acceptance_frontier_v1 SET observed_through_unix_ms=7 WHERE recipient='agent/test'",
        /* sqlite-large-value: allow-for-now lv-h498515c3b59f owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mailbox_owners VALUES('agent/test','delivery','incarnation/test',1)",
        /* sqlite-large-value: allow-for-now lv-h0fed5f3c2be6 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mailbox_owners SET epoch=2 WHERE subject='agent/test'",
        /* sqlite-large-value: allow-for-now lv-hc841c12bb3e0 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('mail_delivery_source_v1_stream_through','8') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        /* sqlite-large-value: allow-for-now lv-h965a17c7db30 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('mail_delivery_source_v1_sent_through','9') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        /* sqlite-large-value: allow-for-now lv-h9c9f164c79d4 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mail_delivery_steps_v1 WHERE message='message/test'",
        /* sqlite-large-value: allow-for-now lv-h64ec7d22f790 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mail_input_acceptance_v1 WHERE message='message/test'",
        /* sqlite-large-value: allow-for-now lv-h2cc52039314d owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mailbox_owners WHERE subject='agent/test'",
        /* sqlite-large-value: allow-for-now lv-hdf97e95fb16e owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mail_delivery_source_v1 WHERE message='message/test'",
    ] {
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hebc3b23d0d07 owner=fractal_store::store::observer_mail_inputs_tests TODO=trace-statement-and-value-representation */ sql, []).unwrap();
        generation = changed(&mut receiver, generation);
        assert!(!store.mail_objective_input_coverage_failed(), "{sql}");
    }
}

#[test]
fn mail_inputs_cover_desired_identity_conflicts_health_and_canonical_evidence() {
    let store = Store::open_memory("local").unwrap();
    let runtime = append(&store, "agent/test", "runtime.observed");
    let mut receiver = store.subscribe_mail_objective_inputs();
    let mut generation = *receiver.borrow_and_update();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h69d0a125acf7 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired(subject,kind,revision,claim_id,body,member) VALUES('agent/test','agent','revision/test',?1,'{}','{\"host\":\"local\"}')",
        [&runtime],
    ).unwrap();
    generation = changed(&mut receiver, generation);
    for sql in [
        /* sqlite-large-value: allow-for-now lv-h189f8514bf9c owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET member='{\"host\":\"other\"}' WHERE subject='agent/test'",
        /* sqlite-large-value: allow-for-now lv-h9575cb4f6e83 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO identity_dirty VALUES(1)",
        /* sqlite-large-value: allow-for-now lv-hffbe56fa06b8 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM identity_dirty",
        /* sqlite-large-value: allow-for-now lv-hdf2bdd17d073 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('operation-conflict-naming-name:agent/test','1')",
        /* sqlite-large-value: allow-for-now lv-h6d90ea2b928f owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='operation-conflict-naming-name:agent/test'",
        /* sqlite-large-value: allow-for-now lv-h7221b4a647a3 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_health(aggregate,status,updated_at_unix_ms,error_message) VALUES('projection:naming-operation-conflict','stale','0','{}')",
        /* sqlite-large-value: allow-for-now lv-h2e179523819d owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM projection_health WHERE aggregate='projection:naming-operation-conflict'",
        /* sqlite-large-value: allow-for-now lv-h99d0aeb2aba7 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
        /* sqlite-large-value: allow-for-now lv-h769f42ef423b owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='healthy' WHERE aggregate='graph'",
    ] {
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-he55112492540 owner=fractal_store::store::observer_mail_inputs_tests TODO=trace-statement-and-value-representation */ sql, []).unwrap();
        generation = changed(&mut receiver, generation);
    }
    // Healthy graph watermark writes are ordinary claim bookkeeping, not new mail evidence.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h0e833f2a5731 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET last_good_store_index=last_good_store_index+1 WHERE aggregate='graph'", [],
    ).unwrap();
    assert_eq!(*receiver.borrow(), generation);
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h730ea9419561 owner=fractal_store::store::observer_mail_inputs_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,updated_at_unix_ms) VALUES('record/test','local',1,'digest/test',0,X'','valid',?1,'0')",
        [&runtime],
    ).unwrap();
    generation = changed(&mut receiver, generation);
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h52aa59f90e6e owner=fractal_store::store::observer_mail_inputs_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='invalid' WHERE record_ref='record/test'", [],
    ).unwrap();
    generation = changed(&mut receiver, generation);
    // Unknown naming records fence even undeclared spellings, without an accepted claim row.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h244a66c3a205 owner=fractal_store::store::observer_mail_inputs_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,subject_hint,kind_hint,updated_at_unix_ms) VALUES('record/naming','local',2,'digest/naming',0,X'','unknown','agent/alias','agent.renamed','0')", [],
    ).unwrap();
    generation = changed(&mut receiver, generation);
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h5086674ad23b owner=fractal_store::store::observer_mail_inputs_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=99 WHERE id=(SELECT batch_id FROM claims WHERE id=?1)", [&runtime]).unwrap();
    generation = changed(&mut receiver, generation);
    store.fractal.forget_views();
    changed(&mut receiver, generation);
}

#[test]
fn mail_inputs_publish_only_at_writer_resolution_and_fence_rollback_conservatively() {
    let store = Store::open_memory("local").unwrap();
    let mut receiver = store.subscribe_mail_objective_inputs();
    let generation = *receiver.borrow_and_update();
    {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        transaction.execute(/* sqlite-large-value: allow-for-now lv-he8bd389cf760 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mail_acceptance_frontier_v1 VALUES('agent/test','incarnation/test',1,1)", []).unwrap();
        assert_eq!(*receiver.borrow(), generation, "uncommitted rows never publish a source cut");
        transaction.rollback().unwrap();
    }
    // Native hooks deliberately permit a conservative wake for a rolled-back write; they
    // never certify that its speculative row exists. The capture reads committed authority.
    changed(&mut receiver, generation);
    let count: i64 = store.readers.get().query_row("SELECT COUNT(*) FROM local_mail_acceptance_frontier_v1", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
    assert!(!store.mail_objective_input_coverage_failed());
}

#[test]
fn mail_inputs_reject_missing_physical_and_derived_coverage_until_verified_recovery() {
    let store = Store::open_memory("local").unwrap();
    let mut receiver = store.subscribe_mail_objective_inputs();
    let mut generation = *receiver.borrow_and_update();
    let original: String = store.readers.get().query_row(
        "SELECT sql FROM sqlite_schema WHERE name='local_mail_acceptance_frontier_v1'", [], |row| row.get(0),
    ).unwrap();
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-hd36954a83773 owner=fractal_store::store::observer_mail_inputs_tests TODO=chunk-or-reference-variable-columns */ "DROP TABLE local_mail_acceptance_frontier_v1; CREATE TABLE local_mail_acceptance_frontier_v1(recipient TEXT,incarnation TEXT,observed_through_unix_ms INTEGER,recorded_at_unix_ms INTEGER,PRIMARY KEY(recipient,incarnation)) WITHOUT ROWID;",
    ).unwrap();
    generation = changed(&mut receiver, generation);
    assert!(store.mail_objective_input_coverage_failed());
    store.connection.write().execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-h59773a9a1440 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE local_mail_acceptance_frontier_v1; {original}")).unwrap();
    generation = changed(&mut receiver, generation);
    assert!(!store.mail_objective_input_coverage_failed());
    let trigger: String = store.readers.get().query_row(
        "SELECT sql FROM sqlite_schema WHERE name='mail_delivery_pending_accept_v1'", [], |row| row.get(0),
    ).unwrap();
    store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h610027cb7039 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER mail_delivery_pending_accept_v1").unwrap();
    generation = changed(&mut receiver, generation);
    assert!(store.mail_objective_input_coverage_failed());
    append(&store, "resource/unrelated", "resource.observed");
    assert!(store.mail_objective_input_coverage_failed(), "ordinary commits cannot recertify missing coverage");
    store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h53adbce7cf50 owner=fractal_store::store::observer_mail_inputs_tests TODO=trace-statement-and-value-representation */ &trigger).unwrap();
    changed(&mut receiver, generation);
    assert!(!store.mail_objective_input_coverage_failed());
}

#[test]
fn mail_inputs_fence_shard_position_before_mirroring_and_propagate_shard_failure() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("store.db"), "local").unwrap());
    let mut receiver = store.subscribe_mail_objective_inputs();
    let mut generation = *receiver.borrow_and_update();
    let shard = messages_shard::open(&store, directory.path()).unwrap();
    generation = changed(&mut receiver, generation);
    shard.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h36d667effbbc owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO stream_messages(subject,recipient,sender,store_index) VALUES('message/test','agent/test','agent/sender',1)", [],
    ).unwrap();
    generation = changed(&mut receiver, generation);
    shard.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h448995205b4b owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO stream_lifecycle VALUES('message/test','message.staged','agent/test','{}',1)", [],
    ).unwrap();
    generation = changed(&mut receiver, generation);
    let position = messages_shard::mailbox_reads::position(&store).unwrap();
    assert_eq!(position.admitted, 1);
    assert_eq!(position.lifecycle, 1);
    shard.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h5652883e06c7 owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('unrelated-mail-test','1')", []).unwrap();
    assert_eq!(*receiver.borrow(), generation);
    shard.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-he9cd28d1fdec owner=fractal_store::store::observer_mail_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE stream_lifecycle").unwrap();
    changed(&mut receiver, generation);
    assert!(store.mail_objective_input_coverage_failed());
}
