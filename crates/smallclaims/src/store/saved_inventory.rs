//! The replication inventory a clean shutdown leaves for the next open.
//!
//! A fresh process has no replication snapshot, and building its inventory from nothing reads
//! every envelope, checkpoint tombstone and retired identity in one query: 19.9 s on a busy
//! host's store, in the minute after a restart, while every peer's export and receive waited
//! for it. The daemon writes the cached inventory next to the database when it stops, and the
//! first snapshot after the next open reads it back and extends it by the envelopes appended
//! since, as a running process extends its own.
//!
//! The file is trusted only while nothing changed the inventory but appends. Each open writes a
//! fresh epoch into `meta`, and triggers delete it on any change to what the inventory lists or
//! which payloads it holds: deleting or rewriting an envelope, inserting one below the tail, and
//! any change to the checkpoint tombstones or the retired ledger. They are in the database, so
//! they fire for every binary that writes it, an older one too. A shutdown saves only while its
//! own open's epoch is still there, and the next open reads the file only when the epoch it
//! found is the one the file names and its own epoch is still there when it reads. The file
//! also names the envelope at its last row, which a store restored from a copy would not hold.
//! Reading consumes it, so a process that stops without saving leaves none behind.
use super::*;
use std::io::{Read as _, Write as _};

const EPOCH: &str = "replication_inventory_epoch";
const MAGIC: &[u8; 32] = b"fractal replication inventory 01";

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    let forget = format!(/* sqlite-large-value: allow-for-now lv-h73fc1394f8ea owner=smallclaims::store::saved_inventory TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='{EPOCH}';");
    for (table, event, condition) in [
        ("replica_envelopes", "DELETE", ""),
        (
            "replica_envelopes",
            "INSERT",
            "WHEN NEW.rowid < (SELECT MAX(rowid) FROM replica_envelopes)",
        ),
        (
            "replica_envelopes",
            "UPDATE",
            "WHEN OLD.rowid IS NOT NEW.rowid OR OLD.writer IS NOT NEW.writer OR OLD.sequence IS NOT NEW.sequence OR OLD.envelope_hash IS NOT NEW.envelope_hash",
        ),
        ("checkpoint_envelopes", "INSERT", ""),
        ("checkpoint_envelopes", "DELETE", ""),
        ("checkpoint_envelopes", "UPDATE", ""),
        ("retired_envelopes", "INSERT", ""),
        ("retired_envelopes", "DELETE", ""),
        ("retired_envelopes", "UPDATE", ""),
    ] {
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h6a059130c899 owner=smallclaims::store::saved_inventory TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER IF NOT EXISTS {table}_saved_inventory_{event} AFTER {event} ON {table}
             {condition} BEGIN {forget} END;"
        ))?;
    }
    Ok(())
}

/// Replace the epoch the last open wrote with a fresh one, and return the one it replaces.
pub(super) fn rotate_epoch(connection: &Connection) -> Result<(Option<String>, String)> {
    let previous = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [EPOCH],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("draw an inventory epoch: {error}"))?;
    let epoch = hex::encode(bytes);
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hcc1be495eace owner=smallclaims::store::saved_inventory TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [EPOCH, epoch.as_str()],
    )?;
    Ok((previous, epoch))
}

fn current_epoch(connection: &Connection) -> Result<Option<String>> {
    Ok(connection
        .prepare_cached("SELECT value FROM meta WHERE key=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-had5f2ddeb015 owner=smallclaims::store::saved_inventory TODO=trace-statement-and-value-representation */ [EPOCH], |row| row.get::<_, String>(0))
        .optional()?)
}

fn anchor(connection: &Connection, rowid: i64) -> Result<Option<ReplicaEnvelopeId>> {
    Ok(connection
        .prepare_cached("SELECT writer, sequence, envelope_hash FROM replica_envelopes WHERE rowid=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h166835b7b8cb owner=smallclaims::store::saved_inventory TODO=trace-statement-and-value-representation */ [rowid], |row| {
            Ok(ReplicaEnvelopeId {
                writer: row.get(0)?,
                sequence: row.get(1)?,
                hash: row.get(2)?,
            })
        })
        .optional()?)
}

/// Where a store at `database` leaves its inventory.
pub fn path(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(".replication-inventory");
    PathBuf::from(name)
}

