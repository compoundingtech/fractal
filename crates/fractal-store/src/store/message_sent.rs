//! Whether a message was already sent, wherever it went (db break-up step 4c-2a). A
//! `message.sent` is a legacy claim in the graph, a stream item in the messages shard, or a row
//! in the outbox that the daemon is about to write to the shard. A sender that asks only the
//! graph whether it already sent would send again once its first send went to the stream, so
//! every check before a send asks all three.
//!
//! [`Store::message_sent`] reads each store's own table. A sender inside a graph transaction
//! cannot read the shard consistently with it, so [`message_sent_tx`] reads the graph's mirror
//! of the shard (`local_stream_sent`) instead. The outbox drain mirrors an item before it
//! deletes the row, so a message this member sent is always in one of the three; only a peer's
//! item can be missed, until the mirror reaches it, as a peer's graph claim is until it
//! replicates.
//!
//! A send also checks at the write. A graph `message.sent` append asks the shard whether it
//! holds the send's idempotency key or subject, and a stream append asks the graph, so a send
//! retried down the other path replays what the first wrote instead of writing it again. The
//! check and the write are in different database files, so [`gated`] holds a lock for the
//! message's subject and key across both on either path; two sends of one message, one down
//! each path, take turns, and the second finds the first's write.

use super::*;

/// The graph's claims, the graph's mirror of the shard, and the outbox, each by its subject index.
pub(super) const SENT_IN_GRAPH_FILE: &str = "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='message.sent')
     OR EXISTS(SELECT 1 FROM local_stream_sent WHERE subject=?1)
     OR EXISTS(SELECT 1 FROM local_message_outbox WHERE subject=?1)";

