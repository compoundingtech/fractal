//! A daily planning estimate. Message subjects contribute once; receipts never contribute.
//! Only explicit probe/test metadata is excluded. No turn attribution or content classifier.
use super::*;


pub const DAY_MS: u64 = 86_400_000;
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h6d2b3ebed355 owner=st3_store::store::agent_messages TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS agent_message_sends (
    subject TEXT PRIMARY KEY,
    sent_ms INTEGER NOT NULL,
    recipient TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS agent_message_sends_time ON agent_message_sends(sent_ms,recipient);
CREATE TABLE IF NOT EXISTS agent_message_days (
    day_ms INTEGER NOT NULL,
    recipient TEXT NOT NULL,
    count INTEGER NOT NULL CHECK(count >= 0),
    PRIMARY KEY(day_ms,recipient)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_agent_message_pending (subject TEXT PRIMARY KEY) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS agent_message_claim_insert AFTER INSERT ON claims
WHEN NEW.kind='message.sent' BEGIN
    INSERT OR IGNORE INTO local_agent_message_pending VALUES(NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS agent_message_claim_delete AFTER DELETE ON claims
WHEN OLD.kind='message.sent' BEGIN
    INSERT OR IGNORE INTO local_agent_message_pending VALUES(OLD.subject);
END;
CREATE TRIGGER IF NOT EXISTS agent_message_claim_update
AFTER UPDATE OF subject,kind,body,accepted_at_unix_ms ON claims
WHEN OLD.kind='message.sent' OR NEW.kind='message.sent' BEGIN
    INSERT OR IGNORE INTO local_agent_message_pending VALUES(OLD.subject);
    INSERT OR IGNORE INTO local_agent_message_pending VALUES(NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS agent_message_send_insert AFTER INSERT ON agent_message_sends BEGIN
    INSERT INTO agent_message_days VALUES((NEW.sent_ms / 86400000)*86400000,NEW.recipient,1)
    ON CONFLICT(day_ms,recipient) DO UPDATE SET count=count+1;
END;
CREATE TRIGGER IF NOT EXISTS agent_message_send_delete AFTER DELETE ON agent_message_sends BEGIN
    UPDATE agent_message_days SET count=count-1
    WHERE day_ms=(OLD.sent_ms / 86400000)*86400000 AND recipient=OLD.recipient;
    DELETE FROM agent_message_days
    WHERE day_ms=(OLD.sent_ms / 86400000)*86400000 AND recipient=OLD.recipient AND count=0;
END;
"#;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h07fcf2b8b85c owner=st3_store::store::agent_messages TODO=trace-statement-and-value-representation */ SCHEMA)?;
    Ok(())
}

pub fn ready(transaction: &Transaction<'_>) -> Result<bool> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='agent_message_days_v1')",
        [],
        |row| row.get(0),
    )?)
}

// Opening the daemon never pays for the optional usage estimate's historical backfill, but
// coordination metadata still bootstraps in bounded pages.
pub fn open(transaction: &Transaction<'_>) -> Result<()> {
    if ready(transaction)? {
        flush_with_coordination(transaction, true, true)?;
    }
    coordination::initialize(transaction)?;
    // A corrupt historical body must leave counts incomplete without preventing startup.
    // Roll back this entire bounded page, including any counts preceding the bad body.
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-ha567eaf1f68b owner=st3_store::store::agent_messages TODO=trace-statement-and-value-representation */ "SAVEPOINT coordination_startup")?;
    if let Err(error) = coordination::backfill(transaction) {
        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h7ab404618ea0 owner=st3_store::store::agent_messages TODO=trace-statement-and-value-representation */ "ROLLBACK TO coordination_startup")?;
        eprintln!("st3: WARN coordination bootstrap remains incomplete: {error}");
    }
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-hf3a17772f919 owner=st3_store::store::agent_messages TODO=trace-statement-and-value-representation */ "RELEASE coordination_startup")?;
    Ok(())
}

pub fn eligible(fields: &Value) -> bool {
    let from = fields["from"].as_str().unwrap_or_default();
    let to = fields["to"].as_str().unwrap_or_default();
    from.starts_with("agent/") && to.starts_with("agent/") && !synthetic(fields)
}

