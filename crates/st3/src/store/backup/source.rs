//! One connection-bound archive emitter for the live snapshot and the private read-only path.
//! This module installs nothing and does not own a live copy, CLI, signer or recovery worker.
use super::*;

pub(super) type OwnedBody = (String, String, String);

pub(super) fn header(connection: &Connection, registry_digest: &str) -> Result<Header> {
    let digests = projection_digest::tables(connection)?;
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
        registry_digest: registry_digest.to_owned(),
        fleet_id: fleet_meta(connection, "fleet_id")?,
        fleet_anchor: fleet_meta(connection, "fleet_anchor_key")?,
        graph_digest: projection_digest::root(&digests),
        log_digest: log_digest(connection)?,
        tables: digests
            .into_iter()
            .map(|(name, digest)| {
                let count = counts[&name];
                (name, TableDigest { digest, count })
            })
            .collect(),
    })
}

fn log_digest(connection: &Connection) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(INVENTORY_DIGEST_DOMAIN);
    let mut after = (String::new(), 0_u64, String::new());
    loop {
        smallclaims::read_budget::check()?;
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

fn message_at(connection: &Connection, subject: &str) -> Result<Option<MessageView>> {
    // created_index is a returned identity/display field, not an as-of bound:
    // message_view_tx folds latest_actual and current_desired on this connection.
    let created: Option<u64> = connection.query_row(
        "SELECT MIN(store_index) FROM claims WHERE subject=?1 AND subject LIKE 'message/%'",
        [subject], |row| row.get(0),
    )?;
    created.map(|index| message_view_tx(connection, subject, index)).transpose()
}

pub(super) fn owned_bodies(
    connection: &Connection,
    bodies: Option<&crate::message_body::Bodies>,
) -> Result<Vec<OwnedBody>> {
    owned_bodies_with(bodies, |subject| message_at(connection, subject))
}

/// Keep the live Store's existing view/cache path while sharing sender/hash selection.
pub(super) fn owned_bodies_with(
    bodies: Option<&crate::message_body::Bodies>,
    mut message: impl FnMut(&str) -> Result<Option<MessageView>>,
) -> Result<Vec<OwnedBody>> {
    let mut owned: Vec<(String, String, String)> = Vec::new();
    if let Some(bodies) = bodies {
        for file in bodies
            .files()
            .into_iter()
            .filter(|file| !file.temporary && !file.expired) {
            let subject = format!("message/{}", file.id);
            let Some(message) = message(&subject)? else {
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
    Ok(owned)
}

/// Emit exactly one existing-format archive on the caller's pinned connection.
/// Framing, ordering, health, manifest, body and chain rules have one implementation.
pub(super) fn write(
    connection: &Connection,
    registry_digest: &str,
    output: &mut impl Write,
    bodies: Option<&crate::message_body::Bodies>,
    owned: &[OwnedBody],
) -> Result<Header> {
    // This connection remains pinned across pages: there is no new pool checkout to
    // check its read budget. Bound Rust/output work cooperatively, including small
    // queries that do not reach SQLite's progress-handler instruction threshold.
    smallclaims::read_budget::check()?;
    // Admission and projection commit separately. Only a snapshot whose persisted
    // projection frontier covers its log can promise the same graph on restore.
    let health: Option<(String, u64)> = connection.query_row(
        "SELECT status,last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let admitted: u64 = connection.query_row(
        "SELECT value FROM meta WHERE key='replication_admitted_index'", [],
        |row| row.get::<_, String>(0),
    )?.parse()?;
    anyhow::ensure!(
        health.map_or(admitted == 0, |(status, through)| status == "healthy" && through >= admitted),
        "backup requires a healthy, fully projected graph; retry after replication catches up"
    );
    let mut header = header(connection, registry_digest)?;
    if !owned.is_empty() {
        header.version = crate::backup::VERSION_WITH_BODIES;
    }
    let mut hash = Sha256::new();
    write_record(output, &mut hash, &Record::Header(header.clone()))?;
    let mut after = (String::new(), 0_u64, String::new());
    let mut count = 0;
    loop {
        smallclaims::read_budget::check()?;
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
        let signatures = replication_signatures_for_at(connection, &identities)?;
        for envelope in replica_envelopes_at(connection, identities)? {
            write_record(output, &mut hash, &Record::Envelope(envelope))?;
            count += 1;
        }
        if !signatures.is_empty() {
            write_record(output, &mut hash, &Record::Signatures(signatures))?;
        }
    }
    // A trim records its complete certified tombstones before deleting any rows.
    // A snapshot between deletion chunks must include that manifest too.
    let checkpoint: Option<(String, u128)> = connection
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
            smallclaims::read_budget::check()?;
            let page = smallclaims::store::checkpoint::tombstones::manifest_page_at(
                connection,
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
        for (subject, actor, sha256) in owned {
            smallclaims::read_budget::check()?;
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
    check_chains(connection)?;
    smallclaims::read_budget::check()?;
    serde_json::to_writer(
        &mut *output,
        &Record::End {
            envelopes: count,
            sha256: hex::encode(hash.finalize()),
        },
    )?;
    output.write_all(b"\n")?;
    Ok(header)
}

pub(super) fn check_chains(connection: &Connection) -> Result<()> {
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
        smallclaims::read_budget::check()?;
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


/// The private stage reader deliberately has no Store constructor or writer runtime.
/// Its caller must still qualify/capacity-bind the eventual copy and publication lifecycle.
pub(super) fn write_read_only(
    path: &Path,
    output: &mut impl Write,
    bodies: Option<&crate::message_body::Bodies>,
) -> Result<Header> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ).context("open the existing backup stage read-only")?;
    connection.busy_timeout(std::time::Duration::ZERO)?;
    connection.pragma_update(None, "query_only", true)?;
    let result = write_read_only_connection(&connection, output, bodies);
    match connection.close() {
        Ok(()) => result,
        Err((_connection, error)) => match result {
            Ok(_) => Err(error).context("close the read-only backup stage"),
            Err(primary) => Err(primary).context(format!("closing the read-only backup stage also failed: {error}")),
        },
    }
}

pub(super) fn write_read_only_connection(
    connection: &Connection,
    output: &mut impl Write,
    bodies: Option<&crate::message_body::Bodies>,
) -> Result<Header> {
    anyhow::ensure!(connection.is_autocommit(), "backup stage reader already holds a transaction");
    connection.execute_batch("BEGIN")?;
    anyhow::ensure!(!connection.is_autocommit(), "backup stage BEGIN did not start a transaction");
    let result = (|| {
        ready(connection)?;
        let owned = owned_bodies(connection, bodies)?;
        // Match the production default runtime's complete projection-version identity,
        // not only its inner claim-registry/layout digest. A path has no custom Runtime:
        // this writes the DEFAULT-registry identity, never a certificate for a custom
        // registry or an arbitrary file's producing build. A custom-registry restore
        // sees a different identity under its existing refusal/reporting rules.
        let registry = crate::store::runtime().schema_digest();
        write(connection, &registry, output, bodies, &owned)
    })();
    // No implicit transaction destructor conceals the end of this read snapshot.
    let cleanup = connection.execute_batch("ROLLBACK")
        .map_err(anyhow::Error::from)
        .and_then(|()| {
            anyhow::ensure!(connection.is_autocommit(), "backup stage ROLLBACK left its snapshot active");
            Ok(())
        });
    match (result, cleanup) {
        (Ok(header), Ok(())) => Ok(header),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(_), Err(error)) => Err(error).context("end the read-only backup stage snapshot"),
        (Err(primary), Err(error)) => Err(primary).context(format!("ending the read-only backup stage snapshot also failed: {error:#}")),
    }
}

/// Refuse a stage that would require ordinary Store opening to migrate, rebuild or seal it.
/// This recognizes this build's installed digest layout; it is not arbitrary-corruption proof
/// or a producer/authority certificate. The complete missing-envelope search is intentional:
/// the persisted sealing cursor alone can hide a skipped older batch.
fn ready(connection: &Connection) -> Result<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    anyhow::ensure!(version == CURRENT_SCHEMA_VERSION,
        "backup stage requires the current installed storage version; no migration is performed");
    let registry = PROJECTION_DIGEST_TABLES.iter()
        .map(|(table, excluded)| Ok((*table, projection_digest::columns(connection, table, excluded)?)))
        .collect::<Result<Vec<_>>>()?;
    let expected = serde_json::to_string(&(7, &registry))?;
    let installed: Option<String> = connection.query_row(
        "SELECT value FROM meta WHERE key='projection_digest_registry'", [], |row| row.get(0),
    ).optional()?;
    anyhow::ensure!(installed.as_deref() == Some(expected.as_str()),
        "backup stage projection digest layout is unavailable or incompatible; no rebuild is performed");
    let mut expected_columns = registry.into_iter()
        .map(|(table, columns)| Ok((table.to_owned(), serde_json::to_string(&columns)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    expected_columns.insert("claim_sources".into(), "[\"id\",\"accepted_at_unix_ms\"]".into());
    let installed_columns = connection.prepare(
        "SELECT table_name,columns_json FROM projection_digest_state ORDER BY table_name",
    )?.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    anyhow::ensure!(installed_columns == expected_columns,
        "backup stage projection digest rows are unavailable or incompatible; no rebuild is performed");
    let cursor: String = connection.query_row(
        "SELECT value FROM meta WHERE key='seeded_batch_rowid'", [], |row| row.get(0),
    )?;
    anyhow::ensure!(max_batch_rowid(connection)? <= cursor.parse::<i64>()?,
        "backup stage has an unsealed batch frontier; no sealing is performed");
    let skipped: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM batches INDEXED BY sqlite_autoindex_batches_1
         WHERE NOT EXISTS (SELECT 1 FROM replica_envelopes WHERE replica_envelopes.batch_id=batches.id)
           AND EXISTS (SELECT 1 FROM claims WHERE claims.batch_id=batches.id
             AND NOT EXISTS (SELECT 1 FROM checkpoint_claims WHERE checkpoint_claims.id=claims.id)))",
        [], |row| row.get(0),
    )?;
    anyhow::ensure!(!skipped,
        "backup stage has retained claims without an envelope, including below the sealing frontier; no sealing is performed");
    Ok(())
}
