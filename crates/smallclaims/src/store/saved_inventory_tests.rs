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

/// A change other than an append after the last snapshot was read makes a stop save nothing;
/// a full read after it takes it in, and saves.
#[test]
fn a_change_beyond_appends_after_the_last_snapshot_leaves_nothing_to_save() {
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
    assert!(matches!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Skipped(_)));
    assert!(!saved_inventory::path(&path).exists());
    store.replication_snapshot().unwrap();
    assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
}

/// A payload deleted under a snapshot without changing what it lists (as a trim does) is not
/// taken in by extending it, so the extended snapshot is not saved either.
#[test]
fn an_extension_over_a_payload_change_is_not_saved() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    let store = open(&path);
    write_notes(&store, "history", 40);
    store.replication_snapshot().unwrap();
    let connection = store.connection.write();
    // As a trim deletes a payload its checkpoint lists: the identity stays, as a tombstone.
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h452c69f7277e owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
             SELECT writer, sequence, envelope_hash, 0, 'invented-checkpoint' FROM replica_envelopes
             WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes);",
        )
        .unwrap();
    drop(connection);
    store.replication_snapshot().unwrap();
    let connection = store.connection.write();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h7adac784a2b7 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes)", [])
        .unwrap();
    drop(connection);
    write_notes(&store, "after-the-trim", 1);
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(snapshot.inventory_changes, None);
    assert!(matches!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Skipped(_)));
}

/// Opening the store writes nothing, so an open that does not build a snapshot, such as a
/// tool's, leaves the saved inventory for the next one that does.
#[test]
fn an_open_between_the_save_and_the_next_read_leaves_the_saved_inventory() {
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
    assert_eq!(full_loads(), 0);
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

/// A small deterministic generator, so a failing seed names its own repeat.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn hash(&mut self) -> String {
        (0..4).map(|_| format!("{:016x}", self.next())).collect()
    }
}

/// A connection as another binary or a tool opens the database, with the writer's functions.
fn other_binary(path: &Path) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(path).unwrap();
    projection_digest::register(&connection).unwrap();
    connection
}

const COLUMNS: &str = "writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms, payload, batch_id, relay, receipt_state, validation_error, received_at_unix_ms";

