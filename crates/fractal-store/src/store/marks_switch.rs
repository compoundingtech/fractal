//! Whether this member records its own seats' message lifecycle as local marks instead of
//! lifecycle claims (db break-up step 4b-2).
//!
//! A mark never replicates. Another member learns of it only by asking this host on the
//! seat-freshness relay. A member that advertises `seat_freshness_reads` (4b-3) asks it in
//! delivery status, doctor and the mail-backlog check, but its message view and lists read only
//! its own records, where another host's mark is missing and the mail would show as sent. So
//! this member writes marks only while every other member advertises [`FEATURE`], which a build
//! advertises once those reads ask the relay too (4b-4): its message view and lists, the targets
//! of its faults and its work wakes. A member beside one that does not keeps claims. The
//! barrier is read the strict way the standby barrier reads its own: the newest
//! `daemon.started` of each current incarnation, any ambiguous membership counting as not ready,
//! and no writer outside the membership. A member alone switches at once: no other member's view
//! can be wrong. The barrier is checked while the daemon runs
//! ([`Store::refresh_marks_switch`]), cached in `meta`, and read by each writer in its own
//! transaction ([`marks_switched_tx`]); a member that joins without the feature turns it off
//! again, and writers return to claims from then on.
use super::*;

/// The `daemon.started` feature every other member advertises before this one records marks: its
/// message view and lists ask a seat's host for the seat's marks.
pub const FEATURE: &str = "message_view_reads";

/// The `daemon.started` feature of a build whose reads of another host's seats count a staging the
/// host answers, below every mark. A staging becomes a mark only once every other member
/// advertises it; this build advertises it and still records stagings as claims.
pub const STAGED_FEATURE: &str = "message_staged_reads";

const WRITER_TABLES: [(&str, &str); 3] = [
    ("replica_envelopes", "writer"), ("batches", "origin"), ("checkpoint_envelopes", "writer"),
];
fn writer_seek(table: &str, column: &str) -> String {
    format!("SELECT {column} FROM {table} WHERE {column}>?1 ORDER BY {column} LIMIT 1")
}

/// Whether `member`'s newest own `daemon.started` at or after `start` advertises [`FEATURE`].
fn advertises(connection: &Connection, member: &str, start: u64) -> Result<bool> {
    let advertised: Option<Option<bool>> = connection
        .query_row(/* sqlite-large-value: allow-for-now lv-h68c3476e60b3 owner=fractal_store::store::marks_switch TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT json_extract(claims.body,'$.fields.features.message_view_reads')=1
             FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind='daemon.started' AND claims.origin=?2
               AND batches.replica_sequence>=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
            params![format!("daemon/{member}"), member, start],
            |row| row.get(0))
        .optional()?;
    Ok(advertised.flatten() == Some(true))
}

/// Why this member may not record marks yet, one reason per member, peer or writer; none means it
/// may. `peers` are the members this daemon is configured to exchange with: one whose start has
/// not reached this member yet, as on a member's first sync, is not ready.
pub fn blockers(connection: &Connection, origin: &str, peers: &[String]) -> Result<Vec<String>> {
    let membership = fleet_membership_tx(connection)?;
    let mut blockers = Vec::new();
    let mut members = BTreeSet::new();
    if membership.anchor().is_some() {
        for member in membership.incarnations().filter(|member| member.end.is_none() && member.name != origin) {
            members.insert(member.name.clone());
            if !matches!(membership.state(&member.name), crate::fleet::MemberState::Current(_)) {
                blockers.push(format!("host/{}: its fleet membership is ambiguous", member.name));
            } else if !advertises(connection, &member.name, member.start)? {
                blockers.push(format!("host/{}: has not advertised {FEATURE}=1", member.name));
            }
        }
    }
    // A writer outside the membership (a fleet without one, or a legacy writer not fenced) reads
    // this member's state from claims unless its own newest start says otherwise.
    let fenced = membership.legacy_removed_names().collect::<BTreeSet<_>>();
    let mut writers = BTreeSet::new();
    for (table, column) in WRITER_TABLES {
        let mut previous = String::new();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h4368bbfe977e owner=fractal_store::store::marks_switch TODO=trace-statement-and-value-representation */ &writer_seek(table, column))?;
        while let Some(writer) = statement.query_row(/* sqlite-large-value: allow-for-now lv-he3fa6b5c879c owner=fractal_store::store::marks_switch TODO=trace-statement-and-value-representation */ [&previous], |row| row.get::<_, String>(0)).optional()? {
            if writer != origin && !members.contains(&writer) && !fenced.contains(&writer)
                && !(membership.anchor().is_some() && membership.is_keyed_writer(&writer)) {
                writers.insert(writer.clone());
            }
            previous = writer;
        }
    }
    writers.extend(peers.iter().filter(|peer| **peer != origin && !members.contains(*peer)).cloned());
    for writer in writers {
        if !advertises(connection, &writer, 0)? {
            blockers.push(format!("{writer}: exchanges with this member outside its fleet membership and has not advertised {FEATURE}=1"));
        }
    }
    Ok(blockers)
}

