//! Bounded actual-state reductions. Arrival order is not canonical order: a late older
//! claim restarts the fold, while an ordered suffix updates only the reduced state.
use super::*;

type SourceRow = (String, String, String, Value);
pub(super) type Key = (usize, String, String, Option<i64>, String, i64, String);

pub(super) fn key(connection: &Connection, id: &str) -> Result<Key> {
    Ok(connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h9f9764d46e29 owner=fractal_store::store::card_actual TODO=trace-statement-and-value-representation */ &format!(
        "SELECT length(claims.accepted_at_unix_ms), claims.accepted_at_unix_ms,
         batches.origin, batches.replica_sequence, claims.batch_id, {}, claims.id
         FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.id=?1",
        canonical::position_sql("claims")
    ))?.query_row(/* sqlite-large-value: allow-for-now lv-h41ad0419d8c7 owner=fractal_store::store::card_actual TODO=trace-statement-and-value-representation */ [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?,
        row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)))?)
}

#[derive(Clone, Default)]
pub(super) struct Reduction {
    pub through: u64,
    pub revision: u64,
    pub last: Option<Key>,
    latest: Option<Value>,
    // Latest fallback and latest runtime/nonterminal runtime per origin are sufficient
    // for source selection, including a declaration moving to an earlier observing host.
    sources: Vec<SourceRow>,
}

impl Reduction {
    pub(super) fn activity_fields(&self) -> Option<Value> {
        self.latest.as_ref().map(|actual| Value::Object(
            ["status", "workspace", "host"].into_iter().filter_map(|field|
                actual.get(field).map(|value| (field.into(), value.clone()))).collect()
        ))
    }
    pub fn apply(&mut self, rows: Vec<(String, String, String, String, bool)>) -> Result<()> {
        for (id, kind, origin, body, folded) in rows {
            #[cfg(any(test, feature = "test-support"))]
            if folded || kind == "runtime.observed" {
                ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(count.get() + 1));
            }
            let value = if folded { serde_json::from_str::<Value>(&body)? }
                else if kind == "runtime.observed" { serde_json::from_str(&body).unwrap_or(Value::Null) }
                else { Value::Null };
            let source_value = if kind == "runtime.observed" { value.clone() } else { Value::Null };
            if folded {
                let merged = self.latest.get_or_insert_with(|| json!({})).as_object_mut().expect("object reduction");
                merge_actual_value(merged, &kind, value);
            }
            let value = source_value;
            if kind == "runtime.observed" {
                let terminal = terminal_observation(&value);
                self.sources.retain(|(_, old_kind, old_origin, old_body)| {
                    old_kind == "runtime.observed" && (old_origin != &origin
                        || (terminal && !terminal_observation(old_body)))
                });
            } else {
                self.sources.retain(|(_, old_kind, _, _)| old_kind == "runtime.observed");
            }
            self.sources.push((id, kind, origin, value));
        }
        Ok(())
    }

    pub fn state(&self, host: Option<&str>) -> card_fold::ActualState {
        let rows = self.sources.iter().map(|(id, kind, origin, body)|
            (id.as_str(), kind.as_str(), origin.as_str(), body)).collect::<Vec<_>>();
        card_fold::ActualState { latest: self.latest.clone(), source: select_actual_source(&rows, host) }
    }
}

