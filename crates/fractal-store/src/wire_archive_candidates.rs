//! Private, offline analysis artifacts. These deliberately incomplete hot stores must never
//! be opened by a daemon: native daemon readers do not consult the accompanying archives.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags, params, types::ValueRef};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use smallclaims::{ReplicaEnvelopeId, wire_archive::{MAX_ENVELOPES, SegmentReader, write_segment}};

use crate::store::Store;

const DAY: u128 = 86_400_000;
const WIRE: [&str; 3] = ["replica_envelopes", "replica_records", "replica_envelope_signatures"];

fn directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

fn new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn database(path: &Path) -> Result<Connection> {
    new_file(path)?;
    Ok(Connection::open(path)?)
}

fn readonly(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?)
}

fn file_digest(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    let mut file = File::open(path)?;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn siblings(path: &Path) -> [PathBuf; 3] {
    let suffix = |suffix: &str| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    [path.to_owned(), suffix("-wal"), suffix("-shm")]
}

fn input_files(source: &Path) -> Result<Vec<(PathBuf, u64)>> {
    let mut result = Vec::new();
    for path in siblings(source) {
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                ensure!(metadata.is_file() && !metadata.file_type().is_symlink(), "input is not a regular non-symlink file: {}", path.display());
                result.push((path, metadata.len()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
    }
    ensure!(result.first().is_some_and(|entry| entry.0 == source), "source database does not exist");
    Ok(result)
}

fn input_identity(source: &Path) -> Result<Vec<(PathBuf, String, u64)>> {
    input_files(source)?.into_iter().map(|(path,bytes)| {
        let digest = file_digest(&path)?;
        Ok((path,digest,bytes))
    }).collect()
}

fn no_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() { continue; }
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            ensure!(!metadata.file_type().is_symlink(), "symlink path component refused: {}", ancestor.display());
        }
    }
    Ok(())
}

