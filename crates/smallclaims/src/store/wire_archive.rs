//! Opt-in native wire readers. No daemon path supplies an archive yet.
//! Immutable duplicate coverage must agree; later mutable overlays cannot be guessed here.
use super::*;
use crate::wire_archive::{ArchiveReaders, ArchivedRecord, WireEvidence, capture};

fn evidence(connection: &Connection, archive: &mut ArchiveReaders, id: &ReplicaEnvelopeId) -> Result<Option<WireEvidence>> {
    let cold = archive.evidence(id)?;
    let hot: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM replica_envelopes WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3)",
        params![id.writer, id.sequence, id.hash], |row| row.get(0),
    )?;
    match (hot, cold) {
        (true, Some(cold)) => {
            anyhow::ensure!(capture(connection, id)? == cold, "hot/archive wire evidence differs; overlay-aware activation required");
            Ok(Some(cold))
        }
        (true, None) => Ok(Some(capture(connection, id)?)),
        (false, cold) => Ok(cold),
    }
}

impl Store {
    /// Export original hot/cold envelope bytes using the ordinary peer envelope type.
    /// Retirement remains authoritative: an archive never resurrects checkpoint-retired payloads.
    pub fn replica_envelopes_with_archive(&self, missing: Vec<ReplicaEnvelopeId>, archive: &mut ArchiveReaders) -> Result<Vec<ReplicaEnvelope>> {
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction()?;
        let authoritative = retired::authoritative(&transaction)?;
        let mut result = Vec::with_capacity(missing.len());
        for id in missing {
            if authoritative && retired::envelope_retired(&transaction, &id.writer, id.sequence, &id.hash)? { continue; }
            if let Some(evidence) = evidence(&transaction, archive, &id)? {
                result.push(evidence.into_replica_envelope()?);
            }
        }
        Ok(result)
    }

    /// Union hot signatures independently of envelope presence with pinned cold signatures.
    pub fn replication_signatures_with_archive(&self, requests: &[ReplicaEnvelopeId], archive: &mut ArchiveReaders) -> Result<Vec<crate::claim::ReplicaEnvelopeSignature>> {
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction()?;
        let mut signatures = Self::replication_signatures_on(&transaction, requests)?;
        for id in requests.iter().take(REPLICATION_SIGNATURE_LIMIT) {
            if let Some(evidence) = archive.evidence(id)? {
                signatures.extend(evidence.into_replication_signatures()?);
            }
        }
        fn key(signature: &crate::claim::ReplicaEnvelopeSignature) -> (&str, u64, &str, &str) {
            (&signature.writer, signature.sequence, &signature.hash, &signature.member_key)
        }
        signatures.sort_unstable_by(|left, right| key(left).cmp(&key(right)));
        anyhow::ensure!(signatures.windows(2).all(|pair| key(&pair[0]) != key(&pair[1])
            || pair[0].signature == pair[1].signature), "contradictory hot/archive envelope signatures");
        signatures.dedup();
        Ok(signatures)
    }

    /// Read a cold repair witness by original envelope and position. This never performs repair.
    /// Missing envelopes are errors; an absent position in an existing envelope is None.
    pub fn replica_record_with_archive(&self, id: &ReplicaEnvelopeId, position: u64, archive: &mut ArchiveReaders) -> Result<Option<ArchivedRecord>> {
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction()?;
        evidence(&transaction, archive, id)?.context("repair witness envelope is unavailable")?.into_record(position)
    }

    /// Existing replication exchange protocol with an explicitly supplied archive read set.
    /// The snapshot is temporary and uncached; all normal writers and exchange entrypoints stay hot-only.
    pub fn export_replication_difference_with_archive(&self, fleet_id: &str, remote: &ReplicationInventory, archive: &mut ArchiveReaders) -> Result<ReplicationExchange> {
        let snapshot = self.replication_snapshot()?;
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction()?;
        let authoritative = retired::authoritative(&transaction)?;
        let mut hot = snapshot.inventory.envelopes.iter().map(|item|
            (snapshot.inventory.identity(item), !snapshot.inventory.has_payload(item))).peekable();
        let mut cold = archive.identities().peekable();
        let mut inventory = CompactReplicationInventory::default();
        while hot.peek().is_some() || cold.peek().is_some() {
            let ordering = match (hot.peek(), cold.peek()) {
                (Some((hot, _)), Some(cold)) => hot.cmp(cold),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => break,
            };
            let (id, mut payloadless, from_archive) = match ordering {
                std::cmp::Ordering::Less => {
                    let (id, payloadless) = hot.next().expect("peeked hot identity");
                    (id, payloadless, false)
                }
                std::cmp::Ordering::Equal => {
                    let (id, payloadless) = hot.next().expect("peeked hot identity");
                    cold.next();
                    (id, payloadless, true)
                }
                std::cmp::Ordering::Greater => (cold.next().expect("peeked archive identity").clone(), false, true),
            };
            if from_archive && authoritative {
                payloadless |= retired::envelope_retired(&transaction, &id.writer, id.sequence, &id.hash)?;
            }
            inventory.push_sorted(id);
            if payloadless { inventory.mark_last_payloadless(); }
        }
        drop(cold);
        inventory.refresh_digest();
        let buckets = inventory.buckets();
        drop(transaction);
        drop(connection);
        self.export_replication_difference_from(fleet_id, remote, &snapshot, &inventory, &buckets, Some(archive))
    }
}

#[cfg(test)]
mod tests;
