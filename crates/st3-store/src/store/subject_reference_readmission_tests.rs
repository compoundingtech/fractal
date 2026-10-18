use smallclaims::fleet::{MemberKey, envelope_signature_message};

#[test]
fn historical_subject_reference_readmission_uses_normal_signed_admission_once_per_schema() {
    for case in ["ack", "invalid-reference", "malformed", "forged", "unsigned", "untrusted"] {
        let source = Store::open_memory("source").unwrap();
        let key = Arc::new(MemberKey::generate().unwrap().0);
        source.set_member_key(Some(key.clone())).unwrap();
        let claim = source.append_claim(&ClaimInput {
            subject: "message/ack".into(),
            kind: "message.sent".into(),
            actor: Some("agent/example/worker".into()),
            fields: BTreeMap::from([
                ("from".into(), json!("agent/example/worker")),
                ("to".into(), json!("assistant/chatgpt")),
                ("content".into(), json!("ACK")),
                ("status".into(), json!("sent")),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let mut exchange = exchange_from(&source, &ReplicationInventory::default());
        let ack_position = exchange.envelopes.iter().position(|envelope| {
            let payload: ReplicaEnvelopePayload =
                ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
            payload.batch.claims.iter().any(|candidate| candidate.id == claim.id)
        }).unwrap();
        let ack = exchange.envelopes.remove(ack_position);
        let bootstrap_envelopes = std::mem::replace(&mut exchange.envelopes, vec![ack]);
        if matches!(case, "invalid-reference" | "malformed") {
            exchange.envelopes[0] = rewrite_envelope(&exchange.envelopes[0], |payload| {
                let claim = &mut payload.batch.claims[0];
                if case == "invalid-reference" {
                    claim.body["fields"]["to"] = json!("future/unknown");
                } else {
                    claim.body["fields"]["status"] = json!(12);
                }
                claim.id = claim_hash(&claim.batch_id, &claim.subject, &claim.kind,
                    &claim.origin, claim.actor.as_deref(), &claim.body, &claim.predecessors).unwrap();
            });
        }
        exchange.signatures.clear();
        let envelope = &mut exchange.envelopes[0];
        envelope.member_key = Some(key.public().into());
        envelope.signature = if case == "unsigned" { None } else {
            let signing_key = if matches!(case, "forged" | "untrusted") {
                Arc::new(MemberKey::generate().unwrap().0)
            } else { key.clone() };
            if case == "untrusted" {
                envelope.member_key = Some(signing_key.public().into());
            }
            Some(signing_key.sign(&envelope_signature_message(TEST_FLEET,
                &envelope.writer, envelope.sequence, &envelope.hash)))
        };
        let original = envelope.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.sqlite");
        let older = Store::open(&path, "target").unwrap();
        older.bind_fleet(TEST_FLEET).unwrap();
        let mut registry = st3_schema::registry().clone();
        registry.subjects.remove("assistant");
        older.set_claim_registry(registry);
        older.set_member_key(Some(key.clone())).unwrap();
        older.pin_fleet_anchor(key.public()).unwrap();
        older.admit_fleet_anchor(TEST_FLEET, key.public(), "listening").unwrap();
        older.append_claim(&ClaimInput {
            subject: "host/source".into(), kind: "fleet.member-admitted".into(), actor: None,
            fields: BTreeMap::from([
                ("fleet_id".into(), json!(TEST_FLEET)),
                ("member_key".into(), json!(key.public())),
                ("via".into(), json!("invite")), ("mode".into(), json!("listening")),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let mut bootstrap = exchange.clone();
        bootstrap.envelopes = bootstrap_envelopes;
        older.receive_replication_exchange("source", TEST_FLEET, &bootstrap).unwrap();
        let bootstrap_admission = older.validate_replication_backlog().unwrap();
        assert_eq!((bootstrap_admission.invalid, bootstrap_admission.held), (0, 0));
        assert!(matches!(
            older.fleet_membership().unwrap().window("source", original.sequence),
            smallclaims::fleet::Window::Keyed(_)
        ), "{case}: fixture requires keyed source membership");
        let receipt = older.receive_replication_exchange("source", TEST_FLEET, &exchange).unwrap();
        assert_eq!(receipt.signatures, usize::from(!matches!(case, "forged" | "unsigned")), "{case}");
        let before = older.validate_replication_backlog().unwrap();
        if matches!(case, "forged" | "unsigned" | "untrusted") {
            assert_eq!(before.held, 1, "{case}: {before:?}");
            assert_eq!(before.valid, 0, "{case}");
        } else {
            assert_eq!(before.invalid, 1, "{case}: {before:?}");
            let records = older.replica_records(true).unwrap();
            assert_eq!(records.len(), 1, "{case}");
            assert_eq!(records[0].error_code.as_deref(), Some("invalid-replicated-claim"));
            if case != "malformed" {
                assert!(records[0].error_message.as_deref().unwrap().contains("violates invalid-subject-reference:"));
            }
            assert_eq!(older.validate_replication_backlog().unwrap().invalid, 0, "{case}: unchanged schema must not retry");
        }
        let retained = |store: &Store| -> Vec<(String, String, u64, String, Vec<u8>, Option<String>)> {
            store.readers.get().prepare(
                "SELECT record_ref, writer, sequence, envelope_hash, raw, claim_id FROM replica_records ORDER BY record_ref"
            ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
                .unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        let records = retained(&older);
        drop(older);
        let upgraded = Store::open(&path, "target").unwrap();
        let after = upgraded.validate_replication_backlog().unwrap();
        match case {
            "ack" => {
                assert_eq!((after.valid, after.invalid, after.held), (1, 0, 0), "{after:?}");
                upgraded.project_replication_backlog().unwrap();
                let admitted = upgraded.latest_claim(&claim.subject, Some(&claim.kind)).unwrap().unwrap();
                assert_eq!(admitted.id, claim.id);
                assert_eq!(admitted.batch_id, claim.batch_id);
                assert_eq!(admitted.origin, claim.origin);
                assert_eq!(admitted.actor, claim.actor);
                assert_eq!(admitted.body, claim.body);
                assert_eq!(admitted.predecessors, claim.predecessors);
                assert_eq!(admitted.accepted_at_unix_ms, claim.accepted_at_unix_ms);
            }
            "invalid-reference" => assert_eq!((after.valid, after.invalid), (0, 1), "{after:?}"),
            "malformed" => assert_eq!((after.valid, after.invalid), (0, 0), "{after:?}"),
            _ => assert_eq!((after.valid, after.held), (0, 1), "{case}: {after:?}"),
        }
        assert_eq!(retained(&upgraded), records, "{case}: record identity and bytes are immutable");
        let relayed = exchange_from(&upgraded, &ReplicationInventory::default());
        let relayed = relayed.envelopes.iter().find(|envelope| envelope.writer == original.writer
            && envelope.sequence == original.sequence && envelope.hash == original.hash).unwrap();
        assert_eq!(relayed.payload.bytes().unwrap(), original.payload.bytes().unwrap(), "{case}");
        assert_eq!(relayed.previous_hash, original.previous_hash, "{case}");
        assert_eq!(relayed.accepted_at_unix_ms, original.accepted_at_unix_ms, "{case}");
        if !matches!(case, "forged" | "unsigned") {
            assert_eq!(relayed.member_key, original.member_key, "{case}");
            assert_eq!(relayed.signature, original.signature, "{case}");
        }
        for _ in 0..2 {
            let converged = upgraded.validate_replication_backlog().unwrap();
            assert_eq!((converged.valid, converged.invalid), (0, 0), "{case}: {converged:?}");
        }
        drop(upgraded);
        let reopened = Store::open(&path, "target").unwrap();
        let converged = reopened.validate_replication_backlog().unwrap();
        assert_eq!((converged.valid, converged.invalid), (0, 0), "{case}: durable schema marker");
        if case != "ack" {
            assert!(reopened.latest_claim(&claim.subject, Some(&claim.kind)).unwrap().is_none(), "{case}");
        }
    }
}