/// Revisions change only when the already-folded prefix can change. Ordinary claim
/// appends use the indexed arrival suffix; repairs, trims and ordering changes restart it.
/// These are disposable derived fences; claims remain the only reduction authority.
pub(super) fn install(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hd43112908de0 owner=fractal_store::store::card_actual TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS card_actual_revisions (
        subject TEXT PRIMARY KEY, revision INTEGER NOT NULL
    );")?;
    let bump = |subject: &str| format!(/* sqlite-large-value: allow-for-now lv-h5b8679124200 owner=fractal_store::store::card_actual TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO card_actual_revisions(subject,revision)
        SELECT {subject},1 WHERE {subject} IS NOT NULL
        ON CONFLICT(subject) DO UPDATE SET revision=revision+1;");
    for (name, event, subject) in [
        ("claim_update", "AFTER UPDATE ON claims", "NEW.subject"),
        ("claim_update_old", "AFTER UPDATE OF subject ON claims", "OLD.subject"),
        ("claim_delete", "AFTER DELETE ON claims", "OLD.subject"),
        ("repair_insert", "AFTER INSERT ON projection_digest_repaired_claims", "(SELECT subject FROM claims WHERE id=NEW.id)"),
        ("repair_delete", "AFTER DELETE ON projection_digest_repaired_claims", "(SELECT subject FROM claims WHERE id=OLD.id)"),
        ("repair_update_old", "AFTER UPDATE ON projection_digest_repaired_claims", "(SELECT subject FROM claims WHERE id=OLD.id)"),
        ("repair_update_new", "AFTER UPDATE ON projection_digest_repaired_claims", "(SELECT subject FROM claims WHERE id=NEW.id)"),
        ("position_insert", "AFTER INSERT ON claim_positions", "(SELECT subject FROM claims WHERE id=NEW.claim_id)"),
        ("position_update", "AFTER UPDATE ON claim_positions WHEN OLD.position IS NOT NEW.position OR OLD.claim_id IS NOT NEW.claim_id", "(SELECT subject FROM claims WHERE id=OLD.claim_id)"),
        ("position_update_new", "AFTER UPDATE OF claim_id ON claim_positions WHEN OLD.claim_id IS NOT NEW.claim_id", "(SELECT subject FROM claims WHERE id=NEW.claim_id)"),
        ("position_delete", "AFTER DELETE ON claim_positions", "(SELECT subject FROM claims WHERE id=OLD.claim_id)"),
    ] {
        connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-h2d32e6b8e798 owner=fractal_store::store::card_actual TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER IF NOT EXISTS card_actual_{name} {event}
            BEGIN {} END;", bump(subject)))?;
    }
    // Position admission can lower an existing claim's canonical position without
    // inserting another replica row. Observe the authoritative position table itself.
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-ha2d22ff13e93 owner=fractal_store::store::card_actual TODO=chunk-or-reference-exact-wire-values */ "DROP TRIGGER IF EXISTS card_actual_record_insert;
        DROP TRIGGER IF EXISTS card_actual_record_update;
        DROP TRIGGER IF EXISTS card_actual_record_update_new;
        DROP TRIGGER IF EXISTS card_actual_record_delete;
        CREATE TRIGGER IF NOT EXISTS card_actual_batch_update
        AFTER UPDATE OF origin, replica_sequence ON batches
        WHEN OLD.origin IS NOT NEW.origin OR OLD.replica_sequence IS NOT NEW.replica_sequence BEGIN
        INSERT INTO card_actual_revisions(subject,revision)
        SELECT DISTINCT subject,1 FROM claims WHERE batch_id=OLD.id
        ON CONFLICT(subject) DO UPDATE SET revision=revision+1; END;")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn incremental_matches_full_fold(
            events in prop::collection::vec((0u8..3, any::<bool>(), any::<bool>(), any::<u16>()), 0..80),
            split in 0usize..80,
        ) {
            let mut reduction = Reduction::default();
            let mut history = Vec::new();
            for (n, (origin, terminal, runtime, marker)) in events.into_iter().enumerate() {
                let kind = if runtime { "runtime.observed" } else { "example.observed" };
                let body = json!({"fields":{"status":if terminal {"stopped"} else {"running"}, "marker":marker}});
                history.push((n.to_string(), kind.to_owned(), origin.to_string(), body));
            }
            // Every partition of a generated canonical history gives the same reduction.
            let split = split.min(history.len());
            for part in [&history[..split], &history[split..]] {
                reduction.apply(part.iter().map(|(id, kind, origin, body)|
                    (id.clone(), kind.clone(), origin.clone(), body.to_string(), true)).collect()).unwrap();
            }
            let full = fold_latest_values(history.iter().map(|(_, kind, _, body)| (kind.clone(), body.clone()))).unwrap();
            let rows = history.iter().map(|(id, kind, origin, body)|
                (id.as_str(), kind.as_str(), origin.as_str(), body)).collect::<Vec<_>>();
            for host in [None, Some("0"), Some("1"), Some("2"), Some("unseen")] {
                let actual = reduction.state(host);
                prop_assert_eq!(&actual.latest, &full);
                prop_assert_eq!(actual.source, select_actual_source(&rows, host));
            }
            prop_assert!(reduction.sources.len() <= 7);
        }
    }

    fn append(store: &Store, time: u128, status: &str) -> ClaimRecord {
        append_from(store, "alder", time, status)
    }

    fn append_from(store: &Store, origin: &str, time: u128, status: &str) -> ClaimRecord {
        store.set_write_clock_at(time).unwrap();
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let claim = append_claim_tx(&tx, origin, "agent/example", "runtime.observed",
            None, &json!({"fields":{"status":status}}), &[], None).unwrap();
        tx.commit().unwrap();
        claim
    }

    fn read(store: &Store, index: u64) -> usize {
        let connection = store.readers.get();
        let mut reads = card_fold::CardReads::new(index);
        ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(0));
        reads.load_actual(store, &connection, &["agent/example".into()]).unwrap();
        let decoded = ACTUAL_HISTORY_BODIES_DECODED.with(std::cell::Cell::get);
        let cached = store.fractal.card_actuals.lock().unwrap().get(&("agent/example".into(), false)).unwrap().clone();
        assert_eq!(cached.state(None).latest, latest_actual_at(&connection, "agent/example", Some(index)).unwrap());
        let expected = selected_actual_source_at(&connection, "agent/example", Some(index), None).unwrap();
        assert_eq!(cached.state(None).source.map(|(id, origin, _)| (Some(id), Some(origin)))
            .unwrap_or((None, None)), (expected.0, expected.1));
        decoded
    }

    #[test]
    fn indexed_suffix_reuses_history_and_restarts_on_prefix_changes() {
        let store = Store::open_memory("alder").unwrap();
        let first = append(&store, 100, "running");
        assert_eq!(read(&store, first.store_index), 1);
        assert_eq!(read(&store, first.store_index), 0);
        let second = append(&store, 200, "stopped");
        assert_eq!(read(&store, second.store_index), 1);
        // write_time floors a local writer at its newest batch, so moving alder's
        // simulation clock backwards cannot produce a late canonical arrival.
        // A previously unseen remote writer has no such local floor.
        let late = append_from(&store, "birch", 150, "running");
        assert_eq!(late.accepted_at_unix_ms, 150);
        assert!(late.accepted_at_unix_ms < second.accepted_at_unix_ms);
        {
            let connection = store.readers.get();
            assert!(key(&connection, &late.id).unwrap() < key(&connection, &second.id).unwrap());
        }
        assert_eq!(read(&store, late.store_index), 3);
        // Historical readers cannot resume from a future prefix.
        assert_eq!(read(&store, first.store_index), 1);
        read(&store, late.store_index);
        {
            let writer = store.connection.write();
            writer.execute("UPDATE claims SET accepted_at_unix_ms='300' WHERE id=?1", [&first.id]).unwrap();
        }
        assert_eq!(read(&store, late.store_index), 3);
        // Transaction rollback also rolls back the durable prefix revision.
        {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            tx.execute("DELETE FROM claims WHERE id=?1", [&second.id]).unwrap();
            tx.rollback().unwrap();
        }
        assert_eq!(read(&store, late.store_index), 0);
        {
            let writer = store.connection.write();
            writer.execute("DELETE FROM claims WHERE id=?1", [&second.id]).unwrap();
        }
        assert_eq!(read(&store, late.store_index), 2);
        {
            let writer = store.connection.write();
            writer.execute("INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)", [&first.id]).unwrap();
        }
        assert_eq!(read(&store, late.store_index), 1);
        assert_eq!(read(&store, late.store_index), 0);
        {
            let writer = store.connection.write();
            writer.execute("DELETE FROM projection_digest_repaired_claims WHERE id=?1", [&first.id]).unwrap();
        }
        assert_eq!(read(&store, late.store_index), 2);
    }

    #[test]
    fn upgrading_a_store_without_graph_repair_membership_installs_dependent_fences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("upgrade.db");
        let claim = {
            let store = Store::open(&path, "alder").unwrap();
            append(&store, 100, "running")
        };
        {
            // Model the pre-digest graph layout, before either the graph-owned
            // repair membership or these runtime-local reduction fences existed.
            let connection = Connection::open(&path).unwrap();
            let triggers = connection.prepare(
                "SELECT name FROM sqlite_schema WHERE type='trigger'
                 AND (name GLOB 'card_actual_*' OR name GLOB 'projection_digest_*')"
            ).unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap();
            for name in triggers {
                connection.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\""))).unwrap();
            }
            connection.execute_batch(
                "DROP TABLE card_actual_revisions;
                 DROP TABLE projection_digest_repaired_claims;
                 DELETE FROM meta WHERE key='projection_digest_registry';"
            ).unwrap();
        }
        // Exercise both the old-store upgrade and the ordinary reopen path.
        for _ in 0..2 {
            let store = Store::open(&path, "alder").unwrap();
            let connection = store.readers.get();
            let fences: usize = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger'
                 AND name GLOB 'card_actual_repair_*'", [], |row| row.get(0)
            ).unwrap();
            assert_eq!(fences, 4);
            assert_eq!(latest_actual_at(&connection, "agent/example", Some(claim.store_index)).unwrap()
                .unwrap()["status"], "running");
            drop(connection);
            assert_eq!(read(&store, claim.store_index), 1);
        }
    }
}