/// What a saved inventory says about the store it was saved from.
struct Header {
    epoch: String,
    inventory_generation: i64,
    max_envelope_rowid: i64,
    envelope_rows: u64,
    retired_authoritative: bool,
    anchor: Option<ReplicaEnvelopeId>,
}

/// What a stop's save did, and when it saved nothing, why: the next start then reads the store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventorySave {
    Saved,
    Skipped(&'static str),
}

impl Store {
    /// Write the cached replication inventory next to the database for the next open, when
    /// nothing but appends changed the inventory since this one. The daemon calls this as it
    /// stops.
    pub fn save_replication_inventory(&self) -> Result<InventorySave> {
        if self.shared_memory {
            return Ok(InventorySave::Skipped("the store is in memory"));
        }
        let destination = path(&self.path);
        // Hold the build lock, so a build that read the store earlier cannot replace the
        // snapshot this saves, nor a trim's patch land after it.
        let _building = crate::read_budget::lock_sync(&self.replication_snapshot_build)?;
        let Some(snapshot) = crate::read_budget::lock_sync(&self.replication_snapshot)?.clone() else {
            return Ok(InventorySave::Skipped("no replication snapshot was built since the start"));
        };
        let changed = |connection: &Connection| -> Result<Option<&'static str>> {
            Ok(if current_epoch(connection)?.as_deref() != Some(self.inventory_epoch.as_str()) {
                Some("a change other than an append, or another open of the store, ended this start's epoch")
            } else if inventory_generation::current(connection)? != snapshot.inventory_generation {
                Some("the inventory changed since the last snapshot")
            } else {
                None
            })
        };
        let (changed_before, header) = self.read_snapshot_unindexed(|| {
            let connection = self.readers.get();
            let header = Header {
                epoch: self.inventory_epoch.clone(),
                inventory_generation: snapshot.inventory_generation,
                max_envelope_rowid: snapshot.max_envelope_rowid,
                envelope_rows: snapshot.envelope_rows as u64,
                retired_authoritative: retired::authoritative(&connection)?,
                anchor: anchor(&connection, snapshot.max_envelope_rowid)?,
            };
            Ok((changed(&connection)?, header))
        })?;
        if let Some(reason) = changed_before {
            return Ok(InventorySave::Skipped(reason));
        }
        if snapshot.max_envelope_rowid > 0 && header.anchor.is_none() {
            return Ok(InventorySave::Skipped("the snapshot's last envelope is gone"));
        }
        let partial = destination.with_extension("replication-inventory.partial");
        write(&partial, &header, &snapshot)?;
        std::fs::rename(&partial, &destination)?;
        // A change that committed while the file was written makes it stale: take it back.
        if let Some(reason) = self.read_snapshot_unindexed(|| changed(&self.readers.get()))? {
            std::fs::remove_file(&destination)?;
            return Ok(InventorySave::Skipped(reason));
        }
        Ok(InventorySave::Saved)
    }

    /// The inventory the last clean shutdown saved, as a snapshot the next build extends, when
    /// it still describes this store; `None` when there is none or it does not. Consumes the
    /// file either way. Runs inside the build's read snapshot.
    pub(super) fn saved_replication_snapshot(
        &self,
        connection: &Connection,
    ) -> Result<Option<ReplicationSnapshot>> {
        let Some(found) = self
            .saved_inventory_epoch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            // Only the first build of an open reads the file.
            return Ok(None);
        };
        let source = path(&self.path);
        let file = match std::fs::File::open(&source) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                eprintln!("st3: the saved replication inventory {} cannot be opened, so the first snapshot reads the store: {error}", source.display());
                return Ok(None);
            }
        };
        if let Err(error) = std::fs::remove_file(&source) {
            // Read it only when it cannot outlive this process's changes.
            eprintln!("st3: the saved replication inventory {} cannot be removed, so the first snapshot reads the store: {error}", source.display());
            return Ok(None);
        }
        let (header, inventory, buckets) = match read(file) {
            Ok(read) => read,
            Err(error) => {
                eprintln!("st3: the saved replication inventory {} is unreadable, so the first snapshot reads the store: {error:#}", source.display());
                return Ok(None);
            }
        };
        let describes = found.as_deref() == Some(header.epoch.as_str())
            && current_epoch(connection)?.as_deref() == Some(self.inventory_epoch.as_str())
            && header.inventory_generation == inventory_generation::current(connection)?
            && header.retired_authoritative == retired::authoritative(connection)?
            && header.max_envelope_rowid <= max_envelope_rowid(connection)?
            && (header.max_envelope_rowid == 0
                || anchor(connection, header.max_envelope_rowid)? == header.anchor);
        if !describes {
            return Ok(None);
        }
        Ok(Some(ReplicationSnapshot {
            store_index: 0,
            replica_generation: 0,
            max_envelope_rowid: header.max_envelope_rowid,
            envelope_rows: header.envelope_rows as usize,
            inventory_generation: header.inventory_generation,
            inventory,
            buckets,
            // The digest states before each range are rebuilt by the build that extends it.
            digest_prefixes: Vec::new(),
            authority_digest: String::new(),
            // No graph generation matches, so the build digests the graph again.
            graph_generation: i64::MIN,
            projection_generation: i64::MIN,
            graph_digest: String::new(),
            legacy_graph_digest: String::new(),
            projection_digests: BTreeMap::new(),
            unsealed: false,
        }))
    }
}

