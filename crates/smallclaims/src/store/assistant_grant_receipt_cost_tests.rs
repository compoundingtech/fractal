// Included by assistant_grant_authority_tests to share its real replication fixtures.

fn receipt_signature_for_claim(store: &Store, exchange: &ReplicationExchange, claim: &ClaimRecord) -> ReplicaEnvelopeSignature {
    signatures(exchange).signatures.into_iter().find(|signature| {
        store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND writer=?2 AND sequence=?3 AND envelope_hash=?4)",
            params![claim.id, signature.writer, signature.sequence, signature.hash],
            |row| row.get::<_, bool>(0),
        ).unwrap()
    }).unwrap()
}

fn fresh_nonmember_proof(envelope: &ReplicaEnvelopeSignature) -> ReplicaEnvelopeSignature {
    let key = MemberKey::generate().unwrap().0;
    let message = crate::fleet::envelope_signature_message_in(
        None, FLEET, &envelope.writer, envelope.sequence, &envelope.hash,
    );
    ReplicaEnvelopeSignature {
        writer: envelope.writer.clone(), sequence: envelope.sequence, hash: envelope.hash.clone(),
        member_key: key.public().to_owned(), signature: key.sign(&message),
    }
}

fn complete_signature_receipt_cost(store: &Store, signature: &ReplicaEnvelopeSignature) -> crate::sqlite::work::SqliteWork {
    // Match receipt's split: cryptographic verification precedes writer admission.
    // Everything from acquiring the writer through BEGIN, store (including all
    // triggers), COMMIT and returning the writer belongs to the measured scope.
    let verified = VerifiedEnvelopeSignature::verify(
        None, FLEET, &signature.writer, signature.sequence, &signature.hash,
        &signature.member_key, &signature.signature,
    ).expect("fresh proof must cryptographically verify");
    let scope = crate::sqlite::work::SqliteWorkScope::start();
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        assert_eq!(verified.store(&transaction, "1").unwrap(), 1);
        transaction.commit().unwrap();
    }
    scope.finish()
}

fn receipt_projection_work_state(connection: &Connection) -> Vec<Vec<Vec<SqlValue>>> {
    [
        "SELECT id,applied_generation FROM assistant_grant_membership_state ORDER BY id",
        "SELECT claim_id,position,record_rowid,member_key FROM assistant_grant_auth_cursor ORDER BY claim_id",
        "SELECT claim_id FROM assistant_grant_dirty ORDER BY claim_id",
        "SELECT writer,sequence,claim_id FROM assistant_grant_membership_dirty ORDER BY writer",
    ].into_iter().map(|sql| {
        let mut statement = connection.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement.query_map([], |row| (0..columns).map(|column| row.get(column)).collect())
            .unwrap().collect::<rusqlite::Result<Vec<Vec<SqlValue>>>>().unwrap()
    }).collect()
}

#[test]
fn fresh_nonmember_proof_receipt_has_constant_complete_transaction_vm_cost() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let fleet_exchange = export(&source, &target);
    receive(&target, &fleet_exchange);
    let anchor = raw_claims(&source).into_iter().find(|claim| claim.kind=="fleet.member-admitted").unwrap();
    let grant = append(&source, SUBJECT, GRANTED, fields());
    let exchange = export(&source, &target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    let envelope = receipt_signature_for_claim(&target, &fleet_exchange, &anchor);
    {
        let connection = target.connection.write();
        // Persist an interrupted authentication checkpoint and its queued claim.
        // A proof from outside the writer's window must neither reset this
        // checkpoint nor consume/recreate its work, even for the same envelope.
        connection.execute(
            "INSERT INTO assistant_grant_auth_cursor SELECT claim_id,position,rowid,?2 FROM replica_records WHERE claim_id=?1 LIMIT 1",
            params![grant.id, key.public()],
        ).unwrap();
        connection.execute("INSERT INTO assistant_grant_dirty VALUES(?1)", [&grant.id]).unwrap();
    }
    let before = receipt_projection_work_state(&target.readers.get());
    assert_eq!(before[1].len(), 1, "checkpoint fixture must be nonempty");
    assert_eq!(before[2].len(), 1, "dirty fixture must be nonempty");
    let initial_signatures: usize = target.readers.get().query_row(
        "SELECT COUNT(*) FROM replica_envelope_signatures WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![envelope.writer,envelope.sequence,envelope.hash], |row| row.get(0),
    ).unwrap();
    let baseline = complete_signature_receipt_cost(&target, &fresh_nonmember_proof(&envelope));
    assert!(baseline.statements >= 3, "must account for BEGIN, insertion and COMMIT");
    assert!(baseline.vm_steps > 0, "SQL VM instrumentation must be active");
    assert_eq!(baseline.fullscan_steps, 0);
    assert_eq!(baseline.sorts, 0);
    assert_eq!(baseline.autoindex_rows, 0);
    let mut retained = initial_signatures+1;
    for count in [128, 1024] {
        for _ in 0..count {
            let proof = fresh_nonmember_proof(&envelope);
            let work = complete_signature_receipt_cost(&target, &proof);
            retained += 1;
            assert_eq!(work, baseline, "complete receipt work grew with {retained} retained fresh proofs");
        }
        let connection = target.readers.get();
        let stored: usize = connection.query_row(
            "SELECT COUNT(*) FROM replica_envelope_signatures WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
            params![envelope.writer, envelope.sequence, envelope.hash], |row| row.get(0),
        ).unwrap();
        assert_eq!(stored, retained, "proofs must be new retained rows, not duplicate no-ops");
        assert_eq!(receipt_projection_work_state(&connection), before,
            "irrelevant proofs changed projection revision, checkpoint or queued work");
    }
}

