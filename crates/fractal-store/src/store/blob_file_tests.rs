//! Blob bytes move into the store's blob file after the window, read the same from everywhere,
//! fence older builds out, and come back on a restore.
use super::*;
use smallclaims::store::blob_file::{self, STORAGE_VERSION};
use std::time::Duration;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const WINDOW_MS: u64 = 3 * DAY_MS;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

fn document(store: &Store, name: &str, bytes: &[u8]) -> String {
    store.put_document(name, bytes, &None, &format!("key-{name}")).unwrap().hash
}

/// The `blobs` row of `hash`: its own byte count and its size.
fn row(store: &Store, hash: &str) -> (i64, i64) {
    store
        .readers
        .get()
        .query_row("SELECT length(bytes), size FROM blobs WHERE hash=?1", [hash], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
}

fn user_version(store: &Store) -> u32 {
    store.readers.get().query_row("PRAGMA main.user_version", [], |row| row.get(0)).unwrap()
}

/// The first step records when this member first ran the build; the window counts from there.
fn start_window(store: &Store) -> u64 {
    let started = now_ms();
    assert!(store.move_blob_bytes_step(started, WINDOW_MS, Duration::from_secs(1)).unwrap().waiting);
    started
}

fn move_all(store: &Store, now: u64) -> (usize, u64) {
    let (mut rows, mut bytes) = (0, 0);
    loop {
        let step = store.move_blob_bytes_step(now, WINDOW_MS, Duration::from_secs(1)).unwrap();
        rows += step.rows;
        bytes += step.bytes;
        if !step.more {
            return (rows, bytes);
        }
    }
}

fn restore_all(store: &Store, now: u64) -> usize {
    let mut rows = 0;
    loop {
        let step = store.restore_blob_bytes_step(now).unwrap();
        rows += step.rows;
        if !step.more {
            return rows;
        }
    }
}

#[test]
fn archive_blob_digest_keeps_the_logical_table_before_and_after_bytes_move() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    let legacy_blob_digest = || {
        smallclaims::store::projection_digest::oracle(&store.readers.get(), &[("blobs", &[])])
            .unwrap()["blobs"].clone()
    };
    let empty = store.backup_header().unwrap();
    assert_eq!(empty.tables["blobs"].count, 0);
    assert_eq!(empty.tables["blobs"].digest, legacy_blob_digest());

    document(&store, "doc/example/archive", b"archive bytes");
    document(&store, "doc/example/empty-archive", b"");
    let before = store.backup_header().unwrap();
    assert_eq!(before.tables["blobs"].count, 2);
    assert_eq!(before.tables["blobs"].digest, legacy_blob_digest());
    let later = start_window(&store) + WINDOW_MS;
    assert_eq!(move_all(&store, later).0, 1);
    let after = store.backup_header().unwrap();
    assert_eq!(after.tables, before.tables);
    assert_eq!(after.graph_digest, before.graph_digest);
    assert!(!projection_digest::tables(&store.readers.get()).unwrap().contains_key("blobs"));
}

#[test]
fn bytes_stay_through_the_window_then_move_and_read_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let body = b"the bytes of a document".repeat(100);
    let hash = document(&store, "doc/example/a", &body);
    let started = now_ms();
    let before = user_version(&store);

    // Inside the window nothing moves: a rollback finds every byte where it looks.
    let step = store.move_blob_bytes_step(started, WINDOW_MS, Duration::from_secs(1)).unwrap();
    assert!(step.waiting);
    assert_eq!(row(&store, &hash), (body.len() as i64, body.len() as i64));
    assert_eq!(user_version(&store), before);

    let (rows, bytes) = move_all(&store, started + WINDOW_MS);
    assert_eq!((rows, bytes), (1, body.len() as u64));
    // The row stays, with its size; only its bytes moved.
    assert_eq!(row(&store, &hash), (0, body.len() as i64));
    assert_eq!(store.blob_bytes(&hash).unwrap().as_deref(), Some(&body[..]));
    assert_eq!(store.get_blob(&hash).unwrap().as_deref(), Some(&body[..]));
    assert_eq!(store.get_document("doc/example/a", &hash).unwrap().as_deref(), Some(&body[..]));
    // The first emptying fenced older builds out.
    assert_eq!(user_version(&store), STORAGE_VERSION);
    assert!(path.with_file_name("claims.blobs.sqlite3").exists());

    // A reopen reads the moved bytes through a fresh connection.
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    assert_eq!(store.get_document("doc/example/a", &hash).unwrap().as_deref(), Some(&body[..]));
    // A second pass has nothing left to move.
    assert_eq!(move_all(&store, started + WINDOW_MS + DAY_MS), (0, 0));
}

