//! Count-only coordination reads. Writers populate metadata; readers never scan claim bodies.
use super::*;

const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h8e049182dd1c owner=fractal_store::store::coordination TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS coordination_sends (
    subject TEXT PRIMARY KEY,
    sent_ms INTEGER NOT NULL,
    agent_to_agent INTEGER NOT NULL,
    silent INTEGER NOT NULL,
    to_person INTEGER NOT NULL,
    held INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_coordination_backfill (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    cursor INTEGER NOT NULL,
    ceiling INTEGER NOT NULL,
    complete INTEGER NOT NULL,
    progress_ms INTEGER NOT NULL DEFAULT 0
);
"#;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hf3b6b997731a owner=fractal_store::store::coordination TODO=trace-statement-and-value-representation */ SCHEMA)?;
    let columns = |table: &str| -> Result<Vec<String>> {
        Ok(connection
            .prepare(/* sqlite-large-value: allow-for-now lv-hd000d5b0745a owner=fractal_store::store::coordination TODO=trace-statement-and-value-representation */ &format!("PRAGMA table_info({table})"))?
            .query_map([], |row| row.get(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    };
    let existing = columns("coordination_sends")?;
    if existing.iter().any(|name| name == "fyi") && !existing.iter().any(|name| name == "silent") {
        // SQLite rewrites the covering index too; canonical message claims stay immutable.
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h0e190a40f840 owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE coordination_sends RENAME COLUMN fyi TO silent;")?;
    }
    if !columns("coordination_sends")?
        .iter()
        .any(|name| name == "held")
    {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h563c3f682af9 owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE coordination_sends ADD COLUMN held INTEGER NOT NULL DEFAULT 0;
             UPDATE local_coordination_backfill SET cursor=0,ceiling=0,complete=0;",
        )?;
    }
    if !columns("local_coordination_backfill")?
        .iter()
        .any(|name| name == "progress_ms")
    {
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h98db3109022b owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE local_coordination_backfill ADD COLUMN progress_ms INTEGER NOT NULL DEFAULT 0;")?;
    }
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hbbba4668bfe8 owner=fractal_store::store::coordination TODO=chunk-or-reference-variable-columns */ "CREATE INDEX IF NOT EXISTS coordination_sends_time ON coordination_sends(sent_ms,agent_to_agent,silent,to_person);
         CREATE INDEX IF NOT EXISTS coordination_sends_held ON coordination_sends(held,sent_ms);
         CREATE TRIGGER IF NOT EXISTS coordination_offer AFTER INSERT ON claims
         WHEN NEW.kind IN ('message.staged','message.delivered','message.read','message.closed')
         BEGIN
             UPDATE coordination_sends SET held=0 WHERE subject=NEW.subject AND held!=0;
         END;",
    )?;
    Ok(())
}

pub fn initialize(transaction: &Transaction<'_>) -> Result<()> {
    let maximum: u64 = transaction.query_row(
        "SELECT COALESCE((SELECT store_index FROM claims INDEXED BY claims_kind_index
         WHERE kind='message.sent' ORDER BY store_index DESC LIMIT 1),0)",
        [],
        |row| row.get(0),
    )?;
    let current: Option<(u64, bool)> = transaction
        .query_row(
            "SELECT ceiling,complete FROM local_coordination_backfill WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let now = u64::try_from(now_ms()).unwrap_or(i64::MAX as u64);
    match current {
        None => {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h993f1953ffb4 owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_coordination_backfill VALUES(1,0,?1,0,?2)",
                params![maximum, now],
            )?;
        }
        Some((ceiling, complete)) if ceiling < maximum => {
            // Old binaries do not populate this cache or raise the ceiling. Recheck the
            // uncovered tail on every reopen, without rebuilding or dropping existing count
            // metadata. Sends this binary synced live are already under the ceiling.
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h76248ca09d70 owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_coordination_backfill SET cursor=CASE WHEN ?4 THEN ?1 ELSE cursor END,ceiling=?2,complete=0,progress_ms=?3 WHERE singleton=1", params![ceiling,maximum,now,complete])?;
        }
        _ => {}
    }
    Ok(())
}

