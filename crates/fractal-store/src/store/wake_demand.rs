//! Disposable v1 wake metadata. No bodies, replicated state, or suspend/wake side effects.
//! A zero-row result is evidence only at its reported, caught-up source frontier.
use super::*;

const TABLE: &str = "local_wake_demand_v1";
const QUEUE: &str = "local_wake_demand_pending_v1";
const COMPLETE: &str = "local_wake_demand_v1_complete";
const AFTER: &str = "local_wake_demand_v1_after";

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h3eddd3ad87ca owner=fractal_store::store::wake_demand TODO=chunk-or-reference-variable-columns */ "
        CREATE TABLE IF NOT EXISTS local_wake_demand_v1 (
            subject TEXT PRIMARY KEY, recipient TEXT NOT NULL, class INTEGER NOT NULL,
            sent_ms INTEGER NOT NULL, pending INTEGER NOT NULL
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS local_wake_demand_seat_v1
            ON local_wake_demand_v1(recipient,class DESC,sent_ms,subject) WHERE pending=1;
        CREATE TABLE IF NOT EXISTS local_wake_demand_pending_v1 (
            subject TEXT PRIMARY KEY
        ) WITHOUT ROWID;
    ")?;
    for (table, subject, condition) in [
        ("claims", "subject", "NEW.kind LIKE 'message.%'"),
        ("local_stream_sent", "subject", "1"),
        ("local_stream_lifecycle", "subject", "1"),
        ("local_message_marks", "message", "1"),
        ("local_message_staged", "message", "1"),
    ] {
        // Receipts clear pending immediately, even before the send reaches this member.
        // Send folds read every receipt source, including receipt-before-send replication.
        let receipt = if table == "local_message_staged" {
            "1".to_owned()
        } else if table == "local_stream_sent" {
            "0".to_owned()
        } else {
            "NEW.kind IN ('message.staged','message.delivered','message.read','message.closed')".to_owned()
        };
        let old_condition = condition.replace("NEW.", "OLD.");
        connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-hc986dc3c5735 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "
            CREATE TRIGGER IF NOT EXISTS wake_v1_{table}_insert AFTER INSERT ON {table}
            WHEN {condition} BEGIN
                UPDATE {TABLE} SET pending=0 WHERE subject=NEW.{subject} AND {receipt};
                INSERT OR IGNORE INTO {QUEUE} SELECT NEW.{subject} WHERE NOT ({receipt});
            END;
            CREATE TRIGGER IF NOT EXISTS wake_v1_{table}_delete AFTER DELETE ON {table}
            WHEN {old_condition}
            BEGIN INSERT OR IGNORE INTO {QUEUE} VALUES(OLD.{subject}); END;
            CREATE TRIGGER IF NOT EXISTS wake_v1_{table}_update AFTER UPDATE ON {table}
            WHEN {condition} OR {old_condition}
            BEGIN
                INSERT OR IGNORE INTO {QUEUE} VALUES(OLD.{subject});
                INSERT OR IGNORE INTO {QUEUE} VALUES(NEW.{subject});
            END;
        "))?;
    }
    // A fresh version rebuilds from both sources in bounded, restartable transactions.
    let complete: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)", [COMPLETE], |row| row.get(0),
    )?;
    if !complete {
        let mut after: String = connection.query_row(
            "SELECT value FROM meta WHERE key=?1", [AFTER], |row| row.get(0),
        ).optional()?.unwrap_or_default();
        loop {
            let subjects = connection.prepare_cached(
                "SELECT subject FROM message_index WHERE subject>?1
                 UNION SELECT subject FROM local_stream_sent WHERE subject>?1
                 ORDER BY subject LIMIT 64",
            )?.query_map([&after], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if subjects.is_empty() { break; }
            let transaction = connection.unchecked_transaction()?;
            for subject in &subjects { fold(&transaction, subject)?; }
            after = subjects.last().unwrap().clone();
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h7de6575aa7e5 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![AFTER, after])?;
            transaction.commit()?;
        }
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h9eed8a526c41 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,'1')", [COMPLETE])?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hcaf12b1aa1c2 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [AFTER])?;
        transaction.commit()?;
    }
    // Also drain work left by an interrupted projection pass.
    let transaction = connection.unchecked_transaction()?;
    flush(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn fold(connection: &Connection, subject: &str) -> Result<()> {
    // Graph claims win duplicate sends, as in the other mirror consumers. Only metadata is read.
    let sent: Option<(String, i64)> = connection.query_row(
        "SELECT json_object('from',json_extract(body,'$.fields.from'),
                            'to',json_extract(body,'$.fields.to'),
                            'tags',json_extract(body,'$.fields.tags')),
                CAST(accepted_at_unix_ms AS INTEGER)
         FROM claims WHERE subject=?1 AND kind='message.sent'
         ORDER BY CAST(accepted_at_unix_ms AS INTEGER),id LIMIT 1",
        [subject], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let sent = match sent {
        Some(sent) => Some(sent),
        None => connection.query_row(
            "SELECT fields,sent_ms FROM local_stream_sent WHERE subject=?1", [subject],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?,
    };
    connection.execute(/* sqlite-large-value: allow-for-now lv-h7883dab4ee98 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_wake_demand_v1 WHERE subject=?1", [subject])?;
    let Some((fields, sent_ms)) = sent else { return Ok(()); };
    let fields: Value = serde_json::from_str(&fields)?;
    let from = fields["from"].as_str().unwrap_or_default();
    let tags: Vec<String> = fields["tags"].as_array().into_iter().flatten()
        .filter_map(Value::as_str).map(str::to_owned).collect();
    let held = tags.iter().any(|tag| crate::silent::is_silent_tag(tag))
        && !crate::silent::always_wakes(from, &tags);
    let class = if held { 0 } else if from.starts_with("person/") || from.starts_with("external/") { 2 } else { 1 };
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hc920614e4c86 owner=fractal_store::store::wake_demand TODO=prepare-body-and-predecessor-values */ "INSERT INTO local_wake_demand_v1(subject,recipient,class,sent_ms,pending)
         SELECT ?1,?2,?3,?4,NOT (
             EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind IN
                 ('message.staged','message.delivered','message.read','message.closed'))
             OR EXISTS(SELECT 1 FROM local_message_marks WHERE message=?1)
             OR EXISTS(SELECT 1 FROM local_message_staged WHERE message=?1)
             OR EXISTS(SELECT 1 FROM local_stream_lifecycle WHERE subject=?1 AND kind IN
                 ('message.staged','message.delivered','message.read','message.closed')))",
        params![subject, fields["to"].as_str().unwrap_or_default(), class, sent_ms],
    )?;
    Ok(())
}

pub fn flush(transaction: &Transaction<'_>) -> Result<()> {
    loop {
        let subjects = transaction.prepare_cached(
            "SELECT subject FROM local_wake_demand_pending_v1 ORDER BY subject LIMIT 64",
        )?.query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if subjects.is_empty() { return Ok(()); }
        for subject in subjects {
            fold(transaction, &subject)?;
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h58fc3f27c31a owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_wake_demand_pending_v1 WHERE subject=?1", [&subject])?;
        }
    }
}

/// Independent source positions; graph and shard indexes are never compared to each other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontier {
    pub graph: u64,
    pub marks: u64,
    pub mirrored_sent: u64,
    pub mirrored_lifecycle: u64,
    /// None means the shard is unavailable, not empty.
    pub source: Option<(u64, u64)>,
    pub caught_up: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWake {
    pub subject: String,
    pub sent_ms: i64,
    pub person: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demand {
    /// Oldest person demand first, otherwise oldest wake; ties use the message subject.
    pub oldest: Option<PendingWake>,
    pub frontier: Frontier,
}

impl Store {
    /// Read-only: never mirrors or populates the projection. A lagging/absent shard cannot
    /// prove absence. The source cut is sampled first; the graph is then read in one snapshot.
    pub fn wake_demand(&self, seat: &str) -> Result<Demand> {
        smallclaims::touched::note_read(|| format!("mailbox:{seat}"));
        let demand = self.wake_demand_untracked(seat)?;
        if let Some(pending) = &demand.oldest {
            smallclaims::touched::note_read(|| pending.subject.clone());
        }
        Ok(demand)
    }

    /// Queue ranking must not subscribe a seat's reactive item to other seats' mailboxes.
    pub(super) fn wake_demand_untracked(&self, seat: &str) -> Result<Demand> {
        let source = messages_shard::shard(self).map(|shard| {
            shard.readers.get().query_row(
                "SELECT COALESCE((SELECT MAX(store_index) FROM stream_messages),0),
                        COALESCE((SELECT MAX(rowid) FROM stream_lifecycle),0)", [],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
            )
        }).transpose()?;
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction()?;
        let mirrored_sent = stream_mirror::sent_through(&transaction)?;
        let mirrored_lifecycle = stream_mirror::lifecycle_through(&transaction)?;
        let dirty: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_wake_demand_pending_v1)", [], |row| row.get(0),
        )?;
        let graph = transaction.query_row("SELECT COALESCE(MAX(store_index),0) FROM claims", [], |row| row.get(0))?;
        let oldest = transaction.query_row(
            "SELECT subject,sent_ms,class FROM local_wake_demand_v1
             WHERE recipient=?1 AND pending=1 AND class>0
             ORDER BY class DESC,sent_ms,subject LIMIT 1", [seat],
            |row| Ok(PendingWake { subject: row.get(0)?, sent_ms: row.get(1)?, person: row.get::<_, i64>(2)? == 2 }),
        ).optional()?;
        Ok(Demand { oldest, frontier: Frontier {
            graph, marks: message_marks::event_position(&transaction)?, mirrored_sent,
            mirrored_lifecycle, source,
            caught_up: !dirty && source.is_some_and(|(sent, lifecycle)| mirrored_sent >= sent && mirrored_lifecycle >= lifecycle),
        } })
    }
}