#[test]
fn a_blob_of_zero_bytes_keeps_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    let hash = document(&store, "doc/example/empty", b"");
    let (rows, _) = move_all(&store, start_window(&store) + WINDOW_MS);
    assert_eq!(rows, 0);
    assert_eq!(store.get_document("doc/example/empty", &hash).unwrap().as_deref(), Some(&b""[..]));
    assert_ne!(user_version(&store), STORAGE_VERSION, "nothing moved, so nothing is fenced");
}

#[test]
fn a_restore_puts_every_byte_back_and_lowers_the_fence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let first = b"first".repeat(50);
    let second = b"second".repeat(50);
    let a = document(&store, "doc/example/a", &first);
    let b = document(&store, "doc/example/b", &second);
    let before = user_version(&store);
    let later = start_window(&store) + WINDOW_MS;
    assert_eq!(move_all(&store, later).0, 2);
    assert_eq!(user_version(&store), STORAGE_VERSION);

    assert_eq!(restore_all(&store, later), 2);
    assert_eq!(row(&store, &a), (first.len() as i64, first.len() as i64));
    assert_eq!(row(&store, &b), (second.len() as i64, second.len() as i64));
    assert_eq!(user_version(&store), before, "back to what it was before the first move");
    // The window starts again: the pass waits before moving bytes anew.
    assert!(store.move_blob_bytes_step(later, WINDOW_MS, Duration::from_secs(1)).unwrap().waiting);
    assert_eq!(move_all(&store, later + WINDOW_MS).0, 2);
}

#[test]
fn a_spent_budget_still_copies_one_blob_a_step_and_the_cursors_survive_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let hashes: Vec<String> =
        (0..3).map(|n| document(&store, &format!("doc/example/{n}"), format!("bytes {n}").repeat(20).as_bytes())).collect();
    let later = start_window(&store) + WINDOW_MS;
    // A spent budget still copies one blob a step; the rows let go of their bytes in later steps.
    let step = store.move_blob_bytes_step(later, WINDOW_MS, Duration::ZERO).unwrap();
    assert_eq!((step.copied, step.rows, step.more), (1, 0, true));
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    let mut moved = 0;
    loop {
        let step = store.move_blob_bytes_step(later, WINDOW_MS, Duration::ZERO).unwrap();
        moved += step.rows;
        if !step.more {
            break;
        }
    }
    assert_eq!(moved, 3);
    for (n, hash) in hashes.iter().enumerate() {
        assert_eq!(row(&store, hash).0, 0);
        assert_eq!(store.blob_bytes(hash).unwrap(), Some(format!("bytes {n}").repeat(20).into_bytes()));
    }
}

#[test]
fn the_blob_file_sits_beside_the_store() {
    assert_eq!(
        blob_file::location(std::path::Path::new("/state/claims.sqlite3")),
        "/state/claims.blobs.sqlite3"
    );
    assert_eq!(blob_file::location(std::path::Path::new("/state/store.db")), "/state/store.blobs.db");
    assert_eq!(
        blob_file::location(std::path::Path::new("file:st3-x?mode=memory&cache=shared")),
        "file:st3-x-blobs?mode=memory&cache=shared"
    );
}

