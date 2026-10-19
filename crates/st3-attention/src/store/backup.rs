//! The part of `store::backup` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use super::backup_format::{Header, PAGE, Record, RestoreReport, TableDigest, write_record};
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicaEnvelopeSignature};
use std::io::Write;
use std::sync::atomic::Ordering;
use crate::message_body::BodyAttachment as _;
pub use st3_store::store::backup::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup;
    use st3_store::store::backup_tests_shared_attention_tests::{diagnostic};

    fn archive(store: &Store, path: &Path) -> Header {
        store
            .write_backup(&mut fs::File::create(path).unwrap())
            .unwrap()
    }

    #[test]
    fn signed_backup_restores_the_log_documents_and_graph_with_a_new_writer() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open(&root.path().join("source.db"), "alder").unwrap();
        let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
        source.bind_fleet("test-fleet").unwrap();
        source.pin_fleet_anchor(key.public()).unwrap();
        source.set_member_key(Some(key.clone())).unwrap();
        source
            .admit_fleet_anchor("test-fleet", key.public(), "listening")
            .unwrap();
        for index in 0..PAGE + 3 {
            diagnostic(&source, &format!("note-{index}"));
        }
        source
            .put_document(
                "doc/backup-example",
                b"# Example\n\nA durable document.\n",
                &None,
                "backup-document",
            )
            .unwrap();
        let backup_path = root.path().join("graph.jsonl");
        let header = archive(&source, &backup_path);
        let path = root.path().join("restored.db");
        let restored = backup::restore(&backup_path, &path).unwrap();
        assert!(restored.projections_match);
        assert_eq!(restored.graph_digest, header.graph_digest);
        assert!(Store::open(&path, "alder").is_err());
        let target = Store::open(&path, &restored.writer).unwrap();
        assert_eq!(
            source.replication_inventory().unwrap().digest,
            target.replication_inventory().unwrap().digest
        );
        let left = source
            .export_replication_exchange("test-fleet", &ReplicationInventory::default())
            .unwrap();
        let right = target
            .export_replication_exchange("test-fleet", &ReplicationInventory::default())
            .unwrap();
        assert_eq!(
            serde_json::to_value(left.envelopes).unwrap(),
            serde_json::to_value(right.envelopes).unwrap()
        );
        diagnostic(&target, "fresh writer");
        assert_eq!(target.writer_head(&restored.writer).unwrap().unwrap().0, 1);
        drop(target);
        assert!(
            backup::restore(&backup_path, &path)
                .unwrap_err()
                .to_string()
                .contains("already holds claims")
        );
    }

    #[test]
    fn an_upgrade_may_project_more_claims_but_must_preserve_the_exact_log() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        diagnostic(&source, "newly readable after upgrade");
        let path = root.path().join("older-registry.jsonl");
        let header = archive(&source, &path);
        // Model an old reader's header: it held this envelope but did not yet know its kind.
        let old_sources = Store::open_memory("empty")
            .unwrap()
            .backup_header()
            .unwrap()
            .tables["claim_sources"]
            .clone();
        rewrite(&path, |records| {
            if let Record::Header(header) = &mut records[0] {
                header.registry_digest = "older-registry".into();
                header.tables.insert("claim_sources".into(), old_sources);
                header.graph_digest = "older-projections".into();
            }
        });
        let report = backup::restore(&path, &root.path().join("upgraded.db")).unwrap();
        assert_eq!(report.log_digest, header.log_digest);
        assert!(!report.projections_match);
        assert_eq!(report.tables["claim_sources"].count, 1);
        rewrite(&path, |records| {
            records.retain(|record| !matches!(record, Record::Envelope(_)));
        });
        assert!(
            backup::restore(&path, &root.path().join("missing.db"))
                .unwrap_err()
                .to_string()
                .contains("log digest")
        );
    }

    #[test]
    fn restore_replaces_an_offline_empty_database_and_can_backup_it_again() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        diagnostic(&source, "note");
        let path = root.path().join("source.jsonl");
        let header = archive(&source, &path);
        let target = root.path().join("empty.db");
        drop(Store::open(&target, "unused").unwrap());
        let report = backup::restore(&path, &target).unwrap();
        assert_eq!(report.graph_digest, header.graph_digest);
        let second =
            backup::create_from_database(&target, &root.path().join("again.jsonl")).unwrap();
        assert_eq!(second.log_digest, header.log_digest);
    }

    #[test]
    fn restore_refuses_an_empty_database_still_open_by_a_node() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        diagnostic(&source, "note");
        let path = root.path().join("source.jsonl");
        archive(&source, &path);
        let destination = root.path().join("live.db");
        let live = Store::open(&destination, "birch").unwrap();
        assert!(backup::restore(&path, &destination).is_err());
        assert!(live.claims_for("daemon/alder", None).unwrap().is_empty());
        diagnostic(&live, "still serving");
        assert_eq!(live.claims_for("daemon/birch", None).unwrap().len(), 1);
    }

    #[test]
    fn a_backup_from_an_older_registry_preserves_and_later_projects_unknown_claims() {
        let root = tempfile::tempdir().unwrap();
        let current = Store::open_memory("alder").unwrap();
        diagnostic(&current, "a newer kind");
        let older = Store::open_memory("birch").unwrap();
        let mut registry = st3_schema::registry().clone();
        registry.claims.remove("daemon.diagnostic").unwrap();
        older.set_claim_registry(registry);
        let exchange = current
            .export_replication_exchange("test-fleet", &ReplicationInventory::default())
            .unwrap();
        older
            .receive_replication_exchange("alder", "test-fleet", &exchange)
            .unwrap();
        assert_eq!(older.validate_replication_backlog().unwrap().unknown, 1);
        older.project_replication_backlog().unwrap();
        let archive_path = root.path().join("older.jsonl");
        let header = archive(&older, &archive_path);
        assert_eq!(header.tables["claim_sources"].count, 0);
        let report = backup::restore(&archive_path, &root.path().join("upgraded.db")).unwrap();
        assert_eq!(report.log_digest, header.log_digest);
        assert_eq!(report.tables["claim_sources"].count, 1);
        assert!(!report.projections_match);
    }

    #[test]
    fn an_unprojected_snapshot_is_refused_before_any_archive_bytes_are_written() {
        let root = tempfile::tempdir().unwrap();
        let writer = Store::open_memory("alder").unwrap();
        diagnostic(&writer, "not projected yet");
        let database = root.path().join("source.db");
        let source = Store::open(&database, "birch").unwrap();
        let worker = Store::open(&database, "birch").unwrap();
        let exchange = writer
            .export_replication_exchange("test-fleet", &ReplicationInventory::default())
            .unwrap();
        worker
            .receive_replication_exchange("alder", "test-fleet", &exchange)
            .unwrap();
        worker.validate_replication_backlog().unwrap();
        let mut output = Vec::new();
        let error = source.write_backup(&mut output).unwrap_err();
        assert!(error.to_string().contains("fully projected"), "{error:#}");
        assert!(output.is_empty());
        diagnostic(
            &source,
            "a local write must not hide the pending projection",
        );
        assert!(source.write_backup(&mut output).is_err());
        assert!(output.is_empty());
        worker.project_replication_backlog().unwrap();
        let path = root.path().join("projected.jsonl");
        let header = archive(&source, &path);
        let report = backup::restore(&path, &root.path().join("restored.db")).unwrap();
        assert_eq!(header.graph_digest, report.graph_digest);
    }

    #[test]
    fn restore_keeps_history_admitted_before_a_later_keyed_membership() {
        // Restore receives the log in writer order. The legacy writer's history must survive
        // whether it sorts before the member that later keys it or after, when that member's
        // admission claim is replayed first.
        for name in ["alder", "cedar"] {
            let root = tempfile::tempdir().unwrap();
            let legacy = Store::open_memory(name).unwrap();
            diagnostic(&legacy, "history already admitted");
            let source = Store::open_memory("birch").unwrap();
            let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
            source.bind_fleet("test-fleet").unwrap();
            source.pin_fleet_anchor(key.public()).unwrap();
            source.set_member_key(Some(key.clone())).unwrap();
            source
                .admit_fleet_anchor("test-fleet", key.public(), "listening")
                .unwrap();
            let exchange = legacy
                .export_replication_exchange("test-fleet", &ReplicationInventory::default())
                .unwrap();
            source
                .receive_replication_exchange(name, "test-fleet", &exchange)
                .unwrap();
            source.validate_replication_backlog().unwrap();
            source.project_replication_backlog().unwrap();
            let later_key = crate::fleet::MemberKey::generate().unwrap().0;
            source
                .append_claim(&ClaimInput {
                    subject: format!("host/{name}"),
                    kind: "fleet.member-admitted".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("fleet_id".into(), json!("test-fleet")),
                        ("member_key".into(), json!(later_key.public())),
                        ("via".into(), json!("invite")),
                        ("sponsor".into(), json!("host/birch")),
                        ("mode".into(), json!("listening")),
                        ("writer_floor".into(), json!(0)),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            source.replication_snapshot().unwrap();
            assert!(
                !source.fleet_admission_residue().unwrap().is_empty(),
                "{name}"
            );
            let archive_path = root.path().join("history.jsonl");
            let header = archive(&source, &archive_path);
            let report = backup::restore(&archive_path, &root.path().join("restored.db"))
                .unwrap_or_else(|error| panic!("{name}: {error:#}"));
            assert!(report.projections_match, "{name}");
            assert_eq!(report.log_digest, header.log_digest, "{name}");
        }
    }

    #[test]
    fn a_backup_reapplies_replicated_repairs_before_comparing_digests() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        diagnostic(&source, "original");
        diagnostic(&source, "replacement");
        let receiver = Store::open_memory("birch").unwrap();
        let exchange = source
            .export_replication_exchange("test-fleet", &ReplicationInventory::default())
            .unwrap();
        receiver
            .receive_replication_exchange("alder", "test-fleet", &exchange)
            .unwrap();
        receiver.validate_replication_backlog().unwrap();
        receiver.project_replication_backlog().unwrap();
        let claims = receiver.claims_for("daemon/alder", None).unwrap();
        let repaired = &claims[0].id;
        let replacement = &claims[1].id;
        let record = receiver
            .replica_records(false)
            .unwrap()
            .into_iter()
            .find(|record| record.claim_id.as_deref() == Some(repaired.as_str()))
            .unwrap();
        receiver
            .connection
            .write()
            .execute(
                "UPDATE replica_records SET state='invalid' WHERE record_ref=?1",
                [&record.record_ref],
            )
            .unwrap();
        receiver
            .repair_replica_record(
                &record.record_ref,
                replacement,
                "fixture repair",
                "person/operator",
                "backup-repair",
            )
            .unwrap();
        receiver.replay_replication_graph().unwrap();
        let path = root.path().join("repaired.jsonl");
        let header = archive(&receiver, &path);
        let report = backup::restore(&path, &root.path().join("restored.db")).unwrap();
        assert_eq!(report.tables, header.tables);
        assert!(report.projections_match);
    }

    #[test]
    fn a_backup_preserves_multiple_wire_envelopes_for_the_same_batch() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        diagnostic(&source, "note");
        let mut exchange = source
            .export_replication_exchange("test-fleet", &ReplicationInventory::default())
            .unwrap();
        let mut variant = exchange.envelopes[0].clone();
        let mut payload: smallclaims::claim::ReplicaEnvelopePayload =
            ciborium::from_reader(variant.payload.bytes().unwrap()).unwrap();
        let blob = b"another wire representation".to_vec();
        payload
            .blobs
            .insert(hex::encode(Sha256::digest(&blob)), blob);
        let mut bytes = Vec::new();
        ciborium::into_writer(&payload, &mut bytes).unwrap();
        variant.hash = smallclaims::hash::replica_envelope_hash(
            &variant.writer,
            variant.sequence,
            variant.previous_hash.as_deref(),
            variant.accepted_at_unix_ms,
            &bytes,
        );
        variant.payload = bytes.into();
        exchange.envelopes.push(variant);
        let receiver = Store::open_memory("birch").unwrap();
        receiver
            .receive_replication_exchange("alder", "test-fleet", &exchange)
            .unwrap();
        receiver.validate_replication_backlog().unwrap();
        receiver.project_replication_backlog().unwrap();
        let path = root.path().join("variants.jsonl");
        let header = archive(&receiver, &path);
        let report = backup::restore(&path, &root.path().join("restored.db")).unwrap();
        assert_eq!(report.envelopes, 2);
        assert_eq!(report.log_digest, header.log_digest);
        assert!(report.projections_match);
    }

    #[test]
    fn create_refuses_a_partial_writer_chain_until_the_missing_envelope_arrives() {
        let root = tempfile::tempdir().unwrap();
        let writer = Store::open_memory("alder").unwrap();
        diagnostic(&writer, "first");
        diagnostic(&writer, "second");
        let source = Store::open_memory("birch").unwrap();
        let mut exchange = writer
            .export_replication_exchange("test-fleet", &ReplicationInventory::default())
            .unwrap();
        let first = exchange.envelopes.remove(0);
        source
            .receive_replication_exchange("alder", "test-fleet", &exchange)
            .unwrap();
        source.validate_replication_backlog().unwrap();
        source.project_replication_backlog().unwrap();
        let error = source.write_backup(&mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("predecessor"), "{error:#}");
        exchange.envelopes = vec![first];
        source
            .receive_replication_exchange("alder", "test-fleet", &exchange)
            .unwrap();
        source.validate_replication_backlog().unwrap();
        source.project_replication_backlog().unwrap();
        let path = root.path().join("complete.jsonl");
        let header = archive(&source, &path);
        let report = backup::restore(&path, &root.path().join("restored.db")).unwrap();
        assert_eq!(report.graph_digest, header.graph_digest);
    }

    #[test]
    fn restore_rejects_a_missing_writer_chain_predecessor() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        diagnostic(&source, "before");
        diagnostic(&source, "after");
        let path = root.path().join("chain.jsonl");
        archive(&source, &path);
        rewrite(&path, |records| {
            records.retain(
                |record| !matches!(record, Record::Envelope(envelope) if envelope.sequence == 1),
            )
        });
        let destination = root.path().join("restored.db");
        let error = backup::restore(&path, &destination).unwrap_err();
        assert!(error.to_string().contains("predecessor"), "{error:#}");
        assert!(!destination.exists());
    }

    #[test]
    fn a_trimmed_backup_restores_verified_tombstones_and_the_same_inventory() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open(&root.path().join("source.db"), "alder").unwrap();
        source.bind_fleet("test-fleet").unwrap();
        for index in 0..12 {
            diagnostic(&source, &format!("note-{index}"));
        }
        let context = CheckpointContext {
            now_unix_ms: now_ms() + 3 * 86_400_000,
            configured_peers: vec![],
            scratch: root.path().join("scratch"),
            reviewer: "person/operator".into(),
        };
        for _ in 0..3 {
            source.checkpoint_step(&context).unwrap();
        }
        assert!(source.trimmed_checkpoint().unwrap().is_some());
        assert!(source.checkpointed_envelopes().unwrap() > 0);
        assert_eq!(
            source
                .restore_checkpoint_history(&CheckpointManifest::default())
                .unwrap_err()
                .code,
            "checkpoint-history-exists"
        );
        let path = root.path().join("trimmed.jsonl");
        let header = archive(&source, &path);
        let target = root.path().join("restored.db");
        let report = backup::restore(&path, &target).unwrap();
        assert_eq!(report.graph_digest, header.graph_digest);
        let restored = Store::open(&target, &report.writer).unwrap();
        assert_eq!(
            source.replication_inventory().unwrap().digest,
            restored.replication_inventory().unwrap().digest
        );
        assert_eq!(
            source.checkpointed_envelopes().unwrap(),
            restored.checkpointed_envelopes().unwrap()
        );
    }

    #[test]
    fn a_backup_during_an_interrupted_trim_preserves_the_complete_log() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        source.bind_fleet("test-fleet").unwrap();
        for index in 0..12 {
            diagnostic(&source, &format!("note-{index}"));
        }
        let context = CheckpointContext {
            now_unix_ms: now_ms() + 3 * 86_400_000,
            configured_peers: vec![],
            scratch: root.path().join("scratch"),
            reviewer: "person/operator".into(),
        };
        for _ in 0..2 {
            source.checkpoint_step(&context).unwrap();
        }
        source.set_trim_fault(Some(TrimFault::AfterChunk(1)));
        assert!(source.checkpoint_step(&context).is_err());
        assert!(source.trimmed_checkpoint().unwrap().is_none());
        assert!(source.checkpointed_envelopes().unwrap() > 0);
        let path = root.path().join("trimming.jsonl");
        let header = archive(&source, &path);
        let report = backup::restore(&path, &root.path().join("restored.db")).unwrap();
        assert_eq!(report.log_digest, header.log_digest);
        assert!(report.projections_match);
    }

    #[test]
    fn a_backup_keeps_trimmed_history_when_a_newer_certificate_is_not_applied_yet() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        source.bind_fleet("test-fleet").unwrap();
        for index in 0..12 {
            diagnostic(&source, &format!("note-{index}"));
        }
        let mut context = CheckpointContext {
            now_unix_ms: now_ms() + 3 * 86_400_000,
            configured_peers: vec![],
            scratch: root.path().join("scratch"),
            reviewer: "person/operator".into(),
        };
        for _ in 0..3 {
            source.checkpoint_step(&context).unwrap();
        }
        let applied = source.trimmed_checkpoint().unwrap().unwrap();
        diagnostic(&source, "after the first trim");
        context.now_unix_ms += 86_400_000;
        for _ in 0..2 {
            source.checkpoint_step(&context).unwrap();
        }
        assert_eq!(source.trimmed_checkpoint().unwrap().unwrap(), applied);
        assert!(
            source
                .checkpoint_status(context.now_unix_ms, &context.configured_peers)
                .unwrap()
                .newest_stable
                .unwrap()
                .terms
                .cut_unix_ms
                > applied.cut_unix_ms
        );
        let path = root.path().join("certified.jsonl");
        let header = archive(&source, &path);
        let report = backup::restore(&path, &root.path().join("restored.db")).unwrap();
        assert_eq!(report.log_digest, header.log_digest);
        assert!(report.projections_match);
    }

    #[test]
    fn a_corrupt_or_truncated_backup_never_publishes_a_database() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        diagnostic(&source, "note");
        let path = root.path().join("graph.jsonl");
        archive(&source, &path);
        let bytes = fs::read(&path).unwrap();
        for corrupted in [bytes[..bytes.len() - 3].to_vec(), {
            let mut bytes = bytes.clone();
            let position = bytes.iter().position(|byte| *byte == b'0').unwrap();
            bytes[position] = b'1';
            bytes
        }] {
            fs::write(&path, corrupted).unwrap();
            let target = root.path().join("target.db");
            assert!(backup::restore(&path, &target).is_err());
            assert!(!target.exists());
        }
    }

    fn rewrite(path: &Path, mutate: impl FnOnce(&mut Vec<Record>)) {
        let mut input = std::io::BufReader::new(fs::File::open(path).unwrap());
        let mut records = Vec::new();
        loop {
            let mut line = String::new();
            use std::io::BufRead;
            if input.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let record: Record = serde_json::from_str(&line).unwrap();
            if !matches!(record, Record::End { .. }) {
                records.push(record);
            }
        }
        mutate(&mut records);
        let count = records
            .iter()
            .filter(|r| matches!(r, Record::Envelope(_)))
            .count() as u64;
        let mut output = fs::File::create(path).unwrap();
        let mut hash = Sha256::new();
        for record in records {
            write_record(&mut output, &mut hash, &record).unwrap();
        }
        serde_json::to_writer(
            &mut output,
            &Record::End {
                envelopes: count,
                sha256: hex::encode(hash.finalize()),
            },
        )
        .unwrap();
        output.write_all(b"\n").unwrap();
    }

    #[test]
    fn checksummed_tampering_still_fails_signature_and_graph_verification() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_memory("alder").unwrap();
        let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
        source.bind_fleet("test-fleet").unwrap();
        source.set_member_key(Some(key.clone())).unwrap();
        diagnostic(&source, "note");
        let path = root.path().join("graph.jsonl");
        archive(&source, &path);
        rewrite(&path, |records| {
            for record in records {
                if let Record::Envelope(envelope) = record {
                    envelope.signature = Some("invalid".into());
                }
            }
        });
        assert!(
            backup::restore(&path, &root.path().join("bad-signature.db"))
                .unwrap_err()
                .to_string()
                .contains("signature")
        );
        archive(&source, &path);
        rewrite(&path, |records| {
            if let Record::Header(header) = &mut records[0] {
                header.graph_digest = "changed".into();
            }
        });
        assert!(
            backup::restore(&path, &root.path().join("bad-digest.db"))
                .unwrap_err()
                .to_string()
                .contains("graph digest")
        );
    }

    // Incident2994 controls use disposable real Stores. They classify existing boundaries;
    // they neither certify reported history gaps nor implement a live backup operation.
    mod incident2994 {
        use super::*;

        fn accept(source: &Store, authenticated_peer: &str, exchange: &ReplicationExchange) {
            // The in-process fixture supplies the known exporting Store's identity,
            // just as the transport supplies its authenticated relay; do not infer it
            // from an untrusted envelope writer or bypass the real peer-label guard.
            assert_eq!(exchange.peer, authenticated_peer);
            source.receive_replication_exchange(authenticated_peer, "test-fleet", exchange).unwrap();
            let admitted = source.validate_replication_backlog().unwrap();
            assert_eq!(admitted.invalid, 0);
            assert_eq!(admitted.held, 0);
            source.project_replication_backlog().unwrap();
            for envelope in &exchange.envelopes {
                let payload: smallclaims::claim::ReplicaEnvelopePayload =
                    ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
                for claim in &payload.batch.claims {
                    assert!(source.claim_by_id(&claim.id).unwrap().is_some(),
                        "the intended canonical claim reached normal admission");
                }
            }
        }

        #[test]
        fn settled_record_admission_does_not_certify_an_uncovered_writer_predecessor() {
            let writer = Store::open_memory("alder").unwrap();
            diagnostic(&writer, "first");
            diagnostic(&writer, "second");
            let mut exchange = writer.export_replication_exchange(
                "test-fleet", &ReplicationInventory::default(),
            ).unwrap();
            let first = exchange.envelopes.remove(0);
            let source = Store::open_memory("birch").unwrap();
            accept(&source, "alder", &exchange);
            let unsettled: u64 = source.readers.get().query_row(
                "SELECT COUNT(*) FROM replica_envelopes WHERE batch_id IS NULL", [],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(unsettled, 0, "settled admission is deliberately not chain completeness");
            assert!(source.check_backup_chains().unwrap_err().to_string().contains("predecessor"));
            assert!(source.write_backup(&mut Vec::new()).is_err());
            exchange.envelopes = vec![first];
            accept(&source, "alder", &exchange);
            source.check_backup_chains().unwrap();
            let root = tempfile::tempdir().unwrap();
            let archive_path = root.path().join("arrived.jsonl");
            let header = archive(&source, &archive_path);
            let restored = backup::restore(&archive_path, &root.path().join("restored.db")).unwrap();
            assert_eq!(restored.log_digest, header.log_digest);
            assert!(restored.projections_match);
        }

        // Rebuild every affected hash/ID through the real format. Admission must succeed so
        // a malformed envelope cannot select the cause of a chain-checker refusal.
        fn relink(envelope: &mut ReplicaEnvelope, sequence: u64, link: String) {
            let mut payload: smallclaims::claim::ReplicaEnvelopePayload =
                ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
            payload.batch.replica_sequence = sequence;
            payload.batch.previous_hash = Some(link.clone());
            payload.batch.hash = smallclaims::hash::batch_header_hash(
                &payload.batch.origin, sequence, Some(&link), payload.batch.accepted_at_unix_ms,
            ).unwrap();
            payload.batch.id = format!("batch/{}/{}/{}",
                payload.batch.origin, sequence, payload.batch.hash);
            for claim in &mut payload.batch.claims {
                claim.batch_id.clone_from(&payload.batch.id);
                claim.id = smallclaims::hash::claim_hash(
                    &claim.batch_id, &claim.subject, &claim.kind, &claim.origin,
                    claim.actor.as_deref(), &claim.body, &claim.predecessors,
                ).unwrap();
            }
            smallclaims::hash::verify_replica_batch_header(&payload.batch).unwrap();
            let mut bytes = Vec::new();
            ciborium::into_writer(&payload, &mut bytes).unwrap();
            envelope.sequence = sequence;
            envelope.previous_hash = Some(link);
            envelope.hash = smallclaims::hash::replica_envelope_hash(
                &envelope.writer, sequence, envelope.previous_hash.as_deref(),
                envelope.accepted_at_unix_ms, &bytes,
            );
            envelope.payload = bytes.into();
        }

        #[test]
        fn valid_payloads_with_wrong_predecessor_hash_or_sequence_still_refuse() {
            for wrong_sequence in [false, true] {
                let writer = Store::open_memory("alder").unwrap();
                diagnostic(&writer, "first");
                diagnostic(&writer, "second");
                let mut exchange = writer.export_replication_exchange(
                    "test-fleet", &ReplicationInventory::default(),
                ).unwrap();
                let predecessor: smallclaims::claim::ReplicaEnvelopePayload =
                    ciborium::from_reader(exchange.envelopes[0].payload.bytes().unwrap()).unwrap();
                let link = if wrong_sequence { predecessor.batch.hash } else { "ab".repeat(32) };
                relink(&mut exchange.envelopes[1], if wrong_sequence { 3 } else { 2 }, link);
                let source = Store::open_memory("birch").unwrap();
                accept(&source, "alder", &exchange);
                assert_eq!(source.readers.get().query_row(
                    "SELECT COUNT(*) FROM batches WHERE origin='alder'", [],
                    |row| row.get::<_, u64>(0),
                ).unwrap(), 2, "both well-formed canonical batches reached the checker");
                let error = source.check_backup_chains().unwrap_err().to_string();
                assert!(error.contains(if wrong_sequence {
                    "retained predecessor absent"
                } else {
                    "retained predecessor wrong hash"
                }), "{error}");
                assert!(error.contains("not covered by a checkpoint"), "{error}");
                assert_eq!(error.contains("predecessor may yet arrive"), wrong_sequence,
                    "arrival is only a conditional possibility for an absent row, not wrong hash");
                assert!(source.write_backup(&mut Vec::new()).is_err());
            }
        }

        #[test]
        fn another_writers_matching_batch_hash_is_not_a_predecessor() {
            let writer = Store::open_memory("alder").unwrap();
            diagnostic(&writer, "first");
            diagnostic(&writer, "second");
            let other = Store::open_memory("cedar").unwrap();
            diagnostic(&other, "another writer");
            let mut exchange = writer.export_replication_exchange(
                "test-fleet", &ReplicationInventory::default(),
            ).unwrap();
            let other_exchange = other.export_replication_exchange(
                "test-fleet", &ReplicationInventory::default(),
            ).unwrap();
            let other_payload: smallclaims::claim::ReplicaEnvelopePayload =
                ciborium::from_reader(other_exchange.envelopes[0].payload.bytes().unwrap()).unwrap();
            let other_hash = other_payload.batch.hash;
            relink(&mut exchange.envelopes[1], 2, other_hash.clone());
            let source = Store::open_memory("birch").unwrap();
            accept(&source, "cedar", &other_exchange);
            accept(&source, "alder", &exchange);
            assert!(source.readers.get().query_row(
                "SELECT EXISTS(SELECT 1 FROM batches WHERE origin='cedar' AND replica_sequence=1 AND hash=?1)",
                [&other_hash], |row| row.get::<_, bool>(0),
            ).unwrap(), "the hash is present, but belongs to another writer");
            let own_predecessor: String = source.readers.get().query_row(
                "SELECT hash FROM batches WHERE origin='alder' AND replica_sequence=1", [],
                |row| row.get(0),
            ).unwrap();
            assert_ne!(own_predecessor, other_hash, "both admitted writers retain different real predecessor hashes");
            let error = source.check_backup_chains().unwrap_err().to_string();
            assert!(error.contains("predecessor"));
            for witness in ["writer=\"alder\"", "sequence=2", "predecessor_sequence=1",
                "retained predecessor wrong hash", "not covered by a checkpoint"] {
                assert!(error.contains(witness), "missing {witness:?} from {error}");
            }
        }

        #[test]
        fn create_and_restore_report_the_same_exact_missing_predecessor_witness() {
            let root = tempfile::tempdir().unwrap();
            let writer = Store::open_memory("alder").unwrap();
            diagnostic(&writer, "first");
            diagnostic(&writer, "second");
            let archive_path = root.path().join("complete.jsonl");
            archive(&writer, &archive_path);
            let mut exchange = writer.export_replication_exchange(
                "test-fleet", &ReplicationInventory::default(),
            ).unwrap();
            exchange.envelopes.remove(0);
            let source = Store::open_memory("birch").unwrap();
            accept(&source, "alder", &exchange);
            let create = source.write_backup(&mut Vec::new()).unwrap_err().to_string();
            rewrite(&archive_path, |records| records.retain(
                |record| !matches!(record, Record::Envelope(envelope) if envelope.sequence == 1),
            ));
            let target = root.path().join("must-not-publish.db");
            let restore = backup::restore(&archive_path, &target).unwrap_err().to_string();
            assert_eq!(create, restore);
            for part in ["writer=\"alder\"", "sequence=2", "predecessor_sequence=1",
                "expected_hash=", "retained predecessor absent", "not covered by a checkpoint"] {
                assert!(create.contains(part), "missing {part:?} from {create}");
            }
            assert!(!create.contains("retry"), "unknown history is not a catch-up promise");
            assert!(create.contains("predecessor may yet arrive"));
            assert!(create.contains("settled replication does not prove a complete chain"));
            assert!(!target.exists());
        }

        #[test]
        fn a_local_writer_floor_alone_is_not_a_backup_retention_certificate() {
            let source = Store::open_memory("alder").unwrap();
            source.set_writer_floor(12).unwrap();
            diagnostic(&source, "after a floor");
            let (sequence, link): (u64, Option<String>) = source.readers.get().query_row(
                "SELECT replica_sequence,previous_hash FROM batches WHERE origin='alder'", [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(sequence, 13);
            assert!(link.is_none());
            assert_eq!(source.readers.get().query_row(
                "SELECT COUNT(*) FROM checkpoint_envelopes", [], |row| row.get::<_, u64>(0),
            ).unwrap(), 0);
            // Deliberately NOT a signed re-admission/complete old-history control. A bare
            // local floor cannot stand in for either of those proofs or certify the report.
            assert!(source.check_backup_chains().unwrap_err().to_string().contains("predecessor"));
        }

        #[test]
        fn cancellation_after_header_releases_the_actual_snapshot_and_request_loan() {
            use smallclaims::read_budget::{self, ReadBudget};
            let source = Store::open_memory("alder").unwrap();
            diagnostic(&source, "before cancellation");
            let budget = ReadBudget::new("/v1/backup", std::time::Duration::from_secs(15));
            struct CancelAfterHeader { bytes: Vec<u8>, budget: ReadBudget }
            impl Write for CancelAfterHeader {
                fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                    self.bytes.extend_from_slice(bytes);
                    self.budget.cancel();
                    Ok(bytes.len())
                }
                fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
            }
            let mut output = CancelAfterHeader { bytes: vec![], budget: budget.clone() };
            let result = read_budget::with(Some(budget), || {
                source.readers.request_read(|| source.write_backup(&mut output))
            }).unwrap();
            assert!(result.is_err());
            assert!(!output.bytes.is_empty(), "cancellation follows a real header write");
            let report = source.readers.live_read_report();
            assert_eq!(report.snapshot_checkouts, 0);
            assert_eq!(report.live_checkouts, 0);
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("interrupted.jsonl");
            fs::write(&path, output.bytes).unwrap();
            assert!(backup::verify_file(&path).is_err(), "no completed archive after cancellation");
            // A normal managed writer on THIS thread also detects an escaped pinned scope.
            diagnostic(&source, "after cancellation");
            let good = root.path().join("complete.jsonl");
            archive(&source, &good);
            backup::verify_file(&good).unwrap();
        }

        #[test]
        fn a_broken_output_releases_the_snapshot_without_a_completed_archive() {
            let source = Store::open_memory("alder").unwrap();
            diagnostic(&source, "before broken output");
            struct BrokenOutput;
            impl Write for BrokenOutput {
                fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "invented sink closed"))
                }
                fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
            }
            let error = source.write_backup(&mut BrokenOutput).unwrap_err();
            assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::BrokenPipe);
            let report = source.readers.live_read_report();
            assert_eq!(report.snapshot_checkouts, 0);
            assert_eq!(report.live_checkouts, 0);
            diagnostic(&source, "after broken output");
            // This synchronous sink analogue is not an HTTP client-disconnect/worker proof.
        }

        #[test]
        fn certified_trim_and_ingest_during_export_preserve_the_old_complete_cut() {
            let root = tempfile::tempdir().unwrap();
            let source = Arc::new(Store::open(&root.path().join("source.db"), "alder").unwrap());
            source.bind_fleet("test-fleet").unwrap();
            for index in 0..12 { diagnostic(&source, &format!("note-{index}")); }
            let context = CheckpointContext {
                now_unix_ms: now_ms() + 3 * 86_400_000,
                configured_peers: vec![], scratch: root.path().join("scratch"),
                reviewer: "person/operator".into(),
            };
            for _ in 0..2 { source.checkpoint_step(&context).unwrap(); }
            struct TrimDuringBackup {
                source: Arc<Store>, context: CheckpointContext, bytes: Vec<u8>, ran: bool,
            }
            impl Write for TrimDuringBackup {
                fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                    if !self.ran {
                        self.ran = true;
                        // Preserve the normal same-thread pinned-read assertion: the writer
                        // commits on another thread while this actual snapshot stays pinned.
                        std::thread::scope(|scope| {
                            scope.spawn(|| {
                                self.source.checkpoint_step(&self.context).unwrap();
                                assert!(self.source.trimmed_checkpoint().unwrap().is_some());
                                assert!(self.source.checkpointed_envelopes().unwrap() > 0);
                                diagnostic(&self.source, "after the retained backup cut");
                            }).join().unwrap();
                        });
                    }
                    self.bytes.extend_from_slice(bytes);
                    Ok(bytes.len())
                }
                fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
            }
            let mut output = TrimDuringBackup { source: source.clone(), context, bytes: vec![], ran: false };
            let header = source.write_backup(&mut output).unwrap();
            assert!(output.ran);
            assert_eq!(source.readers.live_read_report().snapshot_checkouts, 0);
            let old = root.path().join("old-cut.jsonl");
            fs::write(&old, output.bytes).unwrap();
            let report = backup::restore(&old, &root.path().join("old-restored.db")).unwrap();
            assert_eq!(report.log_digest, header.log_digest);
            assert!(report.projections_match);
            let fresh = root.path().join("fresh.jsonl");
            let fresh_header = archive(&source, &fresh);
            assert_ne!(fresh_header.log_digest, header.log_digest);
            let fresh_report = backup::restore(&fresh, &root.path().join("fresh-restored.db")).unwrap();
            assert_eq!(fresh_report.log_digest, fresh_header.log_digest);
            assert!(fresh_report.projections_match);
        }
    }
}