fn vacuum(source: &Connection, path: &Path) -> Result<()> {
    // Reserve exclusively with private permissions; SQLite accepts an empty destination.
    new_file(path)?;
    source.execute(/* sqlite-large-value: allow-for-now lv-hc90a840b82f5 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ "VACUUM INTO ?1", [path.to_str().context("non-UTF8 SQLite destination")?])?;
    Ok(())
}

/// Release only closed databases owned by this construction's private staging directory.
fn remove_private_database(path: &Path) -> Result<()> {
    for file in siblings(path) {
        match fs::remove_file(file) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn quote(identifier: &str) -> String { format!("\"{}\"", identifier.replace('"', "\"\"")) }

/// Hash exact SQLite types and bytes in deterministic primary-key order, one row at a time.
/// Tables without a primary key use rowid. Schema is hashed separately by the caller.
fn table_digest(connection: &Connection, table: &str) -> Result<String> {
    filtered_table_digest(connection, table, "")
}

fn filtered_table_digest(connection: &Connection, table: &str, predicate: &str) -> Result<String> {
    let mut info = connection.prepare(/* sqlite-large-value: allow-for-now lv-h670577d2ec84 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ &format!("PRAGMA table_info({})", quote(table)))?;
    let mut keys = info.query_map([], |row| Ok((row.get::<_, i64>(5)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    keys.retain(|(position, _)| *position > 0);
    keys.sort_unstable();
    let order = if keys.is_empty() { "rowid".to_owned() } else {
        keys.iter().map(|(_, name)| quote(name)).collect::<Vec<_>>().join(",")
    };
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hf2d2b96fb592 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ &format!("SELECT * FROM {} AS wire {predicate} ORDER BY {order}", quote(table)))?;
    let columns = statement.column_count();
    let mut rows = statement.query([])?;
    let mut hash = Sha256::new();
    while let Some(row) = rows.next()? {
        hash.update([0xff]);
        for column in 0..columns {
            match row.get_ref(column)? {
                ValueRef::Null => hash.update([0]),
                ValueRef::Integer(value) => { hash.update([1]); hash.update(value.to_be_bytes()); },
                ValueRef::Real(value) => { hash.update([2]); hash.update(value.to_bits().to_be_bytes()); },
                ValueRef::Text(value) => { hash.update([3]); hash.update((value.len() as u64).to_be_bytes()); hash.update(value); },
                ValueRef::Blob(value) => { hash.update([4]); hash.update((value.len() as u64).to_be_bytes()); hash.update(value); },
            }
        }
    }
    Ok(hex::encode(hash.finalize()))
}

fn validate_wire_schema(connection: &Connection) -> Result<()> {
    // The v1 archive exports these named cells. A newer/extra wire cell must not be
    // silently omitted even if an incorrectly labelled database claims today's version.
    let columns = [
        "writer,sequence,envelope_hash,previous_hash,accepted_at_unix_ms,payload,batch_id,relay,receipt_state,validation_error,received_at_unix_ms",
        "record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,subject_hint,kind_hint,error_code,error_message,replacement_claim_id,updated_at_unix_ms",
        "writer,sequence,envelope_hash,member_key,signature,stored_at_unix_ms",
    ];
    for (table, expected) in WIRE.into_iter().zip(columns) {
        let mut actual = connection.prepare(/* sqlite-large-value: allow-for-now lv-he72a8370a489 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ &format!("PRAGMA table_xinfo({table})"))?
            .query_map([], |row| row.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut expected = expected.split(',').collect::<Vec<_>>();
        actual.sort_unstable();
        expected.sort_unstable();
        ensure!(actual.iter().map(String::as_str).eq(expected), "unsupported wire columns in {table}");
    }
    Ok(())
}

fn schema_digest(connection: &Connection) -> Result<String> {
    let mut hash = Sha256::new();
    let mut statement = connection.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        for column in 0..4 {
            let value: Option<String> = row.get(column)?;
            if let Some(value) = value {
                hash.update([1]);
                hash.update((value.len() as u64).to_be_bytes());
                hash.update(value.as_bytes());
            } else { hash.update([0]); }
        }
    }
    Ok(hex::encode(hash.finalize()))
}

fn wire_counts(connection: &Connection) -> Result<Value> {
    let mut counts = serde_json::Map::new();
    for table in WIRE {
        let count: u64 = connection.query_row(/* sqlite-large-value: allow-for-now lv-h5d7982f2985f owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ &format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))?;
        counts.insert(table.to_owned(), json!(count));
    }
    Ok(Value::Object(counts))
}

fn private_tree(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(!metadata.file_type().is_symlink(), "unexpected output symlink");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        let mode = if metadata.is_dir() { 0o700 } else if metadata.permissions().mode() & 0o200 == 0 { 0o400 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? { private_tree(&entry?.path())?; }
    }
    Ok(())
}

/// Validate committed evidence, not the physical extent of SQLite's recycled WAL.
/// A copied WAL-index supplies a checksummed commit bound; without it every physical
/// frame must validate. Never let SQLite recovery silently shorten committed history.
fn validate_wal(path: &Path) -> Result<()> {
    let wal_path = siblings(path)[1].clone();
    let mut wal = match File::open(&wal_path) {
        Ok(wal) => wal,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ensure!(!siblings(path)[2].exists(), "shared-memory sidecar without WAL; require a consistent offline copy");
            return Ok(());
        },
        Err(error) => return Err(error.into()),
    };
    let length = wal.metadata()?.len();
    if length == 0 { return Ok(()); }
    ensure!(length >= 32, "truncated WAL header");
    let mut header = [0_u8; 32];
    wal.read_exact(&mut header)?;
    let word = |bytes: &[u8]| u32::from_be_bytes(bytes.try_into().expect("four-byte WAL field"));
    let magic = word(&header[..4]);
    ensure!(magic == 0x377f0682 || magic == 0x377f0683, "unknown WAL format");
    ensure!(word(&header[4..8]) == 3_007_000, "unknown WAL version");
    let page_size = word(&header[8..12]) as usize;
    ensure!((512..=65536).contains(&page_size) && page_size.is_power_of_two(), "invalid WAL page size");
    let mut database_header = [0_u8; 100];
    File::open(path)?.read_exact(&mut database_header)?;
    ensure!(&database_header[..16] == b"SQLite format 3\0", "WAL source is not a SQLite database");
    let encoded_size = u16::from_be_bytes([database_header[16],database_header[17]]);
    let database_page_size = if encoded_size == 1 { 65536 } else { usize::from(encoded_size) };
    ensure!(page_size == database_page_size, "WAL/database page size mismatch");
    ensure!((length - 32) % (24 + page_size as u64) == 0, "truncated WAL frame");
    let checksum = |bytes: &[u8], mut sum: [u32; 2], big_endian: bool| {
        for chunk in bytes.chunks_exact(8) {
            let a: [u8; 4] = chunk[..4].try_into().expect("four-byte checksum word");
            let b: [u8; 4] = chunk[4..].try_into().expect("four-byte checksum word");
            let a = if big_endian { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) };
            let b = if big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) };
            sum[0] = sum[0].wrapping_add(a).wrapping_add(sum[1]);
            sum[1] = sum[1].wrapping_add(b).wrapping_add(sum[0]);
        }
        sum
    };
    let big_endian = magic & 1 != 0;
    let mut sum = checksum(&header[..24], [0, 0], big_endian);
    ensure!(sum == [word(&header[24..28]), word(&header[28..32])], "WAL header checksum mismatch");
    let physical_frames = (length - 32) / (24 + page_size as u64);
    // WAL-index fields are host-endian except the salt bytes. Check both published
    // copies before trusting mxFrame, and bind it to this WAL generation and seal.
    let index = match File::open(siblings(path)[2].clone()) {
        Ok(mut shm) => {
            let mut bytes = [0_u8; 96];
            shm.read_exact(&mut bytes).context("truncated WAL-index header")?;
            ensure!(bytes[..48] == bytes[48..], "inconsistent WAL-index header copies");
            let index_big_endian = word(&bytes[..4]) == 3_007_000;
            let index_word = |field: &[u8]| {
                let field = field.try_into().expect("four-byte WAL-index field");
                if index_big_endian { u32::from_be_bytes(field) } else { u32::from_le_bytes(field) }
            };
            ensure!(index_word(&bytes[..4]) == 3_007_000 && bytes[12] == 1, "uninitialized or unknown WAL-index");
            ensure!(checksum(&bytes[..40], [0, 0], index_big_endian) == [index_word(&bytes[40..44]), index_word(&bytes[44..48])], "WAL-index checksum mismatch");
            let size = [bytes[14], bytes[15]];
            let size = if index_big_endian { u16::from_be_bytes(size) } else { u16::from_le_bytes(size) };
            let size = if size == 1 { 65536 } else { usize::from(size) };
            ensure!(size == page_size && bytes[13] == u8::from(big_endian) && bytes[32..40] == header[16..24], "WAL-index generation mismatch");
            let frames = u64::from(index_word(&bytes[16..20]));
            ensure!(frames <= physical_frames, "WAL is shorter than its committed WAL-index bound");
            Some((frames, index_word(&bytes[20..24]), [index_word(&bytes[24..28]), index_word(&bytes[28..32])]))
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let committed_frames = index.map_or(physical_frames, |entry| entry.0);
    let mut page = vec![0; page_size];
    let mut database_pages = 0;
    for number in 1..=physical_frames {
        let mut frame = [0_u8; 24];
        wal.read_exact(&mut frame)?;
        wal.read_exact(&mut page)?;
        if number > committed_frames {
            // Reset/reuse overwrites the beginning, leaving older-generation frames
            // allocated at the end. The index proves these are not current evidence.
            // Current-generation bytes beyond its commit bound instead indicate an
            // uncommitted transaction or an inconsistent offline sidecar capture.
            ensure!(frame[8..16] != header[16..24], "current-generation WAL tail beyond committed WAL-index bound");
            continue;
        }
        ensure!(frame[8..16] == header[16..24] && word(&frame[..4]) != 0, "inconsistent WAL frame identity");
        sum = checksum(&page, checksum(&frame[..8], sum, big_endian), big_endian);
        ensure!(sum == [word(&frame[16..20]), word(&frame[20..24])], "WAL frame checksum mismatch");
        database_pages = word(&frame[4..8]);
    }
    ensure!(committed_frames == 0 || database_pages != 0, "uncommitted WAL tail; require a clean offline copy");
    if let Some((frames, pages, seal)) = index {
        ensure!(frames == 0 || (database_pages == pages && sum == seal), "WAL commit does not match WAL-index seal");
    }
    Ok(())
}

fn non_wire_digests(connection: &Connection) -> Result<Vec<(String, String)>> {
    let tables = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")?
        .query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    tables.into_iter().filter(|name| !WIRE.contains(&name.as_str()))
        .map(|name| Ok((name.clone(), table_digest(connection, &name)?))).collect()
}

fn strict_time(value: ValueRef<'_>) -> Result<u128> {
    let value = match value {
        ValueRef::Text(bytes) => {
            ensure!(!bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit), "malformed wire timestamp");
            std::str::from_utf8(bytes)?.parse::<u128>().context("wire timestamp out of range")?
        },
        ValueRef::Integer(value) if value >= 0 => value as u128,
        _ => bail!("unavailable or malformed wire timestamp"),
    };
    ensure!(value > 0, "unknown zero wire timestamp");
    Ok(value)
}

fn last_touch(source: &Connection, id: &ReplicaEnvelopeId) -> Result<u128> {
    let mut latest = 0;
    for query in [
        "SELECT accepted_at_unix_ms,received_at_unix_ms FROM replica_envelopes WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        "SELECT updated_at_unix_ms FROM replica_records WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        "SELECT stored_at_unix_ms FROM replica_envelope_signatures WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
    ] {
        let mut statement = source.prepare_cached(/* sqlite-large-value: allow-for-now lv-hb2c3638f34f7 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ query)?;
        let columns = statement.column_count();
        let mut rows = statement.query(params![id.writer, id.sequence, id.hash])?;
        while let Some(row) = rows.next()? {
            for column in 0..columns { latest = latest.max(strict_time(row.get_ref(column)?)?); }
        }
    }
    Ok(latest)
}

fn tree_bytes(path: &Path) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(!metadata.file_type().is_symlink(), "unexpected artifact symlink");
    if metadata.is_file() { return Ok(metadata.len()); }
    let mut bytes = 0;
    for entry in fs::read_dir(path)? { bytes += tree_bytes(&entry?.path())?; }
    Ok(bytes)
}

fn json_file(path: &Path, value: &Value) -> Result<()> {
    let mut file = new_file(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Construct both conservative windows from an explicitly supplied offline copy.
/// No source file is opened by SQLite, changed, removed, checkpointed, or activated.
/// Failure leaves INCOMPLETE and never publishes report.json; do not resume that directory.
/// Memory is bounded by one staged page/segment plus a single SQLite row, not store history.
pub fn construct(source_copy: &Path, output: &Path, as_of_unix_ms: u128) -> Result<Value> {
    ensure!(as_of_unix_ms >= 30 * DAY && as_of_unix_ms <= i64::MAX as u128, "as-of time must cover both windows and fit the checkpoint clock");
    no_symlinks(source_copy)?;
    no_symlinks(output)?;
    ensure!(fs::symlink_metadata(output).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound), "output already exists or cannot be inspected");
    let source = fs::canonicalize(source_copy).context("resolve offline source copy")?;
    let parent = fs::canonicalize(output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?;
    let output = parent.join(output.file_name().context("output must name a new directory")?);
    let mut journal = source.as_os_str().to_owned();
    journal.push("-journal");
    match fs::symlink_metadata(Path::new(&journal)) {
        Ok(metadata) => ensure!(metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() == 0, "rollback journal present; require a clean offline copy"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error.into()),
    }
    for protected in siblings(&source).into_iter().chain(std::iter::once(PathBuf::from(journal))) {
        ensure!(!protected.starts_with(&output), "output overlaps source database or SQLite sidecar");
    }
    let source_files_before = input_files(&source)?;
    directory(&output).context("create exclusive output directory")?;
    let result = (|| -> Result<Value> {
    json_file(&output.join("INCOMPLETE"), &json!({"artifact_kind":"offline-wire-archive-candidates","activation_allowed":false}))?;
    let staging = output.join("staging");
    directory(&staging)?;
    let captured = staging.join("capture.sqlite");
    // Copy only the explicitly supplied offline database and its WAL/SHM. This prevents
    // SQLite WAL read marks or recovery from modifying even the source shared-memory file.
    let mut identity = Vec::new();
    for (index, original) in siblings(&source).iter().enumerate() {
        if let Some((_,expected_bytes)) = source_files_before.iter().find(|entry| &entry.0 == original) {
            let mut input = File::open(original)?;
            let mut destination = new_file(&siblings(&captured)[index])?;
            let mut hash = Sha256::new();
            let mut buffer = [0; 65536];
            let mut bytes = 0_u64;
            loop {
                let n = input.read(&mut buffer)?;
                if n == 0 { break; }
                destination.write_all(&buffer[..n])?;
                hash.update(&buffer[..n]);
                bytes += n as u64;
            }
            ensure!(bytes == *expected_bytes, "offline input size changed while copying");
            identity.push((original.clone(),hex::encode(hash.finalize()),bytes));
            destination.sync_all()?;
        }
    }
    ensure!(input_identity(&source)? == identity, "offline input changed while capturing; no candidate accepted");
    validate_wal(&captured)?;
    let captured_connection = readonly(&captured)?;
    let pinned_path = staging.join("pinned.sqlite");
    vacuum(&captured_connection, &pinned_path).context("capture standalone read-only WAL snapshot")?;
    drop(captured_connection);
    remove_private_database(&captured)?;
    let mut pinned = readonly(&pinned_path)?;
    let version: u32 = pinned.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(version == smallclaims::store::CURRENT_SCHEMA_VERSION, "candidate construction requires current storage schema, found {version}");
    validate_wire_schema(&pinned)?;
    let source_wire_counts = wire_counts(&pinned)?;
    let expected_non_wire = non_wire_digests(&pinned)?;
    let expected_schema = schema_digest(&pinned)?;
    let wire_before = WIRE.iter().map(|table| table_digest(&pinned, table)).collect::<Result<Vec<_>>>()?;
    let working_path = staging.join("planning.sqlite");
    vacuum(&pinned, &working_path)?;
    let store = Store::open(&working_path, "offline-wire-archive-analysis")?;
    let plan_dir = staging.join("plan");
    directory(&plan_dir)?;
    let staged = store.checkpoint_staged_set(as_of_unix_ms, None, &plan_dir)?;
    #[cfg(any(test, feature = "test-support"))]
    let plan = smallclaims::store::checkpoint_staging::without_reference_plan(|| store.plan_staged_checkpoint(&staged))?;
    #[cfg(not(any(test, feature = "test-support")))]
    let plan = store.plan_staged_checkpoint(&staged)?;
    let working = readonly(&working_path)?;
    ensure!(schema_digest(&working)? == expected_schema, "native open changed schema; require a current complete source");
    for (index, table) in WIRE.iter().enumerate() {
        ensure!(table_digest(&working, table)? == wire_before[index], "native capture/planning changed {table}; refusing invented evidence (seal source batches before copying)");
    }
    drop(working);
    drop(store);
    remove_private_database(&working_path)?;
    let index_path = output.join("index.sqlite");
    let mut index = database(&index_path)?;
    index.execute_batch(/* sqlite-large-value: allow-for-now lv-h2beefc55ecb6 owner=fractal_store::wire_archive_candidates TODO=chunk-or-reference-variable-columns */ "CREATE TABLE eligible(writer TEXT NOT NULL,sequence INTEGER NOT NULL,envelope_hash TEXT NOT NULL,last_touch TEXT NOT NULL,PRIMARY KEY(writer,sequence,envelope_hash)) WITHOUT ROWID;")?;
    index.execute_batch(/* sqlite-large-value: allow-for-now lv-h8a8db2abd50c owner=fractal_store::wire_archive_candidates TODO=chunk-or-reference-variable-columns */ "CREATE TABLE archived(window_days INTEGER NOT NULL,writer TEXT NOT NULL,sequence INTEGER NOT NULL,envelope_hash TEXT NOT NULL,segment_path TEXT NOT NULL,segment_sha256 TEXT NOT NULL,PRIMARY KEY(window_days,writer,sequence,envelope_hash)) WITHOUT ROWID;")?;
    pinned.execute(/* sqlite-large-value: allow-for-now lv-h21e94fd71140 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ "ATTACH DATABASE ?1 AS archive_index", [index_path.to_str().context("non-UTF8 index path")?])?;
    let mut after = 0;
    loop {
        let page = staged.dropped_envelopes(after, MAX_ENVELOPES)?;
        if page.is_empty() { break; }
        after += page.len();
        let transaction = index.transaction()?;
        for tombstone in page {
            let id = ReplicaEnvelopeId {writer:tombstone.writer, sequence:tombstone.sequence, hash:tombstone.envelope_hash};
            let touch = last_touch(&pinned, &id).with_context(|| format!("cannot date eligible envelope {id:?}"))?;
            // Retain future evidence; exact boundary is recent, not archive eligible.
            if touch < as_of_unix_ms - 14 * DAY {
                transaction.execute(/* sqlite-large-value: allow-for-now lv-hd11656c908d0 owner=fractal_store::wire_archive_candidates TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO eligible VALUES(?1,?2,?3,?4)", params![id.writer,id.sequence,id.hash,touch.to_string()])?;
            }
        }
        transaction.commit()?;
    }
    let restore_path = staging.join("restore.sqlite");
    let mut restore = database(&restore_path)?;
    // Use exact source table declarations (including affinities and constraints), not a
    // CTAS approximation. No graph, full extra source store, or runtime is needed here.
    for table in WIRE {
        let sql: String = pinned.query_row("SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1", [table], |row| row.get(0))?;
        restore.execute_batch(/* sqlite-large-value: allow-for-now lv-h8959f39889d2 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ &sql)?;
    }
    let mut candidates = Vec::new();
    for days in [30_u128, 14] {
        let candidate_dir = output.join(format!("{days}d"));
        directory(&candidate_dir)?;
        let archive_dir = candidate_dir.join("archive");
        directory(&archive_dir)?;
        let hot_work_path = staging.join(format!("hot-{days}.sqlite"));
        vacuum(&pinned, &hot_work_path)?;
        let mut hot = Connection::open(&hot_work_path)?;
        // Preserve trigger definitions, but do not fire projection/inventory mutation hooks:
        // this is a physical offline wire split, never logical checkpoint application.
        hot.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
        hot.pragma_update(None, "foreign_keys", false)?;
        let manifest_path = candidate_dir.join("manifest.jsonl");
        let mut manifest = new_file(&manifest_path)?;
        let mut cursor = (String::new(), 0_u64, String::new());
        let mut segment_count = 0;
        let mut envelope_count = 0;
        let mut archive_bytes = 0_u64;
        loop {
            let page = index.prepare("SELECT writer,sequence,envelope_hash,last_touch FROM eligible WHERE (writer,sequence,envelope_hash)>(?1,?2,?3) ORDER BY writer,sequence,envelope_hash LIMIT ?4")?
                .query_map(params![cursor.0,cursor.1,cursor.2,MAX_ENVELOPES], |row| Ok((ReplicaEnvelopeId {writer:row.get(0)?,sequence:row.get(1)?,hash:row.get(2)?},row.get::<_,String>(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if page.is_empty() { break; }
            let last = &page.last().context("empty index page")?.0;
            cursor = (last.writer.clone(),last.sequence,last.hash.clone());
            let ids = page.into_iter().filter_map(|(id,touch)| match touch.parse::<u128>() {
                Ok(touch) if touch < as_of_unix_ms - days * DAY => Some(Ok(id)),
                Ok(_) => None,
                Err(error) => Some(Err(anyhow::Error::from(error))),
            }).collect::<Result<Vec<_>>>()?;
            if ids.is_empty() { continue; }
            let name = format!("segment-{segment_count:08}.wire");
            let segment_path = archive_dir.join(&name);
            let seal = write_segment(&mut pinned, &ids, &segment_path)?;
            let mut reader = SegmentReader::open(&segment_path, &seal)?;
            ensure!(reader.identities().eq(ids.iter()), "archive index identity mismatch");
            reader.restore_wire_rows(&mut restore)?;
            let reexport = staging.join("reexport.wire");
            let restored_seal = write_segment(&mut restore, &ids, &reexport)?;
            ensure!(seal.sha256 == restored_seal.sha256 && seal.bytes == restored_seal.bytes && seal.envelopes == restored_seal.envelopes, "restore/re-export seal differs");
            fs::remove_file(reexport)?;
            restore.execute_batch(/* sqlite-large-value: allow-for-now lv-h34b202aa6485 owner=fractal_store::wire_archive_candidates TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures; DELETE FROM replica_records; DELETE FROM replica_envelopes;")?;
            // External seals and indexed identities are durable before any hot removal.
            serde_json::to_writer(&mut manifest, &json!({"path":format!("archive/{name}"),"seal":seal,"identities":ids,"proof":"exact-typed-restore-and-identical-reexport"}))?;
            manifest.write_all(b"\n")?;
            manifest.sync_all()?;
            let indexed = index.transaction()?;
            for id in &ids {
                indexed.execute(/* sqlite-large-value: allow-for-now lv-h5d4e2adec751 owner=fractal_store::wire_archive_candidates TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO archived VALUES(?1,?2,?3,?4,?5,?6)", params![days as u32,id.writer,id.sequence,id.hash,format!("{days}d/archive/{name}"),seal.sha256])?;
            }
            indexed.commit()?;
            let transaction = hot.transaction()?;
            for id in &ids {
                for table in ["replica_envelope_signatures","replica_records","replica_envelopes"] {
                    transaction.execute(&format!(/* sqlite-large-value: allow-for-now lv-h3d50e028f454 owner=fractal_store::wire_archive_candidates TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM {table} WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3"), params![id.writer,id.sequence,id.hash])?;
                }
            }
            transaction.commit()?;
            segment_count += 1;
            envelope_count += ids.len();
            archive_bytes += seal.bytes;
        }
        ensure!(non_wire_digests(&hot)? == expected_non_wire, "non-wire data changed during candidate construction");
        ensure!(schema_digest(&hot)? == expected_schema, "candidate schema changed");
        for table in WIRE {
            let predicate = format!("WHERE NOT EXISTS (SELECT 1 FROM archive_index.archived AS cold WHERE cold.window_days={days} AND cold.writer=wire.writer AND cold.sequence=wire.sequence AND cold.envelope_hash=wire.envelope_hash)");
            ensure!(filtered_table_digest(&pinned, table, &predicate)? == table_digest(&hot, table)?, "hot/archive partition differs from pinned {table}");
        }
        drop(hot);
        let hot_path = candidate_dir.join("hot.sqlite");
        let hot_source = readonly(&hot_work_path)?;
        vacuum(&hot_source, &hot_path)?;
        drop(hot_source);
        remove_private_database(&hot_work_path)?;
        let hot_bytes = fs::metadata(&hot_path)?.len();
        let hot_counts = wire_counts(&readonly(&hot_path)?)?;
        let manifest_bytes = fs::metadata(&manifest_path)?.len();
        let manifest_sha256 = file_digest(&manifest_path)?;
        candidates.push(json!({
            "window_days":days, "hot_path":format!("{days}d/hot.sqlite"),
            "manifest_path":format!("{days}d/manifest.jsonl"), "manifest_sha256":manifest_sha256,
            "hot_bytes":hot_bytes, "archive_bytes":archive_bytes, "manifest_bytes":manifest_bytes,
            "archived_envelopes":envelope_count, "hot_counts":hot_counts, "segments":segment_count,
            "age_protected_checkpoint_eligible_envelopes":after.checked_sub(envelope_count).context("archive exceeds native drop plan")?,
            "activation_allowed":false, "production_invariants_preserved":false
        }));
    }
    drop(restore);
    drop(staged);
    drop(pinned);
    let staging_bytes = tree_bytes(&staging)?;
    index.execute_batch(/* sqlite-large-value: allow-for-now lv-h63437ec326c9 owner=fractal_store::wire_archive_candidates TODO=trace-statement-and-value-representation */ "VACUUM")?;
    drop(index);
    let index_bytes = fs::metadata(&index_path)?.len();
    ensure!(input_identity(&source)? == identity, "offline source changed during construction; report withheld");
    let source_files = identity.iter().map(|(path,digest,bytes)| json!({"name":path.file_name().map(|name|name.to_string_lossy()),"sha256":digest,"bytes":bytes})).collect::<Vec<_>>();
    let report = json!({
        "status":"complete", "artifact_kind":"offline-wire-archive-candidates",
        "activation_allowed":false, "production_invariants_preserved":false,
        "scope":"offline fixtures/copies only; not a daemon store, deployment, or production M1 result; source digests do not fence active writers",
        "as_of_unix_ms":as_of_unix_ms, "source_files":source_files, "source_counts":source_wire_counts,
        "source_file_bytes":identity.iter().map(|v|v.2).sum::<u64>(),
        "snapshot_bytes":fs::metadata(pinned_path)?.len(),
        "staging_bytes_before_cleanup":staging_bytes, "staging_bytes_retained":0, "index_bytes":index_bytes,
        "native_plan_retained_sealed_envelopes":plan.sealed_envelopes.checked_sub(plan.dropped_envelopes).context("invalid native plan counts")?,
        "plan":plan,
        "proof":{
            "source_files_unchanged":true, "planning_wire_tables_unchanged":true,
            "non_wire_tables_unchanged":true, "schema_unchanged":true,
            "segment_restore_reexport_identical":true, "exact_hot_archive_partition":true,
            "age_policy":"strictly older accepted, admission, record update/repair, and signature stored times; unknown/malformed eligible timestamps fail closed"
        },
        "candidates":candidates
    });
    fs::remove_dir_all(staging)?;
    private_tree(&output)?;
    // Atomic publication is only this report: partial directories have no success report.
    json_file(&output.join("report.pending"), &report)?;
    fs::rename(output.join("report.pending"), output.join("report.json"))?;
    fs::remove_file(output.join("INCOMPLETE"))?;
    File::open(&output)?.sync_all()?;
    Ok(report)
    })();
    match result {
        Ok(report) => Ok(report),
        Err(error) => {
            // An I/O failure during the final commit must withdraw any success report.
            let cleanup = (|| -> Result<()> {
                let report = output.join("report.json");
                if report.exists() { fs::remove_file(report)?; }
                if !output.join("INCOMPLETE").exists() {
                    json_file(&output.join("INCOMPLETE"), &json!({"activation_allowed":false,"status":"incomplete"}))?;
                }
                private_tree(&output)?;
                Ok(())
            })();
            match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(error.context(format!("also failed to secure incomplete artifacts: {cleanup:#}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests;
