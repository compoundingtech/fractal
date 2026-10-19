//! Backup reads pin a pooled reader and page the sync envelopes; restore uses normal admission.
use super::*;
use super::backup_format::{Header, PAGE, Record, RestoreReport, TableDigest, write_record};
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicaEnvelopeSignature};
use std::io::Write;
use std::sync::atomic::Ordering;
use crate::message_body::BodyAttachment as _;

impl Store {
    pub fn backup_header(&self) -> Result<Header> {
        self.read_snapshot(|_| self.snapshot_backup_header())
    }

    fn snapshot_backup_header(&self) -> Result<Header> {
        let connection = self.readers.get();
        let digests = projection_digest::tables(&connection)?;
        let counts = connection
            .prepare(
                "SELECT table_name,row_count FROM projection_digest_state ORDER BY table_name",
            )?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
        Ok(Header {
            format: super::backup_format::FORMAT.into(),
            version: super::backup_format::VERSION,
            source_build: st_drivers::version::machine_version(),
            source_schema: connection.query_row("PRAGMA user_version", [], |r| r.get(0))?,
            registry_digest: self.runtime.schema_digest(),
            fleet_id: fleet_meta(&connection, "fleet_id")?,
            fleet_anchor: fleet_meta(&connection, "fleet_anchor_key")?,
            graph_digest: projection_digest::root(&digests),
            log_digest: self.backup_log_digest()?,
            tables: digests
                .into_iter()
                .map(|(name, digest)| {
                    let count = counts[&name];
                    (name, TableDigest { digest, count })
                })
                .collect(),
        })
    }

