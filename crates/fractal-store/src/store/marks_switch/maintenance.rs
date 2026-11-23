//! Changed-member maintenance. The durable key set is written by source triggers; native
//! committed-table notifications wake its consumer. No clock or broad API event is involved.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Default)]
struct Work {
    roster_refreshes: AtomicU64,
    member_reevaluations: AtomicU64,
    switch_changes: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Requirement {
    Member { start: u64, ambiguous: bool },
    Legacy,
}

/// One daemon-owned consumer. Registration precedes the initial snapshot; dirty keys are
/// deleted in the same transaction that publishes the switches, so concurrent writes survive.
pub struct Maintenance {
    _observer: smallclaims::sqlite::CommitObserver,
    changes: tokio::sync::watch::Receiver<u64>,
    requirements: BTreeMap<String, Requirement>,
    ready: BTreeMap<String, (bool, bool)>,
    peers: BTreeSet<String>,
    fleet_generation: Option<i64>,
}

impl Store {
    pub fn marks_switch_maintenance(&self) -> Result<Maintenance> {
        let (wake, changes) = tokio::sync::watch::channel(0u64);
        let observed_fleet = std::sync::atomic::AtomicI64::new(-1);
        let observer = self.observe_tables(&["local_marks_switch_dirty_v1", "fleet_generation"], move |connection| {
            let changed = (|| -> rusqlite::Result<bool> {
                let generation = connection.query_row("SELECT value FROM fleet_generation WHERE id=1", [], |row| row.get::<_, i64>(0))?;
                let fleet_changed = observed_fleet.swap(generation, Relaxed) != generation;
                let pending = connection.query_row("SELECT EXISTS(SELECT 1 FROM local_marks_switch_dirty_v1)", [], |row| row.get::<_, bool>(0))?;
                Ok(fleet_changed || pending)
            })();
            if changed.unwrap_or(true) {
                wake.send_modify(|generation| *generation = generation.wrapping_add(1));
            }
        });
        self.connection.batched(|tx| install(tx)).map_err(anyhow::Error::msg)??;
        Ok(Maintenance { _observer: observer, changes, requirements: BTreeMap::new(),
            ready: BTreeMap::new(), peers: BTreeSet::new(), fleet_generation: None })
    }

    pub fn marks_switch_work(&self) -> Value {
        let work = self.extension::<Work>();
        json!({"roster_refreshes": work.roster_refreshes.load(Relaxed),
            "member_reevaluations": work.member_reevaluations.load(Relaxed),
            "switch_changes": work.switch_changes.load(Relaxed)})
    }
}

impl Maintenance {
    pub async fn changed(&mut self) -> bool { self.changes.changed().await.is_ok() }

