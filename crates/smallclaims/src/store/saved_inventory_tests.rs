//! The inventory a clean shutdown saves, and when the next open may extend it instead of
//! reading every envelope. `FULL_INVENTORY_LOADS` counts the full reads on this thread.
use super::runtime::Plain;
use super::*;

const FLEET: &str = "invented-fleet";

fn open(path: &Path) -> Store {
    let store = Store::open(path, "cedar", Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store
}

fn write_notes(store: &Store, prefix: &str, count: usize) {
    for index in 0..count {
        store
            .append_claim(&ClaimInput {
                subject: format!("note/{prefix}/{index}"),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
}

fn full_loads() -> usize {
    FULL_INVENTORY_LOADS.with(std::cell::Cell::get)
}

/// The snapshot's inventory is the one a read from nothing finds.
fn assert_matches_the_store(store: &Store, snapshot: &ReplicationSnapshot) {
    let loads = full_loads();
    let (full, max_rowid) = full_compact_replication_inventory(&store.readers.get()).unwrap();
    FULL_INVENTORY_LOADS.with(|count| count.set(loads));
    assert_eq!(snapshot.inventory.digest, full.digest);
    assert_eq!(snapshot.buckets, full.buckets());
    assert_eq!(snapshot.inventory.envelopes, full.envelopes);
    assert_eq!(snapshot.inventory.payloadless, full.payloadless);
    assert_eq!(snapshot.max_envelope_rowid, max_rowid);
}

/// Building the first inventory after a restart read every envelope, checkpoint tombstone and
/// retired identity in one query: 19.9 s on a busy host's store, which every export and
/// receive in the first minute waited for. A clean shutdown now saves the inventory, and the
/// next open extends it by what was appended since, without that read.
#[test]
fn a_restart_extends_the_inventory_its_shutdown_saved_instead_of_reading_every_envelope() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    {
        let store = open(&path);
        write_notes(&store, "history", 3_000);
        store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    }
    assert!(saved_inventory::path(&path).exists());
    let store = open(&path);
    write_notes(&store, "after-restart", 5);
    FULL_INVENTORY_LOADS.with(|count| count.set(0));
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(full_loads(), 0, "the first snapshot read every envelope again");
    assert!(!saved_inventory::path(&path).exists(), "reading the saved inventory consumes it");
    assert_matches_the_store(&store, &snapshot);
    assert_eq!(snapshot.envelope_rows, 3_005);
    // The process that read it saves again when it stops, and the next one extends that.
    assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    drop(store);
    let store = open(&path);
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(full_loads(), 0);
    assert_matches_the_store(&store, &snapshot);
}

/// A change other than an append, made while no daemon of this build runs, as an older binary
/// would make it: the database's own triggers end the epoch the file names.
#[test]
fn a_change_beyond_appends_after_the_save_makes_the_next_open_read_the_store() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    {
        let store = open(&path);
        write_notes(&store, "history", 40);
        store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    }
    {
        let other = rusqlite::Connection::open(&path).unwrap();
        other
            .execute(
                /* sqlite-large-value: allow-for-now lv-hfe4cb2c2ce82 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes)",
                [],
            )
            .unwrap();
    }
    let store = open(&path);
    FULL_INVENTORY_LOADS.with(|count| count.set(0));
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(full_loads(), 1);
    assert!(!saved_inventory::path(&path).exists());
    assert_matches_the_store(&store, &snapshot);
}

/// Any change but an append during the run ends its epoch, so it saves nothing.
#[test]
fn a_change_beyond_appends_during_the_run_leaves_nothing_to_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    let store = open(&path);
    write_notes(&store, "history", 40);
    store.replication_snapshot().unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h5a19c2f3c52b owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes)",
            [],
        )
        .unwrap();
    store.replication_snapshot().unwrap();
    assert!(matches!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Skipped(_)));
    assert!(!saved_inventory::path(&path).exists());
}

/// An open that does not read the file, such as another process's, ends the epoch it names.
#[test]
fn an_open_between_the_save_and_the_next_read_retires_the_saved_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    {
        let store = open(&path);
        write_notes(&store, "history", 40);
        store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    }
    drop(open(&path));
    let store = open(&path);
    FULL_INVENTORY_LOADS.with(|count| count.set(0));
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(full_loads(), 1);
    assert_matches_the_store(&store, &snapshot);
}

/// A file damaged after it was written is refused and removed, and the store is read.
#[test]
fn a_damaged_saved_inventory_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    {
        let store = open(&path);
        write_notes(&store, "history", 40);
        store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    }
    let saved = saved_inventory::path(&path);
    let mut bytes = std::fs::read(&saved).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&saved, bytes).unwrap();
    let store = open(&path);
    FULL_INVENTORY_LOADS.with(|count| count.set(0));
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(full_loads(), 1);
    assert!(!saved.exists());
    assert_matches_the_store(&store, &snapshot);
}

/// A store put back from a copy taken before the save holds fewer envelopes than the file
/// lists: it is read, not extended.
#[test]
fn a_store_restored_from_an_earlier_copy_does_not_take_a_later_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    let copy = directory.path().join("copy.sqlite3");
    {
        let store = open(&path);
        write_notes(&store, "history", 40);
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(/* sqlite-large-value: allow-for-now lv-h61ed8d5bde9c owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ "VACUUM INTO ?1", [copy.to_str().unwrap()])
            .unwrap();
        write_notes(&store, "after-the-copy", 10);
        store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    }
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
    std::fs::rename(&copy, &path).unwrap();
    let store = open(&path);
    FULL_INVENTORY_LOADS.with(|count| count.set(0));
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(full_loads(), 1);
    assert_matches_the_store(&store, &snapshot);
}