    fn backup_log_digest(&self) -> Result<String> {
        let connection = self.readers.get();
        let mut hash = Sha256::new();
        hash.update(INVENTORY_DIGEST_DOMAIN);
        let mut after = (String::new(), 0_u64, String::new());
        loop {
            // Both index seeks are bounded before the UNION sorts at most two pages.
            let identities = connection.prepare_cached(
                /* sqlite-large-value: allow-for-now lv-hd3cdd3c4d31c owner=st3_store::store::backup TODO=trace-statement-and-value-representation */ "WITH live AS (SELECT writer,sequence,envelope_hash FROM replica_envelopes
                   WHERE (writer,sequence,envelope_hash)>(?1,?2,?3) AND batch_id IS NOT NULL
                   ORDER BY writer,sequence,envelope_hash LIMIT ?4),
                 dropped AS (SELECT writer,sequence,envelope_hash FROM checkpoint_envelopes
                   WHERE (writer,sequence,envelope_hash)>(?1,?2,?3)
                   ORDER BY writer,sequence,envelope_hash LIMIT ?4)
                 SELECT * FROM live UNION SELECT * FROM dropped ORDER BY writer,sequence,envelope_hash LIMIT ?4"
            )?.query_map(params![after.0,after.1,after.2,PAGE], |r| Ok(ReplicaEnvelopeId {
                writer:r.get(0)?,sequence:r.get(1)?,hash:r.get(2)?,
            }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if identities.is_empty() {
                break;
            }
            for identity in &identities {
                update_identity_digest(
                    &mut hash,
                    &identity.writer,
                    identity.sequence,
                    &identity.hash,
                );
            }
            let last = identities.last().unwrap();
            after = (last.writer.clone(), last.sequence, last.hash.clone());
        }
        Ok(hex::encode(hash.finalize()))
    }

    /// Run on a blocking read worker, never the writer queue. No page query scans the full log.
    pub fn write_backup(&self, output: &mut impl Write) -> Result<Header> {
        self.write_backup_with_bodies(output, None)
    }

    /// The same archive, with the texts of the long messages this member owns after its claims.
    pub fn write_backup_with_bodies(
        &self,
        output: &mut impl Write,
        bodies: Option<&crate::message_body::Bodies>,
    ) -> Result<Header> {
        // The bodies this member owns that its claims name, checked once here so the header can
        // say whether the archive carries any, and written again below inside the snapshot.
        let mut owned: Vec<(String, String, String)> = Vec::new();
        if let Some(bodies) = bodies {
            for file in bodies
                .files()
                .into_iter()
                .filter(|file| !file.temporary && !file.expired) {
                let subject = format!("message/{}", file.id);
                let Some(message) = self.message(&subject)? else {
                    continue;
                };
                let (Some(named), true) = (message.body_attachment(), bodies.holds(&message.from, &file))
                else {
                    continue;
                };
                let Some(bytes) = bodies.read(&message.from, &subject)? else {
                    continue;
                };
                if hex::encode(Sha256::digest(&bytes)) == named.sha256 {
                    owned.push((subject, message.from, named.sha256));
                }
            }
        }
        loop {
            self.seal_local_batches()?;
            // Capture before pinning: another sealer may advance this atomic after our reader
            // starts, but that commit would not be visible to the pinned snapshot.
            let seeded_through = self.seeded_batch_rowid.load(Ordering::Acquire);
            let result = self.read_snapshot(|_| {
                // A write can land between sealing and pinning the reader. Retry before emitting
                // anything if that snapshot has claims without their signed sync envelope.
                if max_batch_rowid(&self.readers.get())? > seeded_through {
                    return Ok(None);
                }
                // Admission and projection commit separately. Only a snapshot whose persisted
                // projection frontier covers its log can promise the same graph on restore.
                let health: Option<(String, u64)> = self.readers.get().query_row(
                    "SELECT status,last_good_store_index FROM projection_health WHERE aggregate='graph'",
                    [], |row| Ok((row.get(0)?, row.get(1)?)),
                ).optional()?;
                let admitted: u64 = self.readers.get().query_row(
                    "SELECT value FROM meta WHERE key='replication_admitted_index'", [],
                    |row| row.get::<_, String>(0),
                )?.parse()?;
                anyhow::ensure!(
                    health.map_or(admitted == 0, |(status, through)| status == "healthy" && through >= admitted),
                    "backup requires a healthy, fully projected graph; retry after replication catches up"
                );
                let mut header = self.backup_header()?;
                if !owned.is_empty() {
                    header.version = super::backup_format::VERSION_WITH_BODIES;
                }
                let mut hash = Sha256::new();
                write_record(output, &mut hash, &Record::Header(header.clone()))?;
                let mut after = (String::new(), 0_u64, String::new());
                let mut count = 0;
                loop {
                    let connection = self.readers.get();
                    let identities = connection
                        .prepare_cached(
                            "SELECT writer,sequence,envelope_hash FROM replica_envelopes
                         WHERE (writer,sequence,envelope_hash)>(?1,?2,?3) AND batch_id IS NOT NULL
                         ORDER BY writer,sequence,envelope_hash LIMIT ?4",
                        )?
                        .query_map(params![after.0, after.1, after.2, PAGE], |r| {
                            Ok(ReplicaEnvelopeId {
                                writer: r.get(0)?,
                                sequence: r.get(1)?,
                                hash: r.get(2)?,
                            })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    if identities.is_empty() {
                        break;
                    }
                    let last = identities.last().unwrap();
                    after = (last.writer.clone(), last.sequence, last.hash.clone());
                    drop(connection);
                    let signatures = self.replication_signatures_for(&identities)?;
                    for envelope in self.replica_envelopes(identities)? {
                        write_record(output, &mut hash, &Record::Envelope(envelope))?;
                        count += 1;
                    }
                    if !signatures.is_empty() {
                        write_record(output, &mut hash, &Record::Signatures(signatures))?;
                    }
                }
                // A trim records its complete certified tombstones before deleting any rows.
                // A snapshot between deletion chunks must include that manifest too.
                let checkpoint: Option<(String, u128)> = self
                    .readers
                    .get()
                    .query_row(
                        "SELECT id,cut_unix_ms FROM checkpoints
                     WHERE state IN ('trimming','trimmed') AND drop_digest IS NOT NULL
                     ORDER BY cut_unix_ms DESC LIMIT 1",
                        [],
                        |row| {
                            Ok((
                                row.get(0)?,
                                u128::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                            ))
                        },
                    )
                    .optional()?;
                if let Some((checkpoint, cut_unix_ms)) = checkpoint {
                    // The manifest is the sync representation, verified against signed certificate
                    // claims on restore. No SQL or local checkpoint bookkeeping enters the archive.
                    let mut after = None;
                    loop {
                        let page = self.manifest_page(
                            &CheckpointManifestRequest {
                                checkpoint: checkpoint.clone(),
                                cut_unix_ms,
                                after,
                            },
                            PAGE,
                        )?;
                        after = page.next.clone();
                        write_record(output, &mut hash, &Record::Checkpoint(page))?;
                        if after.is_none() {
                            break;
                        }
                    }
                }
                if let Some(bodies) = bodies {
                    use base64::Engine as _;
                    for (subject, actor, sha256) in &owned {
                        // A body that vanished or changed since it was checked is left out, and
                        // the restore report counts it as not returned.
                        let Some(bytes) = bodies.read(actor, subject)? else {
                            continue;
                        };
                        if hex::encode(Sha256::digest(&bytes)) != *sha256 {
                            continue;
                        }
                        write_record(
                            output,
                            &mut hash,
                            &Record::MessageBody(super::backup_format::MessageBodyRecord {
                                message: subject.clone(),
                                actor: actor.clone(),
                                sha256: sha256.clone(),
                                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                            }),
                        )?;
                    }
                }
                self.check_backup_chains()?;
                serde_json::to_writer(
                    &mut *output,
                    &Record::End {
                        envelopes: count,
                        sha256: hex::encode(hash.finalize()),
                    },
                )?;
                output.write_all(b"\n")?;
                Ok(Some(header))
            })?;
            if let Some(header) = result {
                return Ok(header);
            }
        }
    }

    pub fn prepare_backup_restore(&self, header: &Header, writer: &str) -> Result<()> {
        if let Some(fleet) = &header.fleet_id {
            self.bind_fleet(fleet)?;
        }
        if let Some(anchor) = &header.fleet_anchor {
            self.pin_fleet_anchor(anchor)?;
        }
        self.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h91e74bbcd11f owner=st3_store::store::backup TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('backup_restore_writer',?1)",
            [writer],
        )?;
        Ok(())
    }

    pub /* sqlite-large-value-flow: allow-for-now flow-hfe611a4800c9 owner=st3_store::store::backup TODO=chunk-or-reference-exact-wire-values */ fn receive_backup_page(
        &self,
        header: &Header,
        envelopes: &[ReplicaEnvelope],
    ) -> Result<()> {
        let fleet = header.fleet_id.as_deref().unwrap_or("");
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        for envelope in envelopes {
            match (&envelope.member_key, &envelope.signature) {
                (Some(key), Some(signature)) => {
                    let message = crate::fleet::envelope_signature_message(
                        fleet,
                        &envelope.writer,
                        envelope.sequence,
                        &envelope.hash,
                    );
                    anyhow::ensure!(
                        crate::fleet::verify_signature(key, &message, signature),
                        "invalid backup envelope signature"
                    );
                    store_envelope_signature_tx(
                        &transaction,
                        fleet,
                        &envelope.writer,
                        envelope.sequence,
                        &envelope.hash,
                        key,
                        signature,
                        &now_ms().to_string(),
                    )?;
                }
                (None, None) => {} // Pre-membership sync envelopes have no signature.
                _ => anyhow::bail!("incomplete backup envelope signature"),
            }
            // Sync can hold several wire representations of one historical batch. Preserve
            // every envelope identity, in the same total order the exporter uses.
            let previous: Option<(u64, String)> = transaction
                .query_row(
                    "SELECT sequence,envelope_hash FROM replica_envelopes WHERE writer=?1
                 ORDER BY sequence DESC,envelope_hash DESC LIMIT 1",
                    [&envelope.writer],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((sequence, hash)) = previous {
                anyhow::ensure!(
                    (envelope.sequence, envelope.hash.as_str()) > (sequence, hash.as_str()),
                    "backup envelopes are duplicated or out of order"
                );
            }
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-hd5dfe69b23b9 owner=st3_store::store::backup TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes(writer,sequence,envelope_hash,previous_hash,accepted_at_unix_ms,
                     payload,relay,receipt_state,received_at_unix_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,'backup','pending',?7)",
                params![envelope.writer, envelope.sequence, envelope.hash, envelope.previous_hash,
                    envelope.accepted_at_unix_ms.to_string(), envelope.payload, now_ms().to_string()]
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn receive_backup_signatures(
        &self,
        header: &Header,
        signatures: &[ReplicaEnvelopeSignature],
    ) -> Result<()> {
        let fleet = header.fleet_id.as_deref().unwrap_or("");
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        for signature in signatures {
            let message = crate::fleet::envelope_signature_message(
                fleet,
                &signature.writer,
                signature.sequence,
                &signature.hash,
            );
            anyhow::ensure!(
                crate::fleet::verify_signature(
                    &signature.member_key,
                    &message,
                    &signature.signature
                ),
                "invalid backup envelope signature"
            );
            store_envelope_signature_tx(
                &transaction,
                fleet,
                &signature.writer,
                signature.sequence,
                &signature.hash,
                &signature.member_key,
                &signature.signature,
                &now_ms().to_string(),
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn finish_backup_restore(
        &self,
        header: &Header,
        writer: &str,
        envelopes: u64,
        checkpoint: Option<&CheckpointManifest>,
    ) -> Result<RestoreReport> {
        self.admit_restored_backlog()?;
        self.apply_replication_repairs()?;
        // Reject any undecodable or invalid envelope rather than silently restoring a partial log.
        {
            let connection = self.readers.get();
            let unresolved: u64 = connection.query_row(
                "SELECT COUNT(*) FROM replica_envelopes WHERE batch_id IS NULL",
                [],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                unresolved == 0,
                "backup contains {unresolved} unadmitted envelopes"
            );
        }
        // Normal admission is final: later membership claims do not revoke history
        // admitted before their arrival. Preserve that history, including doctor residue.
        self.replay_replication_graph()?;
        if let Some(manifest) = checkpoint {
            self.restore_checkpoint_history(manifest)?;
            self.replay_replication_graph()?;
        }
        self.check_backup_chains()?;
        let actual = self.read_snapshot(|_| self.backup_header())?;
        anyhow::ensure!(
            actual.log_digest == header.log_digest,
            "restored envelope log digest differs from backup"
        );
        if actual.registry_digest == header.registry_digest {
            anyhow::ensure!(
                actual.tables.get("claim_sources") == header.tables.get("claim_sources"),
                "restored claim source digest/count differs from backup: expected {:?}, got {:?}",
                header.tables.get("claim_sources"),
                actual.tables.get("claim_sources")
            );
        }
        let projections_match =
            actual.graph_digest == header.graph_digest && actual.tables == header.tables;
        if actual.source_schema == header.source_schema
            && actual.registry_digest == header.registry_digest
        {
            anyhow::ensure!(
                projections_match,
                "restored graph digest differs from backup"
            );
        }
        Ok(RestoreReport {
            writer: writer.into(),
            envelopes,
            graph_digest: actual.graph_digest,
            source_graph_digest: header.graph_digest.clone(),
            log_digest: actual.log_digest,
            projections_match,
            tables: actual.tables,
            message_bodies_restored: 0,
            message_bodies_unmatched: 0,
            message_bodies_referenced: 0,
        })
    }

    /// How many long messages the claims here name, by the `text/plain` entry in their
    /// `attachments`. A restore reads it once, offline, to say how many texts it could not return.
    pub fn message_body_references(&self) -> Result<u64> {
        Ok(self.readers.get().query_row(
            "SELECT COUNT(*) FROM claims WHERE kind='message.sent'
               AND json_extract(body,'$.fields.attachments') LIKE '%\"text/plain\"%'",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn check_backup_chains(&self) -> Result<()> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT rowid,origin,replica_sequence,previous_hash FROM batches
             WHERE rowid>?1 ORDER BY rowid LIMIT ?2",
        )?;
        let mut predecessor = connection.prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM batches WHERE origin=?1 AND replica_sequence=?2 AND hash=?3)
                 OR EXISTS(SELECT 1 FROM checkpoint_envelopes WHERE writer=?1 AND sequence=?2)"
        )?;
        let mut after = 0_i64;
        loop {
            let page = statement
                .query_map(params![after, PAGE], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if page.is_empty() {
                break;
            }
            for (rowid, writer, sequence, link) in page {
                after = rowid;
                if let Some(link) = link {
                    anyhow::ensure!(sequence > 0,
                        "backup writer chain has an invalid root: writer={writer:?} sequence={sequence}");
                    // Retained predecessors must match their exact batch hash. A predecessor
                    // removed by a verified checkpoint has only its certified wire identity left.
                    let found: bool =
                        predecessor.query_row(/* sqlite-large-value: allow-for-now lv-h16aded515dfc owner=st3_store::store::backup TODO=trace-statement-and-value-representation */ params![writer, sequence - 1, link], |r| r.get(0))?;
                    if !found {
                        // Classify only a failure: the accepted path retains exactly its old
                        // two indexed existence probes and the same checkpoint coverage rule.
                        let retained: bool = connection.prepare_cached(
                            "SELECT EXISTS(SELECT 1 FROM batches WHERE origin=?1 AND replica_sequence=?2)"
                        )?.query_row(/* sqlite-large-value: allow-for-now lv-h0ed46697fcc4 owner=st3_store::store::backup TODO=trace-statement-and-value-representation */ params![writer, sequence - 1], |row| row.get(0))?;
                        let case = if retained { "wrong hash" } else { "absent" };
                        let arrival = if retained { "" } else {
                            "; if this writer's history is still arriving, the predecessor may yet arrive; \
                             settled replication does not prove a complete chain"
                        };
                        anyhow::bail!(
                            "backup writer chain has a missing or broken predecessor: \
                             writer={writer:?} sequence={sequence} predecessor_sequence={} \
                             expected_hash={link:?}; retained predecessor {case}; \
                             not covered by a checkpoint{arrival}",
                            sequence - 1,
                        );
                    }
                } else {
                    anyhow::ensure!(
                        sequence <= 1,
                        "backup writer chain starts after a missing predecessor: \
                         writer={writer:?} sequence={sequence}; no predecessor hash recorded"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::backup_tests_shared_attention_tests::{diagnostic};
use super::*;
    use crate::backup;

    #[test]
    fn offline_export_finishes_upgrade_recovery_before_recording_digests() {
        let root = tempfile::tempdir().unwrap();
        let current = Store::open_memory("alder").unwrap();
        diagnostic(&current, "a newer kind");
        let path = root.path().join("older.db");
        let older = Store::open(&path, "birch").unwrap();
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
        drop(older);
        let archive_path = root.path().join("upgraded.jsonl");
        let header = backup::create_from_database(&path, &archive_path).unwrap();
        assert_eq!(header.tables["claim_sources"].count, 1);
        let report = backup::restore(&archive_path, &root.path().join("restored.db")).unwrap();
        assert!(report.projections_match);
    }

    #[test]
    fn backup_snapshot_does_not_include_a_concurrent_write() {
        let root = tempfile::tempdir().unwrap();
        let source = Arc::new(Store::open(&root.path().join("source.db"), "alder").unwrap());
        diagnostic(&source, "before");
        struct WriteDuringBackup {
            bytes: Vec<u8>,
            store: Arc<Store>,
            wrote: bool,
        }
        impl Write for WriteDuringBackup {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if !self.wrote {
                    self.wrote = true;
                    let store = self.store.clone();
                    std::thread::spawn(move || diagnostic(&store, "after"))
                        .join()
                        .unwrap();
                }
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = WriteDuringBackup {
            bytes: vec![],
            store: source.clone(),
            wrote: false,
        };
        let header = source.write_backup(&mut output).unwrap();
        let path = root.path().join("snapshot.jsonl");
        fs::write(&path, output.bytes).unwrap();
        let report = backup::restore(&path, &root.path().join("restored.db")).unwrap();
        assert_eq!(report.graph_digest, header.graph_digest);
        assert_eq!(report.tables["claim_sources"].count, 1);
        assert_eq!(
            source.backup_header().unwrap().tables["claim_sources"].count,
            2
        );
    }

}