#[test]
fn a_copy_of_the_store_carries_the_moved_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    let body = b"bytes a store copy must keep".repeat(40);
    let hash = document(&store, "doc/example/copied", &body);
    assert_eq!(move_all(&store, start_window(&store) + WINDOW_MS).0, 1);
    let copy = dir.path().join("copy.sqlite3");
    store.copy_store_to(&copy).unwrap();
    assert!(dir.path().join("copy.blobs.sqlite3").exists(), "the blob file is copied beside the store copy");
    drop(store);
    let copied = Store::open(&copy, "node").unwrap();
    assert_eq!(row(&copied, &hash).0, 0, "the copy holds the emptied row");
    assert_eq!(copied.get_document("doc/example/copied", &hash).unwrap().as_deref(), Some(&body[..]));
}

#[test]
fn a_crash_after_the_copy_and_before_the_emptying_finishes_on_the_next_step() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    let body = b"copied, not yet emptied".repeat(30);
    let hash = document(&store, "doc/example/half", &body);
    let later = start_window(&store) + WINDOW_MS;
    // The first hold committed its copy; the process died before the second.
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h10c52f5e6fca owner=fractal_store::store::blob_file_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO blob_file.blobs(hash, bytes, size) SELECT hash, bytes, size FROM main.blobs WHERE hash=?1",
            [&hash],
        )
        .unwrap();
    assert_eq!(row(&store, &hash).0, body.len() as i64, "the row still holds its own bytes");
    assert_eq!(store.blob_bytes(&hash).unwrap().as_deref(), Some(&body[..]));
    assert_eq!(move_all(&store, later), (1, body.len() as u64));
    assert_eq!(row(&store, &hash).0, 0);
    assert_eq!(store.blob_bytes(&hash).unwrap().as_deref(), Some(&body[..]));
}

#[test]
fn a_row_keeps_its_bytes_until_a_checkpoint_has_put_the_copy_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let body = b"bytes whose copy is not yet on disk".repeat(30);
    let hash = document(&store, "doc/example/undurable", &body);
    let later = start_window(&store) + WINDOW_MS;
    // A reader of the blob file whose snapshot predates the copy: no checkpoint can backfill the
    // frames the copy commits while it reads, so the copy is committed but not in the file.
    let reader = rusqlite::Connection::open(blob_file::location(&path)).unwrap();
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-hd008c9bc7d60 owner=fractal_store::store::blob_file_tests TODO=trace-statement-and-value-representation */ "BEGIN; SELECT count(*) FROM blobs;").unwrap();
    let copy = store.move_blob_bytes_step(later, WINDOW_MS, Duration::from_secs(1)).unwrap();
    assert_eq!((copy.copied, copy.rows, copy.more), (1, 0, true), "the copy");
    for _ in 0..3 {
        let step = store.move_blob_bytes_step(later, WINDOW_MS, Duration::from_secs(1)).unwrap();
        assert_eq!((step.rows, step.more), (0, true), "nothing is emptied while the copy's frames are not backfilled");
        assert_eq!(row(&store, &hash).0, body.len() as i64, "the row keeps its own bytes");
    }
    // The reader ends; the next checkpoint backfills and syncs the copy, and the row lets go.
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-h8c744bc68a88 owner=fractal_store::store::blob_file_tests TODO=trace-statement-and-value-representation */ "ROLLBACK").unwrap();
    drop(reader);
    assert_eq!(move_all(&store, later), (1, body.len() as u64));
    assert_eq!(row(&store, &hash).0, 0);
    assert_eq!(store.blob_bytes(&hash).unwrap().as_deref(), Some(&body[..]));
}

