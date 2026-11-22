//! Versioned logical backups. Envelope payloads use the peer wire format unchanged.
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

use crate::client::Endpoint;
use crate::store::{CheckpointManifest, Store};
use crate::store::backup_shards;
use crate::store::messages_shard::{self, SHARD_ID};
use crate::message_body::BodyAttachment as _;

pub use crate::store::backup_format::{
    FORMAT, Header, MessageBodyRecord, Record, RestoreReport, ShardDigest, TableDigest, VERSION,
    VERSION_WITH_BODIES, VERSION_WITH_LEDGERS, VERSION_WITH_MARKS, VERSION_WITH_SHARDS,
    VERSION_WITH_QUARANTINE, VERSION_WITH_SIGNED_CLAIMS,
};
pub use crate::store::backup_format::{MAX_RECORD_BYTES, PAGE};

/// Download to a private temporary file, then publish only a complete, checksummed archive.
pub async fn create(endpoint: &Endpoint, destination: &Path) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    ensure!(!destination.exists(), "backup file already exists");
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(directory)?;
    let mut output = tokio::fs::File::from_std(temporary.reopen()?);
    let (client, url) = match endpoint {
        Endpoint::Unix(_) | Endpoint::DiscoveredUnix(_) => (
            reqwest::Client::builder()
                .unix_socket(endpoint.unix_socket().expect("Unix endpoint").as_ref())
                .build()?,
            "http://localhost/v1/backup".to_owned(),
        ),
        Endpoint::Http(base) => (reqwest::Client::new(), format!("{base}/v1/backup")),
        Endpoint::Fabric(_) => anyhow::bail!(
            "native Fabric endpoints only support client-v0 and health requests, not backups"
        ),
    };
    let response = client.get(url).send().await?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "backup export failed ({status}): {}",
        response.text().await?
    );
    let mut stream = response.bytes_stream();
    while let Some(bytes) = stream.next().await {
        output.write_all(&bytes?).await?;
    }
    output.sync_all().await?;
    drop(output);
    let temporary = tokio::task::spawn_blocking(move || -> Result<_> {
        verify_file(temporary.path())?;
        Ok(temporary)
    })
    .await??;
    temporary.persist_noclobber(destination)?;
    Ok(())
}

pub const DEFAULT_MAX_WAL_GROWTH: u64 = 4 * 1024 * 1024 * 1024;

/// A receipt commits every stdout byte, including the archive's end record.
#[derive(Debug, serde::Serialize)]
pub struct StreamReceipt {
    pub format: String,
    pub version: u32,
    pub records: u64,
    pub sha256: String,
    pub bytes: u64,
    pub writer_id: String,
    pub max_store_index: u64,
    pub started_at: String,
    pub finished_at: String,
    pub peak_wal_growth: u64,
}

struct CheckedOutput<W> {
    output: W,
    verifier: StreamVerifier,
    hash: Sha256,
    bytes: u64,
    started_at: String,
}

impl<W> CheckedOutput<W> {
    fn new(output: W) -> Self {
        Self { output, verifier: StreamVerifier::default(), hash: Sha256::new(), bytes: 0,
            started_at: chrono::Utc::now().to_rfc3339() }
    }
}

impl<W: Write> CheckedOutput<W> {
    fn finish(mut self) -> Result<StreamReceipt> {
        let records = self.verifier.records;
        let peak_wal_growth = self.verifier.peak_wal_growth;
        let header = self.verifier.finish()?;
        let source = header.source.context("backup source has no persisted local writer identity")?;
        self.output.flush()?;
        Ok(StreamReceipt {
            format: header.format, version: header.version, records,
            sha256: hex::encode(self.hash.finalize()), bytes: self.bytes,
            writer_id: source.writer_id, max_store_index: source.max_store_index,
            started_at: self.started_at, finished_at: chrono::Utc::now().to_rfc3339(),
            peak_wal_growth,
        })
    }
}

impl<W: Write> Write for CheckedOutput<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.verifier.push(bytes).map_err(std::io::Error::other)?;
        self.output.write_all(bytes)?;
        self.hash.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