/// Whether this member records marks, as [`Store::refresh_marks_switch`] last decided, read in a
/// writer's own transaction so the choice and the write agree.
pub fn marks_switched_tx(connection: &Connection) -> Result<bool> {
    Ok(connection
        .query_row("SELECT value FROM meta WHERE key='marks_switched'", [], |row| row.get::<_, String>(0))
        .optional()?
        .as_deref()
        == Some("1"))
}

impl Store {
    /// Whether this member records its seats' lifecycle as marks (see the module).
    pub fn marks_switched(&self) -> Result<bool> {
        marks_switched_tx(&self.readers.get())
    }

    /// Decide the barrier again, with the daemon's configured `peers`, and record it when it
    /// changed. Answers whether marks are on and, when they are not, why.
    pub fn refresh_marks_switch(&self, peers: &[String]) -> Result<(bool, Vec<String>)> {
        let blockers = blockers(&self.readers.get(), &self.origin, peers)?;
        let switched = blockers.is_empty();
        if self.marks_switched()? != switched {
            let value = if switched { "1" } else { "0" };
            self.connection
                .batched(|transaction| {
                    transaction.execute(
                        /* sqlite-large-value: allow-for-now lv-he3398ba708bb owner=fractal_store::store::marks_switch TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES ('marks_switched', ?1)
                         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                        [value],
                    )
                })
                .map_err(anyhow::Error::msg)??;
            tracing::debug!(switched, ?blockers, "message marks switch changed");
        }
        Ok((switched, blockers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::MemberKey;
    use crate::store::directive_notes::tests_shared_delivery::{FLEET, claim, enable, sync};

    fn started(store: &Store, reads: bool) {
        claim(store, &format!("daemon/{}", store.origin), "daemon.started", None,
            json!({"status": "running", "features": {FEATURE: u8::from(reads)}}));
    }

    /// A member alone switches at once, with or without a fleet. With another member it waits
    /// until that member's own newest start advertises the reads, which no other origin can say
    /// for it, and a member that joins without them turns marks off again.
    #[test]
    fn marks_switch_on_only_once_every_other_member_reads_them() {
        let alone = Store::open_memory("alder").unwrap();
        assert!(!alone.marks_switched().unwrap(), "off until decided");
        assert_eq!(alone.refresh_marks_switch(&[]).unwrap(), (true, Vec::new()));
        assert!(alone.marks_switched().unwrap());
        let anchored = Store::open_memory("alder").unwrap();
        enable(&anchored);
        assert!(anchored.refresh_marks_switch(&[]).unwrap().0);
        // A configured peer this member has not heard start yet, as before a first sync.
        let (switched, blockers) = Store::open_memory("alder").unwrap().refresh_marks_switch(&["birch".to_owned()]).unwrap();
        assert!(!switched);
        assert_eq!(blockers, ["birch: exchanges with this member outside its fleet membership and has not advertised message_view_reads=1"]);

        let store = Store::open_memory("alder").unwrap();
        let anchor = enable(&store);
        let peer_key = Arc::new(MemberKey::generate().unwrap().0);
        claim(&store, "host/birch", "fleet.member-admitted", None,
            json!({"fleet_id": FLEET, "member_key": peer_key.public(), "via": "invite", "sponsor": "host/alder", "mode": "listening"}));
        store.replication_snapshot().unwrap();
        let peer = Store::open_memory("birch").unwrap();
        peer.bind_fleet(FLEET).unwrap();
        peer.pin_fleet_anchor(anchor.public()).unwrap();
        peer.set_member_key(Some(peer_key.clone())).unwrap();
        sync(&store, &peer);
        started(&peer, false);
        sync(&peer, &store);
        let (switched, blockers) = store.refresh_marks_switch(&[]).unwrap();
        assert!(!switched);
        assert_eq!(blockers, ["host/birch: has not advertised message_view_reads=1"]);
        // Another origin cannot speak for the member.
        claim(&store, "daemon/birch", "daemon.started", None, json!({"status": "running", "features": {FEATURE: 1}}));
        assert!(!store.refresh_marks_switch(&[]).unwrap().0);
        started(&peer, true);
        sync(&peer, &store);
        assert_eq!(store.refresh_marks_switch(&[]).unwrap(), (true, Vec::new()));
        assert!(store.marks_switched().unwrap());
        // The peer restarts on an older build: marks go off again.
        started(&peer, false);
        sync(&peer, &store);
        assert!(!store.refresh_marks_switch(&[]).unwrap().0);
        assert!(!store.marks_switched().unwrap());
    }

    /// Each statement a barrier check starts, with the full-scan steps it took.
    type Traced = Mutex<Vec<(String, i32)>>;
    unsafe extern "C" fn traced(event: u32, context: *mut std::ffi::c_void, statement: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
        if event == rusqlite::ffi::SQLITE_TRACE_PROFILE as u32 {
            // SAFETY: for a profile event SQLite passes the statement, live for the callback, and
            // `context` is the test's `Traced`, which outlives the registration.
            let (sql, scans) = unsafe {
                let statement = statement.cast::<rusqlite::ffi::sqlite3_stmt>();
                (std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_sql(statement)).to_string_lossy().into_owned(),
                 rusqlite::ffi::sqlite3_stmt_status(statement, rusqlite::ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP, 1))
            };
            unsafe { &*context.cast::<Traced>() }.lock().unwrap().push((sql, scans));
        }
        0
    }
    fn traced_blockers(store: &Store) -> Vec<(String, i32)> {
        let connection = store.readers.get();
        let statements: Traced = Mutex::new(Vec::new());
        // SAFETY: `statements` outlives the registration, which ends before it drops.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(connection.handle(), rusqlite::ffi::SQLITE_TRACE_PROFILE as u32,
                Some(traced), std::ptr::from_ref(&statements).cast_mut().cast());
        }
        let result = blockers(&connection, &store.origin, &["cedar".to_owned()]);
        unsafe { rusqlite::ffi::sqlite3_trace_v2(connection.handle(), 0, None, std::ptr::null_mut()) };
        assert_eq!(result.unwrap().len(), 1, "cedar, configured, has not started");
        statements.into_inner().unwrap()
    }

    /// The daemon checks the barrier every minute, so a check reads indexes only: each writer
    /// seek and each member's start is one indexed step, none scans a table, and the check runs
    /// as many statements over a long history as over a short one.
    #[test]
    fn a_barrier_check_seeks_each_writer_and_never_scans_the_history() {
        let store = Store::open_memory("alder").unwrap();
        enable(&store);
        let peer = Store::open_memory("birch").unwrap();
        started(&store, true);
        started(&peer, true);
        sync(&peer, &store);
        let short = traced_blockers(&store);
        for index in 0..300 {
            for member in [&store, &peer] {
                claim(member, &format!("message/{}-{index}", member.origin), "message.sent", Some("person/sender"),
                    json!({"from": "person/sender", "to": "person/recipient", "content": "history", "status": "sent"}));
            }
        }
        sync(&peer, &store);
        let long = traced_blockers(&store);
        assert_eq!(long.len(), short.len(), "{long:#?}");
        for (table, column) in WRITER_TABLES {
            let seek = writer_seek(table, column);
            // birch, alder and the end of the table, at most.
            assert!(long.iter().filter(|(sql, _)| *sql == seek).count() <= 3, "{long:#?}");
        }
        assert!(long.iter().all(|(_, scans)| *scans == 0), "a barrier statement scanned a table: {long:#?}");
        let connection = store.readers.get();
        for (table, column) in WRITER_TABLES {
            let plans = connection.prepare(&format!("EXPLAIN QUERY PLAN {}", writer_seek(table, column))).unwrap()
                .query_map([""], |row| row.get::<_, String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            assert!(plans.iter().all(|plan| plan.starts_with("SEARCH") && plan.contains("INDEX")), "{table}: {plans:?}");
        }
    }
}
