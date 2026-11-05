//! The shards a backup carries beside the graph, and their restore after it.
//!
//! The messages shard (`super::messages_shard`) is a store of its own. Its stream items are
//! envelopes it replicates under its own shard ID, and this member's staging, delivery, read and
//! close of each item is a local table no envelope carries. An archive carries both once the
//! shard holds either, which makes it [`super::backup_format::VERSION_WITH_SHARDS`]; while the
//! shard holds neither, the archive is byte for byte what it was before shards, so every older
//! build still restores it.
//!
//! The export reads the shard in its own read transaction, pinned after the graph's snapshot and
//! held while the whole archive is written. The shard keeps no checkpoint and never trims, so its
//! log only grows: the shard snapshot holds every stream item the graph snapshot could know of,
//! and perhaps some that arrived after it. Such an item restores as an item that arrives ahead of
//! its graph context does on any member: it is admitted, and a reader resolves its parties against
//! the graph it has. An item older than the graph snapshot is simply in both. Each lifecycle row
//! is read in the same shard snapshot as the items it moves.
//!
//! Restore opens a fresh shard beside the staged graph, bound to it as the daemon binds it,
//! receives the archived envelopes with each signature verified in the shard's signing domain,
//! and adds the local rows by union. Once the graph is restored and checked, the shard's own
//! admission admits the envelopes on the source's authority, as the graph's restore does: the
//! source admitted every one, and judging them by the restored membership would hold or fence the
//! history of a member removed or rekeyed since. The shard's runtime still classifies every
//! claim, and every envelope must be admitted. The restored shard's log digest, envelope count
//! and local table digests then have to equal the header's.
use super::backup_format::{
    Header, PAGE, Record, ShardDigest, ShardEnvelopeRecord, ShardLocalPage, ShardSignaturesRecord,
};
use super::backup_ledgers::LedgerPage;
use super::messages_shard::SHARD_ID;
use super::*;
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicaEnvelopeSignature};
use std::path::{Path, PathBuf};

/// Where a store root keeps the messages shard's database, as `messages_shard::open` places it.
pub fn database_path(root: &Path) -> Result<PathBuf> {
    Ok(smallclaims::shards::ShardId::parse(SHARD_ID)?.database_path(root))
}

/// What the shard holds, read in one snapshot, or `None` when it holds neither an admitted
/// envelope nor a local row. Run inside the snapshot the archive is written from, it is what
/// [`write_records`] writes.
pub fn digest(shard: &GraphStore) -> Result<Option<ShardDigest>> {
    shard.read_snapshot(|_| {
        let (envelope_count, local) = {
            let connection = shard.readers.get();
            let count: u64 = connection.query_row(
                "SELECT COUNT(*) FROM replica_envelopes WHERE batch_id IS NOT NULL",
                [],
                |row| row.get(0),
            )?;
            (count, super::backup_ledgers::shard_local_digests(&connection)?)
        };
        if envelope_count == 0 && local.is_empty() {
            return Ok(None);
        }
        Ok(Some(ShardDigest {
            log_digest: super::backup::log_digest(shard)?,
            envelope_count,
            local,
        }))
    })
}

