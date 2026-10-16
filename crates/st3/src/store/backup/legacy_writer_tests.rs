//! Historical byte oracle copied from backup.rs at 4e97aa67882e061ffc3f9b0696e0cf8d9dbda7d7.
//! Test-only: preserve the pre-factoring Store reads and writer, independent of source::write.
use super::*;
struct LegacyWriter<'a>(&'a Store);
impl std::ops::Deref for LegacyWriter<'_> {
    type Target = Store;
    fn deref(&self) -> &Store { self.0 }
}
pub(super) fn write(store: &Store, output: &mut impl Write, bodies: Option<&crate::message_body::Bodies>) -> Result<Header> {
    LegacyWriter(store).write_backup_with_bodies(output, bodies)
}
impl LegacyWriter<'_> {
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
            format: crate::backup::FORMAT.into(),
            version: crate::backup::VERSION,
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
                "WITH live AS (SELECT writer,sequence,envelope_hash FROM replica_envelopes
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
                let mut header = self.snapshot_backup_header()?;
                if !owned.is_empty() {
                    header.version = crate::backup::VERSION_WITH_BODIES;
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
                            &Record::MessageBody(crate::backup::MessageBodyRecord {
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

    fn check_backup_chains(&self) -> Result<()> {
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
                        predecessor.query_row(params![writer, sequence - 1, link], |r| r.get(0))?;
                    if !found {
                        // Classify only a failure: the accepted path retains exactly its old
                        // two indexed existence probes and the same checkpoint coverage rule.
                        let retained: bool = connection.prepare_cached(
                            "SELECT EXISTS(SELECT 1 FROM batches WHERE origin=?1 AND replica_sequence=?2)"
                        )?.query_row(params![writer, sequence - 1], |row| row.get(0))?;
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
