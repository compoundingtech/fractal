//! Whether this member records its own seats' message lifecycle as local marks instead of
//! lifecycle claims (db break-up step 4b-2).
//!
//! A mark never replicates. Another member learns of it only by asking this host on the
//! seat-freshness relay, which a member does once it advertises `seat_freshness_reads` (4b-3; its
//! build also reads the marks 4b-1 records). So this member writes marks only while every other
//! member advertises that feature, read the strict way the standby barrier reads its own: the
//! newest `daemon.started` of each current incarnation, any ambiguous membership counting as not
//! ready, and no writer outside the membership. A member alone switches at once. The barrier is
//! checked while the daemon runs ([`Store::refresh_marks_switch`]), cached in `meta`, and read by
//! each writer in its own transaction ([`marks_switched_tx`]); a member that joins without the
//! feature turns it off again, and writers return to claims from then on.
use super::*;

/// The `daemon.started` feature every other member advertises before this one records marks.
pub const FEATURE: &str = "seat_freshness_reads";

const WRITER_TABLES: [(&str, &str); 3] = [
    ("replica_envelopes", "writer"), ("batches", "origin"), ("checkpoint_envelopes", "writer"),
];
fn writer_seek(table: &str, column: &str) -> String {
    format!("SELECT {column} FROM {table} WHERE {column}>?1 ORDER BY {column} LIMIT 1")
}

/// Whether `member`'s newest own `daemon.started` at or after `start` advertises [`FEATURE`].
fn advertises(connection: &Connection, member: &str, start: u64) -> Result<bool> {
    let advertised: Option<Option<bool>> = connection
        .query_row(&canonical_sql(
            "SELECT json_extract(claims.body,'$.fields.features.seat_freshness_reads')=1
             FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind='daemon.started' AND claims.origin=?2
               AND batches.replica_sequence>=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
            params![format!("daemon/{member}"), member, start],
            |row| row.get(0))
        .optional()?;
    Ok(advertised.flatten() == Some(true))
}

/// Why this member may not record marks yet, one reason per member or writer; none means it may.
pub fn blockers(connection: &Connection, origin: &str) -> Result<Vec<String>> {
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
        let mut statement = connection.prepare(&writer_seek(table, column))?;
        while let Some(writer) = statement.query_row([&previous], |row| row.get::<_, String>(0)).optional()? {
            if writer != origin && !members.contains(&writer) && !fenced.contains(&writer)
                && !(membership.anchor().is_some() && membership.is_keyed_writer(&writer)) {
                writers.insert(writer.clone());
            }
            previous = writer;
        }
    }
    for writer in writers {
        if !advertises(connection, &writer, 0)? {
            blockers.push(format!("{writer}: writes to this member outside its fleet membership and has not advertised {FEATURE}=1"));
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

    /// Decide the barrier again and record it when it changed. Answers whether marks are on and,
    /// when they are not, why.
    pub fn refresh_marks_switch(&self) -> Result<(bool, Vec<String>)> {
        let blockers = blockers(&self.readers.get(), &self.origin)?;
        let switched = blockers.is_empty();
        if self.marks_switched()? != switched {
            let value = if switched { "1" } else { "0" };
            self.connection
                .batched(|transaction| {
                    transaction.execute(
                        "INSERT INTO meta(key, value) VALUES ('marks_switched', ?1)
                         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                        [value],
                    )
                })
                .map_err(anyhow::Error::msg)??;
            tracing::info!(switched, ?blockers, "message marks switch changed");
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
        assert_eq!(alone.refresh_marks_switch().unwrap(), (true, Vec::new()));
        assert!(alone.marks_switched().unwrap());
        let anchored = Store::open_memory("alder").unwrap();
        enable(&anchored);
        assert!(anchored.refresh_marks_switch().unwrap().0);

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
        let (switched, blockers) = store.refresh_marks_switch().unwrap();
        assert!(!switched);
        assert_eq!(blockers, ["host/birch: has not advertised seat_freshness_reads=1"]);
        // Another origin cannot speak for the member.
        claim(&store, "daemon/birch", "daemon.started", None, json!({"status": "running", "features": {FEATURE: 1}}));
        assert!(!store.refresh_marks_switch().unwrap().0);
        started(&peer, true);
        sync(&peer, &store);
        assert_eq!(store.refresh_marks_switch().unwrap(), (true, Vec::new()));
        assert!(store.marks_switched().unwrap());
        // The peer restarts on an older build: marks go off again.
        started(&peer, false);
        sync(&peer, &store);
        assert!(!store.refresh_marks_switch().unwrap().0);
        assert!(!store.marks_switched().unwrap());
    }
}
