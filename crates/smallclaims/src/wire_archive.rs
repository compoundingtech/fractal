//! Lossless, opt-in wire evidence segments. This does not retire any hot rows.
//!
//! A seal is an externally retained SHA-256 of the entire immutable file, not a digest supplied
//! by the file itself. Restore copies wire evidence only; graph admission, checkpoint manifests,
//! ledgers and shard restoration remain the logical backup protocol's responsibility.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use ciborium::Value as Cbor;
use rusqlite::{Connection, Transaction, params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ReplicaEnvelopeId;

mod readers;
pub use readers::{ArchiveReaders, ArchivedRecord};

const MAGIC: &[u8; 16] = b"fractal-wire\0\0\0\x01";
const MAX_ENTRY: u64 = 128 * 1024 * 1024;
const MAX_INDEX: u64 = 1024 * 1024;
/// Segment size is bounded by envelope count as well as individual encoded entry size.
pub const MAX_ENVELOPES: usize = 512;
const TABLES: [(&str, &str, &str); 3] = [
    ("replica_envelopes", "writer,sequence,envelope_hash,previous_hash,accepted_at_unix_ms,payload,batch_id,relay,receipt_state,validation_error,received_at_unix_ms", "writer,sequence,envelope_hash"),
    ("replica_records", "record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,subject_hint,kind_hint,error_code,error_message,replacement_claim_id,updated_at_unix_ms", "position"),
    ("replica_envelope_signatures", "writer,sequence,envelope_hash,member_key,signature,stored_at_unix_ms", "member_key"),
];

/// Exact SQLite values, including malformed record bytes and receipt/repair metadata.
/// Table order is envelopes, records, signatures; columns are the version-one lists above.
#[derive(Debug, PartialEq)]
pub struct WireEvidence {
    pub id: ReplicaEnvelopeId,
    pub rows: [Vec<Vec<Value>>; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SegmentSeal {
    pub sha256: String,
    pub bytes: u64,
    pub envelopes: usize,
}

#[derive(Serialize, Deserialize)]
struct Address {
    id: ReplicaEnvelopeId,
    offset: u64,
}

fn digest(file: &mut File) -> Result<String> {
    file.rewind()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub(crate) fn capture(source: &Connection, id: &ReplicaEnvelopeId) -> Result<WireEvidence> {
    let mut evidence = WireEvidence { id: id.clone(), rows: Default::default() };
    let mut captured_bytes = 0_u64;
    for (index, (table, columns, order)) in TABLES.iter().enumerate() {
        let mut statement = source.prepare(/* sqlite-large-value: allow-for-now lv-hc952cc1994fe owner=smallclaims::wire_archive TODO=trace-statement-and-value-representation */ &format!(
            "SELECT {columns} FROM {table} WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 ORDER BY {order}"
        ))?;
        let count = statement.column_count();
        let mut rows = statement.query(params![id.writer, id.sequence, id.hash])?;
        while let Some(row) = rows.next()? {
            let mut cells = Vec::with_capacity(count);
            for column in 0..count {
                let value = row.get_ref(column)?;
                captured_bytes += match value {
                    rusqlite::types::ValueRef::Text(v) | rusqlite::types::ValueRef::Blob(v) => v.len() as u64,
                    _ => 8,
                };
                ensure!(captured_bytes <= MAX_ENTRY, "wire evidence exceeds segment entry limit");
                cells.push(match value {
                    rusqlite::types::ValueRef::Text(v) => Value::Text(std::str::from_utf8(v)?.to_owned()),
                    other => Value::from(other),
                });
            }
            evidence.rows[index].push(cells);
        }
    }
    ensure!(evidence.rows[0].len() == 1, "archive envelope does not exist: {id:?}");
    Ok(evidence)
}

struct Cells<'a>(&'a [Value]);

impl Serialize for Cells<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        struct Cell<'a>(&'a Value);
        impl Serialize for Cell<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
                match self.0 {
                    Value::Null => serializer.serialize_none(),
                    Value::Integer(v) => serializer.serialize_i64(*v),
                    Value::Real(v) => serializer.serialize_f64(*v),
                    Value::Text(v) => serializer.serialize_str(v),
                    Value::Blob(v) => serializer.serialize_bytes(v),
                }
            }
        }
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for cell in self.0 { sequence.serialize_element(&Cell(cell))?; }
        sequence.end()
    }
}