/// Version 26 (compacted signatures) carries 25's meaning: a store fenced at 26 after its blob
/// bytes moved opens in this build and reads them, and neither a later move nor a restore
/// touches the higher fence.
#[test]
fn a_store_at_26_opens_and_reads_its_moved_blob_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let body = b"bytes kept in the blob file".repeat(40);
    let hash = document(&store, "doc/example/fenced", &body);
    let later = start_window(&store) + WINDOW_MS;
    assert_eq!(move_all(&store, later).0, 1);
    assert_eq!(user_version(&store), STORAGE_VERSION);
    // As the compaction writer fences it.
    store.connection.write().pragma_update(None, "user_version", 26).unwrap();
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    assert_eq!(user_version(&store), 26);
    assert_eq!(store.get_document("doc/example/fenced", &hash).unwrap().as_deref(), Some(&body[..]));
    let more = b"a later document".repeat(40);
    let other = document(&store, "doc/example/later", &more);
    assert_eq!(move_all(&store, later + DAY_MS).0, 1);
    assert_eq!(user_version(&store), 26, "a later move never lowers a higher fence");
    assert_eq!(restore_all(&store, later + DAY_MS), 2);
    assert_eq!(user_version(&store), 26, "a restore lowers only its own fence, 25");
    assert_eq!(store.get_document("doc/example/later", &other).unwrap().as_deref(), Some(&more[..]));
    assert!(smallclaims::store::MAX_SUPPORTED_SCHEMA_VERSION >= 26, "this build reads a store at 26");
}

/// Storage versions only rise: a store at the valid-record version (27, above the blob file's 25)
/// keeps it through a blob move and a whole restore, whichever came first.
#[test]
fn a_store_past_the_blob_fence_keeps_its_version_through_a_move_and_a_restore() {
    let gone = smallclaims::store::record_bytes::VALID_RECORDS_GONE_VERSION;
    assert!(gone > STORAGE_VERSION);
    let set = |store: &Store, version: u32| store.connection.write().pragma_update(None, "user_version", version).unwrap();
    // At 27 before any blob moves.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    document(&store, "doc/sample-a", b"bytes that move to the blob file");
    set(&store, gone);
    let started = start_window(&store);
    assert!(move_all(&store, started + WINDOW_MS).0 > 0);
    assert_eq!(user_version(&store), gone, "a move never lowers a store past its fence");
    restore_all(&store, started + WINDOW_MS);
    assert_eq!(user_version(&store), gone, "a restore never lowers a store past its fence");
    // At 25 from a move, then 27 from the first valid-record deletion, then restored.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    document(&store, "doc/sample-b", b"bytes that move to the blob file");
    let started = start_window(&store);
    move_all(&store, started + WINDOW_MS);
    assert_eq!(user_version(&store), STORAGE_VERSION);
    set(&store, gone);
    restore_all(&store, started + WINDOW_MS);
    assert_eq!(user_version(&store), gone);
}

fn compact_all(store: &Store, batch: usize) -> usize {
    let mut relocated = 0;
    loop {
        let step = store.compact_blob_rows_step(batch, Duration::from_secs(1)).unwrap();
        relocated += step.relocated;
        if !step.more {
            return relocated;
        }
    }
}

fn pages(store: &Store) -> i64 {
    store.readers.get().query_row("PRAGMA main.page_count", [], |row| row.get(0)).unwrap()
}

/// Documents of a few KiB each, so each moved row leaves most of a page behind.
fn documents(store: &Store, prefix: &str, count: usize) -> Vec<(String, Vec<u8>)> {
    (0..count)
        .map(|n| {
            let bytes = format!("{prefix}-{n}-").repeat(400).into_bytes();
            (document(store, &format!("doc/{prefix}-{n}"), &bytes), bytes)
        })
        .collect()
}

