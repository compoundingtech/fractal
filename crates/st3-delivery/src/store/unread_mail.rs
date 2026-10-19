//! The part of `store::unread_mail` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_store::store::unread_mail::*;
pub use st3_store::store::unread_mail::{create_schema};

const COUNT_SQL: &str = "WITH levels(shift) AS (VALUES(56),(48),(40),(32),(24),(16),(8),(0))
         SELECT COALESCE(SUM((
             SELECT SUM(count) FROM unread_mail_prefixes
             WHERE shift=levels.shift AND prefix>=((?1 >> (levels.shift+8)) << 8)
             AND prefix<(?1 >> levels.shift)
         )),0) FROM levels";

fn flush_pending(connection: &Connection) -> Result<()> {
    let pending = connection
        .prepare_cached("SELECT subject FROM unread_mail_pending")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if pending.is_empty() {
        return Ok(());
    }
    let mut remove = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h2cd264fcfb33 owner=st3_store::store::unread_mail TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM unread_mail WHERE subject=?1")?;
    let mut insert = connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h6960a6f7e0e0 owner=st3_store::store::unread_mail TODO=prepare-body-and-predecessor-values */ "INSERT INTO unread_mail(subject,sent_time)
         SELECT ?1,MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims
         WHERE subject=?1 AND kind='message.sent'
         AND NOT EXISTS (
             SELECT 1 FROM claims WHERE subject=?1
             AND kind IN ('message.read','message.closed')
         ) HAVING COUNT(*)>0",
    )?;
    let mut clear =
        connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h3e57aaaaedbc owner=st3_store::store::unread_mail TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM unread_mail_pending WHERE subject=?1")?;
    for subject in pending {
        remove.execute(/* sqlite-large-value: allow-for-now lv-h55f92542a852 owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ [&subject])?;
        insert.execute(/* sqlite-large-value: allow-for-now lv-hb78ed5e888f8 owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ [&subject])?;
        clear.execute(/* sqlite-large-value: allow-for-now lv-h0941578fae94 owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ [&subject])?;
    }
    Ok(())
}

/// A pinned diagnostic snapshot cannot write. Apply the queued subjects' exact delta
/// to that snapshot's cached count without changing either the snapshot or the queue.
pub fn count_in_snapshot(connection: &Connection, before_unix_ms: u128) -> Result<u64> {
    let before = i64::try_from(before_unix_ms).unwrap_or(i64::MAX);
    let base: i64 = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h3959d604b364 owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ COUNT_SQL)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h1b20fa4f45eb owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ [before], |row| row.get(0))?;
    let delta: i64 = connection
        .prepare_cached(
            "SELECT COALESCE(SUM(
            CASE WHEN (SELECT MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims
                       WHERE subject=pending.subject AND kind='message.sent') < ?1
                 AND NOT EXISTS (SELECT 1 FROM claims WHERE subject=pending.subject
                                 AND kind IN ('message.read','message.closed'))
                 THEN 1 ELSE 0 END
            - CASE WHEN EXISTS (SELECT 1 FROM unread_mail
                                 WHERE subject=pending.subject AND sent_time < ?1)
                   THEN 1 ELSE 0 END
         ),0) FROM unread_mail_pending pending",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h74d986140f45 owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ [before], |row| row.get(0))?;
    Ok(u64::try_from(base + delta)?)
}

/// At most 128 oldest unoffered held subjects, selected from indexed metadata.
/// Only these bounded subjects have their sender/recipient/tags read; a reader never writes.
pub struct UnreadSent {
    pub to: String,
    pub from: String,
    pub tags: Vec<String>,
    pub sent_unix_ms: u128,
}

pub fn sent_before(connection: &Connection, before_unix_ms: u128) -> Result<Vec<UnreadSent>> {
    let before = i64::try_from(before_unix_ms).unwrap_or(i64::MAX);
    let mut statement = connection.prepare_cached(
        "SELECT json_extract(sent.body,'$.fields.to'), json_extract(sent.body,'$.fields.from'),
                json_extract(sent.body,'$.fields.tags'), held.sent_ms
         FROM coordination_sends held INDEXED BY coordination_sends_held
         JOIN claims sent ON sent.id=(SELECT id FROM claims
             WHERE subject=held.subject AND kind='message.sent'
             ORDER BY CAST(accepted_at_unix_ms AS INTEGER),id LIMIT 1)
         WHERE held.held=1 AND held.sent_ms < ?1
         ORDER BY held.sent_ms,held.subject LIMIT 128",
    )?;
    let rows = statement.query_map([before], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut sent = Vec::new();
    for row in rows {
        let (to, from, tags, time) = row?;
        let tags = tags
            .and_then(|tags| serde_json::from_str::<Vec<String>>(&tags).ok())
            .unwrap_or_default();
        sent.push(UnreadSent { to, from, tags, sent_unix_ms: u128::try_from(time).unwrap_or_default() });
    }
    Ok(sent)
}

pub fn count_before(transaction: &Transaction<'_>, before_unix_ms: u128) -> Result<u64> {
    // The caller holds one writer transaction: pending changes and their exact age count
    // become visible together, and a concurrent message cannot disappear between the two.
    flush_pending(transaction)?;
    let before = i64::try_from(before_unix_ms).unwrap_or(i64::MAX);
    let mut statement = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-hcba4ed1041ae owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ COUNT_SQL)?;
    Ok(statement.query_row(/* sqlite-large-value: allow-for-now lv-h2197fceae4c4 owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ [before], |row| row.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_counts_follow_time_boundaries_terminal_history_and_backfill() {
        let store = Store::open_memory("node").unwrap();
        let writer = std::cell::Cell::new(0);
        let append = |subject: &str, kind: &str, time: u128| {
            store.set_write_clock_at(time).unwrap();
            // Populate the claim log like replication, which can arrive out of lifecycle
            // order; the normal message command correctly refuses these local transitions.
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            // Independent writers let accepted times arrive out of order without violating
            // one writer's monotonic clock (including a newer local batch).
            let origin = format!("replica-{}", writer.get());
            writer.set(writer.get() + 1);
            append_claim_record_tx(
                &transaction,
                &origin,
                subject,
                kind,
                Some("person/operator"),
                &json!({"fields":{"status":kind.strip_prefix("message.").unwrap()}}),
                &[],
                None,
            )
            .unwrap();
            transaction.commit().unwrap();
        };
        let times = [1, 255, 256, 257, 65_535, 65_536, 1_700_000_000_000];
        for (index, time) in times.iter().enumerate() {
            append(&format!("message/prefix-{index}"), "message.sent", *time);
        }
        {
            let connection = store.connection.write();
            // Delivery must not maintain eight prefix counters per sent claim. The first
            // exact count folds these queued IDs; later counts reuse the completed index.
            let prefixes: u64 = connection
                .query_row("SELECT COUNT(*) FROM unread_mail_prefixes", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let pending: u64 = connection
                .query_row("SELECT COUNT(*) FROM unread_mail_pending", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(prefixes, 0);
            assert_eq!(pending, times.len() as u64);
        }
        let check = |expected: &[u128]| {
            // Check before any ordinary count flushes the queue: both newly unread messages
            // and pending terminal transitions must be exact inside a read-only snapshot.
            assert_eq!(
                store
                    .read_snapshot(|_| store.unread_mail_count_before(u128::MAX))
                    .unwrap(),
                expected.len() as u64
            );
            for cutoff in times
                .into_iter()
                .flat_map(|time| [time - 1, time, time + 1])
                .chain([0, i64::MAX as u128, u128::MAX])
            {
                let count_in_snapshot = store
                    .read_snapshot(|_| store.unread_mail_count_before(cutoff))
                    .unwrap();
                assert_eq!(
                    count_in_snapshot,
                    expected.iter().filter(|time| **time < cutoff).count() as u64,
                    "pinned cutoff {cutoff}"
                );
                assert_eq!(
                    store.unread_mail_count_before(cutoff).unwrap(),
                    expected.iter().filter(|time| **time < cutoff).count() as u64,
                    "cutoff {cutoff}"
                );
            }
        };
        check(&times);
        append("message/prefix-0", "message.delivered", 500);
        check(&times);
        // A new canonical sent time moves its count; an older replica cannot move it back.
        append("message/prefix-0", "message.sent", 256);
        append("message/prefix-0", "message.sent", 2);
        let mut expected = times.to_vec();
        expected[0] = 256;
        check(&expected);
        append("message/prefix-1", "message.read", 600);
        append("message/prefix-2", "message.closed", 600);
        append("message/prefix-1", "message.sent", 700);
        expected.remove(2);
        expected.remove(1);
        check(&expected);
        // An out-of-order terminal claim also protects a later arriving sent claim.
        append("message/terminal-first", "message.read", 800);
        append("message/terminal-first", "message.sent", 1);
        check(&expected);
        {
            let connection = store.connection.write();
            connection
                .execute_batch(
                    "DELETE FROM unread_mail;
                     DELETE FROM meta WHERE key='unread_mail_prefixes';",
                )
                .unwrap();
            create_schema(&connection).unwrap();
        }
        check(&expected);
        // Removing a terminal claim restores the remaining sent time during checkpoint trim.
        store
            .connection
            .write()
            .execute(
                "DELETE FROM claims WHERE subject='message/prefix-2' AND kind='message.closed'",
                [],
            )
            .unwrap();
        expected.push(256);
        check(&expected);
    }

    #[test]
    fn age_count_work_is_bounded_for_fresh_and_aged_mail() {
        let store = Store::open_memory("node").unwrap();
        let mut connection = store.connection.write();
        let mut insert = connection
            .prepare("INSERT INTO unread_mail(subject,sent_time) VALUES(?1,?2)")
            .unwrap();
        let cutoff = 1_700_000_000_000_i64;
        // Spread timestamps across the boundary and every low-byte prefix. The count must
        // stay bounded even when almost all messages fall into the cutoff's one hour.
        for index in 0..10_000_i64 {
            insert
                .execute(params![
                    format!("message/scale-{index}"),
                    cutoff + index - 5_000
                ])
                .unwrap();
        }
        drop(insert);
        for (before, expected) in [
            (cutoff - 10_000, 0),
            (cutoff, 5_000),
            (cutoff + 10_000, 10_000),
        ] {
            let transaction = connection.transaction().unwrap();
            assert_eq!(
                count_before(&transaction, before as u128).unwrap(),
                expected
            );
            transaction.commit().unwrap();
        }
        let mut statement = connection.prepare_cached(COUNT_SQL).unwrap();
        let count: u64 = statement.query_row([cutoff], |row| row.get(0)).unwrap();
        assert_eq!(count, 5_000);
        assert!(statement.get_status(rusqlite::StatementStatus::VmStep) < 4_000);
        assert!(statement.get_status(rusqlite::StatementStatus::FullscanStep) <= 7);
    }
}