impl Store {
    /// Whether `subject` was sent: the graph holds its `message.sent`, the messages shard holds
    /// it as a stream item, or it waits in the outbox.
    pub fn message_sent(&self, subject: &str) -> Result<bool> {
        if message_sent_tx(&self.readers.get(), subject)? {
            return Ok(true);
        }
        let Some(shard) = messages_shard::shard(self) else {
            return Ok(false);
        };
        Ok(shard
            .readers
            .get()
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h7a15ce6aed05 owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ SENT_IN_STREAM)?
            .query_row(/* sqlite-large-value: allow-for-now lv-hed39aca830cc owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ [subject], |row| row.get(0))?)
    }
}

/// [`Store::message_sent`] on a graph connection, such as a sender's own transaction: the
/// shard's items as the graph has mirrored them.
pub fn message_sent_tx(connection: &Connection, subject: &str) -> Result<bool> {
    Ok(connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h287e13415dd4 owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ SENT_IN_GRAPH_FILE)?.query_row(/* sqlite-large-value: allow-for-now lv-hb9533a9d1605 owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ [subject], |row| row.get(0))?)
}

/// The shard's item under a subject, by `stream_messages`' primary key.
pub(super) const SENT_IN_STREAM: &str = "SELECT EXISTS(SELECT 1 FROM stream_messages WHERE subject=?1)";

/// A stream item's shard index, by `stream_messages`' primary key.
pub(super) const STREAM_ITEM: &str = "SELECT store_index FROM stream_messages WHERE subject=?1";

/// The graph's operation under an idempotency key, by `operations`' primary key.
pub(super) const GRAPH_OPERATION: &str =
    "SELECT request_digest, canonical_claim_id FROM operations WHERE id=?1 AND state='active'";

/// The graph's first `message.sent` for a subject, by `claims_subject_kind_index`.
pub(super) fn graph_sent_claim() -> String {
    canonical_sql(&format!(
        "SELECT {CLAIM_COLUMNS} FROM claims WHERE subject=?1 AND kind='message.sent' ORDER BY CANONICAL_ASC(claims) LIMIT 1"
    ))
}

/// How many locks the send gate spreads subjects and keys over.
const GATE_STRIPES: usize = 64;

/// The send gate: one lock per stripe. Only the daemon writes either store, so a lock in the
/// process covers every writer.
pub struct SendGate(Vec<Mutex<()>>);

impl Default for SendGate {
    fn default() -> Self {
        Self((0..GATE_STRIPES).map(|_| Mutex::new(())).collect())
    }
}

/// Run `write`, the check of the other store and the append to this one, holding the gate's
/// stripes for `input`'s subject and idempotency key. Stripes are taken in index order, so two
/// sends never wait on each other's second stripe. The locks are not reentrant: `write` must
/// not send another message itself, or a send whose stripe it already holds waits forever.
pub fn gated<R>(graph: &Store, input: &ClaimInput, write: impl FnOnce() -> R) -> R {
    let gate = graph.extension::<SendGate>();
    let stripe = |value: &str| usize::from(Sha256::digest(value.as_bytes())[0]) % GATE_STRIPES;
    let mut stripes = vec![stripe(&input.subject)];
    if let Some(key) = &input.idempotency_key {
        stripes.push(stripe(&operation_id_for_key(key)));
    }
    stripes.sort_unstable();
    stripes.dedup();
    let _held = stripes
        .iter()
        .map(|stripe| gate.0[*stripe].lock().unwrap_or_else(PoisonError::into_inner))
        .collect::<Vec<_>>();
    write()
}

/// For a graph send: the stream item an earlier send of the same message wrote, by its
/// idempotency key (the shard's `stream_operations`, by primary key) or its subject (the
/// shard's `stream_messages`, by primary key). `idempotency-mismatch` when the key wrote a
/// different request there.
pub fn sent_to_stream(graph: &Store, input: &ClaimInput) -> Result<Option<ClaimRecord>, FractalError> {
    let Some(shard) = messages_shard::shard(graph) else {
        return Ok(None);
    };
    let connection = shard.readers.get();
    if let Some((operation_id, request_digest)) = smallclaims::store::claim_operation(input)?
        && let Some(claim) =
            messages_shard::stream_operations::replay_tx(&connection, &operation_id, &request_digest)?
    {
        return Ok(Some(claim));
    }
    let index: Option<i64> = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hb8663de1499d owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ STREAM_ITEM)
        .and_then(|mut statement| statement.query_row(/* sqlite-large-value: allow-for-now lv-h524975e1c242 owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ [&input.subject], |row| row.get(0)).optional())
        .map_err(internal)?;
    match index {
        Some(index) => connection
            .query_row(/* sqlite-large-value: allow-for-now lv-ha5d32086bf04 owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ &format!("SELECT {CLAIM_COLUMNS} FROM claims WHERE store_index=?1"), [index], claim_from_row)
            .optional()
            .map_err(internal),
        None => Ok(None),
    }
}

/// For a stream send: the graph claim an earlier send of the same message wrote, by its
/// idempotency key (the graph's `operations`, by primary key) or its subject
/// (`claims_subject_kind_index`). `idempotency-mismatch` when the key wrote a different request
/// there.
pub fn sent_to_graph(graph: &Store, input: &ClaimInput) -> Result<Option<ClaimRecord>, FractalError> {
    let connection = graph.readers.get();
    if let Some((operation_id, request_digest)) = smallclaims::store::claim_operation(input)? {
        let stored: Option<(String, String)> = connection
            .query_row(
                /* sqlite-large-value: allow-for-now lv-hfeb9391f4c21 owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ GRAPH_OPERATION,
                [&operation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(internal)?;
        if let Some((stored_digest, claim_id)) = stored {
            if stored_digest != request_digest {
                return Err(FractalError::new(
                    "idempotency-mismatch",
                    "the idempotency key already identifies a different request",
                )
                .with_detail("operation_id", operation_id)
                .with_detail("stored_digest", stored_digest)
                .with_detail("request_digest", request_digest));
            }
            return claim_by_id_tx(&connection, &claim_id).map_err(internal);
        }
    }
    connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-hf0d3cfe78aeb owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ &graph_sent_claim(),
            [&input.subject],
            claim_from_row,
        )
        .optional()
        .map_err(internal)
}

/// The index a page lists a sent claim at: a graph claim's own, or, for a stream item a send
/// replayed, its index past [`messages_shard::STREAM_INDEX_BASE`]. True for a stream item.
pub fn listed_index(graph: &Store, claim: &ClaimRecord) -> Result<(u64, bool)> {
    let in_graph: bool = graph
        .readers
        .get()
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h013343aee563 owner=fractal_store::store::message_sent TODO=trace-statement-and-value-representation */ [&claim.id], |row| row.get(0))?;
    Ok(if in_graph {
        (claim.store_index, false)
    } else {
        (messages_shard::STREAM_INDEX_BASE + claim.store_index, true)
    })
}