/// Append, as an older binary would, a new envelope at the tail or below it.
fn insert_envelope(connection: &rusqlite::Connection, mix: &mut Mix, below_the_tail: bool) {
    let rowid: i64 = connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h806c710cfd5d owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ if below_the_tail {
                "SELECT MIN(rowid)-1 FROM replica_envelopes"
            } else {
                "SELECT MAX(rowid)+1 FROM replica_envelopes"
            },
            [],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            &format!(
                /* sqlite-large-value: allow-for-now lv-h3a37f31e4de5 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes(rowid, {COLUMNS})
                 SELECT ?1, 'invented-peer', sequence+1000000+?1, ?2, previous_hash, accepted_at_unix_ms, payload, batch_id, relay,
                        receipt_state, validation_error, received_at_unix_ms
                 FROM replica_envelopes WHERE rowid=(SELECT MAX(rowid) FROM replica_envelopes)"
            ),
            rusqlite::params![rowid, mix.hash()],
        )
        .unwrap();
}

/// The rowid of some held envelope.
fn some_envelope(connection: &rusqlite::Connection, mix: &mut Mix) -> i64 {
    let rows: i64 = connection.query_row("SELECT COUNT(*) FROM replica_envelopes", [], |row| row.get(0)).unwrap();
    connection
        .query_row(
            "SELECT rowid FROM replica_envelopes ORDER BY rowid LIMIT 1 OFFSET ?1",
            [mix.below(rows as u64) as i64],
            |row| row.get(0),
        )
        .unwrap()
}

/// One change, as another binary or a tool makes it while no daemon of this build runs.
fn change(connection: &rusqlite::Connection, mix: &mut Mix) -> &'static str {
    let kinds = 10;
    match mix.below(kinds) {
        0 => {
            insert_envelope(connection, mix, false);
            "append at the tail"
        }
        1 => {
            insert_envelope(connection, mix, true);
            "insert below the tail"
        }
        2 => {
            let rowid = some_envelope(connection, mix);
            connection.execute(/* sqlite-large-value: allow-for-now lv-h4e5b95e07354 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=?1", [rowid]).unwrap();
            "delete an envelope"
        }
        3 => {
            let rowid = some_envelope(connection, mix);
            connection
                .execute(/* sqlite-large-value: allow-for-now lv-h0b5217008635 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET envelope_hash=?2 WHERE rowid=?1", rusqlite::params![rowid, mix.hash()])
                .unwrap();
            "rewrite an envelope's identity"
        }
        4 => {
            // As a trim drops a payload its checkpoint lists: the identity stays, as a tombstone.
            let rowid = some_envelope(connection, mix);
            connection
                .execute_batch(&format!(
                    /* sqlite-large-value: allow-for-now lv-hd7d92523e785 owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
                     SELECT writer, sequence, envelope_hash, 0, 'invented-checkpoint' FROM replica_envelopes WHERE rowid={rowid};
                     DELETE FROM replica_envelopes WHERE rowid={rowid};"
                ))
                .unwrap();
            "trim an envelope to a tombstone"
        }
        5 => {
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h8d0cb791bb5d owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_envelopes WHERE rowid=(SELECT rowid FROM checkpoint_envelopes ORDER BY rowid LIMIT 1)",
                    [],
                )
                .unwrap();
            "drop a checkpoint tombstone"
        }
        6 => {
            let rowid = some_envelope(connection, mix);
            connection
                .execute(
                    &format!(
                        /* sqlite-large-value: allow-for-now lv-hf1f2db5bb2d7 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO retired_envelopes(writer, sequence, envelope_hash)
                         SELECT writer, sequence, {} FROM replica_envelopes WHERE rowid=?1",
                        retired::identity_sql("envelope_hash")
                    ),
                    [rowid],
                )
                .unwrap();
            "retire an envelope"
        }
        7 => {
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h8af984feac27 owner=smallclaims::store::saved_inventory_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1, CASE WHEN EXISTS(SELECT 1 FROM meta WHERE key=?1 AND value='1') THEN '0' ELSE '1' END)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    [retired::AUTHORITY],
                )
                .unwrap();
            "switch the retired ledger's authority"
        }
        8 => {
            let rowid = some_envelope(connection, mix);
            connection.execute(/* sqlite-large-value: allow-for-now lv-hd3c9344b8d41 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET payload=x'00' WHERE rowid=?1", [rowid]).unwrap();
            "replace a payload, which the inventory does not list"
        }
        _ => {
            connection
                .execute(/* sqlite-large-value: allow-for-now lv-hafcb47196507 owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "UPDATE checkpoint_envelopes SET accepted_at_unix_ms=accepted_at_unix_ms+1", [])
                .unwrap();
            "touch the checkpoint tombstones"
        }
    }
}

/// Whatever lands between a save and the next open, the next open's inventory is the one a
/// read from nothing finds: the file is either still exact, or refused.
#[test]
fn a_random_mix_of_changes_between_a_save_and_the_next_open_never_loads_a_wrong_inventory() {
    let (mut loaded, mut refused) = (0, 0);
    for seed in 1..=48u64 {
        let mut mix = Mix(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cedar.sqlite3");
        {
            let store = open(&path);
            write_notes(&store, "history", 20 + mix.below(20) as usize);
            store.replication_snapshot().unwrap();
            assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved, "seed {seed}");
        }
        let mut done = Vec::new();
        for _ in 0..mix.below(4) {
            done.push(change(&other_binary(&path), &mut mix));
        }
        if mix.below(5) == 0 {
            // A start that reads the file and then stops without saving, as a crash does.
            let store = open(&path);
            write_notes(&store, "before-the-crash", 3);
            store.replication_snapshot().unwrap();
            done.push("a start that read the file, then a crash");
        }
        for _ in 0..mix.below(2) {
            done.push(change(&other_binary(&path), &mut mix));
        }
        let store = open(&path);
        write_notes(&store, "after", mix.below(3) as usize);
        FULL_INVENTORY_LOADS.with(|count| count.set(0));
        let snapshot = store.replication_snapshot().unwrap();
        let loads = full_loads();
        let (full, max_rowid) = full_compact_replication_inventory(&store.readers.get()).unwrap();
        assert_eq!(snapshot.inventory.digest, full.digest, "seed {seed}: {done:?}");
        assert_eq!(snapshot.buckets, full.buckets(), "seed {seed}: {done:?}");
        assert_eq!(snapshot.inventory.payloadless, full.payloadless, "seed {seed}: {done:?}");
        assert_eq!(snapshot.max_envelope_rowid, max_rowid, "seed {seed}: {done:?}");
        if loads == 0 {
            loaded += 1;
        } else {
            refused += 1;
        }
    }
    // Both outcomes ran, so the check above compared extended inventories as well as reads.
    assert!(loaded >= 5 && refused >= 5, "loaded {loaded}, refused {refused}");
}

/// The triggers are what refuse the file after a change the other checks cannot see (the last
/// rowid and its envelope stay as they were): without the ones that count it, the next open
/// extends an inventory that no longer describes the store. The inventory-generation triggers
/// count some of these changes too, so each case removes every trigger on its table and event;
/// a trim's delete, of an envelope its checkpoint lists, only this file's trigger counts.
#[test]
fn removing_the_triggers_that_count_a_change_lets_a_stale_inventory_load() {
    type Change = fn(&rusqlite::Connection);
    let cases: [(&str, &str, Change); 10] = [
        ("replica_envelopes", "DELETE", |c| {
            c.execute(/* sqlite-large-value: allow-for-now lv-ha3d4449c40cb owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=(SELECT MIN(rowid)+1 FROM replica_envelopes)", []).unwrap();
        }),
        // A trim: the checkpoint listed this envelope before the save.
        ("replica_envelopes", "DELETE", |c| {
            c.execute(/* sqlite-large-value: allow-for-now lv-h096b3ac921e0 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes)", []).unwrap();
        }),
        ("replica_envelopes", "INSERT", |c| insert_envelope(c, &mut Mix(7), true)),
        ("replica_envelopes", "UPDATE", |c| {
            c.execute(
                /* sqlite-large-value: allow-for-now lv-hc55e64044a0a owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET envelope_hash=?1 WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes)",
                [Mix(11).hash()],
            )
            .unwrap();
        }),
        ("checkpoint_envelopes", "INSERT", |c| {
            c.execute(
                /* sqlite-large-value: allow-for-now lv-hbdb15363782d owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
                 SELECT writer, sequence+1000000, ?1, 0, 'invented-checkpoint' FROM replica_envelopes LIMIT 1",
                [Mix(13).hash()],
            )
            .unwrap();
        }),
        ("checkpoint_envelopes", "DELETE", |c| {
            c.execute(/* sqlite-large-value: allow-for-now lv-hc29e4e23e9a9 owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_envelopes WHERE sequence>=2000000", []).unwrap();
        }),
        ("checkpoint_envelopes", "UPDATE", |c| {
            c.execute(/* sqlite-large-value: allow-for-now lv-h89145e643a90 owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "UPDATE checkpoint_envelopes SET envelope_hash=?1 WHERE sequence>=2000000", [Mix(17).hash()]).unwrap();
        }),
        ("retired_envelopes", "INSERT", |c| {
            c.execute(
                &format!(
                    /* sqlite-large-value: allow-for-now lv-h407916e8f5b5 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO retired_envelopes(writer, sequence, envelope_hash)
                     SELECT writer, sequence, {} FROM replica_envelopes WHERE rowid=(SELECT MIN(rowid)+2 FROM replica_envelopes)",
                    retired::identity_sql("envelope_hash")
                ),
                [],
            )
            .unwrap();
        }),
        ("retired_envelopes", "DELETE", |c| {
            c.execute(/* sqlite-large-value: allow-for-now lv-h8db85480d0de owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM retired_envelopes", []).unwrap();
        }),
        ("retired_envelopes", "UPDATE", |c| {
            c.execute(/* sqlite-large-value: allow-for-now lv-h9bc0d44f925c owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE retired_envelopes SET sequence=sequence+1000000", []).unwrap();
        }),
    ];
    for (index, (table, event, make_change)) in cases.into_iter().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cedar.sqlite3");
        {
            let store = open(&path);
            write_notes(&store, "history", 20);
            store.replication_snapshot().unwrap();
            // Two peer envelopes below the tail, which no local seal recreates once deleted; an
            // authoritative ledger with a retired identity; a tombstone; and a checkpoint listing
            // for the lower peer envelope, which the trim case deletes.
            let connection = other_binary(&path);
            insert_envelope(&connection, &mut Mix(37), true);
            insert_envelope(&connection, &mut Mix(41), true);
            connection
                .execute_batch(&format!(
                    /* sqlite-large-value: allow-for-now lv-hc7bd248137ba owner=smallclaims::store::saved_inventory_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO meta(key,value) VALUES('{}','1') ON CONFLICT(key) DO UPDATE SET value='1';
                     INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
                     SELECT writer, sequence+2000000, '{}', 0, 'invented-checkpoint' FROM replica_envelopes LIMIT 1;
                     INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
                     SELECT writer, sequence, envelope_hash, 0, 'invented-checkpoint' FROM replica_envelopes
                     WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes);
                     INSERT INTO retired_envelopes(writer, sequence, envelope_hash)
                     SELECT writer, sequence+3000000, unhex('{}') FROM replica_envelopes LIMIT 1;",
                    retired::AUTHORITY,
                    Mix(19).hash(),
                    Mix(23).hash(),
                ))
                .unwrap();
            drop(connection);
            store.replication_snapshot().unwrap();
            assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved, "case {index}");
        }
        let connection = other_binary(&path);
        let triggers: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND tbl_name=?1 AND name LIKE '%inventory%' AND name LIKE ?2")
            .unwrap()
            .query_map(rusqlite::params![table, format!("%{event}%")], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(triggers.iter().any(|name| name.contains("saved_inventory")), "case {index}: {triggers:?}");
        for trigger in &triggers {
            connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-h6e2aa55bdef6 owner=smallclaims::store::saved_inventory_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER {trigger};")).unwrap();
        }
        make_change(&connection);
        drop(connection);
        let store = open(&path);
        FULL_INVENTORY_LOADS.with(|count| count.set(0));
        let snapshot = store.replication_snapshot().unwrap();
        let loads = full_loads();
        let (full, _) = full_compact_replication_inventory(&store.readers.get()).unwrap();
        assert_eq!(loads, 0, "case {index}, {table} {event}: the file was refused without {triggers:?}");
        assert_ne!(
            (&snapshot.inventory.digest, &snapshot.inventory.payloadless),
            (&full.digest, &full.payloadless),
            "case {index}, {table} {event}: the change left the inventory as it was"
        );
    }
}

/// A copy taken before a save, restored next to that save's file, can hold the same count, the
/// same last rowid and the same envelope there: a peer's envelope that reached both lands at
/// the same rowid. Only the rows below differ. The token refuses it.
#[test]
fn a_store_restored_from_before_the_save_refuses_its_file_though_count_rowid_and_anchor_match() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    let backup = directory.path().join("backup.sqlite3");
    let saved_state = directory.path().join("saved.sqlite3");
    let (saved_max, saved_token) = {
        let store = open(&path);
        write_notes(&store, "history", 30);
        store.replication_snapshot().unwrap();
        other_binary(&path).execute(/* sqlite-large-value: allow-for-now lv-h4f4bee3873b0 owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ "VACUUM INTO ?1", [backup.to_str().unwrap()]).unwrap();
        // Two envelopes after the backup: one the restored store replaces, one it shares.
        write_notes(&store, "after-the-backup", 1);
        store.replication_snapshot().unwrap();
        write_notes(&store, "after-the-backup-too", 1);
        let snapshot = store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
        let connection = other_binary(&path);
        connection.execute(/* sqlite-large-value: allow-for-now lv-hc8cf34d194ca owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ "VACUUM INTO ?1", [saved_state.to_str().unwrap()]).unwrap();
        let token: String = connection
            .query_row("SELECT value FROM meta WHERE key='replication_inventory_saved'", [], |row| row.get(0))
            .unwrap();
        (snapshot.max_envelope_rowid, token)
    };
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
    std::fs::rename(&backup, &path).unwrap();
    // On the restored store: an envelope of its own where the saved store had another, then the
    // saved store's last envelope at the same rowid.
    let connection = other_binary(&path);
    let mut mix = Mix(29);
    let mut own = 0;
    while connection.query_row("SELECT MAX(rowid)+1 FROM replica_envelopes", [], |row| row.get::<_, i64>(0)).unwrap() < saved_max {
        insert_envelope(&connection, &mut mix, false);
        own += 1;
    }
    assert!(own >= 1, "the restored store holds an envelope of its own below the shared one");
    connection.execute(/* sqlite-large-value: allow-for-now lv-hc31b549dff4f owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ "ATTACH ?1 AS saved", [saved_state.to_str().unwrap()]).unwrap();
    connection
        .execute(
            &format!(
                /* sqlite-large-value: allow-for-now lv-h547ee089c932 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes(rowid, {COLUMNS}) SELECT rowid, {COLUMNS} FROM saved.replica_envelopes WHERE rowid=?1"
            ),
            [saved_max],
        )
        .unwrap();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h08ed1d887f98 owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ "DETACH saved;").unwrap();
    let restored_max: i64 = connection.query_row("SELECT MAX(rowid) FROM replica_envelopes", [], |row| row.get(0)).unwrap();
    assert_eq!(restored_max, saved_max, "the restored store reaches the saved store's last rowid");
    drop(connection);
    let mut name = path.as_os_str().to_owned();
    name.push(".replication-inventory");
    let file = PathBuf::from(name);
    let kept = std::fs::read(&file).unwrap();
    for with_the_saved_token in [false, true] {
        std::fs::write(&file, &kept).unwrap();
        if with_the_saved_token {
            // Without the token's check, the same store loads the file and lists the wrong rows.
            other_binary(&path)
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h94ad7e165653 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO meta(key,value) VALUES('replication_inventory_saved',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    [&saved_token],
                )
                .unwrap();
        }
        let store = open(&path);
        FULL_INVENTORY_LOADS.with(|count| count.set(0));
        let snapshot = store.replication_snapshot().unwrap();
        let loads = full_loads();
        let (full, _) = full_compact_replication_inventory(&store.readers.get()).unwrap();
        if with_the_saved_token {
            assert_eq!(loads, 0, "count, rowid and anchor alone accept it");
            assert_ne!(snapshot.inventory.digest, full.digest, "and its inventory is wrong");
        } else {
            assert_eq!(loads, 1, "the restored store took the later file");
            assert_eq!(snapshot.inventory.digest, full.digest);
        }
    }
}

/// A copy whose rowids were renumbered (a VACUUM may close the gaps a delete left, and fires no
/// trigger) no longer has the saved last envelope at the saved last rowid, so it reads the store,
/// even once appends take it past that rowid again.
#[test]
fn a_store_whose_rowids_were_renumbered_reads_the_store() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    {
        let store = open(&path);
        write_notes(&store, "history", 30);
        store.replication_snapshot().unwrap();
        // A peer envelope between local ones, then deleted: a gap no local seal fills.
        insert_envelope(&other_binary(&path), &mut Mix(43), false);
        write_notes(&store, "after-the-peer", 5);
        store.replication_snapshot().unwrap();
        other_binary(&path).execute(/* sqlite-large-value: allow-for-now lv-h038b38395151 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE writer='invented-peer'", []).unwrap();
        store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".replication-inventory");
    let file = PathBuf::from(name);
    let kept = std::fs::read(&file).unwrap();
    for appended in [0, 3] {
        std::fs::write(&file, &kept).unwrap();
        let copy = directory.path().join(format!("renumbered-{appended}.sqlite3"));
        other_binary(&path).execute(/* sqlite-large-value: allow-for-now lv-h6c4fd8d7681d owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ "VACUUM INTO ?1", [copy.to_str().unwrap()]).unwrap();
        // Close the gap as a renumbering copy does: no trigger sees it.
        let connection = other_binary(&copy);
        let triggers: Vec<(String, String)> = connection
            .prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger' AND tbl_name='replica_envelopes'")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for (name, _) in &triggers {
            connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-h9e1cf8a7d6ec owner=smallclaims::store::saved_inventory_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER {name};")).unwrap();
        }
        connection
            .execute_batch(&format!(
                /* sqlite-large-value: allow-for-now lv-h28bd1a5d15fb owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE held AS SELECT {COLUMNS} FROM replica_envelopes ORDER BY rowid;
                 DELETE FROM replica_envelopes;
                 INSERT INTO replica_envelopes({COLUMNS}) SELECT {COLUMNS} FROM held;"
            ))
            .unwrap();
        for (_, sql) in &triggers {
            connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h94183eda703e owner=smallclaims::store::saved_inventory_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        }
        for _ in 0..appended {
            insert_envelope(&connection, &mut Mix(31 + appended), false);
        }
        drop(connection);
        std::fs::copy(&file, saved_inventory::path(&copy)).unwrap();
        let store = open(&copy);
        FULL_INVENTORY_LOADS.with(|count| count.set(0));
        let snapshot = store.replication_snapshot().unwrap();
        assert_eq!(full_loads(), 1, "{appended} appended: the renumbered store took the file");
        assert_matches_the_store(&store, &snapshot);
    }
}

/// Reading the file removes it, so a start that read it and then crashed leaves the next start
/// nothing to read again.
#[test]
fn a_crash_after_the_file_was_read_leaves_none_to_read_again() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cedar.sqlite3");
    {
        let store = open(&path);
        write_notes(&store, "history", 40);
        store.replication_snapshot().unwrap();
        assert_eq!(store.save_replication_inventory().unwrap(), saved_inventory::InventorySave::Saved);
    }
    {
        let store = open(&path);
        FULL_INVENTORY_LOADS.with(|count| count.set(0));
        store.replication_snapshot().unwrap();
        assert_eq!(full_loads(), 0);
        assert!(!saved_inventory::path(&path).exists(), "the file outlived its read");
        // A change the read file does not describe, then a stop without a save.
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-habb792328cd1 owner=smallclaims::store::saved_inventory_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=(SELECT MIN(rowid) FROM replica_envelopes)", []).unwrap();
    }
    let store = open(&path);
    FULL_INVENTORY_LOADS.with(|count| count.set(0));
    let snapshot = store.replication_snapshot().unwrap();
    assert_eq!(full_loads(), 1);
    assert_matches_the_store(&store, &snapshot);
}