#[test]
fn compaction_packs_the_moved_rows_and_gives_their_pages_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    // Nothing fires on a relocated row: no trigger on blobs in a store this build opened.
    let triggers: i64 = store.readers.get().query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND tbl_name='blobs'", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(triggers, 0);
    let written = documents(&store, "doc", 300);
    let later = start_window(&store) + WINDOW_MS;
    // Before the move nothing is packed.
    assert_eq!(store.compact_blob_rows_step(500, Duration::from_secs(1)).unwrap(), Default::default());
    assert_eq!(move_all(&store, later).0, written.len());
    store.connection.write().execute_batch("PRAGMA incremental_vacuum").unwrap();
    let sparse = pages(&store);
    assert_eq!(compact_all(&store, 50), written.len());
    store.connection.write().execute_batch("PRAGMA incremental_vacuum").unwrap();
    let packed = pages(&store);
    assert!(packed < sparse, "packing gave pages back: {sparse} -> {packed}");
    // Every blob reads the same, by hash, and its document still resolves.
    for (hash, bytes) in &written {
        assert_eq!(store.blob_bytes(hash).unwrap().as_deref(), Some(bytes.as_slice()));
        assert_eq!(row(&store, hash).0, 0);
    }
    // Done: a later call finds nothing, and a reopen keeps it done.
    assert_eq!(compact_all(&store, 50), 0);
    drop(store);
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    assert_eq!(compact_all(&store, 50), 0);
    assert_eq!(store.blob_bytes(&written[0].0).unwrap().as_deref(), Some(written[0].1.as_slice()));
}

#[test]
fn the_move_after_a_relocation_neither_revisits_nor_skips_a_row_with_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    let first = documents(&store, "first", 40);
    let later = start_window(&store) + WINDOW_MS;
    assert_eq!(move_all(&store, later).0, first.len());
    // New uploads arrive after the move caught up, then one relocation batch runs: the move is
    // no longer caught up, so its cursors stay, and the relocated rows land past the new ones.
    let second = documents(&store, "second", 30);
    let relocated = store.compact_blob_rows_step(25, Duration::from_secs(1)).unwrap();
    assert_eq!(relocated.relocated, 25);
    assert!(relocated.more);
    // The move's passes take each new row once and nothing else.
    assert_eq!(move_all(&store, later).0, second.len());
    let copies: i64 = store.readers.get().query_row("SELECT COUNT(*) FROM blob_file.blobs", [], |row| row.get(0)).unwrap();
    assert_eq!(copies as usize, first.len() + second.len());
    for (hash, bytes) in first.iter().chain(&second) {
        assert_eq!(row(&store, hash).0, 0, "every row let go of its bytes");
        assert_eq!(store.blob_bytes(hash).unwrap().as_deref(), Some(bytes.as_slice()));
    }
    // The rest of the pass, interleaved with nothing, then a restore, still finds every byte.
    assert_eq!(compact_all(&store, 25), first.len() - 25);
    assert_eq!(restore_all(&store, later), first.len() + second.len());
    for (hash, bytes) in first.iter().chain(&second) {
        assert_eq!(row(&store, hash).0 as usize, bytes.len());
    }
}

#[test]
fn a_caught_up_move_steps_past_the_relocated_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    let written = documents(&store, "doc", 200);
    let later = start_window(&store) + WINDOW_MS;
    move_all(&store, later);
    assert_eq!(compact_all(&store, 1_000), written.len());
    // The move's cursors moved past the relocated rows, so its next pass has nothing to page.
    let step = store.move_blob_bytes_step(later, WINDOW_MS, Duration::from_secs(1)).unwrap();
    assert_eq!((step.rows, step.copied, step.more), (0, 0, false));
}

#[test]
fn compaction_refuses_a_table_with_a_trigger_and_resumes_after_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let written = documents(&store, "doc", 60);
    let later = start_window(&store) + WINDOW_MS;
    move_all(&store, later);
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h2e5a0ad66906 owner=fractal_store::store::blob_file_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER fixture_blobs_watch AFTER UPDATE ON blobs BEGIN SELECT 1; END;",
    ).unwrap();
    assert!(store.compact_blob_rows_step(10, Duration::from_secs(1)).is_err());
    store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h82bc26f03569 owner=fractal_store::store::blob_file_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER fixture_blobs_watch").unwrap();
    // One batch, a reopen, and the rest: every row once.
    assert_eq!(store.compact_blob_rows_step(10, Duration::from_secs(1)).unwrap().relocated, 10);
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    assert_eq!(compact_all(&store, 10), written.len() - 10);
    for (hash, bytes) in &written {
        assert_eq!(store.blob_bytes(hash).unwrap().as_deref(), Some(bytes.as_slice()));
    }
}