/// Every admitted envelope of the shard in writer, sequence and hash order, each page's
/// signatures after it, and then every local row, in pages.
pub fn write_records(shard: &GraphStore, mut write: impl FnMut(&Record) -> Result<()>) -> Result<()> {
    let mut after = (String::new(), 0_u64, String::new());
    loop {
        let identities = shard
            .readers
            .get()
            .prepare_cached(
                "SELECT writer,sequence,envelope_hash FROM replica_envelopes
                 WHERE (writer,sequence,envelope_hash)>(?1,?2,?3) AND batch_id IS NOT NULL
                 ORDER BY writer,sequence,envelope_hash LIMIT ?4",
            )?
            .query_map(params![after.0, after.1, after.2, PAGE], |row| {
                Ok(ReplicaEnvelopeId {
                    writer: row.get(0)?,
                    sequence: row.get(1)?,
                    hash: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some(last) = identities.last() else {
            break;
        };
        after = (last.writer.clone(), last.sequence, last.hash.clone());
        let signatures = shard.replication_signatures_for(&identities)?;
        for envelope in shard.replica_envelopes(identities)? {
            write(&Record::ShardEnvelope(ShardEnvelopeRecord {
                shard: SHARD_ID.into(),
                envelope,
            }))?;
        }
        if !signatures.is_empty() {
            write(&Record::ShardSignatures(ShardSignaturesRecord {
                shard: SHARD_ID.into(),
                signatures,
            }))?;
        }
    }
    super::backup_ledgers::write_shard_local_pages(&shard.readers.get(), |page| {
        write(&Record::ShardLocal(ShardLocalPage {
            shard: SHARD_ID.into(),
            page,
        }))
    })
}

/// Whether a signature is the writer's for this shard envelope in the archive's fleet.
fn verified(fleet: &str, writer: &str, sequence: u64, hash: &str, key: &str, signature: &str) -> bool {
    let message =
        smallclaims::fleet::envelope_signature_message_in(Some(SHARD_ID), fleet, writer, sequence, hash);
    smallclaims::fleet::verify_signature(key, &message, signature)
}

/// Hold one page of archived shard envelopes as pending, each signature verified in the shard's
/// signing domain and kept. The envelopes must come in the exporter's order, without repeats.
pub fn receive_envelopes(
    shard: &GraphStore,
    header: &Header,
    envelopes: &[ReplicaEnvelope],
) -> Result<()> {
    let fleet = header.fleet_id.as_deref().unwrap_or("");
    let mut connection = shard.connection.write();
    let transaction = connection.transaction()?;
    for envelope in envelopes {
        match (&envelope.member_key, &envelope.signature) {
            (Some(key), Some(signature)) => {
                anyhow::ensure!(
                    verified(fleet, &envelope.writer, envelope.sequence, &envelope.hash, key, signature),
                    "invalid backup shard envelope signature"
                );
                store_envelope_signature_tx(
                    &transaction,
                    Some(SHARD_ID),
                    fleet,
                    &envelope.writer,
                    envelope.sequence,
                    &envelope.hash,
                    key,
                    signature,
                    &now_ms().to_string(),
                )?;
            }
            (None, None) => {} // A shard outside a fleet signs nothing.
            _ => anyhow::bail!("incomplete backup shard envelope signature"),
        }
        let previous: Option<(u64, String)> = transaction
            .query_row(
                "SELECT sequence,envelope_hash FROM replica_envelopes WHERE writer=?1
                 ORDER BY sequence DESC,envelope_hash DESC LIMIT 1",
                [&envelope.writer],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((sequence, hash)) = previous {
            anyhow::ensure!(
                (envelope.sequence, envelope.hash.as_str()) > (sequence, hash.as_str()),
                "backup shard envelopes are duplicated or out of order"
            );
        }
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h51f69bd9d8cb owner=fractal_store::store::backup_shards TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes(writer,sequence,envelope_hash,previous_hash,accepted_at_unix_ms,
                 payload,relay,receipt_state,received_at_unix_ms)
             VALUES(?1,?2,?3,?4,?5,?6,'backup','pending',?7)",
            params![envelope.writer, envelope.sequence, envelope.hash, envelope.previous_hash,
                envelope.accepted_at_unix_ms.to_string(), envelope.payload, now_ms().to_string()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// Keep the archived signatures of shard envelopes, each verified in the shard's signing domain.
pub fn receive_signatures(
    shard: &GraphStore,
    header: &Header,
    signatures: &[ReplicaEnvelopeSignature],
) -> Result<()> {
    let fleet = header.fleet_id.as_deref().unwrap_or("");
    let mut connection = shard.connection.write();
    let transaction = connection.transaction()?;
    for signature in signatures {
        anyhow::ensure!(
            verified(
                fleet,
                &signature.writer,
                signature.sequence,
                &signature.hash,
                &signature.member_key,
                &signature.signature
            ),
            "invalid backup shard envelope signature"
        );
        store_envelope_signature_tx(
            &transaction,
            Some(SHARD_ID),
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

/// Add one archived page of the shard's local rows by union. Admission reads none of them.
pub fn receive_local(shard: &GraphStore, page: &LedgerPage) -> Result<()> {
    let mut connection = shard.connection.write();
    let transaction = connection.transaction()?;
    super::backup_ledgers::union_shard_local_page(&transaction, page)?;
    transaction.commit()?;
    Ok(())
}

/// Admit the received shard envelopes and check the restored shard against the header. Run
/// after the graph is restored, so the shard's fleet authority is the restored graph.
pub fn finish_restore(shard: &GraphStore, header: &Header) -> Result<ShardDigest> {
    let expected = header
        .shards
        .get(SHARD_ID)
        .context("the backup header names no messages shard")?;
    shard.admit_restored_backlog()?;
    shard.apply_replication_repairs()?;
    shard.project_replication_backlog()?;
    let unresolved: u64 = shard.readers.get().query_row(
        "SELECT COUNT(*) FROM replica_envelopes WHERE batch_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        unresolved == 0,
        "backup contains {unresolved} unadmitted messages shard envelopes"
    );
    let actual = digest(shard)?.unwrap_or_default();
    anyhow::ensure!(
        actual == *expected,
        "restored messages shard differs from backup: expected {expected:?}, got {actual:?}"
    );
    Ok(actual)
}