pub fn synthetic(fields: &Value) -> bool {
    let from = fields["from"].as_str().unwrap_or_default();
    let to = fields["to"].as_str().unwrap_or_default();
    if [from, to]
        .iter()
        .any(|seat| seat.contains("delivery-probe/") || seat.contains("delivery-soak"))
    {
        return true;
    }
    if fields["tags"].as_array().is_some_and(|tags| {
        tags.iter().any(|tag| {
            matches!(
                tag.as_str(),
                Some(
                    "delivery-probe"
                        | "delivery-test"
                        | "channel-check"
                        | "test"
                        | "soak"
                        | "canary"
                )
            )
        })
    }) {
        return true;
    }
    let title = fields["title"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut title = title.trim();
    while let Some(rest) = title.strip_prefix("re:") {
        title = rest.trim_start();
    }
    [
        "soak request",
        "soak reply",
        "channel check",
        "delivery test",
        "delivery probe",
        "canary reply",
    ]
    .iter()
    .any(|prefix| {
        title.strip_prefix(prefix).is_some_and(|rest| {
            rest.is_empty() || rest.starts_with(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        })
    })
}

pub fn flush(transaction: &Transaction<'_>) -> Result<()> {
    // Before the usage backfill completes, sends still sync coordination metadata. Day counts
    // follow only the sends the backfill has passed; it counts the rest when it reaches them.
    let counts = if ready(transaction)? {
        SendCounts::All
    } else {
        backfill_through(transaction)?.map_or(SendCounts::None, SendCounts::Through)
    };
    let synced = flush_counting(transaction, true, counts)?;
    coordination::cover_synced_sends(transaction, &synced)
}

/// Which flushed subjects have their sends counted for the usage estimate.
#[derive(Clone, Copy)]
pub enum SendCounts {
    None,
    All,
    /// While the backfill runs: a subject it has counted, or one with a send at or below this
    /// `store_index`, which it has passed.
    Through(u64),
}

/// Returns the subjects it flushed.
pub fn flush_with_coordination(
    transaction: &Transaction<'_>,
    coordination_live: bool,
    count_sends: bool,
) -> Result<BTreeSet<String>> {
    flush_counting(transaction, coordination_live, if count_sends { SendCounts::All } else { SendCounts::None })
}

/// Returns the subjects it flushed.
pub fn flush_counting(
    transaction: &Transaction<'_>,
    coordination_live: bool,
    counts: SendCounts,
) -> Result<BTreeSet<String>> {
    let pending = transaction
        .prepare_cached("SELECT subject FROM local_agent_message_pending")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    for subject in &pending {
        sync_subject(transaction, subject, coordination_live, counts)?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h7dc294f64b3e owner=st3_store::store::agent_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_agent_message_pending WHERE subject=?1",
            [&subject],
        )?;
    }
    Ok(pending)
}

fn sync_subject(
    transaction: &Transaction<'_>,
    subject: &str,
    coordination_live: bool,
    counts: SendCounts,
) -> Result<()> {
    // Earliest send time is stable across replication order and duplicate send claims.
    let sent: Option<(u64, String)> = transaction
        .query_row(
            "SELECT CAST(accepted_at_unix_ms AS INTEGER),body FROM claims
         WHERE subject=?1 AND kind='message.sent'
         ORDER BY CAST(accepted_at_unix_ms AS INTEGER),id LIMIT 1",
            [subject],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if coordination_live && sent.is_none() {
        coordination::sync(transaction, subject, None)?;
    }
    let desired = sent
        .map(|(at, body)| -> Result<_> {
            let body: Value = serde_json::from_str(&body)?;
            if coordination_live {
                coordination::sync(transaction, subject, Some((at, &body["fields"])))?;
            }
            Ok(eligible(&body["fields"])
                .then(|| (at, body["fields"]["to"].as_str().unwrap().to_owned())))
        })
        .transpose()?
        .flatten();
    let current: Option<(u64, String)> = transaction
        .query_row(
            "SELECT sent_ms,recipient FROM agent_message_sends WHERE subject=?1",
            [subject],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let count = match counts {
        SendCounts::None => false,
        SendCounts::All => true,
        SendCounts::Through(through) => current.is_some() || transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='message.sent' AND store_index<=?2)",
            params![subject, through],
            |row| row.get(0),
        )?,
    };
    if count && current != desired {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hf5d965fad59a owner=st3_store::store::agent_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM agent_message_sends WHERE subject=?1",
            [subject],
        )?;
        if let Some((at, recipient)) = desired {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-hc00cfd430db8 owner=st3_store::store::agent_messages TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO agent_message_sends VALUES(?1,?2,?3)",
                params![subject, at, recipient],
            )?;
        }
    }
    Ok(())
}

/// While the usage backfill runs, the `store_index` of the last `message.sent` claim it read.
const BACKFILL_THROUGH: &str = "agent_message_days_v1_through";

/// The most `message.sent` claims one background backfill page reads. A page of 500 holds the
/// writer for about 20 ms on the load test's busy-host store (41 us a send).
pub const BACKFILL_PAGE: usize = 500;

fn backfill_through(transaction: &Transaction<'_>) -> Result<Option<u64>> {
    Ok(transaction
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",
            [BACKFILL_THROUGH],
            |row| row.get(0),
        )
        .optional()?)
}

/// One page of the usage backfill: count the subjects of at most `limit` `message.sent` claims
/// past the last page, in `store_index` order, and mark the counts ready after the last page.
/// Returns whether they are ready. The daemon runs it in the background, one page per writer
/// turn, so neither opening the store nor the first estimate rebuilds every retained send; a
/// cursor in `meta` carries it across restarts.
pub fn backfill_page(transaction: &Transaction<'_>, limit: usize) -> Result<bool> {
    if ready(transaction)? {
        return Ok(true);
    }
    let through = match backfill_through(transaction)? {
        Some(through) => through,
        None => {
            transaction.execute_batch(
                /* sqlite-large-value: allow-for-now lv-h92818f79cda5 owner=st3_store::store::agent_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM agent_message_sends;
                 DELETE FROM agent_message_days;",
            )?;
            0
        }
    };
    let claims = transaction
        .prepare_cached(
            "SELECT subject,store_index FROM claims INDEXED BY claims_kind_index
             WHERE kind='message.sent' AND store_index>?1 ORDER BY store_index LIMIT ?2",
        )?
        .query_map(params![through, i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // The historical pass counts sends only; coordination metadata bootstraps on its own.
    for subject in claims.iter().map(|(subject, _)| subject.as_str()).collect::<BTreeSet<_>>() {
        sync_subject(transaction, subject, false, SendCounts::All)?;
    }
    if claims.len() < limit {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h9948e4b91d26 owner=st3_store::store::agent_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1",
            [BACKFILL_THROUGH],
        )?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h25642ab8e771 owner=st3_store::store::agent_messages TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('agent_message_days_v1','1')",
            [],
        )?;
        return Ok(true);
    }
    let through = claims.last().map_or(through, |(_, index)| *index);
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-hd8dd65dbc13a owner=st3_store::store::agent_messages TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![BACKFILL_THROUGH, through.to_string()],
    )?;
    Ok(false)
}