/// Direct HTTP export: no archive is staged by either endpoint.
pub async fn create_stdout(endpoint: &Endpoint, max_wal_growth: u64) -> Result<StreamReceipt> {
    use futures_util::StreamExt;
    let (client, url) = match endpoint {
        Endpoint::Unix(_) | Endpoint::DiscoveredUnix(_) => (
            reqwest::Client::builder()
                .unix_socket(endpoint.unix_socket().expect("Unix endpoint").as_ref()).build()?,
            "http://localhost/v1/backup".to_owned(),
        ),
        Endpoint::Http(base) => (reqwest::Client::new(), format!("{base}/v1/backup")),
        Endpoint::Fabric(_) => anyhow::bail!("native Fabric endpoints do not support backups"),
    };
    let response = client.get(url)
        .query(&[("stream", "true".to_owned()), ("max_wal_growth", max_wal_growth.to_string())])
        .send().await?;
    let status = response.status();
    ensure!(status.is_success(), "backup export failed ({status}): {}", response.text().await?);
    ensure!(
        response.headers().get("x-fractal-backup-stream").is_some_and(|value| value == "1"),
        "daemon does not support direct backup streaming; upgrade it before using stdout mode"
    );
    // Move blocking stdout writes off the async executor. The bounded queue couples
    // network reads to the consumer without retaining an archive in memory.
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<axum::body::Bytes>(2);
    let writer = tokio::task::spawn_blocking(move || {
        let stdout = std::io::stdout();
        let mut output = CheckedOutput::new(stdout.lock());
        while let Some(bytes) = receiver.blocking_recv() {
            output.write_all(&bytes)?;
        }
        output.finish()
    });
    let mut stream = response.bytes_stream();
    let download = async {
        while let Some(bytes) = stream.next().await {
            let bytes = bytes?;
            for offset in (0..bytes.len()).step_by(64 * 1024) {
                let chunk = bytes.slice(offset..(offset + 64 * 1024).min(bytes.len()));
                if sender.send(chunk).await.is_err() {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        }
        Ok(())
    }.await;
    drop(sender);
    let receipt = writer.await??;
    download?;
    Ok(receipt)
}

/// Unlike file export, stdout export never migrates or repairs its offline input.
pub fn create_stdout_from_database(database: &Path, state_root: &Path) -> Result<StreamReceipt> {
    let store = Store::open_backup_read_only(database)?;
    let stdout = std::io::stdout();
    let mut output = CheckedOutput::new(stdout.lock());
    store.write_backup_read_only(&mut output, Some(&crate::message_body::bodies(state_root)))?;
    output.finish()
}

/// An offline export can migrate its input; callers must supply a private database copy.
/// External message bodies remain in the product state root, independently of the database.
pub fn create_from_database(
    database: &Path,
    state_root: &Path,
    destination: &Path,
) -> Result<Header> {
    use rusqlite::OptionalExtension;
    ensure!(database.exists(), "source database does not exist");
    ensure!(!destination.exists(), "backup file already exists");
    let reader = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let writer = reader
        .query_row(
            "SELECT value FROM meta WHERE key='backup_restore_writer'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| "backup-reader".into());
    drop(reader);
    let store = Arc::new(Store::open(database, writer)?);
    // Opening an older copy migrates its schema; finish the normal upgrade recovery
    // before describing the current registry and projections in the archive header.
    store.validate_replication_backlog()?;
    store.apply_replication_repairs()?;
    store.replay_replication_graph()?;
    refuse_queued_mail(&store)?;
    // The messages shard beside the database, when there is one, as the daemon opens it. A copy
    // made without its `shards` directory is archived without stream items.
    let root = database
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if backup_shards::database_path(root)?.exists() {
        let shard = messages_shard::open(&store, root)?;
        shard.validate_replication_backlog()?;
        shard.apply_replication_repairs()?;
        shard.project_replication_backlog()?;
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    let header = store.write_backup_with_bodies(
        &mut temporary,
        Some(&crate::message_body::bodies(state_root)),
    )?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(destination)?;
    Ok(header)
}

/// How many waiting messages an offline export's refusal names.
const QUEUED_NAMED: i64 = 8;

/// Refuse a copy whose message outbox holds mail its member has not yet written to the messages
/// shard. The exporter is not that member: a drain here would append the mail under the copy's
/// writer, to the copy's shard or graph, and the archive would carry writes the member never
/// made. The member's own daemon drains the outbox, after which a new copy exports.
fn refuse_queued_mail(store: &Store) -> Result<()> {
    let (subjects, count) = crate::store::message_outbox::queued(&store.readers.get(), QUEUED_NAMED)?;
    ensure!(
        count == 0,
        "offline export refuses a copy whose message outbox holds {count} message(s) not yet in its messages shard ({}{}); start the member's daemon so it writes them, then copy the database again",
        subjects.join(", "),
        if count > subjects.len() as i64 { format!(" and {} more", count - subjects.len() as i64) } else { String::new() }
    );
    Ok(())
}

fn read_record(input: &mut impl BufRead) -> Result<Option<(Record, Vec<u8>)>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    input
        .take(MAX_RECORD_BYTES + 1)
        .read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    ensure!(
        bytes.len() as u64 <= MAX_RECORD_BYTES && bytes.last() == Some(&b'\n'),
        "oversized or truncated backup record"
    );
    let record = serde_json::from_slice(&bytes).context("decode backup record")?;
    Ok(Some((record, bytes)))
}

fn check_header(header: &Header) -> Result<()> {
    // Builds before version 5 say only "unsupported claim backup format/version"; this one also
    // names what it found and what it restores.
    ensure!(
        header.format == FORMAT && (VERSION..=VERSION_WITH_SIGNED_CLAIMS).contains(&header.version),
        "unsupported claim backup format/version: {:?} version {}; this build restores {FORMAT:?} \
         versions {VERSION} to {VERSION_WITH_SIGNED_CLAIMS}",
        header.format,
        header.version
    );
    ensure!(header.version >= VERSION_WITH_QUARANTINE
        || !header.ledgers.contains_key("replica_restore_quarantine"),
        "operation quarantine requires the current archive version");
    ensure!(
        header.version >= VERSION_WITH_SHARDS || header.shards.is_empty(),
        "shard digests in an archive that does not declare shards"
    );
    if let Some(shard) = header.shards.keys().find(|shard| *shard != SHARD_ID) {
        anyhow::bail!("backup carries an unknown shard {shard:?}");
    }
    ensure!(
        header.tables.contains_key("claim_sources"),
        "backup lacks the claim source digest"
    );
    Ok(())
}

/// A body record's text, which must be what its hash says.
fn body_bytes(body: &MessageBodyRecord) -> Result<Vec<u8>> {
    use base64::Engine as _;
    ensure!(
        crate::blobs::is_sha256(&body.sha256) && body.message.starts_with("message/"),
        "a backed-up message body names no message and hash"
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&body.data)
        .context("decode a backed-up message body")?;
    ensure!(
        hex::encode(Sha256::digest(&bytes)) == body.sha256,
        "a backed-up message body does not match its hash"
    );
    Ok(bytes)
}

/// A shard record must belong to a shard the header lists, in an archive that declares shards.
fn check_shard_record(header: &Header, shard: &str) -> Result<()> {
    ensure!(
        header.version >= VERSION_WITH_SHARDS,
        "a shard record in an archive that does not declare shards"
    );
    ensure!(
        header.shards.contains_key(shard),
        "a record of shard {shard:?}, which the backup header does not list"
    );
    Ok(())
}

/// Check framing and completeness without admitting anything.
pub fn verify_file(path: &Path) -> Result<Header> {
    let mut input = BufReader::new(fs::File::open(path)?);
    let mut verifier = StreamVerifier::default();
    while let Some((record, bytes)) = read_record(&mut input)? {
        verifier.record(record, &bytes)?;
    }
    verifier.finish()
}

#[derive(Default)]
struct StreamVerifier {
    header: Option<Header>,
    hash: Sha256,
    envelopes: u64,
    shard_envelopes: BTreeMap<String, u64>,
    ended: bool,
    pending: Vec<u8>,
    records: u64,
    peak_wal_growth: u64,
}

impl StreamVerifier {
    fn push(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let count = bytes.iter().position(|byte| *byte == b'\n').map_or(bytes.len(), |index| index + 1);
            ensure!(self.pending.len() as u64 + count as u64 <= MAX_RECORD_BYTES, "oversized backup record");
            self.pending.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.pending.last() == Some(&b'\n') {
                let line = std::mem::take(&mut self.pending);
                let record = serde_json::from_slice(&line).context("decode backup record")?;
                self.record(record, &line)?;
                self.pending = line;
                self.pending.clear();
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Header> {
        ensure!(self.pending.is_empty(), "truncated backup record");
        ensure!(self.ended, "backup has no end record");
        self.header.context("empty backup")
    }

    fn record(&mut self, record: Record, bytes: &[u8]) -> Result<()> {
        ensure!(!self.ended, "data follows the backup end record");
        self.records += 1;
        if self.header.is_none() {
            let header = match record {
                Record::Header(header) => header,
                Record::Failure { code, message } => anyhow::bail!("backup export failed ({code}): {message}"),
                _ => anyhow::bail!("backup must start with a header"),
            };
            check_header(&header)?;
            self.header = Some(header);
            self.hash.update(bytes);
            return Ok(());
        }
        let header = self.header.as_ref().expect("header checked");
        match record {
            Record::Failure { code, message } => anyhow::bail!("backup export failed ({code}): {message}"),
            Record::Envelope(_) => self.envelopes += 1,
            Record::Checkpoint(_) | Record::Signatures(_) => {}
            Record::SignedEnvelopes(_) => ensure!(
                header.version >= VERSION_WITH_SIGNED_CLAIMS,
                "signed envelopes in an archive that does not declare them"
            ),
            Record::MessageBody(body) => {
                ensure!(
                    header.version >= VERSION_WITH_BODIES,
                    "a message body in an archive that does not declare them"
                );
                body_bytes(&body)?;
            }
            Record::Ledger(page) => {
                ensure!(
                    header.version >= VERSION_WITH_LEDGERS,
                    "a ledger page in an archive that does not declare them"
                );
                ensure!(
                    header.version >= VERSION_WITH_MARKS
                        || !crate::store::backup_ledgers::MARK_LEDGERS.contains(&page.table.as_str()),
                    "message marks in an archive that does not declare them"
                );
            }
            Record::ShardEnvelope(record) => {
                check_shard_record(header, &record.shard)?;
                *self.shard_envelopes.entry(record.shard).or_default() += 1;
            }
            Record::ShardSignatures(record) => check_shard_record(header, &record.shard)?,
            Record::ShardLocal(page) => check_shard_record(header, &page.shard)?,
            Record::End {
                envelopes: expected,
                sha256,
            } => {
                let end: serde_json::Value = serde_json::from_slice(bytes)?;
                self.peak_wal_growth = end["data"]["peak_wal_growth"].as_u64().unwrap_or(0);
                ensure!(
                    self.envelopes == expected && hex::encode(self.hash.clone().finalize()) == sha256,
                    "backup checksum/count mismatch"
                );
                for (shard, digest) in &header.shards {
                    let found = self.shard_envelopes.get(shard).copied().unwrap_or(0);
                    ensure!(
                        found == digest.envelope_count,
                        "backup shard {shard:?} holds {found} envelopes, not the {} its header lists",
                        digest.envelope_count
                    );
                }
                self.ended = true;
                return Ok(());
            }
            Record::Header(_) => anyhow::bail!("duplicate backup header"),
        }
        self.hash.update(bytes);
        Ok(())
    }
}

/// Build privately at the current schema. Failed restores never publish a partial graph.
pub /* sqlite-large-value-flow: allow-for-now flow-h0f24faa60a9f owner=fractal_store::backup TODO=chunk-or-reference-exact-wire-values */ fn restore(source: &Path, destination: &Path, state_root: &Path) -> Result<RestoreReport> {
    let header = verify_file(source)?;
    let existing = if destination.exists() {
        Some(lock_empty_offline(destination)?)
    } else {
        None
    };
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // An archive with shards restores each beside the database, where the daemon opens it. On
    // every restore, a shard already there must be empty and offline, as the database must: a
    // restored graph never stands beside another store's stream items. Its lock is held through
    // publication; an archive without shards leaves an empty one in place.
    let restores_shard = !header.shards.is_empty();
    let shard_destination = backup_shards::database_path(directory)?;
    let existing_shard = if shard_destination.exists() {
        Some(lock_empty_offline(&shard_destination).with_context(|| {
            format!(
                "restore refuses a destination whose messages shard at {} is not empty and offline",
                shard_destination.display()
            )
        })?)
    } else {
        None
    };
    fs::create_dir_all(directory)?;
    let stage = tempfile::tempdir_in(directory)?;
    let staged_path = stage
        .path()
        .join(fractal_client::product_dirs::STORE_DATABASE_FILES[0]);
    let writer = format!("restored-{}", uuid::Uuid::now_v7().simple());
    let store = Arc::new(Store::open(&staged_path, &writer)?);
    store.prepare_backup_restore(&header, &writer)?;
    // The staged shard takes its fleet from the staged graph, as the daemon's does.
    let shard = if restores_shard {
        Some(messages_shard::open(&store, stage.path())?)
    } else {
        None
    };
    let staged_shard = backup_shards::database_path(stage.path())?;
    let mut input = BufReader::new(fs::File::open(source)?);
    let (Record::Header(restoring_header), bytes) =
        read_record(&mut input)?.context("empty backup")?
    else {
        anyhow::bail!("backup header changed during restore");
    };
    ensure!(
        serde_json::to_value(&restoring_header)? == serde_json::to_value(&header)?,
        "backup header changed during restore"
    );
    let mut restore_hash = Sha256::new();
    restore_hash.update(bytes);
    let mut checkpoint_cursor = None;
    let mut checkpoint_finished = false;
    let mut count = 0;
    let mut page = Vec::with_capacity(PAGE);
    let mut shard_page = Vec::new();
    let mut checkpoint = None;
    let mut restored_bodies = Vec::new();
    let mut signed_admitting = false;
    loop {
        let (record, bytes) = read_record(&mut input)?.context("backup ended during restore")?;
        match record {
            Record::Failure { code, message } => anyhow::bail!("backup export failed ({code}): {message}"),
            Record::MessageBody(body) => {
                let bytes = body_bytes(&body)?;
                let staged = crate::message_body::bodies(stage.path());
                staged.put(&body.actor, &body.message, &bytes)?;
                restored_bodies.push((body.actor, body.message, body.sha256));
            }
            Record::Envelope(envelope) => {
                page.push(envelope);
                count += 1;
                if page.len() == PAGE {
                    store.receive_backup_page(&header, &page)?;
                    page.clear();
                }
            }
            Record::Signatures(signatures) => {
                store.receive_backup_signatures(&header, &signatures)?
            }
            Record::SignedEnvelopes(signed) => {
                // Every envelope is in by now; admit them first, so their membership claims
                // admit the signed claims' writers.
                if !signed_admitting {
                    store.receive_backup_page(&header, &page)?;
                    page.clear();
                    store.admit_restored_backlog()?;
                    signed_admitting = true;
                }
                store.receive_backup_signed_envelopes(&header, &signed)?;
            }
            Record::Ledger(page) => store.receive_backup_ledger(&page)?,
            // `verify_file` admitted shard records only for a shard the header lists, which is
            // open whenever the header lists one.
            Record::ShardEnvelope(record) => {
                shard_page.push(record.envelope);
                if shard_page.len() == PAGE {
                    backup_shards::receive_envelopes(restoring_shard(&shard)?, &header, &shard_page)?;
                    shard_page.clear();
                }
            }
            Record::ShardSignatures(record) => {
                backup_shards::receive_signatures(restoring_shard(&shard)?, &header, &record.signatures)?
            }
            Record::ShardLocal(page) => backup_shards::receive_local(restoring_shard(&shard)?, &page.page)?,
            Record::Checkpoint(page) => {
                ensure!(!checkpoint_finished, "duplicate checkpoint manifest");
                let manifest = checkpoint.get_or_insert_with(|| CheckpointManifest {
                    checkpoint: page.checkpoint.clone(),
                    cut_unix_ms: page.cut_unix_ms,
                    ..CheckpointManifest::default()
                });
                checkpoint_cursor = manifest.append(page)?;
                checkpoint_finished = checkpoint_cursor.is_none();
            }
            Record::End {
                envelopes: expected,
                sha256,
            } => {
                ensure!(
                    count == expected && hex::encode(restore_hash.finalize()) == sha256,
                    "backup changed during restore"
                );
                ensure!(
                    read_record(&mut input)?.is_none() && checkpoint_cursor.is_none(),
                    "incomplete backup or trailing data"
                );
                break;
            }
            Record::Header(_) => anyhow::bail!("duplicate backup header"),
        }
        restore_hash.update(bytes);
    }
    store.receive_backup_page(&header, &page)?;
    if let Some(shard) = &shard {
        backup_shards::receive_envelopes(shard, &header, &shard_page)?;
    }
    let mut report = store.finish_backup_restore(&header, &writer, count, checkpoint.as_ref())?;
    // The shard after the graph: its admission reads the restored graph's fleet.
    if let Some(shard) = &shard {
        report
            .shards
            .insert(SHARD_ID.into(), backup_shards::finish_restore(shard, &header)?);
    }
    // Restore only the texts a restored claim names, with the hash it gives. The count of long
    // messages the claims name says how many texts the archive did not hold: those another
    // member owns, or that were lost before the backup.
    let staged_bodies = crate::message_body::bodies(stage.path());
    let mut verified = Vec::new();
    for (actor, message, sha256) in std::mem::take(&mut restored_bodies) {
        let named = store
            .message(&message)?
            .filter(|view| view.from == actor)
            .and_then(|view| view.body_attachment())
            .is_some_and(|body| body.sha256 == sha256);
        if named {
            verified.push((actor, message));
        } else {
            report.message_bodies_unmatched += 1;
        }
    }
    report.message_bodies_restored = verified.len() as u64;
    report.message_bodies_referenced = store.message_body_references()?;
    // The graph holds the shard too: both close here.
    drop(shard);
    drop(store);
    // Store closes/checkpoints its writer before publication; never publish WAL-dependent data.
    seal_staged(&staged_path)?;
    // The shard is published first, so a graph that restored stream items never stands without
    // them. Should the graph then fail to publish, a shard this restore placed is removed again.
    if restores_shard {
        seal_staged(&staged_shard)?;
        let shards = shard_destination.parent().context("a shard database has a directory")?;
        fs::create_dir_all(shards)?;
        if existing_shard.is_some() {
            fs::rename(&staged_shard, &shard_destination)?;
        } else {
            fs::hard_link(&staged_shard, &shard_destination)?;
        }
        fs::File::open(shards)?.sync_all()?;
    }
    let published = if existing.is_some() {
        fs::rename(&staged_path, destination)
    } else {
        // A concurrent restore cannot overwrite a database that appeared after the initial check.
        fs::hard_link(&staged_path, destination)
    };
    if let Err(error) = published {
        if restores_shard && existing_shard.is_none() {
            let _ = fs::remove_file(&shard_destination);
        }
        return Err(error.into());
    }
    drop(existing);
    drop(existing_shard);
    // External message bodies belong to the product state root, not the database directory.
    let restored = crate::message_body::bodies(state_root);
    for (actor, message) in &verified {
        if let Some(bytes) = staged_bodies.read(actor, message)? {
            restored.put(actor, message, &bytes)?;
        }
    }
    fs::File::open(directory)?.sync_all()?;
    Ok(report)
}

/// The staged shard, for a shard record; `verify_file` refuses a shard record without one.
fn restoring_shard(shard: &Option<Arc<crate::store::GraphStore>>) -> Result<&crate::store::GraphStore> {
    shard
        .as_deref()
        .context("a shard record in an archive that does not declare shards")
}

/// Lock an existing destination that must be an empty database no process has open, and keep
/// the lock through publication so no writer can enter it.
fn lock_empty_offline(path: &Path) -> Result<rusqlite::Connection> {
    let connection = rusqlite::Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::ZERO)?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h84213c6880a6 owner=fractal_store::backup TODO=trace-statement-and-value-representation */ "BEGIN EXCLUSIVE;")?;
    require_empty_database(&connection)?;
    // A WAL file may outlive the last connection. Changing journal mode takes the locks
    // that distinguish a closed database from a live WAL reader, unlike file existence.
    // Keep the exclusive connection lock through publication so no new writer can enter.
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h9a6064fa060f owner=fractal_store::backup TODO=trace-statement-and-value-representation */ "COMMIT; PRAGMA locking_mode=EXCLUSIVE;")?;
    connection
        .execute_batch("PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE;")
        .context("restore requires an offline database")?;
    // Recheck after the journal-mode transition released the first transaction.
    require_empty_database(&connection)?;
    Ok(connection)
}

/// Fold a closed staged database's WAL into it, make it private and sync it, so what is
/// published never depends on a WAL file.
fn seal_staged(path: &Path) -> Result<()> {
    let connection = rusqlite::Connection::open(path)?;
    let checkpoint_started = std::time::Instant::now();
    let checkpoint = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    smallclaims::sqlite::telemetry::checkpoint_finished(
        checkpoint_started.elapsed(),
        smallclaims::sqlite::telemetry::CheckpointOperation::Truncate,
        if checkpoint.is_ok() {
            smallclaims::sqlite::telemetry::CheckpointOutcome::Recycled
        } else {
            smallclaims::sqlite::telemetry::CheckpointOutcome::Error
        },
    );
    checkpoint?;
    drop(connection);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

fn require_empty_database(connection: &rusqlite::Connection) -> Result<()> {
    let has_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='claims')",
        [],
        |r| r.get(0),
    )?;
    if has_table {
        let has_claims: bool =
            connection.query_row("SELECT EXISTS(SELECT 1 FROM claims)", [], |r| r.get(0))?;
        ensure!(
            !has_claims,
            "restore refuses a database that already holds claims"
        );
        let has_checkpoint_table: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='checkpoint_claims')",
            [], |r| r.get(0),
        )?;
        let has_tombstones = has_checkpoint_table
            && connection.query_row("SELECT EXISTS(SELECT 1 FROM checkpoint_claims)", [], |r| {
                r.get::<_, bool>(0)
            })?;
        ensure!(
            !has_tombstones,
            "restore refuses a database that already holds checkpointed claims"
        );
    } else {
        let has_tables: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table')",
            [],
            |r| r.get(0),
        )?;
        ensure!(!has_tables, "restore refuses a nonempty database");
    }
    // An older reader can hold unknown claims only in the wire log. That is still data, as is
    // a shard's record of how far this member took its stream items.
    for (table, what) in [
        ("replica_envelopes", "envelopes"),
        ("checkpoint_envelopes", "envelopes"),
        ("stream_lifecycle", "stream item lifecycle"),
    ] {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |r| r.get(0),
        )?;
        if exists {
            let has_envelopes: bool = connection.query_row(
                /* sqlite-large-value: allow-for-now lv-hf3f3b02320eb owner=fractal_store::backup TODO=trace-statement-and-value-representation */ &format!("SELECT EXISTS(SELECT 1 FROM {table})"),
                [],
                |r| r.get(0),
            )?;
            ensure!(
                !has_envelopes,
                "restore refuses a database that already holds {what}"
            );
        }
    }
    Ok(())
}