struct Rows<'a>(&'a [Vec<Value>]);

impl Serialize for Rows<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for row in self.0 { sequence.serialize_element(&Cells(row))?; }
        sequence.end()
    }
}

fn encode(evidence: &WireEvidence) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let rows = evidence.rows.each_ref().map(|rows| Rows(rows));
    ciborium::into_writer(&(&evidence.id, rows), &mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_ENTRY, "wire evidence exceeds segment entry limit");
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<WireEvidence> {
    let mut input = bytes;
    let (id, rows): (ReplicaEnvelopeId, Vec<Vec<Vec<Cbor>>>) = ciborium::from_reader(&mut input)?;
    ensure!(input.is_empty() && rows.len() == 3, "invalid wire evidence entry");
    let mut result = WireEvidence { id, rows: Default::default() };
    for (index, table) in rows.into_iter().enumerate() {
        for row in table {
            ensure!(row.len() == TABLES[index].1.split(',').count(), "wire evidence column mismatch");
            result.rows[index].push(row.into_iter().map(|cell| Ok(match cell {
                Cbor::Null => Value::Null,
                Cbor::Integer(v) => Value::Integer(v.try_into().context("wire integer out of range")?),
                Cbor::Float(v) => Value::Real(v),
                Cbor::Text(v) => Value::Text(v),
                Cbor::Bytes(v) => Value::Blob(v),
                _ => bail!("invalid SQLite cell in wire archive"),
            })).collect::<Result<_>>()?);
        }
    }
    ensure!(result.rows[0].len() == 1, "wire archive must contain one envelope per entry");
    Ok(result)
}

/// Seal explicitly selected evidence from one SQLite snapshot. No source row is changed.
/// Publication is no-clobber and only follows file sync; a failed write never publishes a segment.
/// Use an offline copy: a large source snapshot can otherwise pin a live WAL.
pub fn write_segment(source: &mut Connection, ids: &[ReplicaEnvelopeId], destination: &Path) -> Result<SegmentSeal> {
    ensure!(!ids.is_empty() && ids.len() <= MAX_ENVELOPES, "invalid segment envelope count");
    ensure!(ids.windows(2).all(|pair| pair[0] < pair[1]), "segment identities must be strictly ordered");
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let temporary = directory.join(format!(".{}.wire-segment", uuid::Uuid::now_v7()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        let snapshot = source.transaction()?;
        file.write_all(MAGIC)?;
        let mut index = Vec::with_capacity(ids.len());
        for id in ids {
            let bytes = encode(&capture(&snapshot, id)?)?;
            index.push(Address { id: id.clone(), offset: file.stream_position()? });
            file.write_all(&(bytes.len() as u64).to_be_bytes())?;
            file.write_all(&bytes)?;
            file.write_all(&Sha256::digest(&bytes))?;
        }
        snapshot.commit()?;
        let mut encoded_index = Vec::new();
        ciborium::into_writer(&index, &mut encoded_index)?;
        ensure!(encoded_index.len() as u64 <= MAX_INDEX, "segment index exceeds limit");
        file.write_all(&encoded_index)?;
        file.write_all(&(encoded_index.len() as u64).to_be_bytes())?;
        file.sync_all()?;
        let seal = SegmentSeal { sha256: digest(&mut file)?, bytes: file.metadata()?.len(), envelopes: ids.len() };
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(true);
        file.set_permissions(permissions)?;
        file.sync_all()?;
        fs::hard_link(&temporary, destination).context("publish wire segment without replacing an existing file")?;
        File::open(directory)?.sync_all()?;
        Ok(seal)
    })();
    // The temporary name is ours exclusively, including when publication fails.
    if temporary.exists() { fs::remove_file(&temporary)?; }
    result
}

/// Verified indexed reader. A caller must keep the segment immutable for the reader's lifetime.
pub struct SegmentReader {
    file: File,
    index: BTreeMap<ReplicaEnvelopeId, u64>,
    index_start: u64,
}

