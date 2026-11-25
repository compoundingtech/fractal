//! Inactive, lossless claim evidence segments. No hot rows are removed.
//!
//! Only the native checkpoint planner may select evidence for export. Default graph readers,
//! admission, nonce protection, operation receipts and checkpoint policy are unchanged.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ClaimRecord, store::operation_parts};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_CLAIMS: usize = 512;
const FORMAT: &str = "fractal-claim-evidence-v1";

/// Supported hot windows; selection between them requires offline measured capacity evidence.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum HotWindow { Days14, Days30 }

impl HotWindow {
    pub fn milliseconds(self) -> u128 {
        (match self { Self::Days14 => 14, Self::Days30 => 30 }) * 86_400_000
    }
}

/// Exact stored claim texts, not a reserialization of decoded JSON. Arrival index is local
/// evidence, not the canonical cross-member ordering key. These bytes alone are not a backup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimEvidence {
    pub id: String,
    pub store_index: u64,
    pub batch_id: String,
    pub subject: String,
    pub kind: String,
    pub origin: String,
    pub actor: Option<String>,
    pub body: String,
    pub predecessors: String,
    pub accepted_at_unix_ms: String,
}

impl ClaimEvidence {
    pub(crate) fn capture(connection: &Connection, id: &str) -> Result<Option<Self>> {
        Ok(connection.query_row("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE id=?1", [id], |row| {
            Ok(Self { id: row.get(0)?, store_index: row.get(1)?, batch_id: row.get(2)?,
                subject: row.get(3)?, kind: row.get(4)?, origin: row.get(5)?, actor: row.get(6)?,
                body: row.get(7)?, predecessors: row.get(8)?, accepted_at_unix_ms: row.get(9)? })
        }).optional()?)
    }

    /// Same decoding contract as the native hot point reader, retaining raw evidence separately.
    pub fn record(&self) -> ClaimRecord {
        let body = serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null);
        ClaimRecord { id: self.id.clone(), store_index: self.store_index, batch_id: self.batch_id.clone(),
            subject: self.subject.clone(), kind: self.kind.clone(), origin: self.origin.clone(),
            actor: self.actor.clone(), operation_id: operation_parts(&body).map(|(id, _)| id.to_owned()),
            request_digest: operation_parts(&body).map(|(_, digest)| digest.to_owned()), body,
            predecessors: serde_json::from_str(&self.predecessors).unwrap_or_default(),
            accepted_at_unix_ms: self.accepted_at_unix_ms.parse().unwrap_or_default() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimSeal { pub sha256: String, pub bytes: u64, pub claims: usize }

#[derive(Serialize, Deserialize)]
struct Segment {
    format: String,
    rules_digest: String,
    as_of_unix_ms: u128,
    window: HotWindow,
    claims: Vec<ClaimEvidence>,
}

struct SealedWriter {
    file: std::io::BufWriter<File>,
    hash: Sha256,
    bytes: u64,
}

impl Write for SealedWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.len() as u64 > MAX_BYTES - self.bytes {
            return Err(std::io::Error::other("claim segment exceeds byte limit"));
        }
        let written = self.file.write(buffer)?;
        self.hash.update(&buffer[..written]);
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> { self.file.flush() }
}

pub(crate) fn write_selected(claims: Vec<ClaimEvidence>, rules_digest: String, as_of_unix_ms: u128,
    window: HotWindow, destination: &Path) -> Result<ClaimSeal> {
    ensure!(!claims.is_empty() && claims.len() <= MAX_CLAIMS, "invalid claim segment count");
    ensure!(claims.windows(2).all(|pair| pair[0].id < pair[1].id), "claim identities must be strictly ordered");
    let cutoff = as_of_unix_ms.checked_sub(window.milliseconds()).ok_or_else(|| anyhow::anyhow!("hot window exceeds observation time"))?;
    for claim in &claims {
        let accepted: u128 = claim.accepted_at_unix_ms.parse()?;
        ensure!(accepted < cutoff, "claim is within the protected hot window: {}", claim.id);
    }
    let count = claims.len();
    let segment = Segment { format: FORMAT.into(), rules_digest, as_of_unix_ms, window, claims };
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let temporary = directory.join(format!(".{}.claim-segment", uuid::Uuid::now_v7()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&temporary)?;
        // Stream bounded output and its seal without allocating another copy of all bodies.
        let mut output = SealedWriter { file: std::io::BufWriter::new(file), hash: Sha256::new(), bytes: 0 };
        serde_json::to_writer(&mut output, &segment)?;
        output.flush()?;
        output.file.get_ref().sync_all()?;
        let seal = ClaimSeal { sha256: hex::encode(output.hash.finalize()), bytes: output.bytes, claims: count };
        let file = output.file.into_inner()?;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(true);
        file.set_permissions(permissions)?;
        file.sync_all()?;
        fs::hard_link(&temporary, destination)?;
        File::open(directory)?.sync_all()?;
        Ok(seal)
    })();
    if temporary.exists() { fs::remove_file(&temporary)?; }
    result
}

/// Verified immutable evidence held in memory: later path replacement cannot change a read.
/// Construct one reader per bounded segment; seals must come from a trusted external catalogue.
pub struct ClaimArchive { claims: BTreeMap<String, ClaimEvidence> }

impl ClaimArchive {
    pub fn open(path: &Path, seal: &ClaimSeal) -> Result<Self> {
        ensure!(seal.bytes <= MAX_BYTES && seal.claims <= MAX_CLAIMS && seal.claims > 0, "invalid claim seal bounds");
        let mut file = File::open(path)?;
        ensure!(file.metadata()?.len() == seal.bytes, "claim archive length mismatch");
        let mut bytes = Vec::new();
        Read::by_ref(&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 == seal.bytes && hex::encode(Sha256::digest(&bytes)) == seal.sha256, "claim archive seal mismatch");
        let segment: Segment = serde_json::from_slice(&bytes)?;
        ensure!(segment.format == FORMAT && segment.claims.len() == seal.claims, "claim archive format/count mismatch");
        ensure!(segment.claims.windows(2).all(|pair| pair[0].id < pair[1].id), "duplicate or unordered archived claim");
        Ok(Self { claims: segment.claims.into_iter().map(|claim| (claim.id.clone(), claim)).collect() })
    }

    pub fn evidence(&self, id: &str) -> Option<&ClaimEvidence> { self.claims.get(id) }
}

impl crate::Store {
    /// Explicit historical evidence read, not an admission/authority query. Contradictory hot
    /// and cold evidence fails closed; absence is only authoritative for the supplied segments.
    pub fn claim_by_id_with_archive(&self, id: &str, archives: &[ClaimArchive]) -> Result<Option<ClaimRecord>> {
        let connection = self.readers.get();
        let hot = ClaimEvidence::capture(&connection, id)?;
        let mut selected = hot.as_ref();
        for archive in archives {
            if let Some(cold) = archive.evidence(id) {
                if let Some(previous) = selected { ensure!(previous == cold, "contradictory archived claim: {id}"); }
                selected = Some(cold);
            }
        }
        crate::read_budget::check()?;
        Ok(selected.map(ClaimEvidence::record))
    }
}

#[cfg(test)]
mod tests;