/// Bytes written or read so far, hashed, so the file ends with a digest of everything before it.
struct Hashed<T> {
    inner: T,
    digest: Sha256,
}

impl<T: std::io::Write> std::io::Write for Hashed<T> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.digest.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<T: std::io::Read> std::io::Read for Hashed<T> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(bytes)?;
        self.digest.update(&bytes[..read]);
        Ok(read)
    }
}

fn put_u64(out: &mut impl std::io::Write, value: u64) -> std::io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

fn put_text(out: &mut impl std::io::Write, text: &str) -> std::io::Result<()> {
    put_u64(out, text.len() as u64)?;
    out.write_all(text.as_bytes())
}

fn put_identity(out: &mut impl std::io::Write, identity: &ReplicaEnvelopeId) -> std::io::Result<()> {
    put_text(out, &identity.writer)?;
    put_u64(out, identity.sequence)?;
    put_text(out, &identity.hash)
}

fn write(destination: &Path, header: &Header, snapshot: &ReplicationSnapshot) -> Result<()> {
    let file = std::fs::File::create(destination)?;
    let mut out = Hashed {
        inner: std::io::BufWriter::with_capacity(1 << 20, file),
        digest: Sha256::new(),
    };
    let inventory = &snapshot.inventory;
    out.write_all(MAGIC)?;
    put_text(&mut out, &header.epoch)?;
    put_u64(&mut out, header.inventory_generation as u64)?;
    put_u64(&mut out, header.max_envelope_rowid as u64)?;
    put_u64(&mut out, header.envelope_rows)?;
    put_u64(&mut out, u64::from(header.retired_authoritative))?;
    match &header.anchor {
        Some(anchor) => {
            put_u64(&mut out, 1)?;
            put_identity(&mut out, anchor)?;
        }
        None => put_u64(&mut out, 0)?,
    }
    put_u64(&mut out, inventory.writers.len() as u64)?;
    for writer in &inventory.writers {
        put_text(&mut out, writer)?;
    }
    put_u64(&mut out, inventory.irregular_hashes.len() as u64)?;
    for hash in &inventory.irregular_hashes {
        put_text(&mut out, hash)?;
    }
    put_u64(&mut out, inventory.envelopes.len() as u64)?;
    for envelope in &inventory.envelopes {
        out.write_all(&envelope.writer.to_le_bytes())?;
        out.write_all(&envelope.irregular.to_le_bytes())?;
        put_u64(&mut out, envelope.sequence)?;
        out.write_all(&envelope.hash)?;
    }
    put_u64(&mut out, inventory.payloadless.len() as u64)?;
    for hash in &inventory.payloadless {
        out.write_all(hash)?;
    }
    put_u64(&mut out, inventory.payloadless_irregular.len() as u64)?;
    for identity in &inventory.payloadless_irregular {
        put_identity(&mut out, identity)?;
    }
    put_u64(&mut out, snapshot.buckets.len() as u64)?;
    for bucket in &snapshot.buckets {
        put_text(&mut out, &bucket.writer)?;
        put_u64(&mut out, bucket.start)?;
        put_u64(&mut out, bucket.count)?;
        put_text(&mut out, &bucket.digest)?;
    }
    let Hashed { inner, digest } = out;
    let mut file = inner.into_inner().map_err(|error| error.into_error())?;
    file.write_all(&digest.finalize())?;
    file.sync_all()?;
    Ok(())
}

