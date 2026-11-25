//! Explicitly supplied immutable readers. No store opens these by default.
use super::*;
use crate::claim::{ReplicaEnvelope, ReplicaEnvelopeSignature};
use crate::replication::ReplicaRecordView;

/// A pinned collection of already verified segments. Duplicate coverage must agree exactly.
/// Production catalogue/fleet binding and mutable overlays are deliberately not activated here.
pub struct ArchiveReaders {
    segments: Vec<SegmentReader>,
    locations: BTreeMap<ReplicaEnvelopeId, Vec<usize>>,
}

impl ArchiveReaders {
    pub fn new(segments: Vec<SegmentReader>) -> Self {
        let mut locations: BTreeMap<ReplicaEnvelopeId, Vec<usize>> = BTreeMap::new();
        for (index, segment) in segments.iter().enumerate() {
            for id in segment.identities() { locations.entry(id.clone()).or_default().push(index); }
        }
        Self { segments, locations }
    }

    pub fn identities(&self) -> impl Iterator<Item = &ReplicaEnvelopeId> { self.locations.keys() }

    pub fn evidence(&mut self, id: &ReplicaEnvelopeId) -> Result<Option<WireEvidence>> {
        let Some(locations) = self.locations.get(id) else { return Ok(None); };
        let mut evidence = None;
        for index in locations {
            let next = self.segments[*index].evidence(id)?;
            if let Some(previous) = &evidence {
                ensure!(previous == &next, "contradictory archived wire evidence for {id:?}");
            } else { evidence = Some(next); }
        }
        Ok(evidence)
    }
}

/// Original cold record bytes together with the native repair/diagnostic interpretation.
/// Reading does not requeue a receipt or mutate an archived repair state.
pub struct ArchivedRecord {
    pub view: ReplicaRecordView,
    pub raw: Vec<u8>,
    pub updated_at_unix_ms: String,
}

fn take(row: &mut [Value], index: usize) -> Value { std::mem::replace(&mut row[index], Value::Null) }
fn text(value: Value) -> Result<String> {
    match value { Value::Text(value) => Ok(value), _ => bail!("archive cell is not TEXT") }
}
fn optional_text(value: Value) -> Result<Option<String>> {
    match value { Value::Null => Ok(None), value => Ok(Some(text(value)?)) }
}
fn bytes(value: Value) -> Result<Vec<u8>> {
    match value { Value::Blob(value) => Ok(value), _ => bail!("archive cell is not BLOB") }
}
fn integer(value: Value) -> Result<u64> {
    match value { Value::Integer(value) => Ok(value.try_into()?), _ => bail!("archive cell is not INTEGER") }
}

impl WireEvidence {
    /// Convert only the outer envelope; original payload bytes are moved, never interpreted.
    pub fn into_replica_envelope(mut self) -> Result<ReplicaEnvelope> {
        let signatures = self.rows[2].first_mut().map(|row| -> Result<_> {
            ensure!(row.len() == 6, "archive signature column mismatch");
            Ok((text(take(row, 3))?, text(take(row, 4))?))
        }).transpose()?;
        let row = self.rows[0].first_mut().context("archive envelope row absent")?;
        ensure!(row.len() == 11, "archive envelope column mismatch");
        ensure!(text(take(row, 0))? == self.id.writer && integer(take(row, 1))? == self.id.sequence
            && text(take(row, 2))? == self.id.hash, "archive row identity mismatch");
        let previous_hash = optional_text(take(row, 3))?;
        let accepted_at_unix_ms = text(take(row, 4))?.parse()?;
        let payload = bytes(take(row, 5))?.into();
        let (member_key, signature) = signatures.map_or((None, None), |(key, signature)| (Some(key), Some(signature)));
        Ok(ReplicaEnvelope { writer: self.id.writer, sequence: self.id.sequence, hash: self.id.hash,
            previous_hash, accepted_at_unix_ms, payload, member_key, signature })
    }

    pub fn into_replication_signatures(self) -> Result<Vec<ReplicaEnvelopeSignature>> {
        let WireEvidence { id, rows: [_, _, rows] } = self;
        rows.into_iter().map(|mut row| {
            ensure!(row.len() == 6, "archive signature column mismatch");
            ensure!(text(take(&mut row, 0))? == id.writer && integer(take(&mut row, 1))? == id.sequence
                && text(take(&mut row, 2))? == id.hash, "archive signature identity mismatch");
            Ok(ReplicaEnvelopeSignature { writer: id.writer.clone(), sequence: id.sequence,
                hash: id.hash.clone(), member_key: text(take(&mut row, 3))?, signature: text(take(&mut row, 4))? })
        }).collect()
    }

    /// Address a repair witness by envelope and exact original position, not a decoded claim ID.
    pub fn into_record(self, position: u64) -> Result<Option<ArchivedRecord>> {
        let WireEvidence { id, rows: [_, rows, _] } = self;
        for mut row in rows {
            ensure!(row.len() == 14, "archive record column mismatch");
            let candidate = integer(take(&mut row, 4))?;
            if candidate != position { continue; }
            let record_ref = text(take(&mut row, 0))?;
            let writer = text(take(&mut row, 1))?;
            let sequence = integer(take(&mut row, 2))?;
            let envelope_hash = text(take(&mut row, 3))?;
            ensure!(writer == id.writer && sequence == id.sequence && envelope_hash == id.hash,
                "archive record identity mismatch");
            return Ok(Some(ArchivedRecord {
                raw: bytes(take(&mut row, 5))?,
                view: ReplicaRecordView { record_ref, writer, sequence, envelope_hash, position,
                    state: text(take(&mut row, 6))?, claim_id: optional_text(take(&mut row, 7))?,
                    subject: optional_text(take(&mut row, 8))?, kind: optional_text(take(&mut row, 9))?,
                    error_code: optional_text(take(&mut row, 10))?, error_message: optional_text(take(&mut row, 11))?,
                    replacement_claim_id: optional_text(take(&mut row, 12))? },
                updated_at_unix_ms: text(take(&mut row, 13))?,
            }));
        }
        Ok(None)
    }
}