    /// Also accepts a changed peer configuration without requiring a database mutation.
    pub fn refresh(&mut self, store: &Store, peers: &[String]) -> Result<()> {
        self.changes.borrow_and_update();
        let peers: BTreeSet<_> = peers.iter().filter(|peer| **peer != store.origin).cloned().collect();
        let result = store.connection.batched(|tx| -> Result<_> {
            let generation: i64 = tx.query_row("SELECT value FROM fleet_generation WHERE id=1", [], |row| row.get(0))?;
            let mut query = tx.prepare_cached("SELECT member FROM local_marks_switch_dirty_v1")?;
            let dirty: BTreeSet<String> = query.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
            drop(query);
            let roster = self.fleet_generation != Some(generation) || peers != self.peers || dirty.contains("");
            let next = if roster { Some(requirements(tx, &store.origin, &peers)?) } else { None };
            let requirements = next.as_ref().unwrap_or(&self.requirements);
            let mut updates = BTreeMap::new();
            for (member, requirement) in requirements {
                if self.requirements.get(member) != Some(requirement) || dirty.contains(member) {
                    let (start, ambiguous) = match requirement {
                        Requirement::Member { start, ambiguous } => (*start, *ambiguous),
                        Requirement::Legacy => (0, false),
                    };
                    let marks = !ambiguous && advertises(tx, member, start, FEATURE)?;
                    let staged = marks && advertises(tx, member, start, STAGED_FEATURE)?;
                    updates.insert(member.clone(), (marks, staged));
                }
            }
            let ready = |member: &String| updates.get(member).or_else(|| self.ready.get(member)).copied().unwrap_or((false, false));
            let marks = requirements.keys().all(|member| ready(member).0);
            let staged = marks && requirements.keys().all(|member| ready(member).1);
            let mut switch_changes = 0;
            for (key, value) in [("marks_switched", marks), ("staged_switched", staged)] {
                switch_changes += tx.execute(
                    "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value<>excluded.value",
                    params![key, if value { "1" } else { "0" }],
                )?;
            }
            tx.execute("DELETE FROM local_marks_switch_dirty_v1", [])?;
            Ok((generation, next, updates, switch_changes))
        }).map_err(anyhow::Error::msg)??;
        let (generation, next, updates, switch_changes) = result;
        let work = store.extension::<Work>();
        if let Some(next) = next {
            self.ready.retain(|member, _| next.contains_key(member));
            self.requirements = next;
            work.roster_refreshes.fetch_add(1, Relaxed);
        }
        work.member_reevaluations.fetch_add(updates.len() as u64, Relaxed);
        work.switch_changes.fetch_add(switch_changes as u64, Relaxed);
        self.ready.extend(updates);
        self.peers = peers;
        self.fleet_generation = Some(generation);
        Ok(())
    }
}

fn requirements(connection: &Connection, origin: &str, peers: &BTreeSet<String>) -> Result<BTreeMap<String, Requirement>> {
    let membership = fleet_membership_tx(connection)?;
    let mut result = BTreeMap::new();
    if membership.anchor().is_some() {
        for member in membership.incarnations().filter(|member| member.end.is_none() && member.name != origin) {
            result.insert(member.name.clone(), Requirement::Member { start: member.start,
                ambiguous: !matches!(membership.state(&member.name), crate::fleet::MemberState::Current(_)) });
        }
    }
    let fenced = membership.legacy_removed_names().collect::<BTreeSet<_>>();
    for (table, column) in WRITER_TABLES {
        let mut previous = String::new();
        let mut query = connection.prepare_cached(&writer_seek(table, column))?;
        while let Some(writer) = query.query_row([&previous], |row| row.get::<_, String>(0)).optional()? {
            if writer != origin && !result.contains_key(&writer) && !fenced.contains(&writer)
                && !(membership.anchor().is_some() && membership.is_keyed_writer(&writer)) {
                result.insert(writer.clone(), Requirement::Legacy);
            }
            previous = writer;
        }
    }
    for peer in peers { result.entry(peer.clone()).or_insert(Requirement::Legacy); }
    Ok(result)
}

fn install(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS local_marks_switch_dirty_v1(member TEXT PRIMARY KEY) WITHOUT ROWID;")?;
    // Writer membership changes only on the first/last row for that writer, not on every batch.
    // Indexed existence probes avoid enumerating unrelated history on ordinary appends.
    // Explicit outer conflict policies override trigger OR IGNORE. Avoid conflicts altogether,
    // as the other durable source ledgers do, including when several triggers name one key.
    for (table, column) in WRITER_TABLES {
        for (event, row) in [("INSERT", "NEW"), ("DELETE", "OLD")] {
            let condition = if event == "INSERT" {
                format!("(SELECT count(*) FROM (SELECT 1 FROM {table} WHERE {column}={row}.{column} LIMIT 2))=1")
            } else { format!("NOT EXISTS(SELECT 1 FROM {table} WHERE {column}={row}.{column})") };
            connection.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS marks_switch_{table}_{event}_v1 AFTER {event} ON {table}
                 WHEN {condition} BEGIN INSERT INTO local_marks_switch_dirty_v1
                 SELECT '' WHERE NOT EXISTS(SELECT 1 FROM local_marks_switch_dirty_v1 WHERE member=''); END;"))?;
        }
        connection.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS marks_switch_{table}_writer_v1 AFTER UPDATE OF {column} ON {table}
             WHEN OLD.{column}<>NEW.{column} BEGIN INSERT INTO local_marks_switch_dirty_v1
             SELECT '' WHERE NOT EXISTS(SELECT 1 FROM local_marks_switch_dirty_v1 WHERE member=''); END;"))?;
    }
    for (event, rows) in [("INSERT", &["NEW"][..]), ("DELETE", &["OLD"][..]), ("UPDATE", &["OLD", "NEW"][..])] {
        let mut body = String::new();
        for row in rows {
            body.push_str(&format!("INSERT INTO local_marks_switch_dirty_v1 SELECT {row}.origin
                WHERE {row}.kind='daemon.started' AND {row}.subject='daemon/'||{row}.origin
                  AND NOT EXISTS(SELECT 1 FROM local_marks_switch_dirty_v1 WHERE member={row}.origin);"));
        }
        connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS marks_switch_claims_{event}_v1 AFTER {event} ON claims BEGIN {body} END;"))?;
        // Canonical rank and incarnation sequence can change without appending a claim.
        for (table, selector) in [("batches", "claims.batch_id={row}.id"),
            ("replica_records", "claims.id={row}.claim_id"),
            ("claim_positions", "claims.id={row}.claim_id"),
            ("projection_digest_repaired_claims", "claims.id={row}.id")] {
            if table == "batches" && event == "INSERT" { continue; }
            let mut body = String::new();
            for row in rows {
                body.push_str(&format!("INSERT INTO local_marks_switch_dirty_v1 SELECT DISTINCT claims.origin FROM claims
                    WHERE {} AND claims.kind='daemon.started' AND claims.subject='daemon/'||claims.origin
                      AND NOT EXISTS(SELECT 1 FROM local_marks_switch_dirty_v1 WHERE member=claims.origin);",
                    selector.replace("{row}", row)));
            }
            connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS marks_switch_{table}_capability_{event}_v1 AFTER {event} ON {table} BEGIN {body} END;"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::directive_notes::tests_shared_delivery::{claim, sync as exchange};

    fn sync(from: &Store, to: &Store) {
        let admitted = exchange(from, to);
        assert_eq!((admitted.invalid, admitted.unknown, admitted.held), (0, 0, 0),
            "source-trigger conflicts must never turn capability records into rejected replication");
    }

    fn started(store: &Store, marks: bool, staged: bool) {
        claim(store, &format!("daemon/{}", store.origin), "daemon.started", None,
            json!({"status": "running", "features": {FEATURE: u8::from(marks), STAGED_FEATURE: u8::from(staged)}}));
    }

    fn count(store: &Store) -> u64 {
        store.marks_switch_work()["member_reevaluations"].as_u64().unwrap()
    }

    #[test]
    fn idle_and_unrelated_writes_do_no_member_work() {
        let store = Store::open_memory("alder").unwrap();
        let peer = Store::open_memory("birch").unwrap();
        let mut maintenance = store.marks_switch_maintenance().unwrap();
        let peers = vec!["birch".to_owned(), "cedar".to_owned()];
        maintenance.refresh(&store, &peers).unwrap();
        assert_eq!(count(&store), 2);
        assert!(!store.marks_switched().unwrap());
        for _ in 0..20 { maintenance.refresh(&store, &peers).unwrap(); }
        assert_eq!(count(&store), 2, "idle performs no reevaluation");
        assert!(!maintenance.changes.has_changed().unwrap(), "draining keys does not wake itself");
        for index in 0..20 {
            claim(&store, &format!("message/unrelated-{index}"), "message.sent", Some("person/sender"),
                json!({"from": "person/sender", "to": "person/recipient", "content": "unrelated", "status": "sent"}));
        }
        // First local writer can change the source roster, but never a peer's requirement.
        maintenance.refresh(&store, &peers).unwrap();
        assert_eq!(count(&store), 2);
        claim(&store, "message/unrelated-final", "message.sent", Some("person/sender"),
            json!({"from": "person/sender", "to": "person/recipient", "content": "unrelated", "status": "sent"}));
        assert!(!maintenance.changes.has_changed().unwrap(), "ordinary appends have no marks-switch wake");
        maintenance.refresh(&store, &peers).unwrap();
        assert_eq!(count(&store), 2);

        started(&peer, true, true);
        sync(&peer, &store);
        assert!(maintenance.changes.has_changed().unwrap());
        maintenance.refresh(&store, &peers).unwrap();
        assert_eq!(count(&store), 3, "only birch's capability is evaluated");
        maintenance.refresh(&store, &["birch".to_owned()]).unwrap();
        assert_eq!(count(&store), 3, "peer removal does not reevaluate the remaining peer");
        assert!(store.marks_switched().unwrap() && store.staged_switched().unwrap());
        started(&peer, true, false);
        sync(&peer, &store);
        maintenance.refresh(&store, &["birch".to_owned()]).unwrap();
        assert_eq!(count(&store), 4);
        assert!(store.marks_switched().unwrap() && !store.staged_switched().unwrap());
        // A raw removal must select the previous capability, not retain the removed head.
        store.connection.batched(|tx| tx.execute(
            "DELETE FROM claims WHERE id=(SELECT id FROM claims WHERE origin='birch' AND kind='daemon.started' ORDER BY store_index DESC LIMIT 1)", []
        )).unwrap().unwrap();
        maintenance.refresh(&store, &["birch".to_owned()]).unwrap();
        assert_eq!(count(&store), 5);
        assert!(store.staged_switched().unwrap());
        maintenance.refresh(&store, &peers).unwrap();
        assert_eq!(count(&store), 6, "only the newly configured cedar is evaluated");
        assert!(!store.marks_switched().unwrap());
    }

    #[test]
    fn canonical_position_reordering_invalidates_only_its_member() {
        let store = Store::open_memory("alder").unwrap();
        let peer = Store::open_memory("birch").unwrap();
        started(&peer, true, true);
        started(&peer, false, false);
        sync(&peer, &store);
        store.connection.batched(|tx| -> Result<()> {
            tx.execute("UPDATE claims SET batch_id=(SELECT batch_id FROM claims WHERE kind='daemon.started' ORDER BY store_index LIMIT 1),
                accepted_at_unix_ms=(SELECT accepted_at_unix_ms FROM claims WHERE kind='daemon.started' ORDER BY store_index LIMIT 1)
                WHERE kind='daemon.started'", [])?;
            tx.execute("INSERT OR REPLACE INTO claim_positions(claim_id,position)
                SELECT id, CASE WHEN json_extract(body,'$.fields.features.message_view_reads')=1 THEN 1 ELSE 0 END
                FROM claims WHERE kind='daemon.started'", [])?;
            Ok(())
        }).unwrap().unwrap();
        let mut maintenance = store.marks_switch_maintenance().unwrap();
        let peers = ["birch".to_owned(), "cedar".to_owned()];
        maintenance.refresh(&store, &peers).unwrap();
        assert_eq!(maintenance.ready["birch"], (true, true));
        assert_eq!(count(&store), 2);
        store.connection.batched(|tx| tx.execute("UPDATE claim_positions SET position=1-position", []))
            .unwrap().unwrap();
        assert!(maintenance.changes.has_changed().unwrap());
        maintenance.refresh(&store, &peers).unwrap();
        assert_eq!(maintenance.ready["birch"], (false, false));
        assert_eq!(count(&store), 3, "cedar is not reevaluated by birch's position change");
        assert!(!store.marks_switched().unwrap());
    }

    #[test]
    fn failed_publication_keeps_keys_until_a_committed_repair() {
        let store = Store::open_memory("alder").unwrap();
        let peer = Store::open_memory("birch").unwrap();
        started(&peer, true, true);
        sync(&peer, &store);
        let mut maintenance = store.marks_switch_maintenance().unwrap();
        maintenance.refresh(&store, &[]).unwrap();
        started(&peer, false, false);
        sync(&peer, &store);
        store.connection.batched(|tx| tx.execute_batch(
            "CREATE TRIGGER marks_switch_test_failure BEFORE UPDATE ON meta
             WHEN NEW.key='marks_switched' BEGIN SELECT RAISE(ABORT,'publication blocked'); END;"
        )).unwrap().unwrap();
        assert!(maintenance.refresh(&store, &[]).is_err());
        assert_eq!(count(&store), 1, "failed transactions do not advance the retained decisions");
        let (wake, recovery) = tokio::sync::watch::channel(0u64);
        let _observer = store.observe_commits(move |_| {
            wake.send_modify(|generation| *generation += 1);
        });
        assert!(maintenance.refresh(&store, &[]).is_err(), "register then recheck remains blocked");
        assert!(!recovery.has_changed().unwrap(), "failed writes cannot busy-loop recovery");
        assert!(store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM local_marks_switch_dirty_v1 WHERE member='birch')", [],
            |row| row.get::<_, bool>(0)).unwrap());
        store.connection.batched(|tx| tx.execute_batch("DROP TRIGGER marks_switch_test_failure"))
            .unwrap().unwrap();
        assert!(recovery.has_changed().unwrap(), "the committed repair wakes recovery");
        maintenance.refresh(&store, &[]).unwrap();
        assert_eq!(count(&store), 2);
        assert!(!store.marks_switched().unwrap());
    }
}
