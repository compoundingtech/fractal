//! What a mailbox stream reads of the messages shard. Neither a stream item nor this member's
//! lifecycle for it is a graph claim, so the graph's cursors, wakes and canonical keys miss
//! them; these reads stand in for them. A member without a shard reads nothing here.
use super::*;
use std::cmp::Ordering;

/// Where the shard stands for a mailbox stream: its newest admitted message and its newest
/// local lifecycle record. Both only grow, so a position has seen all an earlier one saw.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamPosition {
    pub admitted: u64,
    pub lifecycle: i64,
}

/// Whether a message view is a stream item: its index is past every graph index.
pub fn is_stream_item(message: &MessageView) -> bool {
    message.created_index >= STREAM_INDEX_BASE
}

/// Mail in canonical order by `keys`, the graph's mail first. A stream item has no graph claim
/// to key it, so it follows in shard order, as a page lists it.
pub fn mail_order<K: Ord>(keys: &BTreeMap<String, K>, left: &MessageView, right: &MessageView) -> Ordering {
    match (keys.get(&left.subject), keys.get(&right.subject)) {
        (Some(left), Some(right)) => left.cmp(right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => (left.created_index, &left.subject).cmp(&(right.created_index, &right.subject)),
    }
}

/// The shard's current position; the start, without a shard.
pub fn position(graph: &crate::store::Store) -> anyhow::Result<StreamPosition> {
    let Some(shard) = shard(graph) else {
        return Ok(StreamPosition::default());
    };
    let connection = shard.readers.get();
    let mut statement = connection.prepare_cached(
        "SELECT COALESCE((SELECT MAX(store_index) FROM stream_messages),0),
                COALESCE((SELECT MAX(rowid) FROM stream_lifecycle),0)",
    )?;
    Ok(statement.query_row(/* sqlite-large-value: allow-for-now lv-h73c63bdcea91 owner=fractal_store::store::messages_shard::mailbox_reads TODO=trace-statement-and-value-representation */ [], |row| {
        Ok(StreamPosition { admitted: row.get(0)?, lifecycle: row.get(1)? })
    })?)
}

/// The stream items to `recipient` that this member has neither read nor closed, in shard
/// order. A delivery snapshot lists them after the graph's mail, which it orders canonically.
pub fn delivery_items(graph: &crate::store::Store, recipient: &str) -> anyhow::Result<Vec<MessageView>> {
    let Some(shard) = shard(graph) else {
        return Ok(Vec::new());
    };
    let (recipient, names) = {
        let connection = graph.readers.get();
        let recipient = crate::store::identities::resolve_reference_conn(
            &connection,
            &crate::store::normalize_message_party(recipient),
        )?;
        let bare = crate::store::message_bare_recipient(&recipient).to_owned();
        let names = super::super::delivery::recipient_names(&connection, &recipient, &bare)?;
        (recipient, names)
    };
    let subjects = {
        let connection = shard.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject FROM stream_messages
             WHERE recipient IN (SELECT value FROM json_each(?1))
               AND NOT EXISTS(SELECT 1 FROM stream_lifecycle lifecycle
                   WHERE lifecycle.subject=stream_messages.subject
                     AND lifecycle.kind IN ('message.read','message.closed'))
             ORDER BY store_index",
        )?;
        statement
            .query_map([serde_json::to_string(&names)?], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut items = Vec::new();
    for subject in subjects {
        if let Some(message) = message_view(graph, &subject)?
            && message.to == recipient
        {
            items.push(message);
        }
    }
    Ok(items)
}

/// The stream items whose mailbox changed after `after`: those newly sent to `recipient`, and
/// those whose local lifecycle moved and that are sent to it or already in its `messages`.
pub fn changed_subjects(
    graph: &crate::store::Store,
    recipient: &str,
    bare_recipient: &str,
    messages: &[String],
    after: StreamPosition,
) -> anyhow::Result<Vec<String>> {
    let Some(shard) = shard(graph) else {
        return Ok(Vec::new());
    };
    let names = super::super::delivery::recipient_names(&graph.readers.get(), recipient, bare_recipient)?;
    let connection = shard.readers.get();
    let mut statement = connection.prepare_cached(
        "SELECT subject FROM stream_messages
         WHERE recipient IN (SELECT value FROM json_each(?1)) AND store_index>?2
         UNION
         SELECT lifecycle.subject FROM stream_lifecycle lifecycle
         WHERE lifecycle.rowid>?3
           AND (lifecycle.subject IN (SELECT value FROM json_each(?4))
             OR EXISTS(SELECT 1 FROM stream_messages sent WHERE sent.subject=lifecycle.subject
                 AND sent.recipient IN (SELECT value FROM json_each(?1))))",
    )?;
    let subjects = statement
        .query_map(
            rusqlite::params![
                serde_json::to_string(&names)?,
                after.admitted,
                after.lifecycle,
                serde_json::to_string(messages)?
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(subjects)
}

/// Stream items to wake, as (subject, recipient), with the position they reach and whether
/// more remain.
pub type WakeBatch = (StreamPosition, Vec<(String, String)>, bool);

/// At most `limit` stream items admitted, and lifecycle records made, after `after`. The
/// mailbox wake dispatcher routes these, as it routes the graph's new claims.
pub fn wake_batch(
    graph: &crate::store::Store,
    after: StreamPosition,
    limit: usize,
) -> anyhow::Result<WakeBatch> {
    let Some(shard) = shard(graph) else {
        return Ok((after, Vec::new(), false));
    };
    let connection = shard.readers.get();
    let mut next = after;
    let mut changed = Vec::new();
    let admitted = connection
        .prepare_cached(
            "SELECT store_index, subject, recipient FROM stream_messages
             WHERE store_index>?1 ORDER BY store_index LIMIT ?2",
        )?
        .query_map(rusqlite::params![after.admitted, limit], |row| {
            Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut full = admitted.len() == limit;
    for (index, subject, recipient) in admitted {
        next.admitted = index;
        changed.push((subject, recipient));
    }
    let steps = connection
        .prepare_cached(
            "SELECT lifecycle.rowid, lifecycle.subject, COALESCE(sent.recipient,'')
             FROM stream_lifecycle lifecycle
             LEFT JOIN stream_messages sent ON sent.subject=lifecycle.subject
             WHERE lifecycle.rowid>?1 ORDER BY lifecycle.rowid LIMIT ?2",
        )?
        .query_map(rusqlite::params![after.lifecycle, limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    full |= steps.len() == limit;
    for (rowid, subject, recipient) in steps {
        next.lifecycle = rowid;
        changed.push((subject, recipient));
    }
    Ok((next, changed, full))
}

/// A stream item's `message.sent` claim, which carries the time it was accepted.
pub fn sent_claim(graph: &crate::store::Store, subject: &str) -> anyhow::Result<Option<ClaimRecord>> {
    let Some(shard) = shard(graph) else {
        return Ok(None);
    };
    Ok(stream_item(&shard, subject)?.map(|(claim, _)| claim))
}

/// This member's `kind` step for a stream item, as (actor, recorded at), if it took it.
pub fn lifecycle_step(
    graph: &crate::store::Store,
    subject: &str,
    kind: &str,
) -> anyhow::Result<Option<(String, u128)>> {
    let Some(shard) = shard(graph) else {
        return Ok(None);
    };
    let connection = shard.readers.get();
    let mut statement = connection.prepare_cached(
        "SELECT actor, recorded_at_unix_ms FROM stream_lifecycle
         WHERE subject=?1 AND kind=?2 ORDER BY recorded_at_unix_ms DESC LIMIT 1",
    )?;
    Ok(statement
        .query_row(/* sqlite-large-value: allow-for-now lv-hf9196992189f owner=fractal_store::store::messages_shard::mailbox_reads TODO=trace-statement-and-value-representation */ [subject, kind], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .optional()?
        .map(|(actor, at)| (actor, u128::try_from(at).unwrap_or_default())))
}