/// The live projection syncs each send in the transaction that writes it. When the sends after
/// a complete bootstrap's ceiling are all ones it just synced, the ceiling covers them too, so a
/// reopen rechecks only sends an older binary wrote rather than every send since the last
/// start. It reads at most one more send than it synced and otherwise leaves the ceiling alone.
pub fn cover_synced_sends(
    transaction: &Transaction<'_>,
    synced: &BTreeSet<String>,
) -> Result<()> {
    if synced.is_empty() {
        return Ok(());
    }
    let (ceiling, complete): (u64, bool) = transaction.query_row(
        "SELECT ceiling,complete FROM local_coordination_backfill WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !complete {
        return Ok(());
    }
    let after = transaction
        .prepare_cached(
            "SELECT store_index,subject FROM claims INDEXED BY claims_kind_index
             WHERE kind='message.sent' AND store_index>?1 ORDER BY store_index LIMIT ?2",
        )?
        .query_map(params![ceiling, synced.len() + 1], |row| {
            Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if after.len() > synced.len() || !after.iter().all(|(_, subject)| synced.contains(subject)) {
        return Ok(());
    }
    if let Some((latest, _)) = after.last() {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h3551d663abcb owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_coordination_backfill SET ceiling=?1 WHERE singleton=1",
            [latest],
        )?;
    }
    Ok(())
}

pub fn sync(
    transaction: &Transaction<'_>,
    subject: &str,
    sent: Option<(u64, &Value)>,
) -> Result<()> {
    let Some((at, fields)) = sent else {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h61b124217492 owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM coordination_sends WHERE subject=?1", [subject])?;
        return Ok(());
    };
    let from = fields["from"].as_str().unwrap_or_default();
    let to = fields["to"].as_str().unwrap_or_default();
    let agent = from.starts_with("agent/") && !agent_messages::synthetic(fields);
    let agent_to_agent = agent && to.starts_with("agent/");
    let to_person = agent && to.starts_with("person/");
    let tags = fields["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let tagged_held = tags.iter().any(|tag| crate::silent::is_silent_tag(tag))
        && !crate::silent::always_wakes(from, &tags);
    let silent = agent_to_agent && tagged_held;
    let held = tagged_held
        && transaction.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM claims WHERE subject=?1
         AND kind IN ('message.staged','message.delivered','message.read','message.closed'))",
            [subject],
            |row| row.get::<_, bool>(0),
        )?;
    if !agent_to_agent && !to_person && !held {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h132a4364f8f9 owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM coordination_sends WHERE subject=?1", [subject])?;
        return Ok(());
    }
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h2d5a526c7566 owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO coordination_sends VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(subject) DO UPDATE SET sent_ms=excluded.sent_ms,
             agent_to_agent=excluded.agent_to_agent, silent=excluded.silent, to_person=excluded.to_person,
             held=excluded.held
         WHERE sent_ms!=excluded.sent_ms OR agent_to_agent!=excluded.agent_to_agent
             OR silent!=excluded.silent OR to_person!=excluded.to_person OR held!=excluded.held",
        params![subject, at, agent_to_agent, silent, to_person, held],
    )?;
    Ok(())
}

/// At most eight historical subjects per writer batch. No startup scan or read-side build.
/// Live pending sends are maintained by the existing send projection, including replication.
pub fn backfill(transaction: &Transaction<'_>) -> Result<bool> {
    let started = std::time::Instant::now();
    let (cursor, ceiling, complete): (u64, u64, bool) = transaction.query_row(
        "SELECT cursor,ceiling,complete FROM local_coordination_backfill WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if complete {
        return Ok(true);
    }
    let batch = transaction.prepare_cached(
        "SELECT store_index,subject FROM claims INDEXED BY claims_kind_index
         WHERE kind='message.sent' AND store_index>?1 AND store_index<=?2 ORDER BY store_index LIMIT 8",
    )?.query_map(params![cursor, ceiling], |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut processed: usize = 0;
    for (_, subject) in &batch {
        // Leave room for the rest of the FIFO writer batch under its 100 ms hold limit.
        if processed > 0 && started.elapsed() >= std::time::Duration::from_millis(10) {
            break;
        }
        let sent: Option<(u64, String)> = transaction
            .query_row(
                "SELECT CAST(accepted_at_unix_ms AS INTEGER),body FROM claims
             WHERE subject=?1 AND kind='message.sent'
             ORDER BY CAST(accepted_at_unix_ms AS INTEGER),id LIMIT 1",
                [subject],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let body = sent
            .as_ref()
            .map(|(_, body)| serde_json::from_str::<Value>(body))
            .transpose()?;
        sync(
            transaction,
            subject,
            sent.as_ref()
                .zip(body.as_ref())
                .map(|((at, _), body)| (*at, &body["fields"])),
        )?;
        processed += 1;
    }
    let complete = processed == batch.len() && batch.len() < 8;
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-hf2f53c36fcbf owner=fractal_store::store::coordination TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_coordination_backfill SET cursor=?1,complete=?2,progress_ms=?3 WHERE singleton=1",
        params![
            batch
                .get(processed.wrapping_sub(1))
                .map_or(ceiling, |(index, _)| *index),
            complete,
            u64::try_from(now_ms()).unwrap_or(i64::MAX as u64)
        ],
    )?;
    Ok(complete)
}
