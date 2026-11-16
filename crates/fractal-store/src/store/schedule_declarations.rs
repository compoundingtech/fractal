//! Bounded reads of the applied declaration projection. No run/history reduction or writes.
use super::*;

pub const MAX_SCHEDULE_DECLARATIONS: usize = 200;
pub const MAX_SCHEDULE_DECLARATION_BYTES: usize = 64 * 1024;
pub const MAX_SCHEDULE_READ_HOSTS: usize = 601;

#[derive(Clone, Debug)]
pub struct ScheduleDeclarationRow {
    pub subject: String,
    pub revision: String,
    pub claim: String,
    pub body: Option<String>,
    pub body_bytes: u64,
    pub origin: Option<String>,
    pub owner_run: Option<String>,
    pub owner_generation: Option<String>,
    pub owner_step: Option<String>,
}

impl Store {
    /// At most limit+1 primary-key candidates, including opaque or oversized declarations.
    /// Call inside read_snapshot when combining these rows with their exchange metadata.
    pub fn schedule_declaration_rows(
        &self,
        after: &str,
        limit: usize,
    ) -> Result<Vec<ScheduleDeclarationRow>> {
        anyhow::ensure!((1..=MAX_SCHEDULE_DECLARATIONS).contains(&limit), "invalid schedule page limit");
        anyhow::ensure!(after.is_empty() || after.starts_with("schedule/"), "invalid schedule page boundary");
        let connection = self.readers.get();
        let mut query = connection.prepare_cached(
            "SELECT d.subject,d.revision,d.claim_id,
                    CASE WHEN length(CAST(d.body AS BLOB))<=?3 THEN d.body ELSE NULL END,
                    length(CAST(d.body AS BLOB)),c.origin,
                    d.owner_run,d.owner_generation,d.owner_step
             FROM desired d LEFT JOIN claims c ON c.id=d.claim_id
             WHERE d.subject>= 'schedule/' AND d.subject< 'schedule0' AND d.subject>?1
             ORDER BY d.subject LIMIT ?2",
        )?;
        let rows = query.query_map(
            params![after, limit + 1, MAX_SCHEDULE_DECLARATION_BYTES],
            |row| Ok(ScheduleDeclarationRow {
                subject: row.get(0)?, revision: row.get(1)?, claim: row.get(2)?,
                body: row.get(3)?, body_bytes: row.get::<_, i64>(4)?.max(0) as u64,
                origin: row.get(5)?, owner_run: row.get(6)?,
                owner_generation: row.get(7)?, owner_step: row.get(8)?,
            }),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    /// Existing direct-exchange receipt times, not an applied frontier or a freshness proof.
    pub fn schedule_exchange_times(
        &self,
        hosts: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, Option<String>>> {
        anyhow::ensure!(hosts.len() <= MAX_SCHEDULE_READ_HOSTS, "schedule host coverage exceeds its bound");
        let connection = self.readers.get();
        let mut query = connection.prepare_cached(
            "SELECT last_success_at_unix_ms FROM replication_peers WHERE peer=?1",
        )?;
        let mut times = BTreeMap::new();
        for host in hosts {
            // This is a bind on an already prepared read, not SQL assembled from the name.
            let time = query.query_map([host], |row| row.get::<_, Option<String>>(0))?
                .next().transpose()?.flatten();
            times.insert(host.clone(), time);
        }
        Ok(times)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, c_char, c_int, c_void};

    // Test-only guard on a pinned fixture connection. Count actual SQLite VM instructions and
    // refuse history payloads/run tables; this never changes the production reader or schema.
    struct Guard { db: *mut rusqlite::ffi::sqlite3, steps: Box<usize> }
    unsafe extern "C" fn count(context: *mut c_void) -> c_int {
        unsafe { *(context.cast::<usize>()) += 1; }
        0
    }
    unsafe extern "C" fn authorize(_: *mut c_void, action: c_int, table: *const c_char, column: *const c_char, _: *const c_char, _: *const c_char) -> c_int {
        if action != rusqlite::ffi::SQLITE_READ { return rusqlite::ffi::SQLITE_OK; }
        let table=unsafe { CStr::from_ptr(table) }.to_bytes();
        let column=unsafe { CStr::from_ptr(column) }.to_bytes();
        if table == b"mission_runs" || table == b"step_runs" || (table == b"claims" && column != b"id" && column != b"origin") {
            rusqlite::ffi::SQLITE_DENY
        } else { rusqlite::ffi::SQLITE_OK }
    }
    impl Guard {
        fn install(connection: &rusqlite::Connection) -> Self {
            let mut guard=Self { db:unsafe { connection.handle() }, steps:Box::new(0) };
            unsafe {
                assert_eq!(rusqlite::ffi::sqlite3_set_authorizer(guard.db,Some(authorize),std::ptr::null_mut()),rusqlite::ffi::SQLITE_OK);
                rusqlite::ffi::sqlite3_progress_handler(guard.db,1,Some(count),(&mut *guard.steps as *mut usize).cast());
            }
            guard
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) { unsafe {
            rusqlite::ffi::sqlite3_progress_handler(self.db,0,None,std::ptr::null_mut());
            rusqlite::ffi::sqlite3_set_authorizer(self.db,None,std::ptr::null_mut());
        } }
    }
    fn seed(store: &Store, count: usize) {
        let source="version 2\nschedule \"a\" { at \"2030-01-01T00:00:00Z\"; work { mission \"harvest\"; workspace \"/tmp/harvest\" } }\n";
        if store.schedule_declaration_rows("",1).unwrap().is_empty() {
            let intent=crate::graph::parse_test_intent(source,"node").unwrap();
            let plan=store.mission(&intent,crate::model::IntentInput{kdl:source.into(),source_name:None}).unwrap();
            store.apply(&intent,&plan.subject_tokens,"seed").unwrap();
        }
        let connection=store.connection.write();
        for n in 0..count {
            connection.execute("INSERT OR IGNORE INTO desired(subject,kind,revision,body,claim_id) SELECT ?1,'agent','revision','{}',claim_id FROM desired WHERE subject='schedule/a'",[format!("unrelated/{n:06}")]).unwrap();
        }
        for name in ["b","c"] {
            connection.execute("INSERT OR IGNORE INTO desired(subject,kind,revision,body,claim_id) SELECT ?1,'schedule','revision','{}',claim_id FROM desired WHERE subject='schedule/a'",[format!("schedule/{name}")]).unwrap();
        }
    }
    fn measured(store: &Store) -> usize {
        store.read_snapshot(|_| {
            let connection=store.readers.get();
            let guard=Guard::install(&connection);
            let rows=store.schedule_declaration_rows("schedule/a",1)?;
            assert_eq!(rows.iter().map(|r|r.subject.as_str()).collect::<Vec<_>>(),["schedule/b","schedule/c"]);
            assert_eq!(rows[0].origin.as_deref(),Some("node"));
            let times=store.schedule_exchange_times(&BTreeSet::from(["never".to_owned()]))?;
            assert_eq!(times["never"],None);
            let steps=*guard.steps;
            // Negative controls prove that the guard really forbids both history and run reads.
            let history_error=connection.prepare("SELECT body FROM claims LIMIT 1").expect_err("history read was allowed");
            assert!(history_error.to_string().contains("prohibited"),"{history_error}");
            let run_error=connection.prepare("SELECT id FROM mission_runs LIMIT 1").expect_err("run read was allowed");
            assert!(run_error.to_string().contains("prohibited"),"{run_error}");
            Ok(steps)
        }).unwrap()
    }
    #[test]
    fn bounded_schedule_reader_never_reads_history_and_seeks_past_unrelated_declarations() {
        let store=Store::open_memory("node").unwrap();
        seed(&store,8); let small=measured(&store);
        seed(&store,4000); let large=measured(&store);
        assert!(large <= small + 24,"synthetic VM steps grew with unrelated rows: {small} -> {large}");
        assert!(store.schedule_declaration_rows("",0).is_err());
        assert!(store.schedule_declaration_rows("",201).is_err());
        assert!(store.schedule_exchange_times(&(0..602).map(|n|format!("host-{n}")).collect()).is_err());
    }
    #[test]
    fn unchanged_direct_exchange_refreshes_the_existing_timestamp_without_new_payload() {
        let fleet="94cd11ba-c582-4558-9c84-c3bda922eb6d";
        let local=Store::open_memory("amber").unwrap(); local.bind_fleet(fleet).unwrap();
        let peer=Store::open_memory("cobalt").unwrap(); peer.bind_fleet(fleet).unwrap();
        let exchange=peer.export_replication_exchange(fleet,&ReplicationInventory::default()).unwrap();
        local.receive_replication_exchange("cobalt",fleet,&exchange).unwrap();
        // A synthetic old timestamp makes advancement discriminating without sleeping or a
        // production clock override. The second normal receive contains the identical payload.
        local.connection.write().execute("UPDATE replication_peers SET last_success_at_unix_ms='1' WHERE peer='cobalt'",[]).unwrap();
        let before=local.event_bounds().unwrap();
        local.receive_replication_exchange("cobalt",fleet,&exchange).unwrap();
        assert_eq!(local.event_bounds().unwrap(),before,"unchanged exchange adds no claim payload");
        let times=local.schedule_exchange_times(&BTreeSet::from(["cobalt".to_owned()])).unwrap();
        assert!(times["cobalt"].as_ref().unwrap().parse::<u128>().unwrap()>1);
    }
}