impl SegmentReader {
    /// Check the externally retained seal before exposing even one record.
    pub fn open(path: &Path, seal: &SegmentSeal) -> Result<Self> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        ensure!(size == seal.bytes && size >= 24, "wire segment size mismatch");
        ensure!(digest(&mut file)? == seal.sha256, "wire segment seal mismatch");
        file.rewind()?;
        let mut magic = [0; 16];
        file.read_exact(&mut magic)?;
        ensure!(&magic == MAGIC, "unsupported wire segment format/version");
        file.seek(SeekFrom::End(-8))?;
        let mut length = [0; 8];
        file.read_exact(&mut length)?;
        let length = u64::from_be_bytes(length);
        ensure!(length <= MAX_INDEX && length <= size - 24, "invalid wire index length");
        let index_start = size - 8 - length;
        file.seek(SeekFrom::Start(index_start))?;
        let mut bytes = vec![0; length as usize];
        file.read_exact(&mut bytes)?;
        let mut input = bytes.as_slice();
        let addresses: Vec<Address> = ciborium::from_reader(&mut input)?;
        ensure!(input.is_empty() && !addresses.is_empty() && addresses.len() <= MAX_ENVELOPES && addresses.len() == seal.envelopes, "invalid wire index count");
        ensure!(addresses[0].offset == 16 && addresses.windows(2).all(|p| p[0].id < p[1].id && p[0].offset < p[1].offset), "noncanonical wire index");
        ensure!(addresses.iter().all(|a| a.offset < index_start), "wire index address outside entries");
        Ok(Self { file, index: addresses.into_iter().map(|a| (a.id, a.offset)).collect(), index_start })
    }

    pub fn identities(&self) -> impl Iterator<Item = &ReplicaEnvelopeId> { self.index.keys() }

    /// Read the exact archived representation, never reconstruct raw bytes from decoded claims.
    pub fn evidence(&mut self, id: &ReplicaEnvelopeId) -> Result<WireEvidence> {
        let offset = *self.index.get(id).context("envelope absent from wire segment")?;
        let end = self.index.range((std::ops::Bound::Excluded(id), std::ops::Bound::Unbounded))
            .next().map_or(self.index_start, |(_, offset)| *offset);
        self.file.seek(SeekFrom::Start(offset))?;
        let mut length = [0; 8];
        self.file.read_exact(&mut length)?;
        let length = u64::from_be_bytes(length);
        ensure!(length <= MAX_ENTRY && offset.checked_add(40).and_then(|v| v.checked_add(length)) == Some(end), "invalid wire entry length");
        let mut bytes = vec![0; length as usize];
        self.file.read_exact(&mut bytes)?;
        let mut expected = [0; 32];
        self.file.read_exact(&mut expected)?;
        ensure!(Sha256::digest(&bytes)[..] == expected, "wire entry digest mismatch");
        let evidence = decode(&bytes)?;
        ensure!(&evidence.id == id, "wire entry identity mismatch");
        Ok(evidence)
    }

    /// Restore evidence atomically into empty matching wire tables. Conflicts fail, never replace.
    /// This is not graph restore: use logical backup to restore claims, ledgers and projections.
    pub fn restore_wire_rows(&mut self, destination: &mut Connection) -> Result<()> {
        let transaction = destination.transaction()?;
        let identities = self.index.keys().cloned().collect::<Vec<_>>();
        for id in &identities {
            insert(&transaction, &self.evidence(id)?)?;
        }
        // Validate after all INSERTs: a later trigger can also alter an earlier row.
        // Read back every typed cell before committing, never publish coerced data.
        for id in &identities {
            ensure!(capture(&transaction, id)? == self.evidence(id)?, "destination changed archived wire cell types or values");
        }
        transaction.commit()?;
        Ok(())
    }
}

fn insert(transaction: &Transaction<'_>, evidence: &WireEvidence) -> Result<()> {
    for (index, rows) in evidence.rows.iter().enumerate() {
        let (table, columns, _) = TABLES[index];
        let placeholders = std::iter::repeat_n("?", columns.split(',').count()).collect::<Vec<_>>().join(",");
        let mut statement = transaction.prepare(&format!(
            /* sqlite-large-value: allow-for-now lv-hec5759080577 owner=smallclaims::wire_archive TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO {table}({columns}) VALUES({placeholders})"
        ))?;
        for row in rows {
            statement.execute(/* sqlite-large-value: allow-for-now lv-habef37fe7d69 owner=smallclaims::wire_archive TODO=trace-statement-and-value-representation */ params_from_iter(row))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