#[test]
fn sustained_fresh_nonmember_proof_receipts_leave_unrelated_established_credential_live() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let established = append(&source, SUBJECT, GRANTED, fields());
    let fleet_exchange = export(&source, &target);
    receive(&target, &fleet_exchange);
    let anchor = raw_claims(&source).into_iter().find(|claim| claim.kind=="fleet.member-admitted").unwrap();
    let mut pending_fields = fields();
    pending_fields["credential_hash"] = json!("b".repeat(64));
    pending_fields["grant_id"] = json!("assistant-grant/fedcba9876543210fedcba98");
    pending_fields["session_actor"] = json!("assistant/chatgpt/session/assistant-grant/fedcba9876543210fedcba98");
    let pending = append(&source, "custom/client/assistant-v1-fedcba9876543210fedcba98", GRANTED, pending_fields);
    let exchange = export(&source, &target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    let envelope = receipt_signature_for_claim(&target, &fleet_exchange, &anchor);
    let before = receipt_projection_work_state(&target.readers.get());
    for arrival in 0..256 {
        complete_signature_receipt_cost(&target, &fresh_nonmember_proof(&envelope));
        // Deliberately do not advance background maintenance between arrivals.
        // Ongoing irrelevant evidence must not turn a completed grant into a
        // globally pending projection, or authenticate the unsigned grant.
        let connection = target.readers.get();
        assert_eq!(receipt_projection_work_state(&connection), before, "arrival {arrival} queued projection work");
        assert!(!authority::maintenance_pending(&connection).unwrap());
        assert!(authority::contains(&connection, &established.id).unwrap());
        assert_eq!(authority::by_credential(&connection, &"a".repeat(64)).unwrap().unwrap().id, established.id);
        assert!(!authority::contains(&connection, &pending.id).unwrap());
        assert!(authority::by_credential(&connection, &"b".repeat(64)).unwrap().is_none());
    }
}

#[test]
fn membership_window_changes_share_the_fixed_transaction_budget_without_replacing_unchanged_windows() {
    let (store, key) = source();
    drain_authority(&store);
    let stable_rowid: i64 = store.readers.get().query_row(
        "SELECT rowid FROM assistant_grant_membership_windows WHERE writer='anchor' AND member_key=?1",
        [key.public()], |row| row.get(0),
    ).unwrap();
    let extra = authority::CLAIMS_PER_SLICE*2+7;
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        for index in 0..extra {
            transaction.execute(
                "INSERT INTO assistant_grant_membership_windows VALUES('obsolete',?1,0,NULL)",
                [format!("obsolete-{index:08}")],
            ).unwrap();
        }
        transaction.execute(
            "UPDATE meta SET value=value WHERE key='fleet_anchor_key'", [],
        ).unwrap();
        transaction.commit().unwrap();
    }
    assert!(!store.advance_assistant_grant_authority_backfill().unwrap());
    let remaining: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM assistant_grant_membership_windows WHERE writer='obsolete'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(extra-remaining, authority::CLAIMS_PER_SLICE,
        "only the fixed number of changed/deleted windows may be applied");
    let unchanged_rowid: i64 = store.readers.get().query_row(
        "SELECT rowid FROM assistant_grant_membership_windows WHERE writer='anchor' AND member_key=?1",
        [key.public()], |row| row.get(0),
    ).unwrap();
    assert_eq!(unchanged_rowid, stable_rowid, "unchanged windows are never deleted and reinserted");
    assert!(authority::maintenance_pending(&store.readers.get()).unwrap());
    drain_authority(&store);
    let remaining: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM assistant_grant_membership_windows WHERE writer='obsolete'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn unchanged_membership_generation_fold_does_not_enter_the_writer_or_reset_work() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let exchange = export(&source, &target);
    receive(&target, &exchange);
    let anchor = raw_claims(&source).into_iter()
        .find(|claim| claim.kind == "fleet.member-admitted").unwrap();
    let envelope = receipt_signature_for_claim(&target, &exchange, &anchor);
    let before = receipt_projection_work_state(&target.readers.get());
    complete_signature_receipt_cost(&target, &fresh_nonmember_proof(&envelope));
    assert!(target.assistant_grant_authority_maintenance_pending().unwrap());
    let commits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = commits.clone();
    let _observer = target.observe_commits(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    assert!(target.advance_assistant_grant_authority_backfill().unwrap());
    assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 0,
        "an unchanged authoritative window snapshot must not open a writer transaction");
    assert_eq!(receipt_projection_work_state(&target.readers.get()), before);
    assert!(!target.assistant_grant_authority_maintenance_pending().unwrap());
}