/// The most elements any one section may claim, so a damaged length cannot ask for more memory
/// than a store holds identities.
const MOST: u64 = 1 << 32;

fn get_u64(input: &mut impl std::io::Read) -> Result<u64> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn get_u32(input: &mut impl std::io::Read) -> Result<u32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn get_count(input: &mut impl std::io::Read) -> Result<usize> {
    let count = get_u64(input)?;
    anyhow::ensure!(count <= MOST, "a section claims {count} elements");
    Ok(count as usize)
}

fn get_hash(input: &mut impl std::io::Read) -> Result<[u8; 32]> {
    let mut hash = [0; 32];
    input.read_exact(&mut hash)?;
    Ok(hash)
}

fn get_text(input: &mut impl std::io::Read) -> Result<String> {
    let length = get_u64(input)?;
    anyhow::ensure!(length <= 1 << 20, "a text claims {length} bytes");
    let mut bytes = vec![0; length as usize];
    input.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

fn get_identity(input: &mut impl std::io::Read) -> Result<ReplicaEnvelopeId> {
    Ok(ReplicaEnvelopeId {
        writer: get_text(input)?,
        sequence: get_u64(input)?,
        hash: get_text(input)?,
    })
}

fn read(
    file: std::fs::File,
) -> Result<(Header, CompactReplicationInventory, Vec<ReplicationInventoryBucket>)> {
    let mut input = Hashed {
        inner: std::io::BufReader::with_capacity(1 << 20, file),
        digest: Sha256::new(),
    };
    let mut magic = [0; 32];
    input.read_exact(&mut magic)?;
    anyhow::ensure!(&magic == MAGIC, "not a replication inventory this build reads");
    let epoch = get_text(&mut input)?;
    let inventory_generation = get_u64(&mut input)? as i64;
    let max_envelope_rowid = get_u64(&mut input)? as i64;
    let envelope_rows = get_u64(&mut input)?;
    let retired_authoritative = get_u64(&mut input)? == 1;
    let anchor = match get_u64(&mut input)? {
        0 => None,
        _ => Some(get_identity(&mut input)?),
    };
    let mut inventory = CompactReplicationInventory::default();
    for _ in 0..get_count(&mut input)? {
        inventory.writers.push(get_text(&mut input)?);
    }
    for _ in 0..get_count(&mut input)? {
        inventory.irregular_hashes.push(get_text(&mut input)?);
    }
    let envelopes = get_count(&mut input)?;
    inventory.envelopes.reserve_exact(envelopes);
    for _ in 0..envelopes {
        let envelope = CompactEnvelopeId {
            writer: get_u32(&mut input)?,
            irregular: get_u32(&mut input)?,
            sequence: get_u64(&mut input)?,
            hash: get_hash(&mut input)?,
        };
        anyhow::ensure!(
            (envelope.writer as usize) < inventory.writers.len()
                && envelope.irregular as usize <= inventory.irregular_hashes.len(),
            "an envelope names a writer or hash the file does not list"
        );
        inventory.envelopes.push(envelope);
    }
    let payloadless = get_count(&mut input)?;
    inventory.payloadless.reserve(payloadless);
    for _ in 0..payloadless {
        inventory.payloadless.insert(get_hash(&mut input)?);
    }
    for _ in 0..get_count(&mut input)? {
        inventory.payloadless_irregular.insert(get_identity(&mut input)?);
    }
    let mut buckets = Vec::new();
    for _ in 0..get_count(&mut input)? {
        buckets.push(ReplicationInventoryBucket {
            writer: get_text(&mut input)?,
            start: get_u64(&mut input)?,
            count: get_u64(&mut input)?,
            digest: get_text(&mut input)?,
        });
    }
    anyhow::ensure!(
        buckets.iter().map(|bucket| bucket.count).sum::<u64>() == inventory.envelopes.len() as u64,
        "the ranges do not cover the identities"
    );
    let Hashed { mut inner, digest } = input;
    let mut trailer = [0; 32];
    inner.read_exact(&mut trailer)?;
    anyhow::ensure!(trailer[..] == digest.finalize()[..], "the file does not match its digest");
    anyhow::ensure!(inner.read(&mut [0])? == 0, "the file runs on past its digest");
    Ok((
        Header {
            epoch,
            inventory_generation,
            max_envelope_rowid,
            envelope_rows,
            retired_authoritative,
            anchor,
        },
        inventory,
        buckets,
    ))
}
