use super::*;
use std::cell::RefCell;
use std::sync::Barrier;

fn generation_state(connection: &Connection) -> (i64, i64) {
    connection.query_row(
        "SELECT value,trigger_version FROM fleet_generation WHERE id=1", [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap()
}

fn generation_triggers(connection: &Connection) -> Vec<(String, String)> {
    let mut statement = connection.prepare(
        "SELECT name,sql FROM sqlite_schema WHERE type='trigger'
         AND name LIKE 'fleet_generation_%' ORDER BY name",
    ).unwrap();
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

fn install_old_narrow_triggers(connection: &Connection) {
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-h94819a5d6420 owner=smallclaims::store::sealed_view_cache_tests::migration_tests TODO=chunk-or-reference-exact-wire-values */ "BEGIN IMMEDIATE;
         UPDATE fleet_generation SET trigger_version=0 WHERE id=1;
         DROP TRIGGER fleet_generation_claim_update;
         DROP TRIGGER fleet_generation_envelope_update;
         CREATE TRIGGER fleet_generation_claim_update
         AFTER UPDATE OF kind,batch_id,body ON claims
         WHEN NEW.kind IN ({FLEET_CLAIM_KINDS}) OR OLD.kind IN ({FLEET_CLAIM_KINDS})
         BEGIN UPDATE fleet_generation SET value=value+1 WHERE id=1; END;
         CREATE TRIGGER fleet_generation_envelope_update
         AFTER UPDATE OF batch_id ON replica_envelopes
         WHEN NEW.batch_id IS NOT NULL AND EXISTS(
             SELECT 1 FROM claims WHERE batch_id=NEW.batch_id
             AND kind IN ({FLEET_CLAIM_KINDS}))
         BEGIN UPDATE fleet_generation SET value=value+1 WHERE id=1; END;
         COMMIT;"
    )).unwrap();
}

#[test]
fn fleet_generation_current_version_reinitialization_executes_no_ddl() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
    configure(&store, 1);
    let connection = Connection::open(&path).unwrap();
    let schema_before: i64 = connection.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
    let state_before = generation_state(&connection);
    let triggers_before = generation_triggers(&connection);
    let cached = store.fleet_view_sealed().unwrap();
    assert_eq!(state_before.1, FLEET_GENERATION_TRIGGER_VERSION);

    initialize_fleet_generation(&connection).unwrap();
    initialize_fleet_generation(&connection).unwrap();

    let schema_after: i64 = connection.query_row("PRAGMA schema_version", [], |row| row.get(0)).unwrap();
    assert_eq!(schema_after, schema_before, "current-version initialization changed the schema");
    assert_eq!(generation_state(&connection), state_before);
    assert_eq!(generation_triggers(&connection), triggers_before);
    assert_eq!(cached, store.fleet_view_sealed().unwrap());
}

#[test]
fn fleet_generation_version_zero_upgrade_fences_live_cached_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
    configure(&store, 1);
    let connection = Connection::open(&path).unwrap();
    let current_triggers = generation_triggers(&connection);
    install_old_narrow_triggers(&connection);
    let old_triggers = generation_triggers(&connection);
    assert_ne!(old_triggers, current_triggers);
    let cached = store.fleet_view_sealed().unwrap();
    assert_eq!(cached.members.len(), 1);
    let before = generation_state(&connection);
    assert_eq!(before.1, 0);

    // The old narrow trigger misses this authority change. Keep the production
    // source-identity guards intact: subject is mutable, unlike the claim ID.
    assert_eq!(connection.execute(
        /* sqlite-large-value: allow-for-now lv-h850482038bc7 owner=smallclaims::store::sealed_view_cache_tests::migration_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET subject='host/different-anchor' WHERE kind='fleet.member-admitted'", [],
    ).unwrap(), 1);
    assert_eq!(generation_state(&connection), before);
    assert_eq!(cached, store.fleet_view_sealed().unwrap());
    assert!(oracle(&store).members.is_empty());

    initialize_fleet_generation(&connection).unwrap();

    assert_eq!(generation_state(&connection), (before.0 + 1, FLEET_GENERATION_TRIGGER_VERSION));
    assert_eq!(generation_triggers(&connection), current_triggers);
    assert!(refolded(&store).members.is_empty());
}

#[test]
fn fleet_generation_upgrade_adds_version_to_legacy_counter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hbe73cc1e534e owner=smallclaims::store::sealed_view_cache_tests::migration_tests TODO=trace-statement-and-value-representation */ SCHEMA).unwrap();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hafd5d8d79ffc owner=smallclaims::store::sealed_view_cache_tests::migration_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE fleet_generation(id INTEGER PRIMARY KEY CHECK(id=1), value INTEGER NOT NULL);
         INSERT INTO fleet_generation(id,value) VALUES(1,41);",
    ).unwrap();

    initialize_fleet_generation(&connection).unwrap();

    assert_eq!(generation_state(&connection), (42, FLEET_GENERATION_TRIGGER_VERSION));
    assert!(!generation_triggers(&connection).is_empty());
    initialize_fleet_generation(&connection).unwrap();
    assert_eq!(generation_state(&connection), (42, FLEET_GENERATION_TRIGGER_VERSION));
}

