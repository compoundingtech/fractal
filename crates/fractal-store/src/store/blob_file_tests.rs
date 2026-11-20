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
fn a_spent_budget_still_moves_one_blob_a_step_and_the_cursor_survives_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let hashes: Vec<String> =
        (0..3).map(|n| document(&store, &format!("doc/example/{n}"), format!("bytes {n}").repeat(20).as_bytes())).collect();
    let later = start_window(&store) + WINDOW_MS;
    let step = store.move_blob_bytes_step(later, WINDOW_MS, Duration::ZERO).unwrap();
    assert_eq!((step.rows, step.more), (1, true));
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    let mut moved = 1;
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
