//! The messages stream as the graph's readers need it beside their own reads: how far the stream
//! has come, its message events, and where each stream item arrived in this member's graph order.

use super::*;

/// Where each stream item arrived in this member's graph order: a local table in the shard file,
/// outside the replicated log and every digest. One host-local order then sorts graph and stream
/// entries together. A shard trigger cannot see the graph's index, so the daemon writes it as it
/// admits items, and a reader that finds an item without one writes it then.
///
/// `shown_at_unix_ms` is the time a timeline shows for the item: its accepted time, or one
/// millisecond past the item recorded before it at the same graph index, whichever is later.
/// Items that share a graph index share timeline sequences, so a shared millisecond would sort
/// both message entries before both content entries; a client that pairs a message with the
/// content entry that follows it, as phones already installed do, would then pair them wrongly.
pub(super) const LOCAL_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-hfdd2de5042cd owner=fractal_store::store::messages_shard::stream_reads TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS stream_arrivals (
    subject TEXT PRIMARY KEY,
    store_index INTEGER NOT NULL,
    graph_index INTEGER NOT NULL,
    arrived_at_unix_ms INTEGER NOT NULL,
    shown_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS stream_arrivals_store_index ON stream_arrivals(store_index);
CREATE INDEX IF NOT EXISTS stream_arrivals_shown ON stream_arrivals(graph_index, shown_at_unix_ms);
";

/// The stream's store index on this member, or `None` when it holds no messages shard.
pub fn stream_index(graph: &super::super::Store) -> anyhow::Result<Option<u64>> {
    shard(graph).map(|shard| shard.index()).transpose()
}

/// Record, at the graph's current index, every stream item that has no arrival yet. The daemon
/// calls it after the shard admits items; a reader that finds an item without one calls it too.
/// Returns how many items it recorded. Each takes its shown time here, once, in store order.
pub fn record_arrivals(graph: &super::super::Store) -> anyhow::Result<usize> {
    let Some(shard) = shard(graph) else {
        return Ok(0);
    };
    let at = graph.index()?;
    let mut connection = shard.connection.write();
    let transaction = connection.transaction()?;
    let unrecorded = transaction
        .prepare_cached(
            "SELECT stream_messages.subject, stream_messages.store_index, claims.accepted_at_unix_ms
             FROM stream_messages JOIN claims ON claims.store_index=stream_messages.store_index
             WHERE stream_messages.store_index > COALESCE((SELECT MAX(store_index) FROM stream_arrivals), 0)
             ORDER BY stream_messages.store_index",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut shown = transaction
        .prepare_cached("SELECT MAX(shown_at_unix_ms) FROM stream_arrivals WHERE graph_index=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h7d4a5fcca7d0 owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ rusqlite::params![at as i64], |row| row.get::<_, Option<i64>>(0))?;
    let arrived = smallclaims::store::now_ms() as i64;
    for (subject, store_index, accepted) in &unrecorded {
        // An accepted time that does not parse shows when the item arrived, not at 1970.
        let accepted = accepted.parse::<i64>().unwrap_or(arrived);
        let at_ms = shown.map_or(accepted, |before| accepted.max(before.saturating_add(1)));
        transaction.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h0c4f7d0d1e18 owner=fractal_store::store::messages_shard::stream_reads TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO stream_arrivals(subject, store_index, graph_index, arrived_at_unix_ms, shown_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?.execute(/* sqlite-large-value: allow-for-now lv-hd2347a335f03 owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ rusqlite::params![subject, store_index, at as i64, arrived, at_ms])?;
        shown = Some(at_ms);
    }
    transaction.commit()?;
    Ok(unrecorded.len())
}

/// The graph index at which `subject`, a stream item, arrived on this member, as recorded. It
/// reads only: a read path that must see every item recorded calls `arrivals_current` first.
fn recorded_arrival(shard: &GraphStore, subject: &str) -> anyhow::Result<Option<u64>> {
    Ok(shard
        .readers
        .get()
        .query_row(
            "SELECT graph_index FROM stream_arrivals WHERE subject=?1",
            [subject],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map(|index| index.max(0) as u64))
}

/// The graph index at which `subject`, a stream item, arrived on this member, recording the
/// shard's unrecorded items first when it has none.
pub fn arrival(graph: &super::super::Store, subject: &str) -> anyhow::Result<Option<u64>> {
    let Some(shard) = shard(graph) else {
        return Ok(None);
    };
    if let Some(index) = recorded_arrival(&shard, subject)? {
        return Ok(Some(index));
    }
    if stream_item(&shard, subject)?.is_none() {
        return Ok(None);
    }
    record_arrivals(graph)?;
    recorded_arrival(&shard, subject)
}

/// One page of the stream's message events after `after`, in stream order, through the stream's
/// published index. `None` when this member holds no messages shard. Every claim the shard
/// admits is a stream item, and the shard keeps no event positions of its own, so its claims in
/// store order are its events.
pub struct StreamEvents {
    pub items: Vec<crate::model::EventRecord>,
    pub next_after: Option<u64>,
    pub has_more: bool,
    pub frontier: u64,
}

pub fn stream_events_page(
    graph: &super::super::Store,
    after: u64,
    limit: usize,
) -> anyhow::Result<Option<StreamEvents>> {
    let Some(shard) = shard(graph) else {
        return Ok(None);
    };
    let limit = limit.clamp(1, 200);
    let through = shard.index()?;
    let connection = shard.readers.get();
    let raw = connection
        .prepare_cached(
            "SELECT store_index, kind, subject, body FROM claims
             WHERE store_index > ?1 AND store_index <= ?3 AND kind='message.sent'
             ORDER BY store_index LIMIT ?2",
        )?
        .query_map(rusqlite::params![after, limit.saturating_add(1), through], |row| {
            let body = row.get::<_, String>(3)?;
            Ok(crate::model::EventRecord {
                store_index: row.get(0)?,
                kind: row.get(1)?,
                subject: row.get(2)?,
                body: serde_json::from_str(&body).unwrap_or(Value::Null),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = raw.len() > limit;
    let items: Vec<_> = raw.into_iter().take(limit).collect();
    Ok(Some(StreamEvents {
        next_after: items.last().map(|event| event.store_index),
        items,
        has_more,
        frontier: through,
    }))
}

/// Record arrivals only when the shard holds an item that has none, so a reader that finds
/// every item recorded writes nothing.
fn arrivals_current(graph: &super::super::Store, shard: &GraphStore) -> anyhow::Result<()> {
    let behind: bool = shard.readers.get().prepare_cached(
        "SELECT COALESCE((SELECT MAX(store_index) FROM stream_messages),0)
              > COALESCE((SELECT MAX(store_index) FROM stream_arrivals),0)",
    )?.query_row(/* sqlite-large-value: allow-for-now lv-h1ef1fcd0470b owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?;
    if behind {
        record_arrivals(graph)?;
    }
    Ok(())
}

/// Which stream items a conversation read takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamAfter {
    /// Those past this index in the shard, the position a conversation cursor carries.
    Position(u64),
    /// Those that arrived on this member after this graph index.
    Arrival(u64),
}

/// The newest stream item this member has admitted: where a conversation cursor stands in the
/// stream. The start, without a shard.
pub fn conversation_position(graph: &super::super::Store) -> anyhow::Result<u64> {
    let Some(shard) = shard(graph) else {
        return Ok(0);
    };
    Ok(shard.readers.get().prepare_cached(
        "SELECT COALESCE(MAX(store_index),0) FROM stream_messages",
    )?.query_row(/* sqlite-large-value: allow-for-now lv-hcf0a762ac4ce owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?)
}

/// The stream items to or from any of `names` after `after`, newest first, each with the graph
/// index it arrived at. Its claim's `store_index` is its `created_index`, past every graph
/// index, as a message page lists it, and its `accepted_at_unix_ms` is the time its timeline
/// entries show (`stream_arrivals.shown_at_unix_ms`), which can be a few milliseconds past the
/// time the shard accepted it. These claims are for timeline reads: nothing should compare
/// their accepted times with another store's. `limit` bounds a delta; without one the read keeps the
/// newest 10,000 stream items, as the graph's conversation read keeps its newest 10,000 sends.
pub fn conversation_items(
    graph: &super::super::Store,
    names: &[String],
    after: StreamAfter,
    limit: Option<usize>,
) -> anyhow::Result<Vec<(ClaimRecord, u64)>> {
    let Some(shard) = shard(graph) else {
        return Ok(Vec::new());
    };
    arrivals_current(graph, &shard)?;
    let connection = shard.readers.get();
    let floor = match limit {
        Some(_) => 0,
        None => connection
            .prepare_cached(
                "SELECT store_index FROM stream_messages ORDER BY store_index DESC LIMIT 1 OFFSET 9999",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h8e71fd25b754 owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ [], |row| row.get::<_, u64>(0))
            .optional()?
            .unwrap_or(0),
    };
    let (position, arrival) = match after {
        StreamAfter::Position(position) => (position.max(floor), 0),
        StreamAfter::Arrival(arrival) => (floor, arrival),
    };
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hf2c0f457d435 owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ &format!(
        "WITH selected AS (
            SELECT store_index, subject FROM stream_messages INDEXED BY stream_messages_recipient
            WHERE recipient IN (SELECT value FROM json_each(?1)) AND store_index>?2
            UNION
            SELECT store_index, subject FROM stream_messages INDEXED BY stream_messages_sender
            WHERE sender IN (SELECT value FROM json_each(?1)) AND store_index>?2
         ) SELECT {}, arrivals.graph_index, arrivals.shown_at_unix_ms FROM selected
           JOIN stream_arrivals arrivals ON arrivals.subject=selected.subject
           JOIN claims ON claims.store_index=selected.store_index
           WHERE arrivals.graph_index>?3 AND claims.kind='message.sent'
           ORDER BY selected.store_index DESC LIMIT ?4",
        smallclaims::store::CLAIM_COLUMNS
    ))?;
    let rows = statement
        .query_map(
            rusqlite::params![
                serde_json::to_string(names)?,
                position,
                arrival,
                limit.map_or(-1, |limit| i64::try_from(limit).unwrap_or(i64::MAX))
            ],
            |row| {
                let mut claim = smallclaims::store::claim_from_row(row)?;
                claim.store_index += STREAM_INDEX_BASE;
                claim.accepted_at_unix_ms = row.get::<_, i64>(11)?.max(0) as u128;
                Ok((claim, row.get::<_, i64>(10)?.max(0) as u64))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether a stream item to or from any of `names` was admitted after shard index `after`
/// through `through`. No payload leaves SQL.
pub fn conversation_changed(
    graph: &super::super::Store,
    names: &[String],
    after: u64,
    through: u64,
) -> anyhow::Result<bool> {
    let Some(shard) = shard(graph) else {
        return Ok(false);
    };
    if through <= after {
        return Ok(false);
    }
    Ok(shard.readers.get().prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM stream_messages INDEXED BY stream_messages_recipient
                WHERE recipient IN (SELECT value FROM json_each(?1)) AND store_index>?2 AND store_index<=?3)
             OR EXISTS(SELECT 1 FROM stream_messages INDEXED BY stream_messages_sender
                WHERE sender IN (SELECT value FROM json_each(?1)) AND store_index>?2 AND store_index<=?3)",
    )?.query_row(/* sqlite-large-value: allow-for-now lv-h9d7b465db685 owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ rusqlite::params![serde_json::to_string(names)?, after, through], |row| row.get(0))?)
}

/// The first of a message's two timeline sequences, the message entry's; its content entry takes
/// the next. A graph message's is its index times four. A stream item has no graph index, so it
/// takes two past the graph index it arrived at on this member: it sorts after the graph claim
/// it arrived behind and before the next. Items that arrived at the same graph index share
/// sequences, and their shown times differ (`stream_arrivals`), so each item's message and
/// content entries sort together. The sequence orders entries; it never identifies one.
pub fn timeline_base(graph: &super::super::Store, claim: &ClaimRecord) -> anyhow::Result<u64> {
    if claim.store_index < STREAM_INDEX_BASE {
        return Ok(claim.store_index.saturating_mul(4));
    }
    // Read only: the conversation read that brought the item recorded every arrival. An item it
    // did not record sorts at the graph's current index.
    let recorded = match shard(graph) {
        Some(shard) => recorded_arrival(&shard, &claim.subject)?,
        None => None,
    };
    let arrived = match recorded {
        Some(arrived) => arrived,
        None => graph.index()?,
    };
    Ok(arrived.saturating_mul(4).saturating_add(2))
}
