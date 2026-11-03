//! The local-observation trim deletes what it always did, without searching inside the writer's
//! hold (#4610).
use super::*;
use rusqlite::params;
use std::collections::BTreeSet;

type SearchHook = Box<dyn FnOnce(&Store)>;

thread_local! {
    /// Runs once on the trimming thread between a page's search and its deletes.
    pub(super) static AFTER_SEARCH: std::cell::RefCell<Option<SearchHook>> = const { std::cell::RefCell::new(None) };
}

/// The rows the trim deleted before #4610, by its own predicate (no chunk limit).
const DUE_BEFORE: &str = "SELECT id FROM local_observations AS old
    WHERE observed_at_unix_ms < ?1
      AND kind NOT IN ('agent.launcher-bound', 'agent.helper-launched')
      AND NOT EXISTS (SELECT 1 FROM local_runtime_start_evidence WHERE observation_id=old.id)
      AND NOT EXISTS (SELECT 1 FROM local_latest_slots WHERE pending_local_id=old.id)
      AND EXISTS (SELECT 1 FROM local_observations AS newer
                  WHERE newer.subject=old.subject AND newer.kind=old.kind AND newer.id>old.id)";

/// The trim's DELETE before #4610: the search inside the writer's statement.
const DELETE_BEFORE: &str = /* sqlite-large-value: allow-for-now lv-h1e1e50547a90 owner=fractal_store::store::observation_trim_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_observations WHERE id IN (
    SELECT id FROM local_observations AS old
    WHERE observed_at_unix_ms < ?1
      AND kind NOT IN ('agent.launcher-bound', 'agent.helper-launched')
      AND NOT EXISTS (SELECT 1 FROM local_runtime_start_evidence WHERE observation_id=old.id)
      AND NOT EXISTS (SELECT 1 FROM local_latest_slots WHERE pending_local_id=old.id)
      AND EXISTS (SELECT 1 FROM local_observations AS newer
                  WHERE newer.subject=old.subject AND newer.kind=old.kind AND newer.id>old.id)
    ORDER BY observed_at_unix_ms
    LIMIT ?2)";

const INSERT_OBSERVATIONS: &str = /* sqlite-large-value: allow-for-now lv-hb4e5bc0d1e1b owner=fractal_store::store::observation_trim_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_observations(after_store_index, subject, kind, body, observed_at_unix_ms)
    SELECT 0, value->>0, value->>1, '{\"fields\":{}}', value->>2 FROM json_each(?1)";

const INSERT_SLOTS: &str = /* sqlite-large-value: allow-for-now lv-had3c6ca01d84 owner=fractal_store::store::observation_trim_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_latest_slots(subject, kind, slot, published_at_unix_ms, published_fields, pending_local_id)
    SELECT value->>0, 'harness.usage', value->>1, 0, '{}', value->>2 FROM json_each(?1)";

fn insert(store: &Store, sql: &str, rows: Vec<serde_json::Value>) {
    store.connection.lock().unwrap().execute(/* sqlite-large-value: allow-for-now lv-hbfd7f533a9fd owner=fractal_store::store::observation_trim_tests TODO=trace-statement-and-value-representation */ sql, [serde_json::Value::from(rows).to_string()]).unwrap();
}

/// A store's local observations, shaped like a long-running member's: `kept` subjects whose only
/// row is past retention (the newest of its kind, so it stays), `due` older rows beside a newer
/// one, `recent` rows inside retention, and `slots` usage slots, one in four pending on a row
/// past retention. One old row is start evidence and one is a launcher binding; both stay.
fn seed(store: &Store, kept: usize, due: usize, recent: usize, slots: usize) {
    let mut rows = Vec::new();
    for index in 0..kept {
        rows.push(serde_json::json!([format!("message/kept-{index}"), "message.observed", 1_000 + index]));
    }
    for index in 0..due {
        rows.push(serde_json::json!([format!("agent/node.due-{}", index % 50), "harness.timeline", 2_000 + index]));
    }
    for index in 0..50 {
        rows.push(serde_json::json!([format!("agent/node.due-{index}"), "harness.timeline", 1_000_000]));
    }
    for index in 0..recent {
        rows.push(serde_json::json!([format!("agent/node.recent-{}", index % 2_000), "harness.usage", 1_000_000 + index]));
    }
    for (kind, old, new) in [("agent.launcher-bound", 3, 1_000_001), ("runtime.action.succeeded", 4, 1_000_002)] {
        rows.push(serde_json::json!(["agent/node.due-0", kind, old]));
        rows.push(serde_json::json!(["agent/node.due-0", kind, new]));
    }
    insert(store, INSERT_OBSERVATIONS, rows);
    store
        .connection
        .lock()
        .unwrap()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-hcfe1968d6967 owner=fractal_store::store::observation_trim_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_runtime_start_evidence(operation_id, observation_id)
             SELECT 'operation/start', min(id) FROM local_observations WHERE kind='runtime.action.succeeded'",
        )
        .unwrap();
    // Pending readings are recent ones, but for a few that wait on a row past retention.
    let old: Vec<i64> = ids(store, "SELECT id FROM local_observations WHERE kind='harness.timeline' AND observed_at_unix_ms < ?1 ORDER BY id", 1_000_000)
        .into_iter()
        .collect();
    let recent: Vec<i64> = ids(store, "SELECT id FROM local_observations WHERE kind='harness.usage' AND observed_at_unix_ms >= ?1 ORDER BY id", 1_000_000)
        .into_iter()
        .collect();
    let slots = (0..slots)
        .map(|index| {
            let pending = (index % 4 == 0).then(|| match index / 4 {
                pick if pick < 50 && !old.is_empty() => old[pick % old.len()],
                pick if !recent.is_empty() => recent[pick % recent.len()],
                _ => old[0],
            });
            serde_json::json!([format!("agent/node.slot-{index}"), serde_json::json!(["inc", format!("slot-{index}")]).to_string(), pending])
        })
        .collect();
    insert(store, INSERT_SLOTS, slots);
}

