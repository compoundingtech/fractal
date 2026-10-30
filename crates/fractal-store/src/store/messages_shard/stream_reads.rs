//! The messages stream as the graph's readers need it beside their own reads: how far the stream
//! has come, its message events, and where each stream item arrived in this member's graph order.

use super::*;

/// Where each stream item arrived in this member's graph order: a local table in the shard file,
/// outside the replicated log and every digest. One host-local order then sorts graph and stream
/// entries together. A shard trigger cannot see the graph's index, so the daemon writes it as it
/// admits items, and a reader that finds an item without one writes it then.
pub(super) const LOCAL_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h000000000000 owner=fractal_store::store::messages_shard::stream_reads TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS stream_arrivals (
    subject TEXT PRIMARY KEY,
    store_index INTEGER NOT NULL,
    graph_index INTEGER NOT NULL,
    arrived_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS stream_arrivals_store_index ON stream_arrivals(store_index);
";

/// The stream's store index on this member, or `None` when it holds no messages shard.
pub fn stream_index(graph: &super::super::Store) -> anyhow::Result<Option<u64>> {
    shard(graph).map(|shard| shard.index()).transpose()
}

/// Record, at the graph's current index, every stream item that has no arrival yet. The daemon
/// calls it after the shard admits items; a reader that finds an item without one calls it too.
/// Returns how many items it recorded.
pub fn record_arrivals(graph: &super::super::Store) -> anyhow::Result<usize> {
    let Some(shard) = shard(graph) else {
        return Ok(0);
    };
    let at = graph.index()?;
    let connection = shard.connection.write();
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-h000000000000 owner=fractal_store::store::messages_shard::stream_reads TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO stream_arrivals(subject, store_index, graph_index, arrived_at_unix_ms)
         SELECT subject, store_index, ?1, ?2 FROM stream_messages
         WHERE store_index > COALESCE((SELECT MAX(store_index) FROM stream_arrivals), 0)",
        rusqlite::params![at as i64, smallclaims::store::now_ms() as i64],
    )?)
}

/// The graph index at which `subject`, a stream item, arrived on this member.
pub fn arrival(graph: &super::super::Store, subject: &str) -> anyhow::Result<Option<u64>> {
    let Some(shard) = shard(graph) else {
        return Ok(None);
    };
    let read = |shard: &GraphStore| -> anyhow::Result<Option<u64>> {
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
    };
    if let Some(index) = read(&shard)? {
        return Ok(Some(index));
    }
    if stream_item(&shard, subject)?.is_none() {
        return Ok(None);
    }
    record_arrivals(graph)?;
    read(&shard)
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
            /* sqlite-large-value: allow-for-now lv-h000000000000 owner=fractal_store::store::messages_shard::stream_reads TODO=trace-statement-and-value-representation */ "SELECT store_index, kind, subject, body FROM claims
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