struct MigrationPause {
    dropped_claim_trigger: bool,
    reached: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

thread_local! {
    static MIGRATION_PAUSE: RefCell<Option<MigrationPause>> = const { RefCell::new(None) };
}

fn pause_after_trigger_drop(sql: &str) {
    MIGRATION_PAUSE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(pause) = slot.as_mut() else { return; };
        let sql = sql.trim_start();
        if sql.starts_with(/* sqlite-large-value: allow-for-now lv-hb9727851cea8 owner=smallclaims::store::sealed_view_cache_tests::migration_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS fleet_generation_claim_update") {
            pause.dropped_claim_trigger = true;
        }
        if pause.dropped_claim_trigger && sql.starts_with(/* sqlite-large-value: allow-for-now lv-h6b8c6d05ed30 owner=smallclaims::store::sealed_view_cache_tests::migration_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER fleet_generation_claim_update") {
            let pause = slot.take().unwrap();
            // Do not panic inside SQLite's callback. A closed channel or timeout
            // releases the worker; the test observes a missing pause as a failure.
            if pause.reached.send(()).is_ok() {
                let _ = pause.release.recv_timeout(Duration::from_secs(10));
            }
        }
    });
}

#[test]
fn fleet_generation_trigger_replacement_is_atomic_for_other_connections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
    configure(&store, 1);
    let observer = Connection::open(&path).unwrap();
    let current_triggers = generation_triggers(&observer);
    install_old_narrow_triggers(&observer);
    let old_triggers = generation_triggers(&observer);
    let before = generation_state(&observer);
    let competing_writer = Connection::open(&path).unwrap();
    competing_writer.busy_timeout(Duration::ZERO).unwrap();
    let (reached_send, reached_receive) = mpsc::channel();
    let (release_send, release_receive) = mpsc::channel();
    let migration_path = path.clone();
    let worker = std::thread::spawn(move || {
        let mut connection = Connection::open(migration_path).unwrap();
        MIGRATION_PAUSE.with(|slot| {
            *slot.borrow_mut() = Some(MigrationPause {
                dropped_claim_trigger: false,
                reached: reached_send,
                release: release_receive,
            });
        });
        connection.trace(Some(pause_after_trigger_drop));
        let result = initialize_fleet_generation(&connection);
        connection.trace(None);
        MIGRATION_PAUSE.with(|slot| { slot.borrow_mut().take(); });
        result
    });

    let paused = reached_receive.recv_timeout(Duration::from_secs(5));
    // Collect observations without asserting while the migration is paused. Even
    // a regression must release/join the worker before failing the test.
    let write_result = competing_writer.execute(/* sqlite-large-value: allow-for-now lv-h396dbc78afde owner=smallclaims::store::sealed_view_cache_tests::migration_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE fleet_generation SET value=value+1 WHERE id=1", []);
    let visible_state = observer.query_row(
        "SELECT value,trigger_version FROM fleet_generation WHERE id=1", [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    );
    let visible_triggers = (|| -> rusqlite::Result<Vec<(String, String)>> {
        let mut statement = observer.prepare(
            "SELECT name,sql FROM sqlite_schema WHERE type='trigger'
             AND name LIKE 'fleet_generation_%' ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    })();
    let _ = release_send.send(());
    let migration_result = worker.join().unwrap();

    paused.expect("migration never paused between DROP and CREATE");
    let write_error = write_result.expect_err("writer crossed the trigger replacement transaction");
    assert!(matches!(write_error, rusqlite::Error::SqliteFailure(error, _)
        if error.code == rusqlite::ErrorCode::DatabaseBusy));
    assert_eq!(visible_state.unwrap(), before);
    assert_eq!(visible_triggers.unwrap(), old_triggers, "reader saw a partial trigger installation");
    migration_result.unwrap();
    assert_eq!(generation_state(&observer), (before.0 + 1, FLEET_GENERATION_TRIGGER_VERSION));
    assert_eq!(generation_triggers(&observer), current_triggers);
}

#[test]
fn fleet_generation_concurrent_initializers_install_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
    configure(&store, 1);
    let observer = Connection::open(&path).unwrap();
    let current_triggers = generation_triggers(&observer);
    install_old_narrow_triggers(&observer);
    let before = generation_state(&observer);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let connection = Connection::open(path).unwrap();
            connection.busy_timeout(Duration::from_secs(5)).unwrap();
            barrier.wait();
            initialize_fleet_generation(&connection)
        }));
    }
    barrier.wait();
    // Join both workers before asserting either result.
    let results: Vec<_> = workers.into_iter().map(|worker| worker.join()).collect();
    for result in results {
        result.unwrap().unwrap();
    }
    assert_eq!(generation_state(&observer), (before.0 + 1, FLEET_GENERATION_TRIGGER_VERSION));
    assert_eq!(generation_triggers(&observer), current_triggers);
}