fn ids(store: &Store, sql: &str, cutoff: i64) -> BTreeSet<i64> {
    let connection = store.readers.get();
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h431811e49240 owner=fractal_store::store::observation_trim_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
    statement.query_map([cutoff], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

#[test]
fn the_trim_deletes_what_it_deleted_before_across_pages_and_chunks() {
    let store = Store::open_memory("node").unwrap();
    // More rows past retention than one reader page, so the search continues across pages.
    seed(&store, usize::try_from(TRIM_SCAN_PAGE).unwrap() + 1_500, 1_200, 300, 400);
    let cutoff = 500_000;
    let due = ids(&store, DUE_BEFORE, cutoff);
    assert!(due.len() > 1_000);
    let all = ids(&store, "SELECT id FROM local_observations WHERE ?1 IS NOT NULL", cutoff);
    let trim = store.trim_local_observations_pass(cutoff as u128, 20_000, 7).unwrap();
    assert_eq!(trim.deleted, due.len());
    let left = ids(&store, "SELECT id FROM local_observations WHERE ?1 IS NOT NULL", cutoff);
    assert_eq!(left, all.difference(&due).copied().collect::<BTreeSet<_>>(), "exactly the rows the old predicate chose");
    assert_eq!(store.trim_local_observations(cutoff as u128, 20_000, 7).unwrap(), 0, "a second pass finds nothing");
}

#[test]
fn a_row_that_becomes_pending_after_the_search_is_kept() {
    let store = Store::open_memory("node").unwrap();
    seed(&store, 10, 20, 0, 0);
    let cutoff = 500_000;
    let due: Vec<i64> = ids(&store, DUE_BEFORE, cutoff).into_iter().collect();
    // A usage reading becomes pending after the search found its row: the writer's re-check keeps it.
    let late = due[0];
    AFTER_SEARCH.with(|change| {
        *change.borrow_mut() = Some(Box::new(move |store: &Store| {
            insert(store, INSERT_SLOTS, vec![serde_json::json!(["agent/node.late", "[\"inc\",\"late\"]", late])]);
        }));
    });
    let deleted = store.trim_local_observations(cutoff as u128, 20_000, 100).unwrap();
    assert!(AFTER_SEARCH.with(|change| change.borrow().is_none()), "the search ran");
    assert_eq!(deleted, due.len() - 1);
    assert_eq!(ids(&store, "SELECT id FROM local_observations WHERE id=?1", late).len(), 1);
}

/// The longest writer hold of one trim pass on a store shaped like a 57 GB member's (#4610):
/// 450,000 local observations, 26,002 past retention of which 204 were due. `ST_TRIM_SLOTS` sets
/// the usage slots (the table without an index on `pending_local_id`); `ST_TRIM_DIR` keeps the store.
#[test]
#[ignore = "large-store trim measurement (#4610): run alone"]
fn the_trim_holds_the_writer_briefly_on_a_large_store() {
    let slots = std::env::var("ST_TRIM_SLOTS").ok().and_then(|value| value.parse().ok()).unwrap_or(5_000);
    let temporary = tempfile::tempdir().unwrap();
    let root = std::env::var_os("ST_TRIM_DIR").map_or_else(|| temporary.path().to_owned(), std::path::PathBuf::from);
    let path = root.join("trim.sqlite3");
    let _ = std::fs::remove_file(&path);
    {
        let store = Store::open(&path, "node").unwrap();
        seed(&store, 26_002 - 204, 204, 450_000 - 26_002, slots);
    }
    let cutoff = 500_000;
    // Before: the old DELETE with its search inside the statement, rolled back, on a cold store.
    let before = {
        let store = Store::open(&path, "node").unwrap();
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let started = std::time::Instant::now();
        let deleted = transaction.execute(/* sqlite-large-value: allow-for-now lv-hef3380e725e2 owner=fractal_store::store::observation_trim_tests TODO=trace-statement-and-value-representation */ DELETE_BEFORE, params![cutoff, 5_000]).unwrap();
        let held = started.elapsed();
        transaction.rollback().unwrap();
        (deleted, held)
    };
    let store = Store::open(&path, "node").unwrap();
    let kinds = |store: &Store| -> Vec<(String, i64, i64)> {
        let connection = store.readers.get();
        let mut statement = connection
            .prepare("SELECT kind, count(*), sum(observed_at_unix_ms < 500000) FROM local_observations GROUP BY kind ORDER BY kind")
            .unwrap();
        statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    };
    let seeded = kinds(&store);
    let started = std::time::Instant::now();
    let after = store.trim_local_observations_pass(cutoff as u128, 20_000, 500).unwrap();
    println!("by kind (kind, rows, past retention): seeded {seeded:?} left {:?}", kinds(&store));
    println!(
        "{}",
        serde_json::json!({
            "observations": 450_000, "past_retention": 26_002, "slots": slots,
            "before": {"deleted": before.0, "writer_hold_ms": before.1.as_millis()},
            "after": {"deleted": after.deleted, "longest_writer_hold_ms": after.longest_writer_hold.as_millis(),
                      "pass_ms": started.elapsed().as_millis()},
        })
    );
    assert_eq!(after.deleted, before.0);
}
